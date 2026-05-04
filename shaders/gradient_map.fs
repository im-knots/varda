/*{
    "DESCRIPTION": "Gradient Map - maps luminance to a 4-stop color gradient",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "color_a", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.2, 1.0], "LABEL": "Shadow Color"},
        {"NAME": "color_b", "TYPE": "color", "DEFAULT": [0.2, 0.0, 0.6, 1.0], "LABEL": "Dark Mid"},
        {"NAME": "color_c", "TYPE": "color", "DEFAULT": [0.8, 0.3, 0.1, 1.0], "LABEL": "Light Mid"},
        {"NAME": "color_d", "TYPE": "color", "DEFAULT": [1.0, 1.0, 0.7, 1.0], "LABEL": "Highlight Color"},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "contrast", "LABEL": "Contrast", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 2.0}
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
    vec4 color_a;
    vec4 color_b;
    vec4 color_c;
    vec4 color_d;
    float amount;
    float contrast;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    float lum = dot(src.rgb, vec3(0.299, 0.587, 0.114));

    // Contrast
    lum = clamp((lum - 0.5) * contrast + 0.5, 0.0, 1.0);

    // 4-stop gradient
    vec3 mapped;
    if (lum < 0.333) {
        mapped = mix(color_a.rgb, color_b.rgb, lum * 3.0);
    } else if (lum < 0.667) {
        mapped = mix(color_b.rgb, color_c.rgb, (lum - 0.333) * 3.0);
    } else {
        mapped = mix(color_c.rgb, color_d.rgb, (lum - 0.667) * 3.0);
    }

    vec3 result = mix(src.rgb, mapped, amount);
    fragColor = vec4(result, src.a);
}
