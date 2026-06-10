/*{
    "DESCRIPTION": "Checkerboard pattern generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "grid_size", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 64.0, "LABEL": "Grid Size"},
        {"NAME": "rotation", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 6.283, "LABEL": "Rotation"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Scroll Speed"},
        {"NAME": "softness", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.5, "LABEL": "Softness"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Color 1"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Color 2"}
    ],
    "PHASE_INPUTS": [{"PARAM": "anim_speed", "INDEX": 0, "SCALE": 0.05}]
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
    float grid_size;
    float rotation;
    float anim_speed;
    float softness;
    vec4 color1;
    vec4 color2;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv - 0.5;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    // Rotate
    float ca = cos(rotation), sa = sin(rotation);
    p = vec2(p.x * ca - p.y * sa, p.x * sa + p.y * ca);

    // Scroll animation
    p += vec2(PHASE_TIME_0);

    // Checker pattern
    vec2 cell = floor(p * grid_size);
    float checker = mod(cell.x + cell.y, 2.0);

    if (softness > 0.001) {
        vec2 f = fract(p * grid_size);
        float sx = smoothstep(0.0, softness, f.x) * smoothstep(0.0, softness, 1.0 - f.x);
        float sy = smoothstep(0.0, softness, f.y) * smoothstep(0.0, softness, 1.0 - f.y);
        float soft = sx * sy;
        checker = mix(0.5, checker, soft);
    }

    vec4 color = mix(color2, color1, checker);
    fragColor = vec4(color.rgb * color.a, color.a);
}
