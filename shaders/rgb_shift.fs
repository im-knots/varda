/*{
    "DESCRIPTION": "Chromatic aberration / RGB shift effect",
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
            "LABEL": "Shift Amount",
            "TYPE": "float",
            "DEFAULT": 0.01,
            "MIN": 0.0,
            "MAX": 0.05
        },
        {
            "NAME": "angle",
            "LABEL": "Shift Angle",
            "TYPE": "float",
            "DEFAULT": 0.0,
            "MIN": 0.0,
            "MAX": 6.283
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float amount;
    float angle;
};

void main() {
    float shift = amount;
    
    vec2 dir = vec2(cos(angle), sin(angle)) * shift;
    
    float r = texture(sampler2D(inputImage, texSampler), uv + dir).r;
    float g = texture(sampler2D(inputImage, texSampler), uv).g;
    float b = texture(sampler2D(inputImage, texSampler), uv - dir).b;
    
    fragColor = vec4(r, g, b, 1.0);
}

