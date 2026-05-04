/*{
    "DESCRIPTION": "Turing pattern - brain coral reaction-diffusion (Gray-Scott model)",
    "CREDIT": "Varda VJ - Based on Karl Sims tutorial",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {
            "NAME": "reset",
            "LABEL": "Reset",
            "TYPE": "bool",
            "DEFAULT": false
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
            "MAX": 0.2
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
    float reset;
    float feed_rate;
    float kill_rate;
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

        // Sample previous state
        vec4 prev = texture(sampler2D(rdBuffer, texSampler), uv);

        // Check if we need to initialize:
        // - Buffer uninitialized (alpha = 0)
        // - First few frames
        // - Reset button pressed
        bool needsInit = (prev.a < 0.5) || (FRAMEINDEX < 2u) || (reset > 0.5);

        if (needsInit) {
            // Initialize: U=1 everywhere, V=0 everywhere
            float u = 1.0;
            float v = 0.0;

            // Seed: small circle in center with V=1
            float dist = length(uv - 0.5);
            if (dist < seed_size) {
                v = 1.0;
            }

            // Add random noise spots to break symmetry
            float n = hash12(uv * 237.0 + vec2(TIME * 0.1, 0.0));
            if (dist < seed_size * 2.5 && dist > seed_size && n > 0.85) {
                v = 1.0;
            }

            fragColor = vec4(u, v, 0.0, 1.0);
            return;
        }

        // Sample all 8 neighbors for 9-point Laplacian stencil
        vec4 c  = prev;
        vec4 n  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(0.0, texel.y));
        vec4 s  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(0.0, -texel.y));
        vec4 e  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(texel.x, 0.0));
        vec4 w  = texture(sampler2D(rdBuffer, texSampler), uv + vec2(-texel.x, 0.0));
        vec4 ne = texture(sampler2D(rdBuffer, texSampler), uv + vec2(texel.x, texel.y));
        vec4 nw = texture(sampler2D(rdBuffer, texSampler), uv + vec2(-texel.x, texel.y));
        vec4 se = texture(sampler2D(rdBuffer, texSampler), uv + vec2(texel.x, -texel.y));
        vec4 sw = texture(sampler2D(rdBuffer, texSampler), uv + vec2(-texel.x, -texel.y));

        float u = c.r;
        float v = c.g;

        // Karl Sims 9-point weighted Laplacian:
        // adjacent = 0.2, diagonal = 0.05, center = -1
        float lapU = 0.2 * (n.r + s.r + e.r + w.r)
                   + 0.05 * (ne.r + nw.r + se.r + sw.r)
                   - 1.0 * u;
        float lapV = 0.2 * (n.g + s.g + e.g + w.g)
                   + 0.05 * (ne.g + nw.g + se.g + sw.g)
                   - 1.0 * v;

        // Gray-Scott parameters from sliders
        float F = feed_rate;
        float k = kill_rate;

        // Standard diffusion rates
        float Da = 1.0;
        float Db = 0.5;
        float dt = 1.0;

        // Reaction term: U + 2V -> 3V
        float uvv = u * v * v;

        // Gray-Scott update equations
        float newU = u + dt * (Da * lapU - uvv + F * (1.0 - u));
        float newV = v + dt * (Db * lapV + uvv - (F + k) * v);

        // Add perturbation to keep the system moving
        if (perturbation > 0.001) {
            // Use 3D hash with frame index for true randomness per pixel per frame
            float rand1 = hash13(vec3(uv * 500.0, float(FRAMEINDEX) * 0.1));
            float rand2 = hash13(vec3(uv * 500.0 + vec2(100.0, 50.0), float(FRAMEINDEX) * 0.1 + 10.0));

            // Scale perturbation strength
            float perturbStrength = perturbation * 0.02;

            // Randomly inject V chemical at sparse random locations
            // Higher perturbation = more frequent injections
            float threshold = 0.998 - perturbation * 0.02;
            if (rand1 > threshold) {
                newV += perturbStrength * 5.0;
            }

            // Add subtle random noise to destabilize edges
            if (v > 0.05 && v < 0.95) {
                newU += (rand1 - 0.5) * perturbStrength * 0.5;
                newV += (rand2 - 0.5) * perturbStrength;
            }
        }

        // Store in buffer (alpha=1 marks as initialized)
        fragColor = vec4(clamp(newU, 0.0, 1.0), clamp(newV, 0.0, 1.0), 0.0, 1.0);

    } else {
        // === RENDER PASS ===
        vec4 state = texture(sampler2D(rdBuffer, texSampler), uv);

        // V is the pattern value (0 = background, 1 = growth)
        float pattern = state.g;

        // Apply slight contrast enhancement
        pattern = smoothstep(0.1, 0.9, pattern);

        // Mix between background and growth colors based on pattern
        // Support transparent colors by mixing alpha too
        vec3 color = mix(bg_color.rgb, growth_color.rgb, pattern);
        float alpha = mix(bg_color.a, growth_color.a, pattern);

        fragColor = vec4(color, alpha);
    }
}

