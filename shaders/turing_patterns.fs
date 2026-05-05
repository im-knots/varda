/*{
    "DESCRIPTION": "Turing pattern - brain coral reaction-diffusion (Gray-Scott model)",
    "CREDIT": "Varda VJ - Based on Karl Sims tutorial",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {
            "NAME": "seed",
            "LABEL": "Seed (change to reseed)",
            "TYPE": "float",
            "DEFAULT": 0.0,
            "MIN": 0.0,
            "MAX": 1.0
        },
        {
            "NAME": "feed_rate",
            "LABEL": "Feed Rate (F)",
            "TYPE": "float",
            "DEFAULT": 0.055,
            "MIN": 0.01,
            "MAX": 0.1
        },
        {
            "NAME": "kill_rate",
            "LABEL": "Kill Rate (k)",
            "TYPE": "float",
            "DEFAULT": 0.062,
            "MIN": 0.045,
            "MAX": 0.07
        },
        {
            "NAME": "sim_speed",
            "LABEL": "Speed",
            "TYPE": "float",
            "DEFAULT": 1.0,
            "MIN": 0.1,
            "MAX": 3.0
        },
        {
            "NAME": "perturbation",
            "LABEL": "Perturbation",
            "TYPE": "float",
            "DEFAULT": 0.0,
            "MIN": 0.0,
            "MAX": 1.0
        },
        {
            "NAME": "seed_size",
            "LABEL": "Seed Size",
            "TYPE": "float",
            "DEFAULT": 0.05,
            "MIN": 0.01,
            "MAX": 0.3
        },
        {
            "NAME": "bg_color",
            "LABEL": "Background Color",
            "TYPE": "color",
            "DEFAULT": [1.0, 1.0, 1.0, 0.0]
        },
        {
            "NAME": "growth_color",
            "LABEL": "Growth Color",
            "TYPE": "color",
            "DEFAULT": [0.0, 0.0, 0.0, 1.0]
        }
    ],
    "PASSES": [
        {
            "TARGET": "rdBuffer",
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
layout(set = 0, binding = 2) uniform texture2D rdBuffer;

layout(set = 0, binding = 3) uniform UserParams {
    float seed;
    float feed_rate;
    float kill_rate;
    float sim_speed;
    float perturbation;
    float seed_size;
    vec4 bg_color;
    vec4 growth_color;
};

// High quality hash functions for perturbation
float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float hash13(vec3 p3) {
    p3 = fract(p3 * 0.1031);
    p3 += dot(p3, p3.zyx + 31.32);
    return fract((p3.x + p3.y) * p3.z);
}

void main() {
    vec2 texel = 1.0 / RENDERSIZE;

    if (PASSINDEX == 0) {
        // === SIMULATION PASS ===

        vec4 prev = texture(sampler2D(rdBuffer, texSampler), uv);

        // Detect reseed: store seed value in .b channel.
        // When seed slider changes, prev.b != seed → reinitialize.
        // Also init on first frames or uninitialized buffer (alpha=0).
        float prevSeed = prev.b;
        bool needsInit = (prev.a < 0.5) || (FRAMEINDEX < 2u)
                      || (abs(prevSeed - seed) > 0.001);

        if (needsInit) {
            float u = 1.0;
            float v = 0.0;

            // Use seed value for unique random patterns
            float seedHash = seed * 1000.0;

            // Central seed circle
            float dist = length(uv - 0.5);
            if (dist < seed_size) {
                v = 1.0;
            }

            // Random spots around seed to break symmetry
            float n = hash12(uv * 237.0 + vec2(seedHash, seedHash * 0.7));
            if (dist < seed_size * 3.0 && n > 0.8) {
                v = 1.0;
            }

            // Scattered seeds for variety
            float scatter = hash12(uv * 97.0 + vec2(seedHash * 1.3, seedHash * 0.4));
            if (scatter > 0.995) {
                v = 1.0;
            }

            // Store seed value in .b for change detection
            fragColor = vec4(u, v, seed, 1.0);
            return;
        }

        // --- Stable Gray-Scott simulation ---
        vec4 c  = prev;
        vec4 nb_n  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(0.0, texel.y));
        vec4 nb_s  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(0.0, -texel.y));
        vec4 nb_e  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(texel.x, 0.0));
        vec4 nb_w  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(-texel.x, 0.0));
        vec4 nb_ne = texture(sampler2D(rdBuffer, texSampler), uv + vec2(texel.x, texel.y));
        vec4 nb_nw = texture(sampler2D(rdBuffer, texSampler), uv + vec2(-texel.x, texel.y));
        vec4 nb_se = texture(sampler2D(rdBuffer, texSampler), uv + vec2(texel.x, -texel.y));
        vec4 nb_sw = texture(sampler2D(rdBuffer, texSampler), uv + vec2(-texel.x, -texel.y));

        // Clamp inputs to prevent NaN propagation from blown-up state
        float u = clamp(c.r, 0.0, 1.0);
        float v = clamp(c.g, 0.0, 1.0);

        // Karl Sims 9-point weighted Laplacian
        float lapU = 0.2 * (nb_n.r + nb_s.r + nb_e.r + nb_w.r)
                   + 0.05 * (nb_ne.r + nb_nw.r + nb_se.r + nb_sw.r)
                   - 1.0 * u;
        float lapV = 0.2 * (nb_n.g + nb_s.g + nb_e.g + nb_w.g)
                   + 0.05 * (nb_ne.g + nb_nw.g + nb_se.g + nb_sw.g)
                   - 1.0 * v;

        float F = feed_rate;
        float k = kill_rate;
        float Da = 1.0;
        float Db = 0.5;

        // Substep integration: split sim_speed into small stable steps.
        // Each substep uses dt <= 0.5 to stay within CFL stability limit.
        int steps = int(clamp(sim_speed * 2.0, 1.0, 6.0));
        float dt = sim_speed / float(steps);

        for (int i = 0; i < steps; i++) {
            float uvv = u * v * v;
            u += dt * (Da * lapU - uvv + F * (1.0 - u));
            v += dt * (Db * lapV + uvv - (F + k) * v);
            u = clamp(u, 0.0, 1.0);
            v = clamp(v, 0.0, 1.0);
        }

        // Perturbation: sparse V injection to seed new growth
        if (perturbation > 0.001) {
            float rand1 = hash13(vec3(uv * 500.0, float(FRAMEINDEX) * 0.1));
            float perturbStrength = perturbation * 0.02;
            float threshold = 0.998 - perturbation * 0.02;
            if (rand1 > threshold) {
                v = clamp(v + perturbStrength * 5.0, 0.0, 1.0);
            }
        }

        // Preserve seed in .b for change detection
        fragColor = vec4(u, v, seed, 1.0);

    } else {
        // === RENDER PASS ===
        vec4 state = texture(sampler2D(rdBuffer, texSampler), uv);
        float pattern = smoothstep(0.1, 0.9, state.g);

        vec3 color = mix(bg_color.rgb, growth_color.rgb, pattern);
        float alpha = mix(bg_color.a, growth_color.a, pattern);

        fragColor = vec4(color, alpha);
    }
}

