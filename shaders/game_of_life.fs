/*{
    "DESCRIPTION": "Cellular Automata - procedural Game of Life-style patterns",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "grid_size", "TYPE": "float", "DEFAULT": 64.0, "MIN": 16.0, "MAX": 256.0, "LABEL": "Grid Size"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "rule_threshold", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 0.9, "LABEL": "Density"},
        {"NAME": "pattern_scale", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Pattern Scale"},
        {"NAME": "color_alive", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.5, 1.0], "LABEL": "Alive Color"},
        {"NAME": "color_dead", "TYPE": "color", "DEFAULT": [0.0, 0.05, 0.02, 1.0], "LABEL": "Dead Color"},
        {"NAME": "glow", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Glow"}
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

layout(set = 0, binding = 1) uniform UserParams {
    float grid_size;
    float anim_speed;
    float rule_threshold;
    float pattern_scale;
    vec4 color_alive;
    vec4 color_dead;
    float glow;
};

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

// Simulate cell state using deterministic noise seeded by time step
float cellState(vec2 cell, float gen) {
    // Use noise patterns that create automata-like structures
    vec2 p = cell / pattern_scale;
    float n1 = hash(cell + gen * 0.1);
    float n2 = hash(p * 1.7 + gen * 0.13);
    float n3 = hash(p * 3.1 + gen * 0.07);

    // Count "neighbors" via noise sampling
    float neighbors = 0.0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            if (x == 0 && y == 0) continue;
            vec2 nc = cell + vec2(float(x), float(y));
            float nh = hash(nc + (gen - 1.0) * 0.1);
            float nstate = hash(nc / pattern_scale * 1.7 + (gen - 1.0) * 0.13);
            neighbors += step(rule_threshold, (nh + nstate) * 0.5);
        }
    }

    // Game of Life-ish rules
    float wasAlive = step(rule_threshold, (n1 + n2) * 0.5);
    float alive = 0.0;
    if (wasAlive > 0.5) {
        alive = (neighbors >= 2.0 && neighbors <= 3.0) ? 1.0 : 0.0;
    } else {
        alive = (neighbors >= 2.5 && neighbors <= 3.5) ? 1.0 : 0.0;
    }
    // Blend with noise for visual interest
    return mix(alive, step(rule_threshold, n3), 0.2);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 cell = floor(uv * grid_size);
    float gen = floor(TIME * anim_speed * 3.0);

    float state = cellState(cell, gen);

    // Smooth within cell for glow effect
    vec2 f = fract(uv * grid_size);
    float cellGlow = state * (1.0 - glow + glow * smoothstep(0.5, 0.1, length(f - 0.5)));

    vec3 col = mix(color_dead.rgb, color_alive.rgb, cellGlow);
    fragColor = vec4(col, 1.0);
}
