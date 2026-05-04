/*{
    "DESCRIPTION": "Levels - input/output levels with gamma curve",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "in_black", "LABEL": "Input Black", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "in_white", "LABEL": "Input White", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "gamma", "LABEL": "Gamma", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 3.0},
        {"NAME": "out_black", "LABEL": "Output Black", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "out_white", "LABEL": "Output White", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
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
    float in_black;
    float in_white;
    float gamma;
    float out_black;
    float out_white;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Input levels: remap [in_black, in_white] → [0, 1]
    float range = max(in_white - in_black, 0.001);
    vec3 col = clamp((src.rgb - in_black) / range, 0.0, 1.0);

    // Gamma correction
    col = pow(col, vec3(1.0 / gamma));

    // Output levels: remap [0, 1] → [out_black, out_white]
    col = col * (out_white - out_black) + out_black;

    fragColor = vec4(col, src.a);
}
