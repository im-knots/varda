/*{
    "DESCRIPTION": "Point Cloud - reprojects the input image into a pseudo-3D cloud of soft splats (brightness = depth) with parallax orbit, atmospheric depth fade, color modes, and a live motion-reactive disturbance field (wave a hand on camera and the points scatter and recolor, Kinect/TouchDesigner style)",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize", "3D"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "density", "LABEL": "Density", "TYPE": "float", "DEFAULT": 110.0, "MIN": 16.0, "MAX": 240.0},
        {"NAME": "point_size", "LABEL": "Point Size", "TYPE": "float", "DEFAULT": 0.55, "MIN": 0.05, "MAX": 1.5},
        {"NAME": "depth", "LABEL": "Depth", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "orbit", "LABEL": "Orbit", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "rot_speed", "LABEL": "Orbit Speed", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "tilt_x", "LABEL": "Tilt X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "tilt_y", "LABEL": "Tilt Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "perspective", "LABEL": "Perspective", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.5},
        {"NAME": "glow", "LABEL": "Glow", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "brightness", "LABEL": "Brightness", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "depth_fade", "LABEL": "Depth Fade", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "jitter", "LABEL": "Jitter", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "motion_sens", "LABEL": "Motion Sensitivity", "TYPE": "float", "DEFAULT": 0.08, "MIN": 0.01, "MAX": 0.4},
        {"NAME": "disturb_force", "LABEL": "Disturb Force", "TYPE": "float", "DEFAULT": 0.08, "MIN": 0.0, "MAX": 0.4},
        {"NAME": "diffuse", "LABEL": "Disturb Spread", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "decay", "LABEL": "Disturb Linger", "TYPE": "float", "DEFAULT": 0.94, "MIN": 0.5, "MAX": 0.995},
        {"NAME": "spread", "LABEL": "Diffuse Radius", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "color_mode", "LABEL": "Color Mode", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2, 3], "LABELS": ["Source", "Depth", "Thermal", "Mono"]},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"},
        {"NAME": "disturb_color", "TYPE": "color", "DEFAULT": [1.0, 0.25, 0.55, 1.0], "LABEL": "Disturb Color"}
    ],
    "PASSES": [
        {"TARGET": "flowField", "PERSISTENT": true, "FLOAT": true},
        {"TARGET": "prevInput", "PERSISTENT": true}
    ],
    "PHASE_INPUTS": [{"PARAM": "rot_speed", "INDEX": 0}]
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
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
// Pass buffers (declaration order): flowField (charge field) then prevInput.
layout(set = 0, binding = 3) uniform texture2D flowField;
layout(set = 0, binding = 4) uniform texture2D prevInput;

layout(set = 0, binding = 5) uniform UserParams {
    float density;
    float point_size;
    float depth;
    float orbit;
    float rot_speed;
    float tilt_x;
    float tilt_y;
    float perspective;
    float glow;
    float brightness;
    float depth_fade;
    float jitter;
    float motion_sens;
    float disturb_force;
    float diffuse;
    float decay;
    float spread;
    int color_mode;
    vec4 bg_color;
    vec4 disturb_color;
};

#define TAU 6.28318530718
#define MAXR 8

float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }

float hash21(vec2 p) {
    p = fract(p * vec2(123.34, 456.21));
    p += dot(p, p + 45.32);
    return fract(p.x * p.y);
}

// Cool-to-warm depth ramp (deep blue -> cyan -> magenta -> white).
vec3 depth_ramp(float t) {
    vec3 col = mix(vec3(0.04, 0.10, 0.42), vec3(0.10, 0.80, 0.92), smoothstep(0.0, 0.4, t));
    col = mix(col, vec3(1.0, 0.30, 0.70), smoothstep(0.35, 0.72, t));
    col = mix(col, vec3(1.0, 1.0, 0.92), smoothstep(0.72, 1.0, t));
    return col;
}

// Infrared thermal ramp (black -> red -> orange -> yellow -> white).
vec3 thermal_ramp(float t) {
    return clamp(vec3(1.6 * t, 1.7 * t - 0.6, 3.5 * t - 2.6), 0.0, 1.0);
}

vec3 point_color(int mode, vec3 src, float t) {
    if (mode == 1) return depth_ramp(t);
    if (mode == 2) return thermal_ramp(t);
    if (mode == 3) return vec3(t);
    return src;
}

// Disturbance charge stored in the flowField pass buffer (R channel).
float sample_charge(vec2 p) {
    return texture(sampler2D(flowField, texSampler), p).x;
}

void main() {
    vec2 texel = 1.0 / RENDERSIZE;

    // Uniform guard — keep the ISF/audio uniforms live for binding consistency.
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIME + TIMEDELTA + float(FRAMEINDEX)
        + DATE.x + DATE.y + DATE.z + DATE.w
        + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    float keep = (audioSum + timeSum) * 1e-8;

    // ── Pass 0: flow field — diffuse + decay the charge, inject new motion ──
    if (PASSINDEX == 0) {
        float prevCharge = sample_charge(uv);
        float sp = 1.0 + spread * 6.0;
        float c0 = sample_charge(uv + vec2(texel.x * sp, 0.0));
        float c1 = sample_charge(uv - vec2(texel.x * sp, 0.0));
        float c2 = sample_charge(uv + vec2(0.0, texel.y * sp));
        float c3 = sample_charge(uv - vec2(0.0, texel.y * sp));
        float neighborAvg = (c0 + c1 + c2 + c3) * 0.25;
        float diffused = mix(prevCharge, neighborAvg, clamp(diffuse, 0.0, 1.0));

        // Frame-difference against last frame's stored input = live motion.
        vec3 cur = texture(sampler2D(inputImage, texSampler), uv).rgb;
        vec3 prv = texture(sampler2D(prevInput, texSampler), uv).rgb;
        float motion = length(cur - prv);
        float inject = smoothstep(motion_sens * 0.5, motion_sens, motion);

        float newCharge = max(diffused * clamp(decay, 0.0, 1.0), inject);
        fragColor = vec4(newCharge + keep, 0.0, 0.0, 1.0);
        return;
    }

    // ── Pass 1: store current input for next frame's motion diff ───────────
    if (PASSINDEX == 1) {
        vec3 cur = texture(sampler2D(inputImage, texSampler), uv).rgb;
        fragColor = vec4(cur + keep, 1.0);
        return;
    }

    // ── Final pass: render the motion-reactive point cloud ─────────────────
    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    // Isotropic centered "screen" space: y in [-0.5,0.5], x scaled by aspect.
    vec2 q = vec2((uv.x - 0.5) * aspect, uv.y - 0.5);

    float dens = max(density, 4.0);
    float g = 1.0 / dens; // square cell spacing in screen space

    // Parallax view direction: orbit (auto via PHASE_TIME_0) plus manual tilt.
    float phase = PHASE_TIME_0 * TAU;
    vec2 vdir = orbit * vec2(cos(phase), sin(phase)) + vec2(tilt_x, tilt_y);

    // Bounded gather window: depth shear + jitter + disturbance displacement.
    float dmax = 0.5 * depth * length(vdir) + g * (1.0 + jitter) + disturb_force;
    int R = int(clamp(ceil(dmax / g) + 1.0, 1.0, float(MAXR)));

    vec2 ci = floor(q / g + 0.5); // nearest cell index to this pixel

    // Depth-weighted color blend so the frontmost splats dominate, plus a
    // soft coverage alpha and an additive glow halo.
    vec3 colAccum = vec3(0.0);
    float wAccum = 0.0;
    float cover = 0.0;
    vec3 glowAccum = vec3(0.0);

    for (int dy = -MAXR; dy <= MAXR; dy++) {
        if (abs(dy) > R) continue;
        for (int dx = -MAXR; dx <= MAXR; dx++) {
            if (abs(dx) > R) continue;

            vec2 cell = ci + vec2(float(dx), float(dy));
            vec2 c = cell * g;
            vec2 sampleUV = vec2(c.x / aspect, c.y) + 0.5;
            if (sampleUV.x < 0.0 || sampleUV.x > 1.0 || sampleUV.y < 0.0 || sampleUV.y > 1.0) continue;

            vec4 src = texture(sampler2D(inputImage, texSampler), sampleUV);
            float l = luma(src.rgb);

            // Local disturbance: charge magnitude and its gradient. Points are
            // pushed down the gradient so they flee high-motion regions.
            float ch = sample_charge(sampleUV);
            float gx = sample_charge(sampleUV + vec2(texel.x, 0.0))
                     - sample_charge(sampleUV - vec2(texel.x, 0.0));
            float gy = sample_charge(sampleUV + vec2(0.0, texel.y))
                     - sample_charge(sampleUV - vec2(0.0, texel.y));
            vec2 push = vec2(gx, gy) * disturb_force * 4.0;

            float z = (l - 0.5) * depth + ch * 0.6; // disturbed points pop forward
            float zN = clamp(l, 0.0, 1.0);           // 0..1 depth for shading/color
            float scale = 1.0 + z * perspective + ch * 1.5;

            // Per-cell jitter breaks the grid into an organic scatter.
            vec2 jit = (vec2(hash21(cell), hash21(cell + 7.3)) - 0.5) * jitter * g;
            vec2 center = c + jit + vdir * z - push; // depth shear + disturbance

            float d = length(q - center);
            float radius = point_size * g * 0.5 * max(scale, 0.05);

            // Soft round Gaussian splat.
            float splat = exp(-(d * d) / (radius * radius + 1e-6) * 2.5);
            // Atmospheric perspective: far (dark) points recede and dim.
            float fade = mix(1.0 - depth_fade, 1.0, zN);
            float alpha = splat * fade;

            vec3 pc = point_color(color_mode, src.rgb, zN);
            // Disturbed points shift toward the disturbance color.
            pc = mix(pc, disturb_color.rgb, clamp(ch * 1.4, 0.0, 1.0));
            // Front-biased weighting so nearer points win the color.
            float w = alpha * exp(z * 4.0);
            colAccum += pc * w;
            wAccum += w;
            cover = max(cover, alpha);

            float halo = radius * radius * (1.0 + glow * 6.0) + 1e-6;
            glowAccum += pc * exp(-(d * d) / halo) * (glow * 0.12 + ch * 0.5) * fade;
        }
    }

    vec3 pointCol = wAccum > 1e-4 ? colAccum / wAccum : vec3(0.0);

    vec3 col = mix(bg_color.rgb, pointCol, cover);
    col += glowAccum;
    col *= brightness;

    fragColor = vec4(clamp(col + keep, 0.0, 1.0), 1.0);
}
