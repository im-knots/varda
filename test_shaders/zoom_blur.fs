/*{
    "DESCRIPTION": "Zoom Blur - radial blur from center point",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort", "Blur"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "blur_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "center_x", "LABEL": "Center X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "center_y", "LABEL": "Center Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float blur_amount;
    float center_x;
    float center_y;
    float quality;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 center = vec2(center_x, center_y);
    vec2 dir = uv - center;
    int samples = int(clamp(quality, 4.0, 16.0));

    vec4 col = vec4(0.0);
    float total = 0.0;

    for (int i = 0; i < 16; i++) {
        if (i >= samples) break;
        float t = float(i) / float(samples - 1);
        float scale = 1.0 - blur_amount * t;
        vec2 sampleUV = center + dir * scale;
        float w = 1.0 - t * 0.5; // Weight falloff
        col += texture(sampler2D(inputImage, texSampler), sampleUV) * w;
        total += w;
    }

    fragColor = col / total;
}
