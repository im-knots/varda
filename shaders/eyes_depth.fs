/*{
    "DESCRIPTION": "Kinect Eyes - a grid of procedural cartoon eyes that track people seen by a depth sensor. The gaze follows the motion-weighted centroid of whoever is in the sensor's field of view, the lids open as someone approaches, and sudden movement makes the pupils dilate. Requires an attached depth sensor (Kinect v1).",
    "CREDIT": "Varda VJ (eye rendering ported from a Shadertoy 'eyes' generator sketch; cosine palette technique by Inigo Quilez)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "Interactive"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "eyes_x", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 12.0, "LABEL": "Eyes Across"},
        {"NAME": "eyes_y", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 12.0, "LABEL": "Eyes Down"},
        {"NAME": "blink_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Blink Speed"},
        {"NAME": "force_open", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Force Open"},
        {"NAME": "track_amount", "TYPE": "float", "DEFAULT": 0.9, "MIN": 0.0, "MAX": 1.0, "LABEL": "Gaze Tracking"},
        {"NAME": "grain", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.3, "LABEL": "Grain"},
        {"NAME": "wake", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0, "LABEL": "Wake On Presence"},
        {"NAME": "startle", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Startle (Pupil Dilate)"},
        {"NAME": "motion_bias", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Follow Motion vs Body"},
        {"NAME": "gaze_ease", "TYPE": "float", "DEFAULT": 0.12, "MIN": 0.01, "MAX": 1.0, "LABEL": "Gaze Ease"},
        {"NAME": "near_bias", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Prefer Nearer People"},
        {"NAME": "idle_look", "TYPE": "point2D", "DEFAULT": [0.5, 0.5], "LABEL": "Idle Look At"}
    ],
    "PASSES": [
        {"TARGET": "tally", "WIDTH": "32", "HEIGHT": "32", "FLOAT": true},
        {"TARGET": "gaze", "WIDTH": "1", "HEIGHT": "1", "PERSISTENT": true, "FLOAT": true}
    ],
    "PREPROCESSORS": [
        {"NAME": "depth", "TYPE": "depth_sensor"},
        {"NAME": "mask", "TYPE": "depth_sensor"},
        {"NAME": "motion", "TYPE": "depth_sensor"}
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

layout(set = 0, binding = 1) uniform sampler texSampler;

// Pass buffers (declaration order): tally then gaze.
layout(set = 0, binding = 2) uniform texture2D tally;
layout(set = 0, binding = 3) uniform texture2D gaze;

// PREPROCESSOR textures from the depth_sensor preprocessor (declaration order).
layout(set = 0, binding = 4) uniform texture2D depth;
layout(set = 0, binding = 5) uniform texture2D mask;
layout(set = 0, binding = 6) uniform texture2D motion;

layout(set = 0, binding = 7) uniform UserParams {
    float speed;
    float eyes_x;
    float eyes_y;
    float blink_speed;
    float force_open;
    float track_amount;
    float grain;
    float wake;
    float startle;
    float motion_bias;
    float gaze_ease;
    float near_bias;
    vec2 idle_look;
};

// EDGE_SM replaces the original Shadertoy's resolution-relative "smooth"
// constant (renamed since `smooth` is a reserved GLSL interpolation
// qualifier keyword).
#define EDGE_SM (16.0 / RENDERSIZE.x)
#define PI 3.1415926535
#define S(x) smoothstep(-EDGE_SM, EDGE_SM, x)
#define SR(x, y) smoothstep(-EDGE_SM * (y), EDGE_SM * (y), x)

// Kinect v1 is 4:3; decks usually are not. The gaze target is computed in
// sensor UV and mapped back through the same letterbox fit the depth shaders
// use, so the eyes look at where a person actually appears on screen.
const float SENSOR_ASPECT = 4.0 / 3.0;
// Tally grid, matching the PASSES entry. Each texel reduces one block of the
// sensor image; the 1x1 `gaze` pass then reduces the grid to a single target.
const int TALLY = 32;
// Samples per axis within each tally cell.
const int CELL = 6;

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

vec2 fit_scale() {
    float target = RENDERSIZE.x / max(RENDERSIZE.y, 1.0);
    return target > SENSOR_ASPECT
        ? vec2(target / SENSOR_ASPECT, 1.0)
        : vec2(1.0, SENSOR_ASPECT / target);
}

/// Sensor UV -> deck UV, undoing the letterbox fit.
vec2 sensor_to_screen(vec2 s) {
    return (s - 0.5) / fit_scale() + 0.5;
}

// Renders one eye. `fst` is the local eyelid-shaped coordinate for this cell,
// `cst` is the integer cell id (seeds the per-eye randomness), `look` is the
// gaze target relative to this cell's center, and `presence`/`energy` are the
// sensor-derived attention signals.
vec3 eye(vec2 fst, vec2 cst, vec2 look, float t, float presence, float energy) {
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
    // Presence wakes the eyes: an empty field leaves them drowsy, someone
    // stepping into view opens them. This also makes a dead sensor obvious
    // rather than leaving a grid of eyes staring at nothing.
    eyeOpen *= mix(1.0, mix(0.12, 1.0, presence), wake);

    float col = (sin(fst.x) + 1.0) / 2.0;
    float col2 = col * eyeOpen + fst.y * 2.1 - 0.1;
    col = col * eyeOpen - fst.y * 2.1 - 0.1;
    float cs1 = min(col - 0.1, col2 - 0.1);
    float cs2 = S(cs1);
    col = S(min(col, col2));

    float grad = min(eyeOpen * 1.2, 1.0);

    vec2 loc = vec2(fract(fst.x / PI / 2.0 + PI * 2.0) - 0.53, fst.y * RENDERSIZE.y / RENDERSIZE.x);

    // Autonomous random glance, blended toward the tracked target. Tracking is
    // additionally gated on presence so the eyes fall back to idle drifting
    // when there is nobody to follow.
    vec2 pin2 = mix(vec2(cos(pinoise), sin(pinoise)) * ((noise2.y + 1.0) / 2.0),
                     vec2(cos(pinoise2), sin(pinoise2)) * ((noise22.y + 1.0) / 2.0), move);
    pin2 *= 0.25;
    pin2 = mix(pin2, look, track_amount * mix(1.0, presence, wake));

    // Sudden movement dilates the pupil.
    float dilate = 1.0 + energy * startle * 0.9;

    float lloc = length(loc);
    float irisn = mix(1.0, mix(noise2.x, noise22.x, move), 0.25);
    float iris = length(loc - pin2 * (0.5 - lloc));
    float irisWhite = length(loc - pin2 * (0.2 - lloc));
    float irisDark = SR(length(loc - pin2 * (0.4 - lloc)) - 0.05 * irisn * dilate, 0.5);
    float irisShadow = SR(-irisWhite + 0.07 * dilate, 15.0);
    irisWhite = SR(-irisWhite + 0.03 * dilate, 1.4);

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
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w
        + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    float keep = (audioSum + timeSum) * 1e-8;

    // ── Pass 0: tally — reduce the sensor image to a 32x32 grid of partial sums ──
    //
    // Finding where to look is a reduction over the whole sensor image, which a
    // fragment shader cannot do in one step. Doing it naively in the final pass
    // would repeat the same full-image scan for every one of ~3.7M output
    // pixels. Two small passes cost ~110k texture fetches for the entire frame.
    if (PASSINDEX == 0) {
        vec2 cellOrigin = floor(uv * float(TALLY)) / float(TALLY);
        float step_uv = 1.0 / float(TALLY) / float(CELL);

        vec3 acc = vec3(0.0);
        float energy = 0.0;
        for (int b = 0; b < CELL; b++) {
            for (int a = 0; a < CELL; a++) {
                vec2 p = cellOrigin + (vec2(float(a), float(b)) + 0.5) * step_uv;
                float m = texture(sampler2D(mask, texSampler), p).r;
                float d = texture(sampler2D(depth, texSampler), p).r;
                float mo = length(texture(sampler2D(motion, texSampler), p).xy);
                // Nearer subjects command more attention than distant ones.
                float near = d > 0.0 ? 1.0 - d : 0.0;
                float w = m * mix(1.0, 0.25 + 1.5 * near, near_bias)
                        + mo * motion_bias * 2.0;
                acc += vec3(p.x * w, p.y * w, w);
                energy += mo;
            }
        }
        fragColor = vec4(acc + keep, energy);
        return;
    }

    // ── Pass 1: gaze — reduce the grid to one smoothed target ────────────────
    if (PASSINDEX == 1) {
        vec3 acc = vec3(0.0);
        float energy = 0.0;
        for (int j = 0; j < TALLY; j++) {
            for (int i = 0; i < TALLY; i++) {
                vec4 t = texelFetch(sampler2D(tally, texSampler), ivec2(i, j), 0);
                acc += t.xyz;
                energy += t.w;
            }
        }

        float presence = clamp(acc.z * 0.02, 0.0, 1.0);
        // Not `centroid`: that is a reserved GLSL interpolation qualifier, the
        // same trap `smooth` sets in the original eyes.fs.
        vec2 focus = acc.z > 1e-4 ? acc.xy / acc.z : vec2(0.5);
        float energyNorm = clamp(energy * 0.004, 0.0, 1.0);

        vec4 prev = texelFetch(sampler2D(gaze, texSampler), ivec2(0, 0), 0);
        // Held in *sensor* space. `RENDERSIZE` in a sized pass is that pass
        // buffer's dimensions, not the deck's — this pass is 1x1, so the aspect
        // needed to map sensor UV onto the deck is not available here. The
        // conversion, and the idle fallback that is defined in deck space,
        // both happen in the final pass.
        vec2 target = presence > 0.02 ? focus : prev.xy;
        // Ease toward the target so the eyes glide rather than snap, and so a
        // single noisy frame cannot throw the gaze across the room.
        float e = clamp(gaze_ease, 0.01, 1.0);
        vec2 smoothed = mix(prev.xy, target, e);
        float pres = mix(prev.z, presence, 0.08);
        // Energy rises fast and falls slow — a startle should read as a snap
        // followed by a settle, not a symmetric fade.
        float en = energyNorm > prev.w ? mix(prev.w, energyNorm, 0.5) : mix(prev.w, energyNorm, 0.06);

        fragColor = vec4(smoothed + keep, pres, en);
        return;
    }

    // ── Final pass: the eye grid, aimed by the tracked gaze ──────────────────

    vec4 g = texelFetch(sampler2D(gaze, texSampler), ivec2(0, 0), 0);
    float presence = clamp(g.z, 0.0, 1.0);
    float energy = clamp(g.w, 0.0, 1.0);
    // Sensor UV -> deck UV happens here, where RENDERSIZE is the deck's. With
    // nobody in view the tracked point is meaningless, so fade to the idle
    // target rather than letting the eyes lock onto noise.
    vec2 tracked = mix(idle_look, sensor_to_screen(g.xy), smoothstep(0.02, 0.25, presence));

    // Varda's uv is top-left origin (y grows downward); flip y so the eyelid
    // taper and gaze tracking read the same "up" as the original Shadertoy's
    // bottom-left/y-up convention. The gaze target is in deck UV, so it flips
    // with everything else.
    vec2 st = vec2(uv.x, 1.0 - uv.y);
    vec2 look = vec2(tracked.x, 1.0 - tracked.y);
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

    vec2 m1 = look - vec2((cstx + 0.5) / eyes_x, (csty + 0.5) / eyes_y);
    vec2 m2 = look - vec2((cstx2 + 0.5) / eyes_x, (csty2 + 0.5) / eyes_y);

    vec3 col = eye(fst, cst, m1, t, presence, energy);
    vec3 col2 = eye(fst2, cst2, m2, t, presence, energy);
    col = max(col, col2);
    col += grain * (rand((uv * RENDERSIZE) / 3.0 + t) - 0.5);

    col = max(col + keep, 0.0);
    fragColor = vec4(col, 1.0);
}
