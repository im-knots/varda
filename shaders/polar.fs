/*{
    "DESCRIPTION": "Polar - wraps the image around a center point or into an endless tunnel, or unwraps it into a strip",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "mode", "LABEL": "Mode", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["Rectangular to Polar", "Polar to Rectangular"]},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "center_x", "LABEL": "Center X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "center_y", "LABEL": "Center Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "twist", "LABEL": "Twist", "TYPE": "float", "DEFAULT": 0.0, "MIN": -4.0, "MAX": 4.0},
        {"NAME": "rotation_speed", "LABEL": "Rotation Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0},
        {"NAME": "radius_scale", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.25, "MAX": 4.0},
        {"NAME": "wrap", "LABEL": "Edges", "TYPE": "long", "DEFAULT": 1, "VALUES": [0, 1, 2], "LABELS": ["Clamp", "Repeat", "Mirror"]},
        {"NAME": "radius_mode", "LABEL": "Radius Mode", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["Linear", "Tunnel"]},
        {"NAME": "scroll_speed", "LABEL": "Scroll Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0},
        {"NAME": "center_fade", "LABEL": "Center Fade", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "rotation_speed", "INDEX": 0, "SCALE": 1.0},
        {"PARAM": "scroll_speed", "INDEX": 1, "SCALE": 1.0}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    int mode;
    float amount;
    float center_x;
    float center_y;
    float twist;
    float rotation_speed;
    float radius_scale;
    int wrap;
    int radius_mode;
    float scroll_speed;
    float center_fade;
};

const float TAU = 6.28318530718;

vec2 wrapUV(vec2 s) {
    if (wrap == 1) return fract(s);
    if (wrap == 2) return 1.0 - abs(mod(s, 2.0) - 1.0);
    return clamp(s, 0.0, 1.0);
}

void main() {
    vec2 aspect = vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0);
    vec2 center = vec2(center_x, center_y);
    // Radius, in frame heights, that maps to the full image height.
    float radius = 0.5 * radius_scale;

    // r is the distance from the center in units of `radius`; v is the image
    // row it maps to. Linear puts the bottom edge on the center and the top
    // edge on the rim, so text at the top reads upright. Tunnel maps depth,
    // 1 / r, so the image repeats endlessly toward the center.
    bool tunnel = radius_mode == 1;
    vec2 mapped;
    float r;
    if (mode == 0) {
        vec2 d = (uv - center) * aspect;
        r = max(length(d) / radius, 1e-4);
        float v = tunnel ? 0.5 / r : 1.0 - r;
        float turns = atan(d.x, -d.y) / TAU + twist * (tunnel ? v : r) / TAU + PHASE_TIME_0;
        mapped = vec2(fract(turns), v + PHASE_TIME_1);
    } else {
        // x walks clockwise around the circle, y walks inward from the rim.
        float v = max(fract(1.0 - uv.y + PHASE_TIME_1), 1e-4);
        r = tunnel ? 0.5 / v : 1.0 - v;
        float a = (uv.x - twist * (tunnel ? v : r) / TAU - PHASE_TIME_0) * TAU;
        mapped = center + vec2(sin(a), -cos(a)) * (r * radius) / aspect;
    }

    vec2 s = wrapUV(mix(uv, mapped, amount));
    vec4 src = texture(sampler2D(inputImage, texSampler), s);
    float fade = mix(1.0, smoothstep(0.0, 0.6, r), center_fade * amount);
    fragColor = vec4(max(src.rgb * fade, 0.0), src.a);
}
