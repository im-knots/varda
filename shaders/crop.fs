/*{
    "DESCRIPTION": "Crop - mask/crop with adjustable edges",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "crop_left", "LABEL": "Left", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "crop_right", "LABEL": "Right", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "crop_bottom", "LABEL": "Bottom", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "crop_top", "LABEL": "Top", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "feather", "LABEL": "Feather", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.2},
        {"NAME": "invert_crop", "LABEL": "Invert", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    float crop_left;
    float crop_right;
    float crop_bottom;
    float crop_top;
    float feather;
    float invert_crop;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    float f = max(feather, 0.001);
    float maskL = smoothstep(crop_left - f, crop_left + f, uv.x);
    float maskR = smoothstep(crop_right + f, crop_right - f, uv.x);
    float maskB = smoothstep(crop_bottom - f, crop_bottom + f, uv.y);
    float maskT = smoothstep(crop_top + f, crop_top - f, uv.y);

    float mask = maskL * maskR * maskB * maskT;
    if (invert_crop > 0.5) mask = 1.0 - mask;

    fragColor = vec4(src.rgb * mask, src.a * mask);
}
