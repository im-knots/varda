/*{
    "DESCRIPTION": "Emboss - relief/emboss convolution effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "emboss_strength", "LABEL": "Strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "emboss_angle", "LABEL": "Light Angle", "TYPE": "float", "DEFAULT": 0.785, "MIN": 0.0, "MAX": 6.283},
        {"NAME": "blend_original", "LABEL": "Blend Original", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    float emboss_strength;
    float emboss_angle;
    float blend_original;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 texel = 1.0 / RENDERSIZE;
    vec2 dir = vec2(cos(emboss_angle), sin(emboss_angle)) * texel;

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec4 s1 = texture(sampler2D(inputImage, texSampler), uv + dir);
    vec4 s2 = texture(sampler2D(inputImage, texSampler), uv - dir);

    // Emboss: difference along light direction + 0.5 bias
    vec3 embossed = (s1.rgb - s2.rgb) * emboss_strength + 0.5;

    vec3 result = mix(embossed, src.rgb, blend_original);
    result = clamp(result, 0.0, 1.0);

    fragColor = vec4(result, src.a);
}
