/*{
    "DESCRIPTION": "Dull Skull - anatomically detailed skull SDF with jaw animation",
    "CREDIT": "KATUR (CC BY-NC 4.0), ported to Varda ISF",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "rot_speed", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 2.0, "LABEL": "Sway Speed"},
        {"NAME": "sway_range", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Sway Range"},
        {"NAME": "camera_dist", "TYPE": "float", "DEFAULT": 4.5, "MIN": 3.5, "MAX": 9.0, "LABEL": "Camera Distance"},
        {"NAME": "head_turn", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.2, "MAX": 1.2, "LABEL": "Head Turn"},
        {"NAME": "jaw_open", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Mouth Open"},
        {"NAME": "jaw_chatter", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Jaw Chatter"},
        {"NAME": "drift", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Drift"},
        {"NAME": "backdrop_dist", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.0, "MAX": 6.0, "LABEL": "Backdrop"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 3.0, "LABEL": "Brightness"},
        {"NAME": "fresnel_strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Fresnel"},
        {"NAME": "eye_brightness", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 2.0, "LABEL": "Eye Glow"},
        {"NAME": "fog_density", "TYPE": "float", "DEFAULT": 0.0002, "MIN": 0.0, "MAX": 0.001, "LABEL": "Fog Density"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "INDEX": 0, "SCALE": 0.2},
        {"PARAM": "speed", "MULTIPLY_BY": "rot_speed", "INDEX": 1, "SCALE": 1.0}
    ]
}*/

#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME; float TIMEDELTA; uint FRAMEINDEX; int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level; float audio_bass; float audio_mid; float audio_treble;
    float audio_bpm; float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0; float PHASE_TIME_1; float PHASE_TIME_2; float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform UserParams {
    float speed; float rot_speed; float sway_range; float camera_dist;
    float head_turn; float jaw_open; float jaw_chatter; float drift;
    float backdrop_dist; float brightness; float fresnel_strength;
    float eye_brightness; float fog_density; vec4 bg_color;
};

const float PI = 3.141592;
const int MAX_STEPS = 32;
const float MAX_DIST = 24.0;
const float SURF_DIST = 0.005;

// Mean position of the skull in world space — the drift in Transform() swings
// it about this point. The camera orbits and aims here rather than at the world
// origin, which is what keeps the skull framed at every sway angle.
const vec3 PIVOT = vec3(0.0, 0.4, 0.3);

mat2 Rot(float a) { float s = sin(a), c = cos(a); return mat2(c, -s, s, c); }

float sMin(float d1, float d2, float k) {
    float h = clamp(0.5 + 0.5 * (d2 - d1) / k, 0.0, 1.0);
    return mix(d2, d1, h) - k * h * (1.0 - h);
}

float sMax(float d1, float d2, float k) {
    float h = clamp(0.5 - 0.5 * (d2 + d1) / k, 0.0, 1.0);
    return mix(d2, -d1, h) + k * h * (1.0 - h);
}

float Sphere(vec3 p, float s) { return length(p) - s; }

float Ellipsoid(vec3 p, vec3 r) {
    float k0 = length(p / r); float k1 = length(p / (r * r));
    return k0 * (k0 - 1.0) / k1;
}

float rBox(vec3 p, vec3 b, float r) {
    vec3 q = abs(p) - b;
    return length(max(q, 0.0)) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}

