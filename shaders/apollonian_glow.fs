/*{
    "DESCRIPTION": "Apollonian Glow - dual raymarched fractal tunnel (Kali-fold + Apollonian sphere inversion) lit entirely by an accumulated glow trail, with a reflection pass",
    "CREDIT": "Varda VJ (ported from Shane's 'Apollonian Glow', https://www.shadertoy.com/view/7XXGDs, glow technique from mrange, https://www.shadertoy.com/view/N3sGWB)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.5, "MAX": 6.0, "LABEL": "Zoom"},
        {"NAME": "depth", "TYPE": "float", "DEFAULT": 10.0, "MIN": 2.0, "MAX": 30.0, "LABEL": "Tunnel Depth"},
        {"NAME": "spin_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -3.0, "MAX": 3.0, "LABEL": "Spin Speed"},
        {"NAME": "fractal_scale", "TYPE": "float", "DEFAULT": 4.0, "MIN": 1.5, "MAX": 8.0, "LABEL": "Fractal Scale"},
        {"NAME": "fractal1_iters", "TYPE": "float", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 8.0, "LABEL": "Kali-Fold Iterations"},
        {"NAME": "fractal2_iters", "TYPE": "float", "DEFAULT": 6.0, "MIN": 2.0, "MAX": 6.0, "LABEL": "Apollonian Iterations"},
        {"NAME": "room_height", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 10.0, "LABEL": "Room Height"},
        {"NAME": "glow_radius", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 2.0, "LABEL": "Glow Ball Radius"},
        {"NAME": "glow_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Glow Intensity"},
        {"NAME": "primary_steps", "TYPE": "float", "DEFAULT": 100.0, "MIN": 20.0, "MAX": 150.0, "LABEL": "Primary Steps"},
        {"NAME": "reflection_steps", "TYPE": "float", "DEFAULT": 42.0, "MIN": 0.0, "MAX": 80.0, "LABEL": "Reflection Steps"},
        {"NAME": "ao_strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "AO Strength"},
        {"NAME": "exposure", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 3.0, "LABEL": "Exposure"},
        {"NAME": "fog_distance", "TYPE": "float", "DEFAULT": 6.0, "MIN": 1.0, "MAX": 20.0, "LABEL": "Fog Distance"},
        {"NAME": "glow_color", "TYPE": "color", "DEFAULT": [1.0, 0.1667, 0.1111, 1.0], "LABEL": "Glow Color"},
        {"NAME": "fog_tint", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Fog Tint"}
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
    float zoom;
    float depth;
    float spin_speed;
    float fractal_scale;
    float fractal1_iters;
    float fractal2_iters;
    float room_height;
    float glow_radius;
    float glow_intensity;
    float primary_steps;
    float reflection_steps;
    float ao_strength;
    float exposure;
    float fog_distance;
    vec4 glow_color;
    vec4 fog_tint;
};

vec3 g_light;

// Cheap, deliberately non-orthonormal "rotation" built from arbitrarily
// phase-shifted cosines (not a true rotation matrix) — kept exactly as
// in the original so the fractal fold pattern matches.
mat2 pseudoRot(float a) {
    return mat2(cos(vec4(a, a + 33.0, a + 11.0, a)));
}

// Iterated abs(sin())-fold ("Kali fractal" style) distance estimator.
float fractal1(vec3 p, int iters) {
    float l = 1.0, w = 0.5;
    for (int i = 0; i < 8; i++) {
        if (i >= iters) break;
        p = abs(sin(p)) - 1.0;
        l = 1.4 / dot(p, p);
        p *= l;
        w *= l;
    }
    return length(p) / w - 0.00235;
}

// Apollonian sphere-inversion fractal — iq
// https://www.shadertoy.com/view/4ds3zn
float fractal2(vec3 p, float t, int iters) {
    float scale = 1.0, r;
    p.xy -= 3.0;
    for (int i = 0; i < 6; i++) {
        if (i >= iters) break;
        p = mod(p - 1.0, 2.0) - 1.0;
        p.xz *= pseudoRot(t);
        r = dot(p, p) * 0.9;
        p /= r;
        scale /= r;
    }
    return 0.45 * min(abs(p.y), length(p.xz)) / scale - 0.0015;
}

// Scene distance field: shifts/spins the whole fractal field (cheaper
// than moving a camera basis), takes the min of the two fractals plus a
// pair of floor/ceiling planes, and — as a side effect on every call —
// accumulates a glow contribution from a small "light ball" at the
// field's local origin. That side-channel accumulation, added up across
// every raymarch/AO/normal sample, is the entire lighting model.
float map(vec3 p, float t) {
    p.z -= depth;
    p.xz *= pseudoRot(t * spin_speed / 3.0);
    float ball = length(p) - glow_radius;
    float f1 = fractal1(p / fractal_scale, int(fractal1_iters)) * fractal_scale;
    float f2 = fractal2(p / fractal_scale, t, int(fractal2_iters)) * fractal_scale;
    float d = min(f1, f2);
    g_light += (glow_color.rgb * 9.0) * glow_intensity / (0.1 + ball * ball);
    return min(room_height - abs(p.y), d);
}

// @iq
float calcAO(vec3 pos, vec3 nor, float t) {
    float sca = 2.0, occ = 0.0;
    for (int i = 0; i < 5; i++) {
        float hr = 0.01 + float(i) * 0.5 / 4.0;
        float dd = map(nor * hr + pos, t);
        occ += (hr - dd) * sca;
        sca *= 0.7;
    }
    return clamp(1.0 - occ, 0.0, 1.0);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); this shader was
    // ported from Shadertoy's bottom-left/y-up convention, so flip y
    // here before building the ray direction.
    vec2 res = RENDERSIZE;
    vec2 fragXY = vec2(uv.x, 1.0 - uv.y) * res;
    vec2 u2 = (fragXY + fragXY - res) / res.y;

    float t = PHASE_TIME_0;
    g_light = vec3(0.0);

    vec3 p = vec3(0.0);
    vec3 dir = normalize(vec3(u2, zoom));

    // Primary march: rather than stopping at a surface hit, this keeps
    // stepping by the distance-field value for a fixed step count and
    // accumulates it every step — a density/fog trail rather than a
    // hard silhouette, which is what gives the piece its glow-through
    // haze look instead of a lit solid surface.
    float s = 0.0;
    float d = 0.0;
    vec3 col = vec3(0.0);
    int steps1 = int(clamp(primary_steps, 1.0, 150.0));
    for (int i = 0; i < 150; i++) {
        if (i >= steps1) break;
        p += dir * s;
        s = map(p, t);
        d += s;
        col += s;
    }
    col *= fog_tint.rgb;
    col += g_light / 10.0;

    // Normal via tetrahedron technique (iq): https://iquilezles.org/articles/normalsSDF/
    const float h = 0.005;
    const vec2 k = vec2(1.0, -1.0);
    vec3 nrm = normalize(
        k.xyy * map(p + k.xyy * h, t) +
        k.yyx * map(p + k.yyx * h, t) +
        k.yxy * map(p + k.yxy * h, t) +
        k.xxx * map(p + k.xxx * h, t)
    );

    // Reflection march, same density-trail technique as the primary
    // march. g_light is reset here so the AO probes below contribute to
    // (only) the reflection-side glow.
    vec3 ref = vec3(0.0);
    p += nrm * 0.05;
    dir = reflect(dir, nrm);
    g_light = vec3(0.0);
    s = 0.0;
    int steps2 = int(clamp(reflection_steps, 0.0, 80.0));
    for (int i = 0; i < 80; i++) {
        if (i >= steps2) break;
        p += dir * s;
        s = map(p, t);
        ref += s;
    }

    float occ = mix(1.0, calcAO(p, nrm, t), ao_strength);
    col *= occ;
    col += col * ref + g_light / 10.0;

    col = tanh(col * exposure / 16.0 * exp(-d / fog_distance));
    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
