/*{
    "DESCRIPTION": "Threshold / posterize effect - reduces to black and white or limited colors",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "threshold_val", "LABEL": "Threshold", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "smoothness", "LABEL": "Smoothness", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.2},
        {"NAME": "invert_output", "LABEL": "Invert", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "color_above", "LABEL": "Color Above", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0]},
        {"NAME": "color_below", "LABEL": "Color Below", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0]}
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
    float threshold_val;
    float smoothness;
    float invert_output;
    vec4 color_above;
    vec4 color_below;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 color = texture(sampler2D(inputImage, texSampler), uv);

    // Luminance
    float lum = dot(color.rgb, vec3(0.299, 0.587, 0.114));

    // Threshold with optional smoothness
    float t;
    if (smoothness > 0.001) {
        t = smoothstep(threshold_val - smoothness, threshold_val + smoothness, lum);
    } else {
        t = step(threshold_val, lum);
    }

    if (invert_output > 0.5) {
        t = 1.0 - t;
    }

    vec4 result = mix(color_below, color_above, t);
    fragColor = vec4(result.rgb * result.a, result.a);
}
