/*{
    "DESCRIPTION": "Color inversion with blend control",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {
            "NAME": "inputImage",
            "TYPE": "image"
        },
        {
            "NAME": "amount",
            "LABEL": "Invert Amount",
            "TYPE": "float",
            "DEFAULT": 1.0,
            "MIN": 0.0,
            "MAX": 1.0
        },
        {
            "NAME": "invert_r",
            "LABEL": "Invert Red",
            "TYPE": "bool",
            "DEFAULT": true
        },
        {
            "NAME": "invert_g",
            "LABEL": "Invert Green",
            "TYPE": "bool",
            "DEFAULT": true
        },
        {
            "NAME": "invert_b",
            "LABEL": "Invert Blue",
            "TYPE": "bool",
            "DEFAULT": true
        }
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
    float amount;
    float invert_r;
    float invert_g;
    float invert_b;
};

void main() {
    vec4 color = texture(sampler2D(inputImage, texSampler), uv);
    
    vec3 inverted = color.rgb;
    if (invert_r > 0.5) inverted.r = 1.0 - inverted.r;
    if (invert_g > 0.5) inverted.g = 1.0 - inverted.g;
    if (invert_b > 0.5) inverted.b = 1.0 - inverted.b;
    
    fragColor = vec4(mix(color.rgb, inverted, amount), color.a);
}

