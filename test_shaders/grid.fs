/*{
    "DESCRIPTION": "Dot/point grid generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "grid_size", "TYPE": "float", "DEFAULT": 10.0, "MIN": 2.0, "MAX": 50.0, "LABEL": "Grid Size"},
        {"NAME": "dot_size", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.01, "MAX": 0.9, "LABEL": "Dot Size"},
        {"NAME": "style", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Style (0=Dots 1=Crosses 2=Lines)"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Scroll Speed"},
        {"NAME": "dot_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Dot Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"},
        {"NAME": "rotation", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.571, "LABEL": "Rotation"}
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
    float grid_size;
    float dot_size;
    float style;
    float anim_speed;
    vec4 dot_color;
    vec4 bg_color;
    float rotation;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv - 0.5;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    // Rotate
    float ca = cos(rotation), sa = sin(rotation);
    p = vec2(p.x * ca - p.y * sa, p.x * sa + p.y * ca);

    // Scroll animation
    p += vec2(TIME * anim_speed * 0.05);

    // Grid cell
    vec2 cell = fract(p * grid_size) - 0.5;

    float ds = dot_size;

    float pattern = 0.0;
    float st = floor(style + 0.5);

    if (st < 0.5) {
        // Dots
        pattern = 1.0 - smoothstep(ds * 0.4, ds * 0.5, length(cell));
    } else if (st < 1.5) {
        // Crosses
        float arm = ds * 0.15;
        float len = ds * 0.45;
        float h = smoothstep(arm + 0.01, arm, abs(cell.y)) * smoothstep(len + 0.01, len, abs(cell.x));
        float v = smoothstep(arm + 0.01, arm, abs(cell.x)) * smoothstep(len + 0.01, len, abs(cell.y));
        pattern = max(h, v);
    } else {
        // Grid lines
        float lineW = ds * 0.1;
        float h = smoothstep(lineW + 0.01, lineW, abs(cell.y));
        float v = smoothstep(lineW + 0.01, lineW, abs(cell.x));
        pattern = max(h, v);
    }

    vec4 color = mix(bg_color, dot_color, pattern);
    fragColor = vec4(color.rgb * color.a, color.a);
}
