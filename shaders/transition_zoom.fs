/*{
    "DESCRIPTION": "Zoom transition - zooms into source revealing destination",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Transition"],
    "INPUTS": [
        {"NAME": "progress", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Progress"},
        {"NAME": "startImage", "TYPE": "image"},
        {"NAME": "endImage", "TYPE": "image"},
        {"NAME": "zoom_amount", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.5, "MAX": 10.0, "LABEL": "Zoom Amount"}
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
    float zoom_amount;
};

void main() {
    // Zoom the source image
    float fromScale = 1.0 + progress * (zoom_amount - 1.0);
    vec2 fromUV = (uv - 0.5) / fromScale + 0.5;

    // Reverse zoom the destination
    float toScale = zoom_amount - progress * (zoom_amount - 1.0);
    vec2 toUV = (uv - 0.5) / toScale + 0.5;

    vec4 fromColor = texture(sampler2D(startImage, texSampler), fromUV);
    vec4 toColor = texture(sampler2D(endImage, texSampler), toUV);

    // Fade based on progress with smooth crossover
    float fade = smoothstep(0.3, 0.7, progress);

    // Fade out source as it zooms
    fromColor.a *= 1.0 - smoothstep(0.0, 0.6, progress);
    toColor.a *= smoothstep(0.4, 1.0, progress);

    fragColor = mix(fromColor, toColor, fade);
}
