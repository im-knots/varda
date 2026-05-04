/*{
    "DESCRIPTION": "Push transition - slides one image pushing the other off",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Transition"],
    "INPUTS": [
        {"NAME": "progress", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Progress"},
        {"NAME": "startImage", "TYPE": "image"},
        {"NAME": "endImage", "TYPE": "image"},
        {"NAME": "direction", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Direction (0=Left 1=Right 2=Up 3=Down)"}
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
    float direction;
};

void main() {
    int dir = int(floor(direction + 0.5));

    vec2 offset;
    if (dir == 0) offset = vec2(-progress, 0.0);       // Push left
    else if (dir == 1) offset = vec2(progress, 0.0);    // Push right
    else if (dir == 2) offset = vec2(0.0, progress);    // Push up
    else offset = vec2(0.0, -progress);                 // Push down

    vec2 fromUV = uv + offset;
    vec2 toUV = uv + offset - sign(offset);
    // Fix zero sign for the non-offset axis
    if (dir <= 1) toUV.y = uv.y;
    else toUV.x = uv.x;

    vec4 fromColor = texture(sampler2D(startImage, texSampler), fromUV);
    vec4 toColor = texture(sampler2D(endImage, texSampler), toUV);

    // Determine which image is visible based on UV bounds
    bool fromVisible, toVisible;
    if (dir == 0) {
        fromVisible = fromUV.x >= 0.0 && fromUV.x <= 1.0;
        toVisible = toUV.x >= 0.0 && toUV.x <= 1.0;
    } else if (dir == 1) {
        fromVisible = fromUV.x >= 0.0 && fromUV.x <= 1.0;
        toVisible = toUV.x >= 0.0 && toUV.x <= 1.0;
    } else if (dir == 2) {
        fromVisible = fromUV.y >= 0.0 && fromUV.y <= 1.0;
        toVisible = toUV.y >= 0.0 && toUV.y <= 1.0;
    } else {
        fromVisible = fromUV.y >= 0.0 && fromUV.y <= 1.0;
        toVisible = toUV.y >= 0.0 && toUV.y <= 1.0;
    }

    if (fromVisible) fragColor = fromColor;
    else if (toVisible) fragColor = toColor;
    else fragColor = vec4(0.0);
}
