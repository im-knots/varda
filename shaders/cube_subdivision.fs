/*{
    "DESCRIPTION": "Cube Subdivision - an animated cube recursively split into cuboid cells with random holes and raised faces, over a polar tiled floor, with depth of field and bloom",
    "CREDIT": "inspired by Cube Subdivision by Shane",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D"],
    "INPUTS": [
        {"NAME": "speed", "LABEL": "Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "glow", "LABEL": "Glow", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "face_push", "LABEL": "Face Push", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.4},
        {"NAME": "hue_shift", "LABEL": "Hue Shift", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},

        {"NAME": "spin_rate", "LABEL": "Spin Rate", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "turn_duration", "LABEL": "Turn Duration", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 0.25, "MIN": 0.05, "MAX": 1.0},
        {"NAME": "split_rate", "LABEL": "Split Rate", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "wave_rate", "LABEL": "Face Wave Rate", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "turn_flare", "LABEL": "Turn Flare", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "turn_ripple", "LABEL": "Turn Ripple", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "ripple_speed", "LABEL": "Ripple Speed", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 24.0, "MIN": 4.0, "MAX": 60.0},
        {"NAME": "ripple_width", "LABEL": "Ripple Width", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 6.0},
        {"NAME": "ripple_trail", "LABEL": "Ripple Trail", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0},

        {"NAME": "subdivisions", "LABEL": "Subdivisions", "TYPE": "long", "GROUP": "Form", "DEFAULT": 3, "VALUES": [1, 2, 3, 4], "LABELS": ["1", "2", "3", "4"]},
        {"NAME": "split_range", "LABEL": "Split Range", "TYPE": "float", "GROUP": "Form", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 0.9},
        {"NAME": "hole_chance", "LABEL": "Hole Chance", "TYPE": "float", "GROUP": "Form", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "hole_rim", "LABEL": "Hole Rim", "TYPE": "float", "GROUP": "Form", "DEFAULT": 0.025, "MIN": 0.005, "MAX": 0.06},
        {"NAME": "cell_gap", "LABEL": "Cell Gap", "TYPE": "float", "GROUP": "Form", "DEFAULT": 0.0015, "MIN": 0.0, "MAX": 0.02},
        {"NAME": "bevel", "LABEL": "Bevel", "TYPE": "float", "GROUP": "Form", "DEFAULT": 0.005, "MIN": 0.0, "MAX": 0.03},

        {"NAME": "orbit_angle", "LABEL": "Orbit Angle", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159},
        {"NAME": "cam_height", "LABEL": "Height", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.5647, "MIN": 0.05, "MAX": 1.4},
        {"NAME": "cam_distance", "LABEL": "Distance", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 1.0, "MIN": 0.6, "MAX": 2.0},
        {"NAME": "lens_warp", "LABEL": "Lens Warp", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.2, "MIN": 0.1, "MAX": 0.4},

        {"NAME": "light_angle", "LABEL": "Light Angle", "TYPE": "float", "GROUP": "Lighting", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159},
        {"NAME": "shadow_hardness", "LABEL": "Shadow Hardness", "TYPE": "float", "GROUP": "Lighting", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 32.0},
        {"NAME": "ao_strength", "LABEL": "Ambient Occlusion", "TYPE": "float", "GROUP": "Lighting", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "back_light", "LABEL": "Back Light", "TYPE": "float", "GROUP": "Lighting", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "reflection", "LABEL": "Reflections", "TYPE": "float", "GROUP": "Lighting", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0},

        {"NAME": "palette", "LABEL": "Palette", "TYPE": "long", "GROUP": "Palette", "DEFAULT": 0, "VALUES": [0, 1, 2], "LABELS": ["Orange and Pink", "Purple and Green", "Blue and Green"]},
        {"NAME": "glow_cells", "LABEL": "Glowing Cells", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.362, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "ring_glow", "LABEL": "Ring Glow", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "ring_fill", "LABEL": "Ring Fill", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "shell_color", "LABEL": "Shell Color", "TYPE": "color", "GROUP": "Palette", "DEFAULT": [0.349, 0.332, 0.365, 1.0]},

        {"NAME": "focus", "LABEL": "Focus Distance", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0},
        {"NAME": "dof", "LABEL": "Depth of Field", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "bloom", "LABEL": "Bloom", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "vignette", "LABEL": "Vignette", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},

        {"NAME": "render_scale", "LABEL": "Render Scale", "TYPE": "float", "GROUP": "Detail", "DEFAULT": 0.75, "MIN": 0.25, "MAX": 1.0},
        {"NAME": "grime", "LABEL": "Grime", "TYPE": "float", "GROUP": "Detail", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "grime_scale", "LABEL": "Grime Scale", "TYPE": "float", "GROUP": "Detail", "DEFAULT": 1.0, "MIN": 0.25, "MAX": 4.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "MULTIPLY_BY": "spin_rate", "INDEX": 0, "SCALE": 1.0},
        {"PARAM": "speed", "MULTIPLY_BY": "split_rate", "INDEX": 1, "SCALE": 1.0},
        {"PARAM": "speed", "MULTIPLY_BY": "wave_rate", "INDEX": 2, "SCALE": 1.0},
        {"PARAM": "speed", "INDEX": 3, "SCALE": 1.0}
    ],
    "COLUMNS": [
        {"TITLE": "Shape", "GROUPS": ["Motion", "Form"]},
        {"TITLE": "Light", "GROUPS": ["Lighting", "Palette", "Grade"]}
    ],
    "PASSES": [
        {"TARGET": "scene", "FLOAT": true, "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale"},
        {}
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
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D scene;

layout(set = 0, binding = 3) uniform UserParams {
    float speed;
    float glow;
    float face_push;
    float hue_shift;
    float spin_rate;
    float turn_duration;
    float split_rate;
    float wave_rate;
    float turn_flare;
    float turn_ripple;
    float ripple_speed;
    float ripple_width;
    float ripple_trail;
    int subdivisions;
    float split_range;
    float hole_chance;
    float hole_rim;
    float cell_gap;
    float bevel;
    float orbit_angle;
    float cam_height;
    float cam_distance;
    float lens_warp;
    float light_angle;
    float shadow_hardness;
    float ao_strength;
    float back_light;
    float reflection;
    int palette;
    float glow_cells;
    float ring_glow;
    float ring_fill;
    vec4 shell_color;
    float focus;
    float dof;
    float bloom;
    float vignette;
    float render_scale;
    float grime;
    float grime_scale;
};

#define FAR 10.
#define PI 3.14159265357989
#define TAU 6.2831853

const float BLOOM_CAP = 3.0;
const int MAX_SUBDIVISIONS = 4;

mat2 r2(in float a) { float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }

// Hash without Sine by Dave Hoskins.
float hash21(vec2 p) {
    p = fract(p * vec2(623.34, 456.21));
    p += dot(p, p + 145.123);
    return fract(p.x * p.y);
}

float hash31(vec3 p3) {
    p3 = fract(p3 * vec3(.6031, .5030, .4973));
    p3 += dot(p3, p3.zyx + 43.527);
    return fract((p3.x + p3.y) * p3.z);
}

// Integer hash by iq.
const uint HASH_K = 20170906U;

vec3 hash33B(vec3 f) {
    uvec3 x = floatBitsToUint(f);
    x = ((x >> 8U) ^ x.yzx) * HASH_K;
    x = ((x >> 8U) ^ x.yzx) * HASH_K;
    x = ((x >> 8U) ^ x.yzx) * HASH_K;
    return vec3(x) * (1.0 / float(0xffffffffU));
}

vec3 hash33(vec3 p) {
    p = fract(p * vec3(.5031, .6030, .4973));
    p += dot(p, p.yxz + 142.5453);
    return fract((p.xxy + p.yxx) * p.zyx);
}

// Rotate a color around the grey axis.
vec3 hueRotate(vec3 c, float a) {
    const vec3 k = vec3(0.57735);
    float ca = cos(a);
    return max(c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1. - ca), 0.);
}

// Microfacet BRDF.
float GGX_Schlick(float nv, float rough) {
    float r = .5 + .5 * rough;
    float k = (r * r) / 2.;
    float denom = nv * (1. - k) + k;
    return max(nv, .001) / denom;
}

float G_Smith(float nr, float nl, float rough) {
    return GGX_Schlick(nl, rough) * GGX_Schlick(nr, rough);
}

vec3 getSpec(vec3 FS, float nh, float nr, float nl, float rough) {
    float alpha = pow(rough, 4.);
    float b = (nh * nh * (alpha - 1.) + 1.);
    float D = alpha / (3.14159265 * b * b);
    float G = G_Smith(nr, nl, rough);
    return FS * D * G / (4. * max(nr, .001)) * 3.14159265;
}

vec3 getDiff(vec3 FS, float nl, float rough, float type) {
    vec3 diff = nl * (1. - FS);
    return diff * (1. - type);
}

// Value noise, standing in for the original's surface texture and cube map.
float vnoise(vec3 p) {
    vec3 i = floor(p);
    vec3 f = p - i;
    f = f * f * (3. - 2. * f);
    float n000 = hash31(i);
    float n100 = hash31(i + vec3(1, 0, 0));
    float n010 = hash31(i + vec3(0, 1, 0));
    float n110 = hash31(i + vec3(1, 1, 0));
    float n001 = hash31(i + vec3(0, 0, 1));
    float n101 = hash31(i + vec3(1, 0, 1));
    float n011 = hash31(i + vec3(0, 1, 1));
    float n111 = hash31(i + vec3(1, 1, 1));
    return mix(mix(mix(n000, n100, f.x), mix(n010, n110, f.x), f.y),
               mix(mix(n001, n101, f.x), mix(n011, n111, f.x), f.y), f.z);
}

float fbm(vec3 p) {
    return vnoise(p) * .5 + vnoise(p * 2.03) * .25 + vnoise(p * 4.01) * .125 + vnoise(p * 8.07) * .0625;
}

// Grimy metal surface color, returned in linear space.
vec3 surfaceTex(vec3 p) {
    p *= grime_scale;
    float n = fbm(p * 12.);
    float speck = smoothstep(.55, .75, vnoise(p * 60.));
    vec3 c = vec3(.62, .56, .5) * mix(.8, .35 + .9 * n, grime) * (1. - .35 * speck * grime);
    return c * c;
}

vec3 tex3D(in vec3 p, in vec3 n) {
    n = max(n * n - .2, .001);
    n /= dot(n, vec3(1));
    vec3 tx = surfaceTex(vec3(p.zy, 0.));
    vec3 ty = surfaceTex(vec3(p.xz, 7.3));
    vec3 tz = surfaceTex(vec3(p.xy, 13.1));
    return mat3(tx, ty, tz) * n;
}

// Soft overcast environment in linear space.
vec3 envMap(vec3 d) {
    float horizon = smoothstep(-.3, .8, d.y);
    vec3 c = mix(vec3(.28, .3, .22), vec3(.7, .72, .75), horizon);
    c *= .55 + .45 * fbm(d * 3.);
    return c * c;
}

float smax(float a, float b, float k) {
    float f = max(0., 1. - abs(b - a) / max(k, 1e-5));
    return max(a, b) + k * .25 * f * f;
}

const vec3 gScale = vec3(.5);

vec3 a3;

// Six keyframed scenes, each rotating the cube a half turn about one axis.
vec3 objMove(vec3 p) {
    float sceneTotal = 6.;
    float tm = PHASE_TIME_0 / 4.;
    float fT = fract(tm);
    float fNum = mod(floor(tm), sceneTotal);
    float ease = smoothstep(0., turn_duration, fT);

    float aXZ = 0., aYZ = 0., aXY = 0.;
    if (fNum == 0.) {
        aXZ = mix(0., PI, ease);
    } else if (fNum == 1.) {
        aXZ = PI;
        aYZ = mix(0., PI, ease);
    } else if (fNum == 2.) {
        aXZ = PI;
        aYZ = PI;
        aXY = mix(0., PI, ease);
    } else if (fNum == 3.) {
        aXZ = mix(PI, TAU, ease);
        aYZ = PI;
        aXY = PI;
    } else if (fNum == 4.) {
        aYZ = mix(PI, TAU, ease);
        aXY = PI;
    } else {
        aXY = mix(PI, TAU, ease);
    }

    p.xz *= r2(aXZ);
    p.yz *= r2(aYZ);
    p.xy *= r2(aXY);
    a3 = vec3(aXZ, aYZ, aXY);
    return p;
}

float sBoxS(in vec2 p, in vec2 b, in float rf) {
    vec2 d = abs(p) - b + rf;
    return min(max(d.x, d.y), 0.) + length(max(d, 0.)) - rf;
}

float sBoxS(in vec3 p, in vec3 b, in float rf) {
    vec3 d = abs(p) - b + rf;
    return min(max(max(d.x, d.y), d.z), 0.) + length(max(d, 0.)) - rf;
}

vec4 objD;
vec3 gDir;
vec3 gRd;
float gCD;
vec3 gSc;
vec3 gID;
vec3 gP;
vec2 gID2;

float m(vec3 p) {
    float fl = p.y + gScale.y * 2.;
    float smF = .01, ew2 = .07;

    // Polar tiled floor, with two raised rings that turn with the cube.
    if (fl - .05 < 0.) {
        vec2 q2 = p.xz - vec2(-.45, -.9);
        float scA = min(1. / floor(length(q2 * 3.)), 1.);
        vec3 sc2 = vec3(1. / 6., 1. / 6., scA);

        float rN = floor(length(q2 / sc2.x));
        if (rN == 10. || rN == 12.) {
            q2 *= r2(sign(11. - rN) * dot(a3, vec3(1)));
        } else {
            q2 *= r2(PHASE_TIME_3 / 16.);
        }

        vec2 q2O = q2;
        q2 = vec2(length(q2), atan(q2.y, q2.x) / TAU * 6.);

        vec2 iq2 = floor(q2 / sc2.xz);
        if (mod(iq2.x, 2.) == 1.) q2.y += sc2.z / 2.;
        iq2 = floor(q2 / sc2.xz);
        q2 -= (iq2 + .5) * sc2.xz;

        vec3 qq = vec3(q2.x, p.y, q2.y);
        qq.y -= -gScale.y * 2. - sc2.y / 2.;
        if (rN != 10. && rN != 12.) qq.y += .05;
        float bx2;

        float dd2 = sBoxS(qq.xz, sc2.xz / 2., smF / 2.);

        if (iq2.x == 0.) {
            bx2 = length(q2O.xy);
            bx2 = sBoxS(vec2(bx2, qq.y), vec2(sc2.y, sc2.y / 2.), smF);
            iq2 = vec2(0);
        } else {
            bx2 = sBoxS(qq, sc2 / 2., smF * 1.5);
            if (hash21(iq2 + .2) < hole_chance) {
                bx2 = smax(bx2, -(dd2 + ew2), smF);
            }
        }

        fl = min(fl + .75 / 6., bx2 + .004);
        gID2 = iq2;
        objD = vec4(fl, 1e5, 1e5, 1e5);
        gCD = 1e5;
        gP = qq;
        return fl;
    }

    vec3 q = objMove(p);

    // Split the cube along random, animated planes in each axis.
    vec3 sc = gScale;
    vec3 dim = sc;
    vec3 left = -dim / 2., right = dim / 2.;
    vec3 rnd3 = vec3(0);
    vec3 idd = vec3(0);
    float divF = 2.;
    int iter = clamp(subdivisions, 1, MAX_SUBDIVISIONS);

    for (int i = 0; i < MAX_SUBDIVISIONS; i++) {
        if (i >= iter) break;
        rnd3 = idd + (vec3(1, 3, 5) / float(i + 1) * 117.3);
        vec3 rnd3Ani = sin(TAU * rnd3 + PHASE_TIME_1) * .5 * split_range + .5;
        vec3 split = mix(left, right, rnd3Ani);
        vec3 stepLn = step(0., q - split);
        idd += stepLn / divF;
        divF *= 2.;
        left = mix(left, split, stepLn);
        right = mix(split, right, stepLn);
    }

    gID = idd;

    // Cells on the outer faces get pushed out by a moving height field.
    vec3 stepL = step(abs(idd), vec3(1e-3));
    vec3 stepR = step(abs(idd - 1. + 1. / float(1 << iter)), vec3(1e-3));

    vec3 sRndL = mat3x3(0, 1, 1, 1, 0, 1, 1, 1, 0) * mix(left, right, .5);
    sRndL = sin(mod(sRndL * TAU * 2. + PHASE_TIME_2, TAU)) * .5 + .5;
    left -= stepL * sRndL * face_push * sc;
    right += stepR * sRndL * face_push * sc;

    dim = right - left;
    gSc = max(dim / 2. - cell_gap, vec3(1e-4));
    gP = q - mix(left, right, .5);

    float bv = min(bevel, min(min(gSc.x, gSc.y), gSc.z) * .9);
    float d = sBoxS(gP, gSc, bv);

    vec3 bxRnd = hash33(gID + .43);
    if (any(lessThan(bxRnd, vec3(hole_chance)))) {
        float dA = sBoxS(gP.yz, gSc.yz, bv / 2.);
        float dB = sBoxS(gP.xz, gSc.xz, bv / 2.);
        float dC = sBoxS(gP.xy, gSc.xy, bv / 2.);
        if (bxRnd.x < hole_chance) d = smax(d, -(dA + hole_rim), bv);
        if (bxRnd.y < hole_chance) d = smax(d, -(dB + hole_rim), bv);
        if (bxRnd.z < hole_chance) d = smax(d, -(dC + hole_rim), bv);
    }

    // Distance to the next cell wall along the ray, so the march never skips a cell.
    vec3 rC = abs((gDir * dim - gP) / gRd);
    gCD = min(min(rC.x, rC.y), rC.z) + .0001;

    objD = vec4(fl, d, 1e5, 1e5);
    return min(fl, d);
}

float rayMarch(vec3 ro, vec3 rd) {
    float d, t = hash31(ro + rd) * .25;
    vec2 dt = vec2(1e8, 0);

    const int iter = 128;
    int i = 0;

    gRd = objMove(rd);
    gDir = step(0., gRd) - .5;

    for (i = 0; i < iter; i++) {
        d = m(ro + rd * t);
        if (d < dt.x) { dt = vec2(d, t); }
        if (abs(d) < .001 || t > FAR) {
            break;
        }
        t += min(d * .85, gCD);
    }

    if (i == iter - 1) { t = dt.y; }
    return min(t, FAR);
}

float softShadow(vec3 ro, vec3 lp, vec3 n, float k) {
    const int maxIterationsShad = 48;
    ro += n * .0015;
    vec3 rd = lp - ro;
    float shade = 1.;
    float t = 0.;
    float end = max(length(rd), .0001);
    rd /= end;

    gRd = objMove(rd);
    gDir = step(0., gRd) - .5;

    for (int i = 0; i < maxIterationsShad; i++) {
        float d = m(ro + rd * t);
        shade = min(shade, k * d / t);
        if (d < 0. || t > end) break;
        t += clamp(min(d, gCD), .01, .2);
    }
    return max(shade, 0.);
}

vec3 nr(in vec3 p) {
    float sgn = 1.;
    vec3 e = vec3(.001, 0, 0), mp = e.zzz;
    for (int i = 0; i < 6; i++) {
        mp.x += m(p + sgn * e) * sgn;
        sgn = -sgn;
        if ((i & 1) == 1) { mp = mp.yzx; e = e.zxy; }
    }
    return normalize(mp);
}

float cao(in vec3 p, in vec3 n) {
    float sca = 2., occ = 0.;
    for (int i = 0; i < 6; i++) {
        float hr = .01 + float(i) * .25 / 6.;
        float d = m(p + n * hr);
        occ += (hr - d) * sca;
        sca *= .7;
    }
    return clamp(1. - occ, 0., 1.);
}

// Subsurface scattering, after Poisson's "Conetraced Soft Shadows".
float subsurface(vec3 ro, vec3 rd, float ra) {
    const int sN = 10;
    float sss = 0.;
    for (int i = 0; i < sN; i++) {
        float rnd = hash31(ro + float(i)) * .1;
        float d = float(i) * ra * (1. + rnd);
        sss += clamp(m(ro + rd * d) / d, 0., 1.);
    }
    sss /= float(sN);
    return smoothstep(0., 1., sss);
}

// How strongly a floor cell glows, driven by the same clock as the cube's turns.
// The raised rings flare while the cube turns, and each turn sends a wave of
// glow outward across the rings.
float floorGlow(vec2 id) {
    float fT = fract(PHASE_TIME_0 / 4.);
    float x = clamp(fT / turn_duration, 0., 1.);
    float activity = 4. * x * (1. - x);

    float base = (id.x == 10. || id.x == 12.) ? ring_glow * mix(1., activity, turn_flare) : 0.;

    float front = fT * ripple_speed;
    float dr = id.x + hash21(id + .31) * .5 - front;
    float wave = dr > 0.
        ? exp(-dr * dr / (ripple_width * ripple_width))
        : exp(dr / max(ripple_trail * 20., ripple_width));
    // Fade out before the next turn starts a new wave at the center.
    wave *= 1. - smoothstep(.6, 1., fT);
    return max(base, turn_ripple * wave);
}

vec3 glowPalette(vec3 c, vec2 u) {
    if (palette == 1) {
        c = c.yzx;
    } else if (palette == 2) {
        c = mix(c.yzx * .5, c.zyx, 1. - smoothstep(0., 1., u.y + .25));
    }
    return hueRotate(c, hue_shift * TAU);
}

vec3 sphereCam(in vec2 p) {
    float t = 1. / (1. + dot(p, p) / 1.5);
    return vec3(p * t, 2. * t - 1.);
}

vec4 renderScene() {
    vec2 fc = vec2(gl_FragCoord.x, RENDERSIZE.y - gl_FragCoord.y);
    vec2 u = (fc - RENDERSIZE * .5) / RENDERSIZE.y;

    // The default camera matches the original's offset of (0.6, 0.85, 1.2).
    vec3 lk = vec3(.0, -.05, 0);
    float yaw = 0.4636 + orbit_angle;
    float camDist = 1.5882 * cam_distance;
    vec3 o = lk + camDist * vec3(cos(cam_height) * sin(yaw), sin(cam_height), cos(cam_height) * cos(yaw));
    vec3 lightOffset = vec3(-.6, 1, -1);
    lightOffset.xz *= r2(light_angle);
    vec3 l = o + lightOffset;

    vec3 fwd = normalize(lk - o);
    vec3 rgt = normalize(cross(vec3(0, 1, 0), fwd));
    vec3 up = cross(fwd, rgt);
    mat3 mCam = mat3(rgt, up, fwd);
    vec3 r = mCam * sphereCam(u * PI * lens_warp);

    float t = rayMarch(o, r);

    // Capture the hit's identity before the normal and shadow probes overwrite it.
    int objID = objD.x < objD.y ? 0 : 1;
    vec3 svID = gID;
    vec3 svP = gP;
    vec2 svID2 = gID2;

    vec3 p = o + r * t;
    vec3 ld = l - p;
    float lDist = max(length(ld), 1e-5);
    ld /= lDist;

    vec3 sky = envMap(r) * vec3(.012, .01, .008) * 16.;
    vec3 col = sky;

    if (t < FAR) {
        vec3 n = nr(p);
        float sh = softShadow(p, l, n, shadow_hardness);
        float ao = mix(1., cao(p, n) * (.5 + .5 * n.y), ao_strength);
        float atten = 1. / (1. + lDist * lDist * .05);

        int sssID = 0;
        float glowW = 1.;
        vec3 oCol, tx;
        vec3 shell = shell_color.rgb;

        if (objID == 1) {
            vec3 rnd3 = hash33B(svID + .15);
            oCol = .5 + .45 * cos(TAU * rnd3.x / 8. + vec3(0, 1.5, 3));
            oCol = mix(oCol, vec3(1, .7, .3) * vec3(rnd3.x * .1 + .2), .65);
            sssID = 1;

            // Corner cells stay dark, and so does every cell above the glow share.
            float cubeCenter = .5 - .5 / float(1 << clamp(subdivisions, 1, MAX_SUBDIVISIONS));
            if (length(svID - cubeCenter) > 1.6 * cubeCenter) rnd3.z = 0.;
            if (rnd3.z < 1. - glow_cells) {
                oCol = shell * (dot(oCol, vec3(.299, .587, .114)) * .5 + .5);
                sssID = 0;
            } else {
                oCol = glowPalette(oCol, u);
            }
            tx = tex3D(svP, n);
        } else {
            float rnd = hash21(svID2 + .12);
            oCol = .5 + .45 * cos(TAU * rnd / 8. + vec3(0, 1.5, 3));
            oCol = mix(oCol, vec3(1, .7, .3) * vec3(rnd * .1 + .2), .65);

            vec3 shellCol = shell * (dot(oCol, vec3(.299, .587, .114)) * .5 + .5);
            glowW = hash21(svID2 + .16) < 1. - ring_fill ? 0. : floorGlow(svID2);
            oCol = mix(shellCol, glowPalette(oCol, u), glowW);
            if (glowW > .01) sssID = 1;
            tx = tex3D(svP / 2. + hash21(svID2 + .09) * .2, n);
        }

        oCol = mix(oCol.yzx, oCol, smoothstep(.3, .7, u.y + .6));

        float sss = 0.;
        if (sssID == 1) {
            sss = subsurface(p - n * .0015, ld, .05);
            float sssF = 4.;
            if (objID == 0) { sss *= 4.; sssF = 16.; }
            oCol = mix(oCol, oCol.yzx, min(sss / sssF, 1.));
        }

        oCol *= tx * 3.;

        float amb = .5 * pow(length(sin(n * 2.) * .5 + .5), 2.);

        float fresRef = .5;
        float type = .8;
        float rough = dot(tx, vec3(.299, .587, .114)) * 4.;
        if (sssID == 1) type = .25;

        vec3 h = normalize(ld - r);
        float ndl = dot(n, ld);
        float nrv = clamp(dot(n, -r), 0., 1.);
        float nl = clamp(ndl, 0., 1.);
        float nh = clamp(dot(n, h), 0., 1.);
        float vh = clamp(dot(-r, h), 0., 1.);

        vec3 f0 = vec3(.16 * (fresRef * fresRef));
        f0 = mix(f0, oCol, type);
        vec3 FS = f0 + (1. - f0) * pow(1. - vh, 5.);

        vec3 spec = getSpec(FS, nh, nrv, nl, rough);
        vec3 diff = getDiff(FS, nl, rough, type);

        float bl = max(dot(-normalize(vec3(ld.x, 0, ld.z)), n), 0.);
        oCol = oCol + vec3(1, .6, .3) * oCol * bl * 8. * back_light;
        oCol += oCol * sky * (n.y * .35 + .65);

        col = oCol * (diff * sh + amb * (sh * .75 + .25) + vec3(1) * spec * sh);
        col *= atten * ao;

        if (sssID == 1) {
            col += col * oCol * sss * (1. - diff * sh) * 16. * glow * glowW;
        }

        float speR = pow(nh, 8.);
        vec3 rTx = envMap(reflect(r, n));
        float rF = sssID == 0 ? 16. : 2.;
        col = col + col * speR * rTx * rF * reflection;
    }

    col = mix(col, sky, smoothstep(.4, .8, t / FAR));

    // Alpha carries the hit distance for the depth of field pass.
    return vec4(max(col, 0.), t);
}

vec3 depthOfField(vec2 st) {
    float coc = .75;
    float l = abs(texture(sampler2D(scene, texSampler), st).w - focus) - coc;
    float blur = clamp(l / coc, 0., 1.);
    // A little extra blur toward the top and bottom of the frame.
    blur = mix(blur * 2., smoothstep(.2, .6, abs(st.y - .5)) * 2., .5) * dof;

    vec3 acc = vec3(0);
    vec2 iRes = vec2(RENDERSIZE.x / RENDERSIZE.y, 1) * 450.;
    for (int i = 0; i < 25; i++) {
        acc += texture(sampler2D(scene, texSampler), st + (vec2(i / 5, i % 5) - 2.) / iRes * blur).xyz;
    }
    return acc / 25.;
}

vec4 post() {
    vec3 col = depthOfField(uv);

    // Wide, cheap bloom: a jittered golden-angle spiral of taps, nearer taps weighted more.
    float rot = hash21(gl_FragCoord.xy) * TAU;
    float a = 1., w = 1.;
    vec3 colB = vec3(0);
    for (int i = 0; i < 12; i++) {
        float ang = float(i) * 2.39996 + rot;
        float rad = sqrt((float(i) + .5) / 12.) * 28.;
        vec2 off = vec2(cos(ang), sin(ang)) * rad / RENDERSIZE.y;
        off.x *= RENDERSIZE.y / RENDERSIZE.x;
        // Clamped so single specular fireflies don't bloom into sparkles.
        colB += min(texture(sampler2D(scene, texSampler), uv + off).xyz, vec3(BLOOM_CAP)) * w;
        a += w;
        w *= .85;
    }
    colB /= a;
    col += smoothstep(vec3(.05), vec3(1), colB) * bloom;

    float vig = pow(16. * uv.x * uv.y * (1. - uv.x) * (1. - uv.y), 1. / 16.);
    col *= mix(1., vig, vignette);

    return vec4(max(col, 0.), 1.);
}

void main() {
    if (PASSINDEX == 0) {
        fragColor = renderScene();
    } else {
        fragColor = post();
    }
}
