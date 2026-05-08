/*{
    "DESCRIPTION": "Tunnel lines - infinite tunnel with animated lines",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "tunnel_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "line_count", "TYPE": "float", "DEFAULT": 16.0, "MIN": 2.0, "MAX": 64.0, "LABEL": "Line Count"},
        {"NAME": "twist", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 3.0, "LABEL": "Twist"},
        {"NAME": "line_width", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.05, "MAX": 0.9, "LABEL": "Line Width"},
        {"NAME": "perspective", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0, "LABEL": "Perspective"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 0.8, 1.0, 1.0], "LABEL": "Line Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.1, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [{"PARAM": "tunnel_speed", "INDEX": 0}]
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

layout(set = 0, binding = 1) uniform UserParams {
    float tunnel_speed;
    float line_count;
    float twist;
    float line_width;
    float perspective;
    vec4 color1;
    vec4 color2;
};

#define PI 3.14159265359
#define TAU 6.28318530718

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float t = PHASE_TIME_0;

    // Polar coordinates for tunnel
    float dist = length(p);
    float angle = atan(p.y, p.x);

    // Tunnel mapping: distance → depth
    float depth = perspective / (dist + 0.01);

    // Scrolling depth + twist
    float tunnelU = angle / TAU + twist * depth * 0.1;
    float tunnelV = depth + t * 0.5;

    // Angular lines (stripes along tunnel)
    float angularPattern = fract(tunnelU * line_count);
    float angularLine = smoothstep(0.5 - line_width * 0.5, 0.5, angularPattern)
                      - smoothstep(0.5, 0.5 + line_width * 0.5, angularPattern);

    // Depth rings
    float depthPattern = fract(tunnelV * 0.5);
    float depthLine = smoothstep(0.4, 0.45, depthPattern) - smoothstep(0.55, 0.6, depthPattern);

    // Combine
    float pattern = max(angularLine, depthLine * 0.6);

    // Fade with depth (brighter in center/far away)
    float depthFade = 1.0 - smoothstep(0.0, 3.0, dist);
    float centerGlow = exp(-dist * 2.0) * 0.5;

    vec3 col = mix(color2.rgb, color1.rgb, pattern * depthFade);
    col += color1.rgb * centerGlow;
    col = clamp(col, 0.0, 1.0);

    fragColor = vec4(col, 1.0);
}
