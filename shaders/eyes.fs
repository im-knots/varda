/*{
    "DESCRIPTION": "Eyes - a tiled grid of procedural cartoon eyes with autonomous blinking, drifting gaze, and IQ cosine-palette irises",
    "CREDIT": "Varda VJ (ported from a Shadertoy 'eyes' generator sketch; cosine palette technique by Inigo Quilez)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "eyes_x", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 12.0, "LABEL": "Eyes Across"},
        {"NAME": "eyes_y", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 12.0, "LABEL": "Eyes Down"},
        {"NAME": "blink_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Blink Speed"},
        {"NAME": "force_open", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Force Open"},
        {"NAME": "track_amount", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Gaze Tracking"},
        {"NAME": "grain", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.3, "LABEL": "Grain"},
        {"NAME": "look_at", "TYPE": "point2D", "DEFAULT": [0.5, 0.5], "LABEL": "Look At"}
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
    float eyes_x;
    float eyes_y;
    float blink_speed;
    float force_open;
    float track_amount;
    float grain;
    vec2 look_at;
};

// EDGE_SM replaces the original Shadertoy's resolution-relative "smooth"
// constant (renamed since `smooth` is a reserved GLSL interpolation
// qualifier keyword).
#define EDGE_SM (16.0 / RENDERSIZE.x)
#define PI 3.1415926535
#define S(x) smoothstep(-EDGE_SM, EDGE_SM, x)
#define SR(x, y) smoothstep(-EDGE_SM * (y), EDGE_SM * (y), x)

// ---- IQ cosine palette: https://iquilezles.org/articles/palettes ----
vec3 pal(in float t, in vec3 a, in vec3 b, in vec3 c, in vec3 d) {
    return a + b * cos(6.28318 * (c * t + d));
}

vec3 pal1(in float t) {
    return pal(t, vec3(0.5, 0.5, 0.5), vec3(0.5, 0.5, 0.5), vec3(1.0, 1.0, 1.0), vec3(0.0, 0.33, 0.67));
}

float rand(vec3 v) {
    return fract(cos(dot(v, vec3(13.46543, 67.1132, 123.546123))) * 43758.5453);
}

float rand(vec2 v) {
    return fract(sin(dot(v, vec2(5.11543, 71.3177))) * 43758.5453);
}

float rand(float v) {
    return fract(sin(v * 71.3132) * 43758.5453);
}

vec2 rand2(vec2 v) {
    return vec2(
        fract(sin(dot(v, vec2(5.11543, 71.3132))) * 43758.5453),
        fract(sin(dot(v, vec2(7.3113, 21.5723))) * 31222.1234)
    );
}

// Renders one eye. `fst` is the local eyelid-shaped coordinate for this
// cell, `cst` is the integer cell id (seeds the per-eye randomness), and
// `mouse` is the gaze-target position relative to this cell's center.
vec3 eye(vec2 fst, vec2 cst, vec2 mouse, float t) {
    float noise = rand(cst);

    float nt = t * 2.0 * (noise + 0.8) + noise * 100.0;
    float fnt = floor(nt);
    vec2 noise2 = rand2(cst + vec2(fnt));
    vec2 noise22 = rand2(cst + vec2(fnt + 1.0));
    float pinoise = noise2.x * PI * 2.0;
    float pinoise2 = noise22.x * PI * 2.0;
    float move = 1.0 - (cos(fract(nt) * PI) + 1.0) / 2.0;
    move = pow(move, 4.0);

    // Autonomous blink cycle, with a manual override to pin the eyes open.
    float autoOpen = (sin(t * 2.0 * blink_speed + noise * 100.0) + 1.0) / 2.0;
    autoOpen = 1.0 - pow(autoOpen, 3.0);
    float eyeOpen = mix(autoOpen, 1.0, force_open);

    float col = (sin(fst.x) + 1.0) / 2.0;
    float col2 = col * eyeOpen + fst.y * 2.1 - 0.1;
    col = col * eyeOpen - fst.y * 2.1 - 0.1;
    float cs1 = min(col - 0.1, col2 - 0.1);
    float cs2 = S(cs1);
    col = S(min(col, col2));

    float grad = min(eyeOpen * 1.2, 1.0);

    vec2 loc = vec2(fract(fst.x / PI / 2.0 + PI * 2.0) - 0.53, fst.y * RENDERSIZE.y / RENDERSIZE.x);

    // Autonomous random glance target, blended toward the tracked look_at
    // point (replaces the original mouse-follow behavior).
    vec2 pin2 = mix(vec2(cos(pinoise), sin(pinoise)) * ((noise2.y + 1.0) / 2.0),
                     vec2(cos(pinoise2), sin(pinoise2)) * ((noise22.y + 1.0) / 2.0), move);
    pin2 *= 0.25;
    pin2 = mix(pin2, mouse, track_amount);

    float lloc = length(loc);
    float irisn = mix(1.0, mix(noise2.x, noise22.x, move), 0.25);
    float iris = length(loc - pin2 * (0.5 - lloc));
    float irisWhite = length(loc - pin2 * (0.2 - lloc));
    float irisDark = SR(length(loc - pin2 * (0.4 - lloc)) - 0.05 * irisn, 0.5);
    float irisShadow = SR(-irisWhite + 0.07, 15.0);
    irisWhite = SR(-irisWhite + 0.03, 1.4);

    vec3 irisColor = irisDark * pal1(irisShadow + nt / 10.0);
    irisColor = max(irisColor, irisWhite * 0.9);
    vec3 baseCol = vec3(SR(-lloc + 0.25, 15.0));
    baseCol = baseCol + 0.25 * pal1(baseCol.x + nt / 10.0);

    vec3 finCol = mix(baseCol, irisColor, S(-iris + 0.15));
    finCol = mix(pal1(noise + nt / 10.0) * grad, finCol, cs2);
    finCol = min(finCol, col);

    return finCol;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); flip y so the
    // eyelid taper and look_at tracking read the same "up" as the
    // original Shadertoy's bottom-left/y-up convention.
    vec2 st = vec2(uv.x, 1.0 - uv.y);
    vec2 mouse = vec2(look_at.x, 1.0 - look_at.y);
    float t = PHASE_TIME_0;

    float scx = eyes_x * PI * 2.0;

    // Two interleaved eye grids, offset by half a cell in x and y, so each
    // eye's eyelid taper blends into its neighbors (matches the original
    // tiling trick rather than leaving hard seams between cells).
    float fsty = fract(st.y * eyes_y) - 0.5;
    float fsty2 = fract(st.y * eyes_y + 0.5) - 0.5;
    float csty = floor(st.y * eyes_y);
    float csty2 = floor(st.y * eyes_y + 0.5);
    float cstx = floor(st.x * eyes_x);
    float cstx2 = floor(st.x * eyes_x + 0.5);
    vec2 cst = vec2(cstx, csty);
    vec2 cst2 = vec2(cstx2, csty2 + 1234.0);
    vec2 fst = vec2(st.x * scx - 0.5 * PI, fsty);
    vec2 fst2 = vec2(st.x * scx + 0.5 * PI, fsty2);

    vec2 m1 = mouse - vec2((cstx + 0.5) / eyes_x, (csty + 0.5) / eyes_y);
    vec2 m2 = mouse - vec2((cstx2 + 0.5) / eyes_x, (csty2 + 0.5) / eyes_y);

    vec3 col = eye(fst, cst, m1, t);
    vec3 col2 = eye(fst2, cst2, m2, t);
    col = max(col, col2);
    col += grain * (rand((uv * RENDERSIZE) / 3.0 + t) - 0.5);

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
