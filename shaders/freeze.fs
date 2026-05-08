/*{
    "DESCRIPTION": "Freeze - holds/freezes the current frame (simulated via static noise overlay)",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "freeze_on", "LABEL": "Freeze", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "freeze_mix", "LABEL": "Freeze Mix", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "static_amount", "LABEL": "Static/Noise", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "hold_color", "TYPE": "color", "DEFAULT": [0.5, 0.5, 0.5, 1.0], "LABEL": "Hold Color"}
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
    float freeze_on;
    float freeze_mix;
    float static_amount;
    vec4 hold_color;
};

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    if (freeze_on > 0.5) {
        // When frozen: show hold color mixed with source, plus optional static
        vec3 frozen = mix(src.rgb, hold_color.rgb, freeze_mix);

        if (static_amount > 0.001) {
            float n = hash(uv * RENDERSIZE + vec2(float(FRAMEINDEX)));
            frozen = mix(frozen, vec3(n), static_amount);
        }

        fragColor = vec4(frozen, src.a);
    } else {
        // Pass through
        fragColor = src;
    }
}
