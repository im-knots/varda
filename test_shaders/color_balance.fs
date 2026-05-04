/*{
    "DESCRIPTION": "Color Balance - adjust shadows, midtones, highlights independently",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "shadow_r", "LABEL": "Shadow Red", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "shadow_g", "LABEL": "Shadow Green", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "shadow_b", "LABEL": "Shadow Blue", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "mid_r", "LABEL": "Midtone Red", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "mid_g", "LABEL": "Midtone Green", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "mid_b", "LABEL": "Midtone Blue", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "high_r", "LABEL": "Highlight Red", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "high_g", "LABEL": "Highlight Green", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "high_b", "LABEL": "Highlight Blue", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0}
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
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float shadow_r;
    float shadow_g;
    float shadow_b;
    float mid_r;
    float mid_g;
    float mid_b;
    float high_r;
    float high_g;
    float high_b;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    float lum = dot(src.rgb, vec3(0.299, 0.587, 0.114));

    // Weight functions for shadows, midtones, highlights
    float shadowW = 1.0 - smoothstep(0.0, 0.5, lum);
    float highW = smoothstep(0.5, 1.0, lum);
    float midW = 1.0 - shadowW - highW;

    vec3 shadowShift = vec3(shadow_r, shadow_g, shadow_b) * shadowW * 0.3;
    vec3 midShift = vec3(mid_r, mid_g, mid_b) * midW * 0.3;
    vec3 highShift = vec3(high_r, high_g, high_b) * highW * 0.3;

    vec3 result = clamp(src.rgb + shadowShift + midShift + highShift, 0.0, 1.0);
    fragColor = vec4(result, src.a);
}
