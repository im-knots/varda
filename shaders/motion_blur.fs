/*{
    "DESCRIPTION": "Motion Blur - directional blur along an angle",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort", "Blur"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "blur_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.1},
        {"NAME": "angle", "LABEL": "Angle", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 6.283},
        {"NAME": "quality", "LABEL": "Quality", "TYPE": "float", "DEFAULT": 8.0, "MIN": 4.0, "MAX": 16.0}
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
    float blur_amount;
    float angle;
    float quality;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 dir = vec2(cos(angle), sin(angle)) * blur_amount;
    int samples = int(clamp(quality, 4.0, 16.0));

    vec4 col = vec4(0.0);
    float total = 0.0;

    for (int i = 0; i < 16; i++) {
        if (i >= samples) break;
        float t = (float(i) / float(samples - 1) - 0.5) * 2.0; // -1 to 1
        vec2 offset = dir * t;
        float w = 1.0 - abs(t) * 0.3;
        col += texture(sampler2D(inputImage, texSampler), uv + offset) * w;
        total += w;
    }

    fragColor = col / total;
}
