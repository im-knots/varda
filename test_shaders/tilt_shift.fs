/*{
    "DESCRIPTION": "Tilt Shift - fake miniature/selective focus blur",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "focus_pos", "LABEL": "Focus Position", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "focus_width", "LABEL": "Focus Width", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.02, "MAX": 0.8},
        {"NAME": "blur_amount", "LABEL": "Blur Amount", "TYPE": "float", "DEFAULT": 3.0, "MIN": 0.0, "MAX": 8.0},
        {"NAME": "saturation_boost", "LABEL": "Saturation Boost", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "horizontal", "LABEL": "Horizontal", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    float focus_pos;
    float focus_width;
    float blur_amount;
    float saturation_boost;
    float horizontal;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float coord = (horizontal > 0.5) ? uv.x : uv.y;
    float dist = abs(coord - focus_pos);
    float blurFactor = smoothstep(focus_width * 0.5, focus_width, dist);

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Blur (box blur approximation weighted by distance from focus)
    vec3 blurred = vec3(0.0);
    float total = 0.0;
    float radius = blurFactor * blur_amount;
    vec2 texel = 1.0 / RENDERSIZE;

    for (int x = -3; x <= 3; x++) {
        for (int y = -3; y <= 3; y++) {
            vec2 off = vec2(float(x), float(y)) * texel * radius;
            float w = exp(-float(x * x + y * y) * 0.2);
            blurred += texture(sampler2D(inputImage, texSampler), uv + off).rgb * w;
            total += w;
        }
    }
    blurred /= total;

    vec3 result = mix(src.rgb, blurred, blurFactor);

    // Boost saturation (miniature look)
    float lum = dot(result, vec3(0.299, 0.587, 0.114));
    result = mix(vec3(lum), result, 1.0 + saturation_boost);
    result = clamp(result, 0.0, 1.0);

    fragColor = vec4(result, src.a);
}
