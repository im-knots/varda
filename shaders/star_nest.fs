/*{
    "DESCRIPTION": "Star Nest - volumetric raymarched star-field/nebula tunnel built from an iterated absolute-inversion fractal density field, with a look_at-driven dual-plane camera rotation",
    "CREDIT": "Varda VJ (ported from Pablo Roman Andrioli's 'Star Nest', MIT licensed, https://www.shadertoy.com/view/XlfGRj)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "iterations", "TYPE": "float", "DEFAULT": 17.0, "MIN": 4.0, "MAX": 24.0, "LABEL": "Fractal Iterations"},
        {"NAME": "formuparam", "TYPE": "float", "DEFAULT": 0.53, "MIN": 0.3, "MAX": 0.9, "LABEL": "Fractal Detail"},
        {"NAME": "volsteps", "TYPE": "float", "DEFAULT": 20.0, "MIN": 8.0, "MAX": 40.0, "LABEL": "Volume Steps"},
        {"NAME": "stepsize", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.02, "MAX": 0.3, "LABEL": "Step Size"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.2, "MAX": 2.0, "LABEL": "Zoom"},
        {"NAME": "tile", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.3, "MAX": 1.5, "LABEL": "Tiling"},
        {"NAME": "drift_speed", "TYPE": "float", "DEFAULT": 0.01, "MIN": 0.0, "MAX": 0.05, "LABEL": "Drift Speed"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 0.0015, "MIN": 0.0005, "MAX": 0.005, "LABEL": "Brightness"},
        {"NAME": "darkmatter", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Dark Matter"},
        {"NAME": "distfading", "TYPE": "float", "DEFAULT": 0.73, "MIN": 0.3, "MAX": 0.95, "LABEL": "Distance Fading"},
        {"NAME": "saturation", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.0, "MAX": 2.0, "LABEL": "Saturation"},
        {"NAME": "look_at", "TYPE": "point2D", "DEFAULT": [0.0, 0.0], "LABEL": "Look At"},
        {"NAME": "tint", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Tint"}
    ],
    "PHASE_INPUTS": [{"PARAM": "speed", "INDEX": 0, "SCALE": 1.0}]
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
    float speed;
    float iterations;
    float formuparam;
    float volsteps;
    float stepsize;
    float zoom;
    float tile;
    float drift_speed;
    float brightness;
    float darkmatter;
    float distfading;
    float saturation;
    vec2 look_at;
    vec4 tint;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward), the opposite of
    // Shadertoy's bottom-left/y-up fragCoord convention, so flip y here.
    // This shader (unusually) scales the *y* axis for aspect correction
    // rather than x, so that exact axis choice from the original is
    // preserved below.
    vec2 p = vec2(uv.x, 1.0 - uv.y) - 0.5;
    p.y *= RENDERSIZE.y / RENDERSIZE.x;

    vec3 dir = vec3(p * zoom, 1.0);
    float t = PHASE_TIME_0;
    float time = t * drift_speed + 0.25;

    // Mouse rotation replaced with a `look_at` point2D control. Unlike
    // eyes.fs's `look_at` (a screen-space gaze target that gets y-flipped
    // to match on-screen "up"), this look_at feeds two abstract rotation
    // angles analogous to raw iMouse pixel coordinates -- it isn't a
    // screen-space position, so there's no "up" semantic to preserve by
    // flipping. Left unflipped so the default (0,0) exactly reproduces
    // the original's idle-mouse-at-origin look (a1=0.5, a2=0.8).
    float a1 = 0.5 + look_at.x * 2.0;
    float a2 = 0.8 + look_at.y * 2.0;
    mat2 rot1 = mat2(cos(a1), sin(a1), -sin(a1), cos(a1));
    mat2 rot2 = mat2(cos(a2), sin(a2), -sin(a2), cos(a2));
    dir.xz *= rot1;
    dir.xy *= rot2;

    vec3 from = vec3(1.0, 0.5, 0.5);
    from += vec3(time * 2.0, time, -2.0);
    from.xz *= rot1;
    from.xy *= rot2;

    int iters = int(clamp(iterations, 1.0, 24.0));
    int steps = int(clamp(volsteps, 1.0, 40.0));

    // Volumetric raymarch: at each step, fold the sample point into a
    // tiled cell, then run an iterated absolute-inversion fractal
    // (p = abs(p)/dot(p,p) - formuparam) whose accumulated "average
    // change" doubles as both a dark-matter mask and a density value
    // used to accumulate distance-tinted color with depth fade.
    float s = 0.1;
    float fade = 1.0;
    vec3 v = vec3(0.0);
    for (int r = 0; r < 40; r++) {
        if (r >= steps) break;
        vec3 pp = from + s * dir * 0.5;
        pp = abs(vec3(tile) - mod(pp, vec3(tile * 2.0)));

        float pa = 0.0;
        float a = 0.0;
        for (int i = 0; i < 24; i++) {
            if (i >= iters) break;
            pp = abs(pp) / dot(pp, pp) - formuparam;
            a += abs(length(pp) - pa);
            pa = length(pp);
        }

        float dm = max(0.0, darkmatter - a * a * 0.001);
        a *= a * a;
        if (r > 6) fade *= 1.0 - dm;
        v += fade;
        v += vec3(s, s * s, s * s * s * s) * a * brightness * fade;
        fade *= distfading;
        s += stepsize;
    }

    v = mix(vec3(length(v)), v, saturation);
    vec3 col = v * 0.01;
    col *= tint.rgb;

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
