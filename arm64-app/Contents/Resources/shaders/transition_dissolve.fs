/*{
    "DESCRIPTION": "Smooth crossfade dissolve between two sources",
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
layout(set = 0, binding = 2) uniform texture2D startImage;
layout(set = 0, binding = 3) uniform texture2D endImage;

layout(set = 0, binding = 4) uniform TransitionParams {
    float progress;
};

void main() {
    vec4 fromColor = texture(sampler2D(startImage, texSampler), uv);
    vec4 toColor = texture(sampler2D(endImage, texSampler), uv);
    fragColor = mix(fromColor, toColor, progress);
}

