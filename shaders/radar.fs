/*{
    "DESCRIPTION": "Radar sweep generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "sweep_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Sweep Speed"},
        {"NAME": "sweep_width", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.02, "MAX": 1.0, "LABEL": "Sweep Width"},
        {"NAME": "ring_count", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.0, "MAX": 16.0, "LABEL": "Ring Count"},
        {"NAME": "ring_width", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.005, "MAX": 0.1, "LABEL": "Ring Width"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.3, 1.0], "LABEL": "Sweep Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.0, 0.15, 0.05, 1.0], "LABEL": "Background"},
        {"NAME": "center_x", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Center X"},
        {"NAME": "center_y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Center Y"}
    ],
    "PHASE_INPUTS": [{"PARAM": "sweep_speed", "INDEX": 0, "SCALE": 0.2}]
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
    float sweep_speed;
    float sweep_width;
    float ring_count;
    float ring_width;
    vec4 color1;
    vec4 color2;
    float center_x;
    float center_y;
};

#define PI 3.14159265359
#define TAU 6.28318530718

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv - vec2(center_x, center_y);
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float dist = length(p);
    float angle = atan(p.y, p.x) / TAU + 0.5; // 0..1

    // Sweep beam
    float sweep_angle = fract(PHASE_TIME_0);
    float diff = fract(angle - sweep_angle);
    float beam = smoothstep(sweep_width, 0.0, diff);
    beam *= beam; // Exponential falloff for trailing glow

    // Concentric rings
    float rings = 0.0;
    if (ring_count > 0.0) {
        float rp = fract(dist * ring_count);
        rings = smoothstep(ring_width, 0.0, abs(rp - 0.5) - 0.5 + ring_width);
    }

    // Cross hairs
    float cross_h = smoothstep(0.003, 0.0, abs(p.y)) * 0.3;
    float cross_v = smoothstep(0.003, 0.0, abs(p.x)) * 0.3;

    // Outer circle
    float outer = smoothstep(ring_width, 0.0, abs(dist - 0.48));

    // Compose
    vec3 bg = color2.rgb;
    vec3 col = bg;
    col += color1.rgb * rings * 0.4;
    col += color1.rgb * (cross_h + cross_v);
    col += color1.rgb * outer * 0.5;
    col += color1.rgb * beam;

    // Fade at edge
    col *= smoothstep(0.52, 0.48, dist);

    fragColor = vec4(col, 1.0);
}
