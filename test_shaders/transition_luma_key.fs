/*{
    "DESCRIPTION": "Luma-based transition — brighter areas transition first",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Transition"],
    "INPUTS": [
        {
            "NAME": "progress",
            "TYPE": "float",
            "DEFAULT": 0.0,
            "MIN": 0.0,
            "MAX": 1.0,
            "LABEL": "Progress"
        },
        {
            "NAME": "startImage",
            "TYPE": "image"
        },
        {
            "NAME": "endImage",
            "TYPE": "image"
        },
        {
            "NAME": "softness",
            "TYPE": "float",
            "DEFAULT": 0.1,
            "MIN": 0.0,
            "MAX": 0.5,
            "LABEL": "Edge Softness"
        },
        {
            "NAME": "invert",
            "TYPE": "bool",
            "DEFAULT": false,
            "LABEL": "Invert Luma"
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
layout(set = 0, binding = 2) uniform texture2D startImage;
layout(set = 0, binding = 3) uniform texture2D endImage;

layout(set = 0, binding = 4) uniform TransitionParams {
    float progress;
    float softness;
    uint invert;  // bool stored as uint
};

void main() {
    vec4 fromColor = texture(sampler2D(startImage, texSampler), uv);
    vec4 toColor = texture(sampler2D(endImage, texSampler), uv);

    // Compute luma from the start image
    float luma = dot(fromColor.rgb, vec3(0.299, 0.587, 0.114));
    if (invert != 0u) {
        luma = 1.0 - luma;
    }

    // Map progress through luma — bright areas switch first
    float threshold = smoothstep(progress - softness, progress + softness, luma);
    fragColor = mix(toColor, fromColor, threshold);
}

