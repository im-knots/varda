/*{
    "DESCRIPTION": "Animated geometric lines generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "line_count", "TYPE": "float", "DEFAULT": 12.0, "MIN": 1.0, "MAX": 64.0, "LABEL": "Line Count"},
        {"NAME": "line_width", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.001, "MAX": 0.1, "LABEL": "Line Width"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "style", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Style (0=Horiz 1=Vert 2=Radial 3=Diagonal)"},
        {"NAME": "wave_amount", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Wave Amount"},
        {"NAME": "wave_freq", "TYPE": "float", "DEFAULT": 3.0, "MIN": 0.5, "MAX": 10.0, "LABEL": "Wave Frequency"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Line Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
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

layout(set = 0, binding = 1) uniform UserParams {
    float line_count;
    float line_width;
    float anim_speed;
    float style;
    float wave_amount;
    float wave_freq;
    vec4 color1;
    vec4 color2;
};

#define PI 3.14159265359

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = TIME * anim_speed;

    int st = int(floor(style + 0.5));
    float coord;

    if (st == 0) {
        // Horizontal lines
        float wave = sin(p.x * wave_freq * PI * 2.0 + t) * wave_amount * 0.1;
        coord = p.y + wave + t * 0.05;
    } else if (st == 1) {
        // Vertical lines
        float wave = sin(p.y * wave_freq * PI * 2.0 + t) * wave_amount * 0.1;
        coord = p.x + wave + t * 0.05;
    } else if (st == 2) {
        // Radial lines from center
        vec2 c = p - vec2(0.5 * RENDERSIZE.x / RENDERSIZE.y, 0.5);
        float angle = atan(c.y, c.x) / (2.0 * PI) + 0.5;
        float wave = sin(length(c) * wave_freq * PI * 8.0 + t) * wave_amount * 0.05;
        coord = angle + wave;
    } else {
        // Diagonal lines
        float wave = sin((p.x + p.y) * wave_freq * PI * 2.0 + t) * wave_amount * 0.1;
        coord = (p.x + p.y) * 0.707 + wave + t * 0.05;
    }

    float pattern = fract(coord * line_count);
    float lw = line_width * line_count;
    float line = smoothstep(0.5 - lw - 0.01, 0.5 - lw, pattern) - smoothstep(0.5 + lw, 0.5 + lw + 0.01, pattern);

    vec4 color = mix(color2, color1, line);
    fragColor = vec4(color.rgb * color.a, color.a);
}
