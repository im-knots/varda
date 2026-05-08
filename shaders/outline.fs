/*{
    "DESCRIPTION": "Outline/Silhouette - edge detection with filled or outline rendering",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "edge_threshold", "LABEL": "Threshold", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.01, "MAX": 0.5},
        {"NAME": "edge_width", "LABEL": "Width", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 3.0},
        {"NAME": "edge_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Edge Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"},
        {"NAME": "show_fill", "LABEL": "Show Fill", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "invert_edges", "LABEL": "Invert", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    float edge_threshold;
    float edge_width;
    vec4 edge_color;
    vec4 bg_color;
    float show_fill;
    float invert_edges;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 texel = edge_width / RENDERSIZE;
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Sobel operator
    float tl = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(-texel.x, texel.y)).rgb, vec3(0.299, 0.587, 0.114));
    float t  = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(0.0, texel.y)).rgb, vec3(0.299, 0.587, 0.114));
    float tr = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, texel.y)).rgb, vec3(0.299, 0.587, 0.114));
    float l  = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(-texel.x, 0.0)).rgb, vec3(0.299, 0.587, 0.114));
    float r  = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, 0.0)).rgb, vec3(0.299, 0.587, 0.114));
    float bl = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(-texel.x, -texel.y)).rgb, vec3(0.299, 0.587, 0.114));
    float b  = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(0.0, -texel.y)).rgb, vec3(0.299, 0.587, 0.114));
    float br = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, -texel.y)).rgb, vec3(0.299, 0.587, 0.114));

    float gx = -tl - 2.0*l - bl + tr + 2.0*r + br;
    float gy = -tl - 2.0*t - tr + bl + 2.0*b + br;
    float edge = sqrt(gx * gx + gy * gy);

    float edgeMask = smoothstep(edge_threshold - 0.02, edge_threshold + 0.02, edge);
    if (invert_edges > 0.5) edgeMask = 1.0 - edgeMask;

    vec3 result;
    if (show_fill > 0.5) {
        result = mix(src.rgb, edge_color.rgb, edgeMask);
    } else {
        result = mix(bg_color.rgb, edge_color.rgb, edgeMask);
    }

    fragColor = vec4(result, src.a);
}
