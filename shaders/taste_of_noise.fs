/*{
    "DESCRIPTION": "Taste Of Noise 7 - organic fractal structures using iterative folding and smooth blending, with temporal accumulation feedback",
    "CREDIT": "Varda VJ (ported from leon's 'Taste Of Noise 7', https://www.shadertoy.com/view/NddSWs)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed",      "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0,  "LABEL": "Speed"},
        {"NAME": "fold_start", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 10.0, "LABEL": "Fold Start"},
        {"NAME": "falloff",    "TYPE": "float", "DEFAULT": 1.9, "MIN": 1.1, "MAX": 5.0,  "LABEL": "Falloff"},
        {"NAME": "fold_count", "TYPE": "float", "DEFAULT": 4.0, "MIN": 1.0, "MAX": 10.0, "LABEL": "Fold Count"},
        {"NAME": "grid_size",  "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 20.0, "LABEL": "Grid Size"},
        {"NAME": "trail_decay","TYPE": "float", "DEFAULT": 0.01,"MIN": 0.001,"MAX": 0.1,  "LABEL": "Trail Decay"}
    ],
    "PASSES": [
        {"TARGET": "bufferA", "PERSISTENT": true, "FLOAT": true},
        {}
    ],
    "PHASE_INPUTS": [{"PARAM": "speed", "INDEX": 0, "SCALE": 1.0}]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(std140, set = 0, binding = 0) uniform ISFUniforms {
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
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D bufferA;

layout(std140, set = 0, binding = 3) uniform UserParams {
    float speed;
    float fold_start;
    float falloff;
    float fold_count;
    float grid_size;
    float trail_decay;
};

// --- Hash functions (Dave Hoskins) ---
float hash13(vec3 p3) {
    p3 = fract(p3 * 0.1031);
    p3 += dot(p3, p3.zyx + 31.32);
    return fract((p3.x + p3.y) * p3.z);
}

// --- SDF utilities (Inigo Quilez) ---
float smin(float d1, float d2, float k) {
    float h = clamp(0.5 + 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) - k * h * (1.0 - h);
}

float smoothBlend(float d1, float d2, float k) {
    return clamp(0.5 + 0.5 * (d2 - d1) / k, 0.0, 1.0);
}

mat2 rot(float a) {
    float c = cos(a), s = sin(a);
    return mat2(c, -s, s, c);
}

// --- SDF scene ---
float map(vec3 p, float rng, float t, out float material) {
    float grid = grid_size;
    vec3 cell = floor(p / grid);
    p = mod(p, grid) - grid * 0.5;

    float dp = length(p);
    vec3 angle = vec3(0.1, -0.5, 0.1) + dp * 0.5 + p * 0.1 + cell;
    float size = sin(rng * 3.14159);
    float wave = sin(-dp + t + hash13(cell) * 6.28318) * 0.5;

    int count = int(fold_count);
    float a = fold_start;
    float scene = 1000.0;
    float shape = 1000.0;
    material = 0.0;

    for (int i = 0; i < 10; ++i) {
        if (i >= count) break;
        p.xz = abs(p.xz) - (0.5 + wave) * a;
        p.xz = p.xz * rot(angle.y / a);
        p.yz = p.yz * rot(angle.x / a);
        p.yx = p.yx * rot(angle.z / a);
        shape = length(p) - 0.2 * a * size;
        material = mix(material, float(i), smoothBlend(shape, scene, 0.3 * a));
        scene = smin(scene, shape, 1.0 * a);
        a /= falloff;
    }
    return scene;
}

void main() {
    // Uniform keeper to prevent SPIRV stripping
    float keep = (audio_level + audio_bass + audio_mid + audio_treble
                + audio_bpm + audio_beat_phase) * 0.000001
                + (TIMEDELTA + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w
                + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3) * 0.000001;

    if (PASSINDEX == 0) {
        // === Buffer A: raymarching + temporal accumulation ===
        fragColor = vec4(0.0, 0.0, 0.0, 1.0);

        vec2 fragCoord = uv * RENDERSIZE;
        vec2 coord = (fragCoord - RENDERSIZE * 0.5) / RENDERSIZE.y;

        // Camera
        vec3 eye = vec3(1.0, 1.0, 1.0);
        vec3 at  = vec3(0.0);
        vec3 zDir = normalize(at - eye);
        vec3 xDir = normalize(cross(zDir, vec3(0.0, 1.0, 0.0)));
        vec3 yDir = normalize(cross(xDir, zDir));
        vec3 ray = normalize(zDir + coord.x * xDir + coord.y * yDir);
        vec3 pos = eye;

        float t = PHASE_TIME_0;
        vec3 seed = vec3(fragCoord, t);
        float rng = hash13(seed);

        // Raymarch
        float mat_out = 0.0;
        for (int i = 20; i > 0; --i) {
            float dist = map(pos, rng, t, mat_out);
            if (dist < 0.01) {
                float shade = float(i) / 20.0;

                // Normal via central differences
                vec2 off = vec2(0.001, 0.0);
                float dummy;
                float d0 = map(pos, rng, t, dummy);
                vec3 normal = normalize(d0 - vec3(
                    map(pos - off.xyy, rng, t, dummy),
                    map(pos - off.yxy, rng, t, dummy),
                    map(pos - off.yyx, rng, t, dummy)
                ));

                // Iq color palette
                vec3 tint = 0.5 + 0.5 * cos(vec3(3.0, 2.0, 1.0) + mat_out * 0.5 + length(pos) * 0.5);

                // Lighting
                float ld = dot(reflect(ray, normal), vec3(0.0, 1.0, 0.0)) * 0.5 + 0.5;
                vec3 light = vec3(1.0, 0.502, 0.502) * pow(ld, 0.5);
                ld = dot(reflect(ray, normal), vec3(0.0, 0.0, -1.0)) * 0.5 + 0.5;
                light += vec3(0.4, 0.714, 0.145) * pow(ld, 0.5) * 0.5;

                fragColor = vec4((tint + light) * pow(shade, 1.0) + keep, 1.0);
                break;
            }
            dist *= 0.9 + 0.1 * rng;
            pos += ray * dist;
        }

        // Temporal accumulation: keep max of current and decayed previous
        vec4 prev = texture(sampler2D(bufferA, texSampler), uv);
        fragColor = max(fragColor, prev - trail_decay);

    } else {
        // === Final pass: read buffer and output ===
        vec4 col = texture(sampler2D(bufferA, texSampler), uv);
        col.rgb = pow(max(col.rgb, vec3(0.0)), vec3(0.95));
        fragColor = vec4(col.rgb + keep, col.a);
    }
}