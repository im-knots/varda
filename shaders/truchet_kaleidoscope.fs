/*{
    "DESCRIPTION": "Truchet + Kaleidoscope - layered truchet patterns viewed through a smoothly-animated kaleidoscope tunnel with color modes and geometry controls",
    "CREDIT": "Varda VJ (ported from mrange's 'Truchet + Kaleidoscope FTW', CC0)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed",          "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.0, "MAX": 3.0,   "LABEL": "Speed"},
        {"NAME": "rotation_speed", "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.0, "MAX": 3.0,   "LABEL": "Rotation Speed"},
        {"NAME": "kaleidoscope",   "TYPE": "float", "DEFAULT": 12.0, "MIN": 2.0, "MAX": 40.0,  "LABEL": "Kaleidoscope Folds"},
        {"NAME": "truchet_radius", "TYPE": "float", "DEFAULT": 0.38, "MIN": 0.1, "MAX": 0.49,  "LABEL": "Truchet Radius"},
        {"NAME": "line_width",     "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.2, "MAX": 3.0,   "LABEL": "Line Width"},
        {"NAME": "plane_count",    "TYPE": "float", "DEFAULT": 6.0,  "MIN": 1.0, "MAX": 10.0,  "LABEL": "Plane Depth"},
        {"NAME": "plane_gap",      "TYPE": "float", "DEFAULT": 0.75, "MIN": 0.2, "MAX": 2.0,   "LABEL": "Plane Spacing"},
        {"NAME": "zoom",           "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.3, "MAX": 3.0,   "LABEL": "Zoom"},
        {"NAME": "path_amplitude", "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.0, "MAX": 3.0,   "LABEL": "Path Wobble"},
        {"NAME": "smooth_edge",    "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0, "MAX": 1.0,   "LABEL": "Edge Smoothness"},
        {"NAME": "color_mode",     "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0, "MAX": 5.0,   "LABEL": "Color Mode (0=BW 1=Custom 2=Rainbow 3=Neon 4=Warm 5=Cool)"},
        {"NAME": "color_a",        "TYPE": "color", "DEFAULT": [1.0, 0.2, 0.4, 1.0], "LABEL": "Color A"},
        {"NAME": "color_b",        "TYPE": "color", "DEFAULT": [0.2, 0.5, 1.0, 1.0], "LABEL": "Color B"},
        {"NAME": "saturation",     "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.0, "MAX": 2.0,   "LABEL": "Saturation"},
        {"NAME": "brightness",     "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.2, "MAX": 3.0,   "LABEL": "Brightness"},
        {"NAME": "vignette_amount","TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.0, "MAX": 2.0,   "LABEL": "Vignette"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed",          "INDEX": 0, "SCALE": 0.25},
        {"PARAM": "rotation_speed", "INDEX": 1, "SCALE": 0.5}
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

layout(set = 0, binding = 1) uniform UserParams {
    float speed;
    float rotation_speed;
    float kaleidoscope;
    float truchet_radius;
    float line_width;
    float plane_count;
    float plane_gap;
    float zoom;
    float path_amplitude;
    float smooth_edge;
    float color_mode;
    vec4 color_a;
    vec4 color_b;
    float saturation;
    float brightness;
    float vignette_amount;
};

// =========================================================================
//  Constants & helpers
// =========================================================================
#define PI  3.141592654
#define TAU (2.0*PI)
#define PCOS(x) (0.5+0.5*cos(x))

float hash1(float co) {
    return fract(sin(co * 12.9898) * 13758.5453);
}

float hash2(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453123);
}

float tanh_approx(float x) {
    float x2 = x * x;
    return clamp(x * (27.0 + x2) / (27.0 + 9.0 * x2), -1.0, 1.0);
}

float pmin(float a, float b, float k) {
    float h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

float pmax(float a, float b, float k) {
    return -pmin(-a, -b, k);
}

float pabs(float a, float k) {
    return pmax(a, -a, k);
}

vec2 toPolar(vec2 p) {
    return vec2(length(p), atan(p.y, p.x));
}

vec2 toRect(vec2 p) {
    return vec2(p.x * cos(p.y), p.x * sin(p.y));
}

float modMirror1(inout float p, float size) {
    float halfsize = size * 0.5;
    float c = floor((p + halfsize) / size);
    p = mod(p + halfsize, size) - halfsize;
    p *= mod(c, 2.0) * 2.0 - 1.0;
    return c;
}

float smoothKaleidoscope(inout vec2 p, float sm, float rep) {
    vec2 hp = p;
    vec2 hpp = toPolar(hp);
    float rn = modMirror1(hpp.y, TAU / rep);
    float sa = PI / rep - pabs(PI / rep - abs(hpp.y), sm);
    hpp.y = sign(hpp.y) * sa;
    hp = toRect(hpp);
    p = hp;
    return rn;
}

// =========================================================================
//  Tunnel path
// =========================================================================
vec3 tunnelOffset(float z) {
    float a = z;
    vec2 p = -0.075 * path_amplitude * (
        vec2(cos(a), sin(a * 1.41421)) +
        vec2(cos(a * 0.86603), sin(a * 0.70711))
    );
    return vec3(p, z);
}

vec3 dtunnelOffset(float z) {
    float eps = 0.1;
    return 0.5 * (tunnelOffset(z + eps) - tunnelOffset(z - eps)) / eps;
}

vec3 ddtunnelOffset(float z) {
    float eps = 0.1;
    return 0.125 * (dtunnelOffset(z + eps) - dtunnelOffset(z - eps)) / eps;
}

// =========================================================================
//  Truchet cell
// =========================================================================
vec2 cell_df(float r, vec2 np, vec2 mp, vec2 off) {
    const vec2 n0 = normalize(vec2(1.0, 1.0));
    const vec2 n1 = normalize(vec2(1.0, -1.0));

    np += off;
    mp -= off;

    float hh = hash2(np);
    float h0 = hh;

    vec2 p0 = abs(mp) - 0.5;
    float d0 = length(p0);
    float d1 = abs(d0 - r);

    float dot0 = dot(n0, mp);
    float dot1 = dot(n1, mp);

    float d2 = abs(dot0);
    d2 = abs(dot1) > 0.70711 ? d0 : d2;

    float d3 = abs(dot1);
    d3 = abs(dot0) > 0.70711 ? d0 : d3;

    float d = d0;
    d = min(d, d1);
    if (h0 > 0.85) {
        d = min(d, d2);
        d = min(d, d3);
    } else if (h0 > 0.5) {
        d = min(d, d2);
    } else if (h0 > 0.15) {
        d = min(d, d3);
    }

    return vec2(d, d0 - r);
}

vec2 truchet_df(float r, vec2 p) {
    vec2 np = floor(p + 0.5);
    vec2 mp = fract(p + 0.5) - 0.5;
    return cell_df(r, np, mp, vec2(0.0));
}

// =========================================================================
//  Color palettes
// =========================================================================
vec3 paletteColor(float t, int mode, vec3 cA, vec3 cB) {
    if (mode == 0) {
        // BW
        return vec3(t);
    } else if (mode == 1) {
        // Custom two-color
        return mix(cA, cB, t);
    } else if (mode == 2) {
        // Rainbow
        return 0.5 + 0.5 * cos(TAU * (t + vec3(0.0, 0.33, 0.67)));
    } else if (mode == 3) {
        // Neon
        return 0.5 + 0.5 * cos(TAU * (t * 0.8 + vec3(0.0, 0.15, 0.35)));
    } else if (mode == 4) {
        // Warm
        vec3 a = vec3(0.1, 0.0, 0.0);
        vec3 b = vec3(1.0, 0.3, 0.0);
        vec3 c = vec3(1.0, 0.9, 0.3);
        if (t < 0.5) return mix(a, b, t * 2.0);
        return mix(b, c, (t - 0.5) * 2.0);
    } else {
        // Cool
        vec3 a = vec3(0.0, 0.0, 0.15);
        vec3 b = vec3(0.0, 0.5, 0.8);
        vec3 c = vec3(0.7, 0.9, 1.0);
        if (t < 0.5) return mix(a, b, t * 2.0);
        return mix(b, c, (t - 0.5) * 2.0);
    }
}

// =========================================================================
//  Plane rendering
// =========================================================================
vec4 renderPlane(vec3 ro, vec3 rd, vec3 pp, vec3 off, float aa, float n) {
    float h_ = hash1(n);
    float h0 = fract(1777.0 * h_);
    float h1 = fract(2087.0 * h_);
    float h2 = fract(2687.0 * h_);
    float h3 = fract(3167.0 * h_);
    float h4 = fract(3499.0 * h_);

    float l = length(pp - ro);

    vec2 p = (pp - off * vec3(1.0, 1.0, 0.0)).xy;
    // Use PHASE_TIME_1 for smooth rotation changes
    p *= mat2(cos(0.5 * (h4 - 0.5) * PHASE_TIME_1), sin(0.5 * (h4 - 0.5) * PHASE_TIME_1),
             -sin(0.5 * (h4 - 0.5) * PHASE_TIME_1), cos(0.5 * (h4 - 0.5) * PHASE_TIME_1));

    float rep = 2.0 * round(mix(3.0, kaleidoscope, h2));
    float sm = smooth_edge * 0.05 * 20.0 / rep;
    float sn = smoothKaleidoscope(p, sm, rep);

    float rotAngle = TAU * h0 + 0.025 * PHASE_TIME_1;
    p *= mat2(cos(rotAngle), sin(rotAngle), -sin(rotAngle), cos(rotAngle));

    float z = mix(0.2, 0.4, h3);
    p /= z;
    p += 0.5 + floor(h1 * 1000.0);

    float tl = tanh_approx(0.33 * l);
    float r = mix(0.30, truchet_radius, PCOS(0.1 * n));
    vec2 d2 = truchet_df(r, p);
    d2 *= z;
    float d = d2.x;
    float lw = 0.025 * z * line_width;
    d -= lw;

    // Coloring
    int cMode = int(floor(color_mode + 0.5));
    float colorT = 0.5 + 0.5 * sin(d2.y * 8.0 + n * 0.3 + PHASE_TIME_0);
    vec3 lineCol = paletteColor(colorT, cMode, color_a.rgb, color_b.rgb);

    vec3 col = mix(lineCol * brightness, vec3(0.0), smoothstep(aa, -aa, d));
    col = mix(col, vec3(0.0), smoothstep(mix(1.0, -0.5, tl), 1.0, sin(PI * 100.0 * d)));
    col = mix(col, vec3(0.0), step(d2.y, 0.0));

    float t = smoothstep(aa, -aa, -d2.y - 3.0 * lw) *
              mix(0.5, 1.0, smoothstep(aa, -aa, -d2.y - lw));

    return vec4(col, t);
}

// =========================================================================
//  Alpha blending
// =========================================================================
vec4 alphaBlendVec4(vec4 back, vec4 front) {
    float w = front.w + back.w * (1.0 - front.w);
    vec3 xyz = (front.xyz * front.w + back.xyz * back.w * (1.0 - front.w)) / max(w, 0.0001);
    return w > 0.0 ? vec4(xyz, w) : vec4(0.0);
}

vec3 alphaBlendVec3(vec3 back, vec4 front) {
    return mix(back, front.xyz, front.w);
}

// =========================================================================
//  Post-processing
// =========================================================================
vec3 postProcess(vec3 col, vec2 q) {
    col = clamp(col, 0.0, 1.0);
    col = pow(col, vec3(1.0 / 2.2));
    col = col * 0.6 + 0.4 * col * col * (3.0 - 2.0 * col);
    // Saturation
    float grey = dot(col, vec3(0.33));
    col = mix(vec3(grey), col, saturation);
    // Vignette
    col *= 0.5 + 0.5 * pow(19.0 * q.x * q.y * (1.0 - q.x) * (1.0 - q.y), 0.7 * vignette_amount);
    return col;
}

// =========================================================================
//  Main scene color
// =========================================================================
vec3 sceneColor(vec3 ww, vec3 uu, vec3 vv, vec3 ro, vec2 p) {
    float lp = length(p);
    vec2 np = p + 1.0 / RENDERSIZE.xy;
    float rdd = 2.0 + tanh_approx(lp);
    rdd /= zoom;
    vec3 rd = normalize(p.x * uu + p.y * vv + rdd * ww);
    vec3 nrd = normalize(np.x * uu + np.y * vv + rdd * ww);

    float planeDist = plane_gap;
    int furthest = int(floor(plane_count + 0.5));
    int fadeFrom = max(furthest - 5, 0);

    float nz = floor(ro.z / planeDist);

    vec3 skyCol = vec3(pow(max(dot(rd, vec3(0.0, 0.0, 1.0)), 0.0), 20.0));

    vec4 acol = vec4(0.0);

    for (int i = 1; i <= 10; ++i) {
        if (i > furthest) break;
        float pz = planeDist * nz + planeDist * float(i);
        float pd = (pz - ro.z) / rd.z;

        if (pd > 0.0 && acol.w < 0.95) {
            vec3 pp = ro + rd * pd;
            vec3 npp = ro + nrd * pd;
            float aa = 3.0 * length(pp - npp);

            vec3 off = tunnelOffset(pp.z);
            vec4 pcol = renderPlane(ro, rd, pp, off, aa, nz + float(i));

            float fadeNz = pp.z - ro.z;
            float fadeIn = smoothstep(planeDist * float(furthest), planeDist * float(fadeFrom), fadeNz);
            float fadeOut = smoothstep(0.0, planeDist * 0.1, fadeNz);
            pcol.xyz = mix(skyCol, pcol.xyz, fadeIn);
            pcol.w *= fadeOut;
            pcol = clamp(pcol, 0.0, 1.0);
            acol = alphaBlendVec4(pcol, acol);
        } else {
            break;
        }
    }

    return alphaBlendVec3(skyCol, acol);
}

// =========================================================================
//  Main
// =========================================================================
void main() {
    // Uniform keeper
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda top-left origin → flip Y
    vec2 fragXY = vec2(uv.x, 1.0 - uv.y) * RENDERSIZE;
    vec2 q = fragXY / RENDERSIZE;
    vec2 p = -1.0 + 2.0 * q;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float tm = PHASE_TIME_0;
    vec3 ro = tunnelOffset(tm);
    vec3 dro = dtunnelOffset(tm);
    vec3 ddro = ddtunnelOffset(tm);

    vec3 ww = normalize(dro);
    vec3 uu = normalize(cross(normalize(vec3(0.0, 1.0, 0.0) + ddro), ww));
    vec3 vv = normalize(cross(ww, uu));

    vec3 col = sceneColor(ww, uu, vv, ro, p);
    col *= smoothstep(0.0, 4.0, TIME);
    col = postProcess(col, q);

    fragColor = vec4(col, 1.0);
}
