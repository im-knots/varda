/*{
    "DESCRIPTION": "Luma Key - keys out pixels based on brightness, with analog simulation",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "threshold", "LABEL": "Threshold", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "softness", "LABEL": "Edge Softness", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "invert", "LABEL": "Invert", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "opacity", "LABEL": "Opacity", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
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
    float threshold;
    float softness;
    uint invert;
    float opacity;
    float noise;
    float edge_blur;
    float color_fringe;
};

float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Rec.709 luma coefficients
float luma(vec3 rgb) {
    return dot(rgb, vec3(0.2126, 0.7152, 0.0722));
}

// Compute key match for a given pixel
float compute_luma_match(vec3 rgb, float thresh) {
    float l = luma(rgb);
    if (invert != 0u) {
        l = 1.0 - l;
    }
    // Key out pixels BELOW threshold (darks by default, brights when inverted)
    return 1.0 - smoothstep(thresh, thresh + softness, l);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Analog noise: jitter the threshold
    float thresh = threshold + noise * 0.2 * (hash12(uv * RENDERSIZE + float(FRAMEINDEX) * 1.37) - 0.5);

    float match_amount = compute_luma_match(src.rgb, thresh);

    // Analog edge blur: horizontal neighbor sampling
    if (edge_blur > 0.0) {
        float blur_radius = edge_blur * 8.0 / RENDERSIZE.x;
        float total = match_amount;
        float weight = 1.0;
        for (int i = 1; i <= 3; i++) {
            float offset = blur_radius * float(i) / 3.0;
            vec4 ls = texture(sampler2D(inputImage, texSampler), uv + vec2(-offset, 0.0));
            vec4 rs = texture(sampler2D(inputImage, texSampler), uv + vec2(offset, 0.0));
            float n_l = noise * 0.2 * (hash12((uv + vec2(-offset, 0.0)) * RENDERSIZE + float(FRAMEINDEX) * 1.37) - 0.5);
            float n_r = noise * 0.2 * (hash12((uv + vec2(offset, 0.0)) * RENDERSIZE + float(FRAMEINDEX) * 1.37) - 0.5);
            float t_l = threshold + n_l;
            float t_r = threshold + n_r;
            float w = 1.0 - float(i) / 4.0;
            total += compute_luma_match(ls.rgb, t_l) * w;
            total += compute_luma_match(rs.rgb, t_r) * w;
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
