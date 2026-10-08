/*{
    "DESCRIPTION": "Freeze - holds a frame, or re-grabs it at a set rate for a stutter",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "freeze_on", "LABEL": "Freeze", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "freeze_mix", "LABEL": "Mix", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "grab", "LABEL": "Grab Frame", "TYPE": "event"},
        {"NAME": "stutter", "LABEL": "Stutter", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "stutter_rate", "LABEL": "Stutter Rate", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.0, "MAX": 16.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "stutter_rate", "INDEX": 0, "SCALE": 1.0}
    ],
    "PASSES": [
        {"TARGET": "state", "PERSISTENT": true, "FORMAT": "rgba32float", "WIDTH": "1", "HEIGHT": "1"},
        {"TARGET": "held", "PERSISTENT": true}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform texture2D state;
layout(set = 0, binding = 4) uniform texture2D held;

layout(set = 0, binding = 5) uniform UserParams {
    float freeze_on;
    float freeze_mix;
    uint grab;
    uint stutter;
    float stutter_rate;
};

// state texel: x = freeze_on last frame, y = stutter step last frame,
// z = grab this frame, w = 1 once written.
void main() {
    bool frozen = freeze_on > 0.5;

    if (PASSINDEX == 0) {
        vec4 last = texelFetch(sampler2D(state, texSampler), ivec2(0), 0);
        float step = floor(PHASE_TIME_0);
        bool grabNow = last.w < 0.5
            || (frozen && last.x < 0.5)
            || grab == 1u
            || (stutter == 1u && step != last.y);
        fragColor = vec4(float(frozen), step, float(grabNow), 1.0);
        return;
    }

    if (PASSINDEX == 1) {
        bool grabNow = texelFetch(sampler2D(state, texSampler), ivec2(0), 0).z > 0.5;
        fragColor = grabNow
            ? texture(sampler2D(inputImage, texSampler), uv)
            : texture(sampler2D(held, texSampler), uv);
        return;
    }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec4 hold = texture(sampler2D(held, texSampler), uv);
    float k = freeze_mix * float(frozen || stutter == 1u);
    fragColor = vec4(max(mix(src.rgb, hold.rgb, k), 0.0), clamp(mix(src.a, hold.a, k), 0.0, 1.0));
}