float Capsule(vec3 p, vec3 a, vec3 b, float r) {
    vec3 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

float HollowSphere(vec3 p, float r, float h, float t) {
    float w = sqrt(r * r - h * h);
    vec2 q = vec2(length(p.xz), p.y);
    return ((h * q.x < w * q.y) ? length(q - vec2(w, h)) : abs(length(q) - r)) - t;
}

float pModPolar(inout vec2 p, float repetitions) {
    float angle = 2.0 * PI / repetitions;
    float a = atan(p.y, p.x) + angle;
    float r = length(p);
    float c = floor(a / angle);
    a = mod(a, angle) - angle / 2.0;
    p = vec2(cos(a), sin(a)) * r;
    if (abs(c) >= (repetitions / 2.0)) c = abs(c);
    return c;
}

vec3 GetRayDir(vec2 suv, vec3 p, vec3 l, float z) {
    vec3 f = normalize(l - p), r = normalize(cross(vec3(0, 1, 0), f)), u = cross(f, r);
    return normalize(f * z + suv.x * r + suv.y * u);
}

float tFunc(float time) { float tv = 3.0 + time * 0.5; return tv + sin(time * 0.5) * 0.3; }

vec3 Transform(vec3 p, float t) {
    p.y -= 0.4; p.y += sin(t + 1.6) * 0.3 * drift;
    p.z += sin(t * 0.9 - 1.6) * 0.6 * drift - 0.3;
    p.yz *= Rot(sin(-t + 1.0) * 0.3 * drift);
    p.xy *= Rot(cos(-t * 0.7 + 4.0) * 0.4 * drift);
    p.xz *= Rot(sin(t * 0.5) * cos(t * 0.3 + 1.0) * drift + head_turn);
    return p;
}


vec2 map(vec3 p, float time) {
    float t = tFunc(time);
    // Lower-jaw hinge. More negative opens the mouth.
    mat2 ani = Rot(sin(t - 1.7) * 0.2 * jaw_chatter - 0.1 - jaw_open * 0.45);
    vec3 p_skull = Transform(p, t);

    // HEAD
    vec3 p_head = p_skull;
    float d = Ellipsoid(p_head, vec3(0.9, 1.1, 1.2));
    float p_cutb = p_head.y + 0.7 + sin(p_head.x + sin(cos(p_head.z * 1.4)) * 21.0) * 0.02;
    p_cutb = sMin(p_cutb, Ellipsoid(p_head - vec3(0, -0.3, -0.2), vec3(0.7)), 0.05);
    p_cutb = sMin(p_cutb, Ellipsoid(p_head - vec3(0, -0.24, 0.5), vec3(0.51)), 0.1);
    d = sMax(p_cutb, d, 0.05);
    d = sMax(-p_head.z + 1.1, d, 0.2);
    float cuts_temple = Capsule(vec3(-abs(p_head.x), p_head.yz), vec3(-1.0, -1.0, 0.8), vec3(-1.8, 3.0, 0.0), 0.5);
    d = sMax(cuts_temple, d, 0.3);
    d = sMax(Capsule(p_head, vec3(-2.0, -1.1, 0.6), vec3(2.0, -1.1, 0.6), 0.6), d, 0.3);

    // UPPER JAW
    vec3 p_jaw = p_skull - vec3(0, 0.36, 0.1);
    p_jaw.yz *= Rot(PI);
    p_jaw.y -= sin(p_jaw.x * 37.0) * 0.007 - cos(p_jaw.z * 59.0) * 0.001;
    p_jaw.z *= 0.9;
    float ujaw = HollowSphere(p_jaw + vec3(0, -0.95, 0.5), 0.38, 0.02, 0.05);
    ujaw = sMax(p_skull.z - 0.6, ujaw, 0.05);
    vec3 p_jawsc = vec3(abs(p_skull.x), p_skull.yz);
    p_jawsc.xy *= Rot(-1.0); p_jawsc.yz *= Rot(-0.4); p_jawsc.y += 0.3;
    ujaw = sMax(p_jawsc.y, ujaw, 0.04);
    d = sMin(ujaw, d, 0.1);
    d -= sin(10.0 * p_skull.x) * sin(8.0 * p_skull.y) * sin(6.0 * p_skull.z) * 0.03;

    // CHEEKBONES
    vec3 p_eyesur = p_skull - vec3(0, 0.3, 0);
    float eyesur = Ellipsoid(vec3(abs(p_eyesur.x), p_eyesur.yz) + vec3(-0.34, 0.5, -0.87), vec3(0.25, 0.3, 0.2));
    eyesur += sin(12.0 * p_skull.x) * sin(11.0 * p_skull.y) * sin(13.0 * p_skull.z) * 0.02;
    d = sMin(eyesur, d, 0.1);

    // ZYGOMATIC ARCH
    vec3 p_zyg = vec3(abs(p_skull.x), p_skull.yz);
    p_zyg.x += sin(p_zyg.z * 4.0 + PI) * 0.08;
    p_zyg.y += cos(p_zyg.z * 9.0) * 0.03;
    d = sMin(d, Capsule(p_zyg, vec3(0.5, -0.3, 0.8), vec3(0.75, -0.3, 0.1), p_zyg.z * 0.1), 0.06);

    // NOSE
    vec3 p_nbone = p_skull; p_nbone.yz *= Rot(-2.2);
    d = sMin(d, HollowSphere(p_nbone + vec3(0, -1.0, 0.4), 0.1, 0.08, 0.04), 0.05);
    vec3 p_nose = p_skull; p_nose.xy *= Rot(0.25);
    float nose = Ellipsoid(p_nose - vec3(0.04, -0.35, 1.0), vec3(0.03, 0.1, 0.8));
    p_nose.xy *= Rot(-0.4);
    nose = sMin(nose, Ellipsoid(p_nose - vec3(0.02, -0.36, 1.0), vec3(0.04, 0.1, 0.8)), 0.1);
    d = sMax(nose, d, 0.06);
    d = sMax(Ellipsoid(p_nose + vec3(0.0, 0.3, -0.4), vec3(0.1, 0.1, 0.6)), d, 0.1);

    // LOWER JAW
    vec3 pN = p_skull;
    pN.z -= 0.5; pN.y += 0.4; pN.yz *= ani; pN.z += 0.5; pN.y -= 0.4;
    pN -= sin(pN.y * 15.0) * 0.01 - cos(pN.z * 39.0) * 0.002;
    vec3 p_ljaw = pN; p_ljaw.y *= 0.8;
    p_ljaw.z -= sin(pN.y * 26.0) * 0.008;
    p_ljaw.y -= cos(pN.x * 15.0 + sin(pN.y * 7.0) * 2.0) * 0.01;
    float ljaw = HollowSphere(p_ljaw + vec3(0, 0.77, -0.74), 0.38, 0.03, 0.04);
    ljaw = sMax(p_ljaw.z - 0.65, ljaw, 0.1);
    vec3 p_maB = vec3(abs(pN.x), pN.yz);
    p_maB.yz *= Rot(-1.3); p_maB.xz *= Rot(-0.34); p_maB.xy *= Rot(-0.39);
    p_maB -= vec3(0.85, 0.0, 0.63);
    ljaw = sMin(ljaw, rBox(p_maB, vec3(0.0, smoothstep(0.0, 6.0, abs(-p_maB.z) + 0.9), 0.45), 0.04), 0.17);
    ljaw = sMax(Ellipsoid(p_maB - vec3(0.0, 0.0, -0.55), vec3(0.5, 0.15, 0.26)), ljaw, 0.04);
    p_ljaw -= sin(p_ljaw.y * 22.0) * 0.001 - cos(p_ljaw.z * 19.0) * 0.006;
    ljaw = sMax(p_ljaw.y + 0.93, ljaw, 0.02);
    d = sMin(ljaw, d, 0.002);

    // EYE HOLES
    vec3 p_eyeH = p_skull;
    p_eyeH += sin(p_eyeH.x * 29.0 + cos(p_eyeH.y * 32.0)) * 0.005;
    float eyes = Ellipsoid(vec3(abs(p_eyeH.x), p_eyeH.y - 0.4, p_eyeH.z) + vec3(-0.29, 0.49, -1.1), vec3(0.21, 0.25, 0.25));
    float eyeH = sMin(eyes, Sphere(vec3(abs(p_skull.x), p_skull.yz) - vec3(0.25, 0.0, 0.7), 0.35), 0.05);
    d = sMax(sMax(-p_eyeH.y, eyeH, 0.2), d, 0.05);

    // BACKDROP — a half-space the skull melts into. Offset by backdrop_dist so
    // its ripple stays behind the skull instead of periodically swallowing it.
    vec3 pPla = p; pPla.z += sin(p.y * 0.2 - t * 0.7) * 0.5 + backdrop_dist;
    d = sMin(d, pPla.z, 0.8);

    // EYEBALLS
    vec3 p_eye = p_skull; p_eye.x = abs(p_eye.x); p_eye.y -= 0.4;
    p_eye += vec3(-0.29, 0.57, -0.9);
    eyes = Ellipsoid(p_eye, vec3(0.2));

    // UPPER TEETH
    vec3 p_tooth = p_skull - vec3(0, -0.77, 0.7); p_tooth *= vec3(1.2, 1.0, 1.0);
    pModPolar(p_tooth.xz, 32.0);
    float teeth = Ellipsoid(p_tooth - vec3(0.43, 0.0, 0.0), vec3(0.03, 0.15, 0.045));
    teeth = max(teeth, -p_skull.y - 0.73 + sin(p_skull.x * 32.0) * 0.006);
    teeth = max(teeth, -p_skull.z + 0.7);
    teeth = sMax(Sphere(p_skull - vec3(0.02, -0.88, 0.98), 0.23), teeth, 0.01);
    d = min(d, teeth);

    // LOWER TEETH
    vec3 p_ltooth = pN - vec3(0, -0.77, 0.7); p_ltooth *= vec3(1.2, 1.0, 1.0);
    pModPolar(p_ltooth.xz, 32.0);
    float lteeth = Ellipsoid(p_ltooth - vec3(0.42, 0.0, 0.0), vec3(0.03, 0.15, 0.045));
    lteeth = max(lteeth, pN.y + 0.79 + sin(p_skull.x * 29.0) * 0.004);
    lteeth = max(lteeth, -pN.z + 0.7);
    lteeth = sMax(Sphere(pN - vec3(0.005, -0.87, 0.89), 0.24), lteeth, 0.02);
    d = min(d, lteeth);

    vec2 res = vec2(d, 0.0);
    if (eyes < d) res = vec2(eyes, 1.0);
    return res;
}

vec2 RM(vec3 ro, vec3 rd, float time) {
    float t = 0.0; float mat = 0.0;
    for (int i = 0; i < MAX_STEPS; i++) {
        vec3 p = ro + rd * t;
        vec2 h = map(p, time);
        mat = h.y;
        t += h.x;
        if (t > MAX_DIST || abs(h.x) < SURF_DIST) break;
    }
    return vec2(t, mat);
}

vec3 calcNormal(vec3 p, float time) {
    vec3 n = vec3(0.0);
    for (int i = 0; i < 4; i++) {
        vec3 e = 0.5773 * (2.0 * vec3((((i + 3) >> 1) & 1), ((i >> 1) & 1), (i & 1)) - 1.0);
        n += e * map(p + 0.001 * e, time).x;
    }
    return normalize(n);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + DATE.x + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 vUv = vec2(uv.x, 1.0 - uv.y);
    vec2 suv = (vUv - 0.5) * vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0);

    float time = PHASE_TIME_0;
    // Bounded sway rather than a full orbit: PHASE_TIME_1 integrates
    // speed * rot_speed, and the sine caps the swing at sway_range radians so
    // the camera stays in front of the backdrop and the skull stays in frame.
    float rotAngle = sin(PHASE_TIME_1) * sway_range;

    vec3 ro = vec3(0.0, 0.0, camera_dist);
    ro.xz *= Rot(-rotAngle);
    ro.yz *= Rot(sin(time * 0.13) * 0.3);
    ro += PIVOT;
    vec3 rd = GetRayDir(suv, ro, PIVOT, 1.0);

    vec3 col = bg_color.rgb;
    vec2 res = RM(ro, rd, time);
    float d = res.x;
    float mat = res.y;

    if (d < MAX_DIST) {
        vec3 p = ro + rd * d;
        vec3 n = calcNormal(p, time);
        float fresnel = pow(1.0 + dot(rd, n), 2.0);
        col = vec3(0.0);
        col += fresnel * fresnel_strength;
        if (mat == 1.0) col += eye_brightness;
    }

    col = mix(col, bg_color.rgb, 1.0 - exp(-fog_density * d * d * d));
    col *= brightness;
    col = pow(max(col, vec3(0.0)), vec3(0.4545));
    fragColor = vec4(col, 1.0);
}