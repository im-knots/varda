/*{
    "DESCRIPTION": "Gaussian blur effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Blur"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "radius", "LABEL": "Blur Radius", "TYPE": "float", "DEFAULT": 5.0, "MIN": 0.0, "MAX": 20.0},
        {"NAME": "quality", "LABEL": "Quality", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 5.0}
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
    float radius;
    float quality;
};

#define TAU 6.28318530718

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float r = radius;

    vec2 texelSize = 1.0 / RENDERSIZE;
    vec4 color = texture(sampler2D(inputImage, texSampler), uv);

    if (r < 0.1) {
        fragColor = color;
        return;
    }

    // Multi-sample gaussian blur
    int q = int(clamp(quality, 1.0, 5.0));
    float directions = 8.0 + float(q) * 4.0; // 12 to 28 directions
    float steps = float(q);

    float totalWeight = 1.0;

    for (float d = 0.0; d < 28.0; d += 1.0) {
        if (d >= directions) break;
        float a = d * TAU / directions;
        vec2 dir = vec2(cos(a), sin(a));
        for (float s = 1.0; s <= 5.0; s += 1.0) {
            if (s > steps) break;
            float weight = 1.0 - (s / (steps + 1.0));
            vec2 offset = dir * texelSize * r * (s / steps);
            color += texture(sampler2D(inputImage, texSampler), uv + offset) * weight;
            totalWeight += weight;
        }
    }

    color /= totalWeight;
    fragColor = color;
}
