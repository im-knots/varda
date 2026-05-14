/*{
    "DESCRIPTION": "Hue Key - keys out pixels matching a target hue range, with analog simulation",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "target_hue", "LABEL": "Target Hue", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "hue_width", "LABEL": "Hue Width", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "min_saturation", "LABEL": "Min Saturation", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "opacity", "LABEL": "Opacity", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "softness", "LABEL": "Edge Softness", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "noise", "LABEL": "Analog Noise", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "edge_blur", "LABEL": "Edge Blur", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "color_fringe", "LABEL": "Color Fringe", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float target_hue;
    float hue_width;
    float min_saturation;
    float opacity;
    float softness;
    float noise;
    float edge_blur;
    float color_fringe;
};

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

vec3 rgb2hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y);
    float e = 1.0e-10;
    return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

// Hue distance accounting for wrap-around (0.0 and 1.0 are the same hue)
float hue_distance(float h1, float h2) {
    float d = abs(h1 - h2);
    return min(d, 1.0 - d);
}

// Compute key match for a given pixel color
float compute_hue_match(vec3 rgb, float tol_offset) {
    vec3 hsv = rgb2hsv(rgb);
    float hue_dist = hue_distance(hsv.x, target_hue);
    float half_width = hue_width * 0.5 + tol_offset;
    float hue_match = 1.0 - smoothstep(half_width, half_width + softness, hue_dist);
    // Reject low-saturation pixels (grays have undefined hue)
    float sat_mask = smoothstep(min_saturation * 0.5, min_saturation, hsv.y);
    return hue_match * sat_mask;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Analog noise: jitter the hue width threshold
    float tol_offset = noise * 0.15 * (hash12(uv * RENDERSIZE + float(FRAMEINDEX) * 1.37) - 0.5);

    float match_amount = compute_hue_match(src.rgb, tol_offset);

    // Analog edge blur: horizontal neighbor sampling
    if (edge_blur > 0.0) {
        float blur_radius = edge_blur * 8.0 / RENDERSIZE.x;
        float total = match_amount;
        float weight = 1.0;
        for (int i = 1; i <= 3; i++) {
            float offset = blur_radius * float(i) / 3.0;
            vec4 ls = texture(sampler2D(inputImage, texSampler), uv + vec2(-offset, 0.0));
            vec4 rs = texture(sampler2D(inputImage, texSampler), uv + vec2(offset, 0.0));
            float n_l = noise * 0.15 * (hash12((uv + vec2(-offset, 0.0)) * RENDERSIZE + float(FRAMEINDEX) * 1.37) - 0.5);
            float n_r = noise * 0.15 * (hash12((uv + vec2(offset, 0.0)) * RENDERSIZE + float(FRAMEINDEX) * 1.37) - 0.5);
            float w = 1.0 - float(i) / 4.0;
            total += compute_hue_match(ls.rgb, n_l) * w;
            total += compute_hue_match(rs.rgb, n_r) * w;
            weight += 2.0 * w;
        }
        match_amount = total / weight;
    }

    // Analog color fringe: shift RGB channels at key edges
    vec3 final_rgb = src.rgb;
    if (color_fringe > 0.0 && match_amount > 0.01 && match_amount < 0.99) {
        float fringe_offset = color_fringe * 3.0 / RENDERSIZE.x;
        float r = texture(sampler2D(inputImage, texSampler), uv + vec2(-fringe_offset, 0.0)).r;
        float b = texture(sampler2D(inputImage, texSampler), uv + vec2(fringe_offset, 0.0)).b;
        final_rgb = vec3(r, src.g, b);
    }

    float new_alpha = mix(src.a, opacity, match_amount);
    fragColor = vec4(final_rgb, new_alpha);
}
