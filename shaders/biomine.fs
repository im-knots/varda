/*{
    "DESCRIPTION": "Biomine - raymarched biotube lattice (gyroid surfaces) pumping fluid through a mine tunnel, with cellular bump mapping and fake reflective/refractive fluid glow",
    "CREDIT": "Varda VJ (ported from Shane's 'Biomine', https://www.shadertoy.com/view/4lyGzR)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "fov", "TYPE": "float", "DEFAULT": 1.5708, "MIN": 0.5, "MAX": 2.5, "LABEL": "FOV"},
        {"NAME": "tunnel_radius", "TYPE": "float", "DEFAULT": 3.25, "MIN": 2.0, "MAX": 4.5, "LABEL": "Tunnel Radius"},
        {"NAME": "heave_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Biotube Heave"},
        {"NAME": "bump_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Bump Amount"},
        {"NAME": "ao_strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "AO Strength"},
        {"NAME": "translucency", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Translucency"},
        {"NAME": "fresnel_strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Fresnel Strength"},
        {"NAME": "raymarch_steps", "TYPE": "float", "DEFAULT": 72.0, "MIN": 30.0, "MAX": 120.0, "LABEL": "Raymarch Steps"},
        {"NAME": "biotube_color", "TYPE": "color", "DEFAULT": [0.35, 0.25, 0.2, 1.0], "LABEL": "Biotube Color"},
        {"NAME": "wall_color", "TYPE": "color", "DEFAULT": [0.3, 0.3, 0.3, 1.0], "LABEL": "Wall Color"},
        {"NAME": "sky_color", "TYPE": "color", "DEFAULT": [1.0, 0.45, 0.4, 1.0], "LABEL": "Sky Color"},
        {"NAME": "fluid_tint", "TYPE": "color", "DEFAULT": [1.0, 0.1, 0.15, 1.0], "LABEL": "Fluid Tint"}
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
    float fov;
    float tunnel_radius;
    float heave_amount;
    float bump_amount;
    float ao_strength;
    float translucency;
    float fresnel_strength;
    float raymarch_steps;
    vec4 biotube_color;
    vec4 wall_color;
    vec4 sky_color;
    vec4 fluid_tint;
};

#define FAR 50.0

// Object IDs: biotubes = 0, tunnel walls = 1. Mutated inside map() and
// read back out in main() — kept as file-scope globals exactly as the
// original does (harmless in a single fragment invocation).
float objID = 0.0;
float saveID = 0.0;

float hash(float n) { return fract(cos(n) * 45758.5453); }

mat2 rot2(float a) {
    vec2 v = sin(vec2(1.570796, 0.0) + a);
    return mat2(v, -v.y, v.x);
}

// IQ's 3D value noise, compact form.
float noise3D(in vec3 p) {
    const vec3 s = vec3(7, 157, 113);
    vec3 ip = floor(p);
    p -= ip;
    vec4 h = vec4(0.0, s.yz, s.y + s.z) + dot(ip, s);
    p = p * p * (3.0 - 2.0 * p);
    h = mix(fract(sin(h) * 43758.5453), fract(sin(h + s.x) * 43758.5453), p.x);
    h.xy = mix(h.xz, h.yw, p.y);
    return mix(h.x, h.y, p.z);
}

float drawSphere(in vec3 p) {
    p = fract(p) - 0.5;
    return dot(p, p);
}

// Cellular tile: four overlapping spheres on a wrappable cubic tile,
// combined with a first-minus-second-order blend for a beveled
// Voronoi look. Note: the original used a chained comma-operator
// expression here (`v.xy = ..., v.z = ..., v.w = ...;`); split into
// separate statements below since that's the safer/more portable form
// for the shaderc/naga pipeline this repo targets.
float cellTile(in vec3 p) {
    vec4 v, d;
    d.x = drawSphere(p - vec3(0.81, 0.62, 0.53));
    p.xy = vec2(p.y - p.x, p.y + p.x) * 0.7071;
    d.y = drawSphere(p - vec3(0.39, 0.2, 0.11));
    p.yz = vec2(p.z - p.y, p.z + p.y) * 0.7071;
    d.z = drawSphere(p - vec3(0.62, 0.24, 0.06));
    p.xz = vec2(p.z - p.x, p.z + p.x) * 0.7071;
    d.w = drawSphere(p - vec3(0.2, 0.82, 0.64));

    v.xy = min(d.xz, d.yw);
    v.z = min(max(d.x, d.y), max(d.z, d.w));
    v.w = max(v.x, v.y);

    d.x = min(v.z, v.w) - min(v.x, v.y);

    return d.x * 2.66;
}

// The path is a 2D sinusoid the whole scene wraps around.
vec2 path(in float z) {
    float a = sin(z * 0.11);
    float b = cos(z * 0.14);
    return vec2(a * 4.0 - b * 1.5, b * 1.7 + a * 1.5);
}

float smaxP(float a, float b, float s) {
    float h = clamp(0.5 + 0.5 * (a - b) / s, 0.0, 1.0);
    return mix(b, a, h) + h * (1.0 - h) * s;
}

// Distance function: a gyroid lattice forms the biotubes; the tunnel
// is the negative space, bored out with a cylinder and smooth-maxed
// against the gyroid. Everything is wrapped around path().
// iTime -> PHASE_TIME_0 for the heave term so the "speed" parameter
// drives it smoothly instead of jumping on change.
float map(vec3 p) {
    p.xy -= path(p.z);

    p += cos(p.zxy * 1.5707963) * 0.2;

    float d = dot(cos(p * 1.5707963), sin(p.yzx * 1.5707963)) + 1.0;

    float bio = d + 0.25 + dot(sin(p * 1.0 + PHASE_TIME_0 * 6.283 + sin(p.yzx * 0.5)), vec3(0.033 * heave_amount));

    float tun = smaxP(tunnel_radius - length(p.xy - vec2(0, 1)) + 0.5 * cos(p.z * 3.14159 / 32.0), 0.75 - d, 1.0) - abs(1.5 - d) * 0.375;

    objID = step(tun, bio);

    return min(tun, bio);
}

float bumpSurf3D(in vec3 p) {
    float bmp;
    float noi = noise3D(p * 96.0);

    if (saveID > 0.5) {
        float sf = cellTile(p * 0.75);
        float vor = cellTile(p * 1.5);
        bmp = sf * 0.66 + (vor * 0.94 + noi * 0.06) * 0.34;
    } else {
        p /= 3.0;
        float ct = cellTile(p * 2.0 + sin(p * 12.0) * 0.5) * 0.66 + cellTile(p * 6.0 + sin(p * 36.0) * 0.5) * 0.34;
        bmp = (1.0 - smoothstep(-0.2, 0.25, ct)) * 0.9 + noi * 0.1;
    }

    return bmp;
}

vec3 doBumpMap(in vec3 p, in vec3 nor, float bumpfactor) {
    const vec2 e = vec2(0.001, 0);
    float ref = bumpSurf3D(p);
    vec3 grad = (vec3(bumpSurf3D(p - e.xyy), bumpSurf3D(p - e.yxy), bumpSurf3D(p - e.yyx)) - ref) / e.x;

    grad -= nor * dot(nor, grad);

    return normalize(nor + grad * bumpfactor);
}

float trace(in vec3 ro, in vec3 rd) {
    float t = 0.0, h;
    int steps = int(clamp(raymarch_steps, 30.0, 120.0));
    for (int i = 0; i < 120; i++) {
        if (i >= steps) break;
        h = map(ro + rd * t);
        if (abs(h) < 0.002 * (t * 0.125 + 1.0) || t > FAR) break;
        t += step(h, 1.0) * h * 0.2 + h * 0.5;
    }
    return min(t, FAR);
}

vec3 getNormal(in vec3 p) {
    const vec2 e = vec2(0.002, 0);
    return normalize(vec3(map(p + e.xyy) - map(p - e.xyy), map(p + e.yxy) - map(p - e.yxy), map(p + e.yyx) - map(p - e.yyx)));
}

// XT95's cheap SSS-style thickness function.
float thickness(in vec3 p, in vec3 n, float maxDist, float falloff) {
    const float nbIte = 6.0;
    float ao = 0.0;
    for (float i = 1.0; i < nbIte + 0.5; i++) {
        float l = (i * 0.75 + fract(cos(i) * 45758.5453) * 0.25) / nbIte * maxDist;
        ao += (l + map(p - n * l)) / pow(1.0 + l, falloff);
    }
    return clamp(1.0 - ao / nbIte, 0.0, 1.0);
}

float calculateAO(in vec3 p, in vec3 n) {
    float ao = 0.0, l;
    const float maxDist = 4.0;
    const float nbIte = 6.0;
    for (float i = 1.0; i < nbIte + 0.5; i++) {
        l = (i + hash(i)) * 0.5 / nbIte * maxDist;
        ao += (l - map(p + n * l)) / (1.0 + l);
    }
    return clamp(1.0 - ao / nbIte, 0.0, 1.0);
}

// Simple environment mapping: index the reflected/refracted ray into
// cellular noise for a cheap "pumping fluid" look without a real
// reflective/refractive pass. iTime -> PHASE_TIME_0.
vec3 eMap(vec3 rd, vec3 sn) {
    rd.y += PHASE_TIME_0;
    rd /= 3.0;

    float ct = cellTile(rd * 2.0 + sin(rd * 12.0) * 0.5) * 0.66 + cellTile(rd * 6.0 + sin(rd * 36.0) * 0.5) * 0.34;
    vec3 texCol = (vec3(0.25, 0.2, 0.15) * (1.0 - smoothstep(-0.1, 0.3, ct)) + vec3(0.02, 0.02, 0.53) / 6.0);
    return smoothstep(0.0, 1.0, texCol);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); Shadertoy's
    // fragCoord is bottom-left origin (y grows upward). The original
    // maps uv.y directly onto the camera's "up" vector (positive uv.y
    // == screen top == physically up), so flip y before building the
    // screen-space coordinate or the tunnel renders upside down.
    vec2 fragXY = vec2(uv.x, 1.0 - uv.y) * RENDERSIZE;
    vec2 su = (fragXY - RENDERSIZE * 0.5) / RENDERSIZE.y;

    // Camera setup. Camera travel and the biotube heave term both use
    // PHASE_TIME_0 (bound to "speed"), matching the original's shared
    // use of iTime for both.
    vec3 camPos = vec3(0, 1, PHASE_TIME_0 * 2.0);
    vec3 lookAt = camPos + vec3(0, 0, 0.1);

    vec3 lightPos = camPos + vec3(0, 0.5, 5);

    lookAt.xy += path(lookAt.z);
    camPos.xy += path(camPos.z);
    lightPos.xy += path(lightPos.z);

    vec3 forward = normalize(lookAt - camPos);
    vec3 right = normalize(vec3(forward.z, 0.0, -forward.x));
    vec3 up = cross(forward, right);

    vec3 rd = normalize(forward + fov * su.x * right + fov * su.y * up);

    rd.xy = rot2(path(lookAt.z).x / 16.0) * rd.xy;

    float t = trace(camPos, rd);

    saveID = objID;

    vec3 sceneCol = vec3(0);

    if (t < FAR) {
        vec3 sp = t * rd + camPos;
        vec3 sn = getNormal(sp);

        if (saveID > 0.5) sn = doBumpMap(sp, sn, 0.2 * bump_amount);
        else sn = doBumpMap(sp, sn, 0.008 * bump_amount);

        float aoRaw = calculateAO(sp, sn);
        float ao = mix(1.0, aoRaw, ao_strength);

        vec3 ld = lightPos - sp;
        float distlpsp = max(length(ld), 0.001);
        ld /= distlpsp;

        float atten = 1.0 / (1.0 + distlpsp * 0.25);

        float ambience = 0.5;
        float diff = max(dot(sn, ld), 0.0);
        float spec = pow(max(dot(reflect(-ld, sn), -rd), 0.0), 32.0);

        float fre = pow(clamp(dot(sn, rd) + 1.0, 0.0, 1.0), 1.0);

        vec3 texCol;

        if (saveID > 0.5) {
            texCol = wall_color.rgb * (noise3D(sp * 32.0) * 0.66 + noise3D(sp * 64.0) * 0.34) * (1.0 - cellTile(sp * 16.0) * 0.75);
            texCol *= smoothstep(-0.1, 0.5, cellTile(sp * 0.75) * 0.66 + cellTile(sp * 1.5) * 0.34) * 0.85 + 0.15;
        } else {
            vec3 sps = sp / 3.0;
            float ct = cellTile(sps * 2.0 + sin(sps * 12.0) * 0.5) * 0.66 + cellTile(sps * 6.0 + sin(sps * 36.0) * 0.5) * 0.34;
            texCol = biotube_color.rgb * (1.0 - smoothstep(-0.1, 0.25, ct)) + vec3(0.1, 0.01, 0.004);
        }

        vec3 hf = normalize(ld + sn);
        float th = thickness(sp, sn, 1.0, 1.0);
        float tdiff = pow(clamp(dot(rd, -hf), 0.0, 1.0), 1.0);
        float trans = (tdiff + 0.0) * th;
        trans = pow(trans, 4.0);

        float shading = 1.0;

        sceneCol = texCol * (diff + ambience) + vec3(0.7, 0.9, 1.0) * spec;
        if (saveID < 0.5) sceneCol += vec3(0.7, 0.9, 1.0) * spec * spec;
        sceneCol += texCol * vec3(0.8, 0.95, 1.0) * pow(fre, 4.0) * 2.0 * fresnel_strength;
        sceneCol += vec3(1, 0.07, 0.15) * trans * 1.5 * translucency;

        vec3 ref, em;

        if (saveID < 0.5) {
            ref = reflect(rd, sn);
            em = eMap(ref, sn);
            sceneCol += em * 0.5;
            ref = refract(rd, sn, 1.0 / 1.3);
            em = eMap(ref, sn);
            // fluid_tint is stored halved (0..1 picker range); scale
            // back up by 2.0 so the default reproduces the original's
            // vec3(2, .2, .3) refraction tint exactly.
            sceneCol += em * fluid_tint.rgb * 2.0 * 1.5;
        }

        sceneCol *= atten * shading * ao;
    }

    // sky_color is likewise stored halved so the 0..1 color picker can
    // represent the original's over-1.0 vec3(2, .9, .8) sky value.
    vec3 sky = sky_color.rgb * 2.0;
    sceneCol = mix(sky, sceneCol, 1.0 / (t * t / FAR / FAR * 8.0 + 1.0));

    fragColor = vec4(sqrt(clamp(sceneCol, 0.0, 1.0)), 1.0);
}
