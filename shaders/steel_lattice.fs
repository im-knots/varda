/*{
    "DESCRIPTION": "Steel Lattice - raymarched gyroid-like lattice of interlocking steel tubes with cellular bump mapping and a subtle blackbody-tinted fire-reflection glow",
    "CREDIT": "Varda VJ (ported from 'Steel Lattice' by Shane)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "look_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Look Rotation Speed"},
        {"NAME": "bump_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Bump Amount"},
        {"NAME": "ao_strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "AO Strength"},
        {"NAME": "shadow_softness", "TYPE": "float", "DEFAULT": 32.0, "MIN": 8.0, "MAX": 64.0, "LABEL": "Shadow Softness"},
        {"NAME": "fire_reflection_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Fire Reflection Amount"},
        {"NAME": "raymarch_steps", "TYPE": "float", "DEFAULT": 128.0, "MIN": 40.0, "MAX": 160.0, "LABEL": "Raymarch Steps"},
        {"NAME": "metal_color_a", "TYPE": "color", "DEFAULT": [0.35, 0.36, 0.38, 1.0], "LABEL": "Metal Color A"},
        {"NAME": "metal_color_b", "TYPE": "color", "DEFAULT": [0.62, 0.63, 0.66, 1.0], "LABEL": "Metal Color B"},
        {"NAME": "diffuse_tint", "TYPE": "color", "DEFAULT": [1.0, 0.97, 0.92, 1.0], "LABEL": "Diffuse Tint"},
        {"NAME": "specular_tint", "TYPE": "color", "DEFAULT": [1.0, 0.9, 0.92, 1.0], "LABEL": "Specular Tint"}
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
    float look_speed;
    float bump_amount;
    float ao_strength;
    float shadow_softness;
    float fire_reflection_amount;
    float raymarch_steps;
    vec4 metal_color_a;
    vec4 metal_color_b;
    vec4 diffuse_tint;
    vec4 specular_tint;
};

#define sEPS 0.005
#define FAR 20.0

float getGrey(vec3 p) { return p.x * 0.299 + p.y * 0.587 + p.z * 0.114; }

float sminP(float a, float b, float smoothing) {
    float h = clamp(0.5 + 0.5 * (b - a) / smoothing, 0.0, 1.0);
    return mix(b, a, h) - smoothing * h * (1.0 - h);
}

mat2 rot(float th) {
    float cs = cos(th), si = sin(th);
    return mat2(cs, -si, si, cs);
}

// -- Procedural tri-planar noise (replaces both iChannel0/iChannel1 texture
// lookups). Varda generators are self-contained with no bound input
// textures, so the original's tex3D(sampler2D, p, n) is rewritten to call a
// hash-based 2D value noise instead of texture(); the n-weighted mix of the
// three axis-projected samples (p.yz / p.zx / p.xy) is preserved exactly, so
// the "wrapped around 3D geometry without stretching" tri-planar look
// carries over unchanged — only the leaf-level 2D lookup changed.
float hash12(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float valueNoise2D(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(a, b, u.x) + (c - a) * u.y * (1.0 - u.x) + (d - b) * u.x * u.y;
}

// Muted metallic grey-ish steel look: mix two grey tones by two octaves of
// value noise. Stands in for the flat iChannel0 texture sample.
vec3 proceduralTex(vec2 p) {
    float n = valueNoise2D(p * 8.0) * 0.6 + valueNoise2D(p * 21.0) * 0.4;
    return mix(metal_color_a.rgb, metal_color_b.rgb, clamp(n, 0.0, 1.0));
}

vec3 tex3D(in vec3 p, in vec3 n) {
    n = max((abs(n) - 0.2) * 7.0, 0.001);
    n /= (n.x + n.y + n.z);
    return proceduralTex(p.yz) * n.x + proceduralTex(p.zx) * n.y + proceduralTex(p.xy) * n.z;
}

vec3 blackbodyPalette(float t) {
    t *= 4000.0;
    float cx = (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t) / (1.0 + 8.42420235e-4 * t + 7.08145163e-7 * t * t);
    float cy = (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t) / (1.0 - 2.89741816e-5 * t + 1.61456053e-7 * t * t);
    float d = (2.0 * cx - 8.0 * cy + 4.0);
    vec3 XYZ = vec3(3.0 * cx / d, 2.0 * cy / d, 1.0 - (3.0 * cx + 2.0 * cy) / d);
    vec3 RGB = mat3(3.240479, -0.969256, 0.055648,
                     -1.537150, 1.875992, -0.204043,
                     -0.498535, 0.041556, 1.057311) * vec3(1.0 / XYZ.y * XYZ.x, 1.0, 1.0 / XYZ.y * XYZ.z);
    return max(RGB, 0.0) * pow(t * 0.0004, 4.0);
}

float bumpSurf3D(in vec3 p, in vec3 n) {
    p = abs(mod(p, 0.0625) - 0.03125);
    float x = min(p.x, min(p.y, p.z)) / 0.03125;
    p = sin(p * 380.0 + sin(p.yzx * 192.0 + 64.0));
    float surfaceNoise = (p.x * p.y * p.z);
    return clamp(x + surfaceNoise * 0.05, 0.0, 1.0);
}

vec3 doBumpMap(in vec3 p, in vec3 nor, float bumpfactor) {
    const float eps = 0.001;
    float ref = bumpSurf3D(p, nor);
    vec3 grad = vec3(bumpSurf3D(vec3(p.x - eps, p.y, p.z), nor) - ref,
                      bumpSurf3D(vec3(p.x, p.y - eps, p.z), nor) - ref,
                      bumpSurf3D(vec3(p.x, p.y, p.z - eps), nor) - ref) / eps;
    grad -= nor * dot(nor, grad);
    return normalize(nor + bumpfactor * grad);
}

float map(vec3 p) {
    p = mod(p, 2.0) - 1.0;
    float x1 = sminP(length(p.xy), sminP(length(p.yz), length(p.xz), 0.25), 0.25) - 0.5;

    p = abs(mod(p, 0.5) - 0.25);
    float x2 = min(p.x, min(p.y, p.z));

    return sqrt(x1 * x1 + x2 * x2) - 0.05;
}

float raymarch(vec3 ro, vec3 rd) {
    float d, t = 0.0;
    int steps = int(clamp(raymarch_steps, 40.0, 160.0));
    for (int i = 0; i < 160; i++) {
        if (i >= steps) break;
        d = map(ro + rd * t);
        if (d < sEPS || t > FAR) break;
        t += d * 0.75;
    }
    return t;
}

float calculateAO(vec3 p, vec3 n) {
    const float AO_SAMPLES = 5.0;
    float r = 0.0, w = 1.0, d;
    for (float i = 1.0; i < AO_SAMPLES + 1.1; i++) {
        d = i / AO_SAMPLES;
        r += w * (d - map(p + n * d));
        w *= 0.5;
    }
    return 1.0 - clamp(r, 0.0, 1.0);
}

float softShadow(vec3 ro, vec3 rd, float start, float end, float k) {
    float shade = 1.0;
    const int maxIterationsShad = 16;
    float dist = start;
    float stepDist = end / float(maxIterationsShad);
    for (int i = 0; i < maxIterationsShad; i++) {
        float h = map(ro + rd * dist);
        shade = min(shade, k * h / dist);
        dist += clamp(h, 0.0005, stepDist * 2.0);
        if (h < 0.001 || dist > end) break;
    }
    return min(max(shade, 0.0) + 0.4, 1.0);
}

vec3 getNormal(in vec3 p) {
    const float eps = 0.001;
    return normalize(vec3(
        map(vec3(p.x + eps, p.y, p.z)) - map(vec3(p.x - eps, p.y, p.z)),
        map(vec3(p.x, p.y + eps, p.z)) - map(vec3(p.x, p.y - eps, p.z)),
        map(vec3(p.x, p.y, p.z + eps)) - map(vec3(p.x, p.y, p.z - eps))
    ));
}

float curve(in vec3 p) {
    vec2 e = vec2(-1.0, 1.0) * 0.05;
    float t1 = map(p + e.yxx), t2 = map(p + e.xxy);
    float t3 = map(p + e.xyx), t4 = map(p + e.yyy);
    return 7.0 * (t1 + t2 + t3 + t4 - 4.0 * map(p));
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); Shadertoy's
    // fragCoord is bottom-left origin (y grows upward). The original builds
    // uv = (fragCoord - iResolution.xy*0.5)/iResolution.y directly from
    // fragCoord, so flip y first or the lattice camera looks upside down.
    vec2 fragXY = vec2(uv.x, 1.0 - uv.y) * RENDERSIZE;
    vec2 su = (fragXY - RENDERSIZE * 0.5) / RENDERSIZE.y;

    vec3 rd = normalize(vec3(su, 0.5));

    // iTime -> PHASE_TIME_0 (bound to "speed") for smooth motion that
    // doesn't jump when the user drags the slider. look_speed additionally
    // scales the rotation rate on top of the phase accumulator.
    rd.xy *= rot(PHASE_TIME_0 * 0.5 * look_speed);
    rd.xz *= rot(PHASE_TIME_0 * 0.25 * look_speed);

    vec3 ro = vec3(0.0, 0.0, PHASE_TIME_0 * 1.0);

    vec3 lp = vec3(0.0, 0.125, -0.125);
    lp.xy *= rot(PHASE_TIME_0 * 0.5 * look_speed);
    lp.xz *= rot(PHASE_TIME_0 * 0.25 * look_speed);
    lp += ro + vec3(0.0, 1.0, 0.0);

    vec3 sceneCol = vec3(0.0);

    float dist = raymarch(ro, rd);

    if (dist < FAR) {
        vec3 sp = ro + rd * dist;
        vec3 sn = getNormal(sp);

        sn = doBumpMap(sp, sn, 0.01 * bump_amount);

        vec3 ld = lp - sp;

        vec3 objCol = tex3D(sp, sn);
        objCol *= bumpSurf3D(sp, sn) * 0.5 + 0.5;

        float lDist = max(length(ld), 0.001);
        ld /= lDist;
        float atten = min(1.0 / (lDist * 0.5 + lDist * lDist * 0.1), 1.0);

        float ambient = 0.25;
        float diffuse = max(0.0, dot(sn, ld));
        float specular = max(0.0, dot(reflect(-ld, sn), -rd));
        specular = pow(specular, 8.0);

        float shadow = softShadow(sp, ld, sEPS * 2.0, lDist, shadow_softness);
        float aoRaw = calculateAO(sp, sn) * 0.5 + 0.5;
        float ao = mix(1.0, aoRaw, ao_strength);

        // FIRE_REFLECTION was a compile-time #ifdef in the original; it's
        // now always computed and scaled by fire_reflection_amount so a VJ
        // can dial the glow in/out live instead of a hard on/off toggle.
        // The iChannel1 noisy-grayscale lookup reuses the same procedural
        // tri-planar noise as objCol, just sampled at different
        // coordinates/scale — the PHASE_TIME_0/64.0 offset keeps it animated.
        vec3 sf = reflect(rd, sn);
        float crv = clamp(curve(sp), 0.0, 1.0);
        float refShade = getGrey(tex3D(sp / 4.0 + PHASE_TIME_0 / 64.0, sf));
        refShade = refShade * 0.4 + max(dot(sf, vec3(0.166)), 0.0);
        vec3 refCol = blackbodyPalette(refShade * (crv * 0.5 + 0.5)) * fire_reflection_amount;

        sceneCol = objCol * (diffuse_tint.rgb * diffuse + ambient) + specular_tint.rgb * specular * 0.75;
        sceneCol += refCol;

        sceneCol *= atten * ao * shadow;
    }

    fragColor = vec4(clamp(sceneCol, 0.0, 1.0), 1.0);
}
