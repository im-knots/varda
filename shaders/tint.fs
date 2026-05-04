/*{
    "DESCRIPTION": "Tint - color tint overlay",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "tint_color", "TYPE": "color", "DEFAULT": [1.0, 0.8, 0.5, 1.0], "LABEL": "Tint Color"},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "mode", "LABEL": "Mode (0=Multiply 1=Overlay 2=Screen)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "preserve_luma", "LABEL": "Preserve Luminance", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    vec4 tint_color;
    float amount;
    float mode;
    float preserve_luma;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec3 tint = tint_color.rgb;

    int m = int(floor(mode + 0.5));
    vec3 tinted;

    if (m == 0) {
        // Multiply
        tinted = src.rgb * tint;
    } else if (m == 1) {
        // Overlay
        vec3 a = 2.0 * src.rgb * tint;
        vec3 b = 1.0 - 2.0 * (1.0 - src.rgb) * (1.0 - tint);
        tinted = mix(a, b, step(0.5, src.rgb));
    } else {
        // Screen
        tinted = 1.0 - (1.0 - src.rgb) * (1.0 - tint);
    }

    // Optionally preserve original luminance
    if (preserve_luma > 0.01) {
        float origLum = dot(src.rgb, vec3(0.299, 0.587, 0.114));
        float tintLum = dot(tinted, vec3(0.299, 0.587, 0.114));
        if (tintLum > 0.001) {
            tinted = tinted * (origLum / tintLum);
        }
        tinted = mix(tinted, clamp(tinted, 0.0, 1.0), preserve_luma);
    }

    vec3 result = mix(src.rgb, tinted, amount);
    fragColor = vec4(result, src.a);
}
