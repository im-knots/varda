/*{
    "DESCRIPTION": "Glow/Bloom - soft glow around bright areas",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "glow_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "glow_radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 4.0, "MIN": 1.0, "MAX": 12.0},
        {"NAME": "threshold", "LABEL": "Threshold", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "glow_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Glow Tint"}
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
    float glow_amount;
    float glow_radius;
    float threshold;
    vec4 glow_color;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec2 texel = glow_radius / RENDERSIZE;

    // Simple box blur of bright areas (approximation of gaussian bloom)
    vec3 bloom = vec3(0.0);
    float total = 0.0;
    for (int x = -3; x <= 3; x++) {
        for (int y = -3; y <= 3; y++) {
            vec2 off = vec2(float(x), float(y)) * texel;
            vec4 s = texture(sampler2D(inputImage, texSampler), uv + off);
            float lum = dot(s.rgb, vec3(0.299, 0.587, 0.114));
            float bright = max(lum - threshold, 0.0);
            float w = exp(-float(x * x + y * y) * 0.15);
            bloom += s.rgb * bright * w;
            total += w;
        }
    }
    bloom /= total;

    vec3 result = src.rgb + bloom * glow_amount * glow_color.rgb;
    result = clamp(result, 0.0, 1.0);

    fragColor = vec4(result, src.a);
}
