/*{
    "DESCRIPTION": "Fractal - Mandelbrot / Julia set generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "fractal_type", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Type (0=Mandelbrot 1=Julia)"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 50.0, "LABEL": "Zoom"},
        {"NAME": "center_x", "TYPE": "float", "DEFAULT": -0.5, "MIN": -2.0, "MAX": 2.0, "LABEL": "Center X"},
        {"NAME": "center_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0, "LABEL": "Center Y"},
        {"NAME": "julia_cx", "TYPE": "float", "DEFAULT": -0.7, "MIN": -2.0, "MAX": 2.0, "LABEL": "Julia CX"},
        {"NAME": "julia_cy", "TYPE": "float", "DEFAULT": 0.27, "MIN": -2.0, "MAX": 2.0, "LABEL": "Julia CY"},
        {"NAME": "max_iter", "TYPE": "float", "DEFAULT": 64.0, "MIN": 16.0, "MAX": 256.0, "LABEL": "Iterations"},
        {"NAME": "color_speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 3.0, "LABEL": "Color Speed"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 0.2, 0.8, 1.0], "LABEL": "Color 1"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [1.0, 0.5, 0.0, 1.0], "LABEL": "Color 2"}
    ],
    "PHASE_INPUTS": [{"PARAM": "color_speed", "INDEX": 0, "SCALE": 0.1}]
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
    float fractal_type;
    float zoom;
    float center_x;
    float center_y;
    float julia_cx;
    float julia_cy;
    float max_iter;
    float color_speed;
    vec4 color1;
    vec4 color2;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    p /= zoom;
    p += vec2(center_x, center_y);

    int iters = int(clamp(max_iter, 16.0, 256.0));
    bool julia = fractal_type > 0.5;

    vec2 z, c;
    if (julia) {
        z = p;
        c = vec2(julia_cx, julia_cy);
    } else {
        z = vec2(0.0);
        c = p;
    }

    int i;
    for (i = 0; i < 256; i++) {
        if (i >= iters) break;
        if (dot(z, z) > 4.0) break;
        z = vec2(z.x * z.x - z.y * z.y, 2.0 * z.x * z.y) + c;
    }

    if (i >= iters) {
        fragColor = vec4(0.0, 0.0, 0.0, 1.0);
    } else {
        // Smooth iteration count
        float smooth_i = float(i) - log2(log2(dot(z, z))) + 4.0;
        float t = fract(smooth_i / float(iters) * 4.0 + PHASE_TIME_0);

        // Palette
        vec3 col = 0.5 + 0.5 * cos(6.283 * (t + vec3(0.0, 0.1, 0.2)));
        col *= mix(color1.rgb, color2.rgb, t);
        col = clamp(col * 1.5, 0.0, 1.0);
        fragColor = vec4(col, 1.0);
    }
}
