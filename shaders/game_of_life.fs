/*{
    "DESCRIPTION": "Conway's Game of Life - real cellular automaton with persistent state",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "grid_size", "TYPE": "float", "DEFAULT": 128.0, "MIN": 32.0, "MAX": 512.0, "LABEL": "Grid Size"},
        {"NAME": "sim_speed", "TYPE": "float", "DEFAULT": 10.0, "MIN": 1.0, "MAX": 60.0, "LABEL": "Generations/Sec"},
        {"NAME": "seed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Seed (change to reseed)"},
        {"NAME": "density", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.05, "MAX": 0.8, "LABEL": "Initial Density"},
        {"NAME": "color_alive", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.5, 1.0], "LABEL": "Alive Color"},
        {"NAME": "color_dead", "TYPE": "color", "DEFAULT": [0.0, 0.05, 0.02, 1.0], "LABEL": "Dead Color"},
        {"NAME": "trail_fade", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.98, "LABEL": "Trail Fade"},
        {"NAME": "glow", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Cell Glow"}
    ],
    "PASSES": [
        {
            "TARGET": "golBuffer",
            "PERSISTENT": true,
            "FLOAT": true
        },
        {}
    ]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4 DATE;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D golBuffer;

layout(set = 0, binding = 3) uniform UserParams {
    float grid_size;
    float sim_speed;
    float seed;
    float density;
    vec4 color_alive;
    vec4 color_dead;
    float trail_fade;
    float glow;
};

// Hash for random initial state
float hash(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Sample the buffer at the CENTER of a grid cell.
// This avoids bilinear blending at cell boundaries.
vec2 cellCenterUV(vec2 cellCoord) {
    // Wrap toroidally, then offset to cell center (+0.5)
    vec2 wrapped = mod(cellCoord, grid_size);
    return (wrapped + 0.5) / grid_size;
}

// Read cell state at a grid position (wrapping at edges)
float readCell(vec2 cellCoord) {
    vec4 val = texture(sampler2D(golBuffer, texSampler), cellCenterUV(cellCoord));
    return step(0.5, val.r);
}

void main() {
    // Uniform guard
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    if (PASSINDEX == 0) {
        // === SIMULATION PASS — writes to golBuffer ===

        // All pixels in the same cell must behave identically.
        // Snap to cell coordinate first, then sample from cell center.
        vec2 cellCoord = floor(uv * grid_size);
        vec2 centerUV = cellCenterUV(cellCoord);
        vec4 prev = texture(sampler2D(golBuffer, texSampler), centerUV);

        // Detect reseed: seed value stored in .b channel
        float prevSeed = prev.b;
        bool needsInit = (prev.a < 0.5) || (FRAMEINDEX < 2u)
                      || (abs(prevSeed - seed) > 0.001);

        if (needsInit) {
            // Random initial state based on seed + cell position
            float h = hash(cellCoord + seed * 137.0 + 42.0);
            float alive = step(1.0 - density, h);
            fragColor = vec4(alive, 0.0, seed, 1.0);
            return;
        }

        // Rate-limit: use FRAMEINDEX to step at fixed intervals.
        // This is frame-count based so every pixel in the grid steps
        // on the exact same frame — no per-pixel drift.
        uint stepInterval = max(1u, uint(60.0 / sim_speed));
        bool shouldStep = (FRAMEINDEX % stepInterval) == 0u;

        if (!shouldStep) {
            // Not time for a new generation — pass through unchanged
            fragColor = prev;
            return;
        }

        // Count live neighbors (Moore neighborhood, toroidal wrap)
        float neighbors = 0.0;
        neighbors += readCell(cellCoord + vec2(-1.0, -1.0));
        neighbors += readCell(cellCoord + vec2( 0.0, -1.0));
        neighbors += readCell(cellCoord + vec2( 1.0, -1.0));
        neighbors += readCell(cellCoord + vec2(-1.0,  0.0));
        neighbors += readCell(cellCoord + vec2( 1.0,  0.0));
        neighbors += readCell(cellCoord + vec2(-1.0,  1.0));
        neighbors += readCell(cellCoord + vec2( 0.0,  1.0));
        neighbors += readCell(cellCoord + vec2( 1.0,  1.0));

        float wasAlive = step(0.5, prev.r);

        // Conway's rules exactly:
        //   Alive + 2 or 3 neighbors → stays alive
        //   Dead  + exactly 3 neighbors → becomes alive
        //   Everything else → dead
        float alive = 0.0;
        if (wasAlive > 0.5) {
            // Survival: 2 or 3 neighbors
            if (neighbors > 1.5 && neighbors < 3.5) {
                alive = 1.0;
            }
        } else {
            // Birth: exactly 3 neighbors
            if (neighbors > 2.5 && neighbors < 3.5) {
                alive = 1.0;
            }
        }

        fragColor = vec4(alive, 0.0, seed, 1.0);

    } else {
        // === RENDER PASS — reads golBuffer, outputs visible pixels ===

        // Map pixel to grid cell
        vec2 cellCoord = floor(uv * grid_size);
        vec2 cellUV = fract(uv * grid_size);

        // Read current cell state from cell center
        vec4 state = texture(sampler2D(golBuffer, texSampler), cellCenterUV(cellCoord));
        float alive = step(0.5, state.r);

        // Cell glow: soft circle inside each cell
        float dist = length(cellUV - 0.5);
        float cellShape = 1.0 - glow + glow * smoothstep(0.5, 0.1, dist);

        // Trail effect: blend with neighbors for soft fade
        float trailVal = 0.0;
        if (trail_fade > 0.01) {
            for (int y = -1; y <= 1; y++) {
                for (int x = -1; x <= 1; x++) {
                    float ns = texture(sampler2D(golBuffer, texSampler),
                        cellCenterUV(cellCoord + vec2(float(x), float(y)))).r;
                    trailVal += ns;
                }
            }
            trailVal /= 9.0;
        }

        float intensity = max(alive * cellShape, trailVal * trail_fade);
        vec3 col = mix(color_dead.rgb, color_alive.rgb, clamp(intensity, 0.0, 1.0));
        fragColor = vec4(col, 1.0);
    }
}
