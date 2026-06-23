/*{
    "DESCRIPTION": "Edge Detect - clean Sobel edge detection with color options",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "sensitivity", "LABEL": "Sensitivity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 5.0},
        {"NAME": "edge_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Edge Color"},
        {"NAME": "use_source_color", "LABEL": "Source Color", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "bg_opacity", "LABEL": "BG Opacity", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    float sensitivity;
    vec4 edge_color;
    float use_source_color;
    float bg_opacity;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 texel = 1.0 / RENDERSIZE;
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // 3x3 luminance samples
    float s[9];
    int idx = 0;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec3 c = texture(sampler2D(inputImage, texSampler), uv + vec2(float(x), float(y)) * texel).rgb;
            s[idx++] = dot(c, vec3(0.2126, 0.7152, 0.0722));
        }
    }

    // Sobel
    float gx = -s[0] - 2.0*s[3] - s[6] + s[2] + 2.0*s[5] + s[8];
    float gy = -s[0] - 2.0*s[1] - s[2] + s[6] + 2.0*s[7] + s[8];
    float edge = clamp(sqrt(gx*gx + gy*gy) * sensitivity, 0.0, 1.0);

    vec3 eColor = mix(edge_color.rgb, src.rgb, use_source_color);
    vec3 bg = src.rgb * bg_opacity;
    vec3 result = mix(bg, eColor, edge);

    fragColor = vec4(result, max(edge, bg_opacity) * src.a);
}
