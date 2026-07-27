/*{
    "DESCRIPTION": "Warped Extruded Skewed Grid - raymarched pinwheel-skewed grid of extruded pylons along a warped/twisted tunnel path, in an early-2000s demoscene style, with a per-cell glow-blink trail",
    "CREDIT": "Varda VJ (ported from Shane's 'Warped Extruded Skewed Grid', https://www.shadertoy.com/view/WlsfWM)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "skew_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Skew Amount"},
        {"NAME": "twist_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Twist Amount"},
        {"NAME": "path_amplitude", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Path Amplitude"},
        {"NAME": "pylon_height", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Pylon Height"},
        {"NAME": "glow_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Glow Intensity"},
        {"NAME": "fog_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Fog Amount"},
        {"NAME": "fresnel_strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Fresnel Strength"},
        {"NAME": "spec_strength", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Specular Strength"},
        {"NAME": "grayscale_amount", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Grayscale Amount"},
        {"NAME": "palette_swap", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Palette Swap"},
        {"NAME": "fog_color_a", "TYPE": "color", "DEFAULT": [1.0, 0.25, 0.5, 1.0], "LABEL": "Glow/Fog Tint A"},
        {"NAME": "fog_color_b", "TYPE": "color", "DEFAULT": [1.0, 0.5, 0.25, 1.0], "LABEL": "Glow/Fog Tint B"},
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
    float skew_amount;
    float twist_amount;
    float path_amplitude;
    float pylon_height;
    float glow_intensity;
    float fog_amount;
    float fresnel_strength;
    float spec_strength;
    float grayscale_amount;
    float palette_swap;
    vec4 fog_color_a;
    vec4 fog_color_b;
    vec4 tint;
};

#define FAR 20.0

// File-scope mutable globals mutated inside map()/blocks(), read back out
// in main() — kept as the original does (harmless in a single fragment
// invocation). QUANTIZE_HEIGHTS, FLAT_GRID and PTH_INDPNT_GRD are debug/
// alternate-look toggles the original author left off by default; per the
// porting brief they are baked OFF entirely (their #ifdef branches are
// simply not implemented) rather than exposed as INPUTS, to keep the
// parameter list focused.
float objID = 0.0;
vec3 gID = vec3(0.0);
vec4 gGlow = vec4(0.0);
vec2 gP = vec2(0.0);
vec2 gCandP = vec2(0.0);

mat2 rot2(in float a) { float c = cos(a), s = sin(a); return mat2(c, -s, s, c); }

float hash21(vec2 p) { return fract(sin(dot(p, vec2(27.609, 57.583))) * 43758.5453); }

float hash31(vec3 p) {
    return fract(sin(dot(p, vec3(12.989, 78.233, 57.263))) * 43758.5453);
}

vec2 hash22(vec2 p) {
    return fract(sin(vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)))) * 43758.5453);
}

// Smooth 2D value noise (bilinear-interpolated hash lattice).
float noise2D(vec2 p) {
    vec2 ip = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21(ip);
    float b = hash21(ip + vec2(1.0, 0.0));
    float c = hash21(ip + vec2(0.0, 1.0));
    float d = hash21(ip + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

vec2 path(in float z) {
    return vec2(3.0 * sin(z * 0.1) + 0.5 * cos(z * 0.4), 0.0) * path_amplitude;
}

// Procedural substitute for the original's iChannel0 texture lookup:
//   vec3 tx = texture(iChannel0, p/8.).xyz; return tx*tx;
// Used for the final per-pixel surface color in main() (svGID.xy, called
// once per pixel) — geometry height comes from the cheaper hm() below,
// not from this function (see hm()'s comment for why they were split).
// Varda generators have no bound input image, so the three channels are
// rebuilt from three noise2D() lookups at offset frequencies/phases (so
// they aren't perfectly correlated, like a de-saturated RGB noise
// texture would be), reusing hash21 via noise2D. The final tx*tx
// squaring is kept verbatim from the original (a rough sRGB->linear /
// contrast tweak). The `q = p * 3.5` frequency (multiplying rather than
// the original's `/8.` dividing) is tuned so cells advance close to one
// full noise period each — see hm()'s comment for the full math.
vec3 getTex(in vec2 p) {
    vec2 q = p * 3.5;
    float r = noise2D(q);
    float g = noise2D(q * 1.37 + vec2(19.1, 7.3));
    float b = noise2D(q * 0.71 + vec2(-8.4, 33.2));
    vec3 tx = vec3(r, g, b);
    return tx * tx;
}

// hm() drives pylon *geometry* height and is called from blockCandidate()
// during raymarching/shadows/AO/normals — roughly 100+ times per pixel
// (trace()'s up to 128 steps, getNormal()'s 6 taps, softShadow()'s up to
// 24 steps, calcAO()'s 5 taps, each evaluating map() -> blocks() -> 4x
// blockCandidate() -> 2x hm()). Routing that through getTex()'s 3x
// noise2D() chain (12 hash21 calls per hm(), ~96 per map() call) is
// wasted work: only a single scalar height is needed here, not a full
// RGB triple. getTex() itself is left untouched — it's still used for
// the final per-pixel surface color in main(), called just once per
// pixel, where the extra cost doesn't matter. A single hash + square
// keeps the same low-biased "varied skyline" height distribution
// (squaring skews toward shorter buildings, same as tx*tx did) at a
// fraction of the cost, using the same tuned frequency (q = p * 3.5)
// established when fixing the "all pylons same height" bug.
float hm(in vec2 p) {
    float h = hash21(p * 3.5 + vec2(19.1, 7.3));
    return h * h;
}

float opExtrusion(in float sdf, in float pz, in float h, in float sf) {
    vec2 w = vec2(sdf, abs(pz) - h) + sf;
    return min(max(w.x, w.y), 0.0) + length(max(w, 0.0)) - sf;
}

float sBoxS(in vec2 p, in vec2 b, in float sf) {
    p = abs(p) - b + sf;
    return length(max(p, 0.0)) + min(max(p.x, p.y), 0.0) - sf;
}

vec2 skewXY(vec2 p, vec2 s) {
    return mat2(1, -s.y, -s.x, 1) * p;
}

vec2 unskewXY(vec2 p, vec2 s) {
    return inverse(mat2(1, -s.y, -s.x, 1)) * p;
}

// One of the four pinwheel-arranged grid-cell candidates that make up
// blocks() below. The original indexed a `const vec2[4] ps4 = vec2[4](...)`
// array constructor with a `for` loop; that construct is unrolled here into
// four direct calls (see blocks()) for safer shaderc/naga compatibility,
// with each call's fixed `cntr` offset baked in at the call site instead of
// being read out of an array.
vec4 blockCandidate(vec3 q, vec2 sk, vec2 cntr, vec2 offs, vec2 dim, vec2 scale, vec2 s, float hs) {
    vec2 p = skewXY(q.xz, sk);
    vec2 ip = floor(p / s - cntr) + 0.5;
    p -= (ip + cntr) * s;
    p = unskewXY(p, sk);
    vec2 idi = unskewXY((ip + cntr) * s, sk);

    vec2 idi1 = idi;
    float h1 = hm(idi1) * hs;
    float face1 = sBoxS(p, 2.0 / 5.0 * dim - 0.02 * scale.x, 0.015);
    float face1Ext = opExtrusion(face1, q.y + h1, h1, 0.006);

    vec2 idi2 = idi + offs;
    float h2 = hm(idi2) * hs;
    float face2 = sBoxS(p - offs, 1.0 / 5.0 * dim - 0.02 * scale.x, 0.015);
    float face2Ext = opExtrusion(face2, q.y + h2, h2, 0.006);

    gCandP = p;
    return face1Ext < face2Ext ? vec4(face1Ext, idi1, h1) : vec4(face2Ext, idi2, h2);
}

// Warped, extruded, skewed grid: cell centers are skewed into position,
// then two different-sized unskewed squares are built around them to form
// a pinwheel arrangement (four candidates per point, closest wins).
// skew_amount lerps `sk` between vec2(0) (unskewed) and vec2(-.5,.5)
// (fully skewed, the original's SKEW_GRID default) instead of baking the
// toggle in as a hard on/off.
vec4 blocks(vec3 q, vec2 sk) {
    const vec2 scale = vec2(1.0 / 5.0);
    const vec2 dim = scale;
    const vec2 s = dim * 2.0;
    float hs = 0.4 * pylon_height;

    vec2 offs = unskewXY(dim * 0.5, sk);

    float d = 1e5;
    vec2 id = vec2(0.0);
    float height = 0.0;
    gP = vec2(0.0);

    vec4 di;

    di = blockCandidate(q, sk, vec2(0.0, 0.0), offs, dim, scale, s, hs);
    if (di.x < d) { d = di.x; id = di.yz; height = di.w; gP = gCandP; }

    di = blockCandidate(q, sk, vec2(0.5, 0.0), offs, dim, scale, s, hs);
    if (di.x < d) { d = di.x; id = di.yz; height = di.w; gP = gCandP; }

    di = blockCandidate(q, sk, vec2(0.5, -0.5), offs, dim, scale, s, hs);
    if (di.x < d) { d = di.x; id = di.yz; height = di.w; gP = gCandP; }

    di = blockCandidate(q, sk, vec2(0.0, -0.5), offs, dim, scale, s, hs);
    if (di.x < d) { d = di.x; id = di.yz; height = di.w; gP = gCandP; }

    return vec4(d, id, height);
}

float getTwist(float z) { return z * 0.08 * twist_amount; }

float map(vec3 p) {
    p.xy -= path(p.z);
    p.xy *= rot2(getTwist(p.z));
    p.y = abs(p.y) - 1.25;
    float fl = -p.y + 0.01;

    // PTH_INDPNT_GRD baked OFF: the grid follows the path (the original's
    // default), so the `p.xy += path(p.z)` re-offset is omitted.

    vec2 sk = mix(vec2(0.0), vec2(-0.5, 0.5), clamp(skew_amount, 0.0, 1.0));
    vec4 d4 = blocks(p, sk);
    gID = d4.yzw;

    float rnd = hash21(gID.xy);
    gGlow.w = smoothstep(0.992, 0.997, sin(rnd * 6.2831 + PHASE_TIME_0 / 4.0) * 0.5 + 0.5);

    objID = fl < d4.x ? 1.0 : 0.0;

    return min(fl, d4.x);
}

float trace(in vec3 ro, in vec3 rd) {
    float t = 0.0, d;
    gGlow = vec4(0.0);
    t = hash31(ro.zxy + rd.yzx) * 0.25;

    for (int i = 0; i < 128; i++) {
        d = map(ro + rd * t);
        float ad = abs(d + (hash31(ro + rd) - 0.5) * 0.05);
        const float dst = 0.25;
        if (ad < dst) {
            gGlow.xyz += gGlow.w * (dst - ad) * (dst - ad) / (1.0 + t);
        }
        if (abs(d) < 0.001 * (1.0 + t * 0.05) || t > FAR) break;
        t += i < 32 ? d * 0.4 : d * 0.7;
    }

    return min(t, FAR);
}

// Simplified to a plain 6-tap central-difference normal (matching
// biomine.fs's getNormal). The original used a deliberate
// `mp[6]`-array + `if(sgn>2.) break;` "fake conditional break" as a
// compiler-timing hack from the author; that trick isn't needed for
// correctness and is dropped for naga/shaderc portability.
vec3 getNormal(in vec3 p) {
    const vec2 e = vec2(0.001, 0.0);
    return normalize(vec3(
        map(p + e.xyy) - map(p - e.xyy),
        map(p + e.yxy) - map(p - e.yxy),
        map(p + e.yyx) - map(p - e.yyx)
    ));
}

float softShadow(vec3 ro, vec3 lp, vec3 n, float k) {
    const int iter = 24;
    ro += n * 0.0015;
    vec3 rd = lp - ro;

    float shade = 1.0;
    float t = 0.0;
    float end = max(length(rd), 0.0001);
    rd /= end;

    for (int i = 0; i < iter; i++) {
        float d = map(ro + rd * t);
        shade = min(shade, k * d / t);
        t += clamp(d, 0.01, 0.25);
        if (d < 0.0 || t > end) break;
    }

    return max(shade, 0.0);
}

float calcAO(in vec3 p, in vec3 n) {
    float sca = 3.0, occ = 0.0;
    for (int i = 0; i < 5; i++) {
        float hr = float(i + 1) * 0.15 / 5.0;
        float d = map(p + n * hr);
        occ += (hr - d) * sca;
        sca *= 0.7;
    }
    return clamp(1.0 - occ, 0.0, 1.0);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); Shadertoy's
    // fragCoord is bottom-left origin (y grows upward). This shader builds
    // an explicit camera "up" vector (vec3 up = vec3(sin(a), cos(a), 0))
    // and a lighting rig from screen-space suv, so getting the vertical
    // orientation right matters — flip y before building it, or the scene
    // renders upside down (same fix as biomine.fs/bicycle_day.fs).
    vec2 fragXY = vec2(uv.x, 1.0 - uv.y) * RENDERSIZE;
    vec2 suv = (fragXY - RENDERSIZE * 0.5) / RENDERSIZE.y;

    vec3 ro = vec3(0.0, 0.0, PHASE_TIME_0 * 1.5);
    ro.xy += path(ro.z);
    vec2 roTwist = vec2(0.0, 0.0);
    roTwist *= rot2(-getTwist(ro.z));
    ro.xy += roTwist;

    vec3 lk = vec3(0.0, 0.0, ro.z + 0.25);
    lk.xy += path(lk.z);
    vec2 lkTwist = vec2(0.0, -0.1);
    lkTwist *= rot2(-getTwist(lk.z));
    lk.xy += lkTwist;

    vec3 lp = vec3(0.0, 0.0, ro.z + 3.0);
    lp.xy += path(lp.z);
    vec2 lpTwist = vec2(0.0, -0.3);
    lpTwist *= rot2(-getTwist(lp.z));
    lp.xy += lpTwist;

    float FOV = 1.0;
    float a = getTwist(ro.z);
    a += (path(ro.z).x - path(lk.z).x) / (ro.z - lk.z) / 4.0;
    vec3 fw = normalize(lk - ro);
    vec3 up = vec3(sin(a), cos(a), 0.0);
    vec3 cu = normalize(cross(up, fw));
    vec3 cv = cross(fw, cu);

    vec3 rd = normalize(suv.x * cu + suv.y * cv + fw / FOV);

    float t = trace(ro, rd);

    vec3 svGID = gID;
    float svObjID = objID;
    vec3 svGlow = gGlow.xyz;

    vec3 col = vec3(0.0);

    if (t < FAR) {
        vec3 sp = ro + rd * t;
        vec3 sn = getNormal(sp);

        vec3 texCol;

        vec3 txP = sp;
        txP.xy -= path(txP.z);
        txP.xy *= rot2(getTwist(txP.z));
        // PTH_INDPNT_GRD baked OFF (see map()) — no path re-offset here either.

        if (svObjID < 0.5) {
            vec3 tx = getTex(svGID.xy);
            texCol = smoothstep(-0.5, 1.0, tx) * vec3(1.0, 0.8, 1.8);

            const float lvls = 8.0;

            float yDist = (1.25 + abs(txP.y) + svGID.z * 2.0);
            float hLn = abs(mod(yDist + 0.5 / lvls, 1.0 / lvls) - 0.5 / lvls);
            float hLn2 = abs(mod(yDist + 0.5 / lvls - 0.008, 1.0 / lvls) - 0.5 / lvls);

            if (yDist - 2.5 < 0.25 / lvls) hLn = 1e5;
            if (yDist - 2.5 < 0.25 / lvls) hLn2 = 1e5;

            texCol = mix(texCol, texCol * 2.0, 1.0 - smoothstep(0.0, 0.003, hLn2 - 0.0035));
            texCol = mix(texCol, texCol / 2.5, 1.0 - smoothstep(0.0, 0.003, hLn - 0.0035));

            float fDot = length(txP.xz - svGID.xy) - 0.0086;
            texCol = mix(texCol, texCol * 2.0, 1.0 - smoothstep(0.0, 0.005, fDot - 0.0035));
            texCol = mix(texCol, vec3(0.0), 1.0 - smoothstep(0.0, 0.005, fDot));
        } else {
            texCol = vec3(0.0);
        }

        vec3 ld = lp - sp;
        float lDist = max(length(ld), 0.001);
        ld /= lDist;

        float sh = softShadow(sp, lp, sn, 16.0);
        float ao = calcAO(sp, sn);
        sh = min(sh + ao * 0.25, 1.0);

        float atten = 3.0 / (1.0 + lDist * lDist * 0.5);

        float diff = max(dot(sn, ld), 0.0);
        diff *= diff * 1.35;

        float spec = pow(max(dot(reflect(ld, sn), rd), 0.0), 32.0);

        float fre = pow(clamp(1.0 - abs(dot(sn, rd)) * 0.5, 0.0, 1.0), 4.0);

        col = texCol * (diff + ao * 0.25 + vec3(1.0, 0.4, 0.2) * fre * 0.25 * fresnel_strength + vec3(1.0, 0.4, 0.2) * spec * 4.0 * spec_strength);

        col *= ao * sh * atten;
    }

    // fog_color_a/b are stored quartered (0..1 color-picker range) so the
    // default reproduces the original's over-1.0 vec3(4,1,2)/vec3(4,2,1)
    // glow+fog tint pair exactly; scale back up by 4.0. The same pair
    // drives both the glow tint and the fog tint, as in the original.
    vec3 colA = fog_color_a.rgb * 4.0;
    vec3 colB = fog_color_b.rgb * 4.0;

    svGlow.xyz *= mix(colA, colB, min(svGlow.xyz * 3.5, 1.25));
    col *= 0.25 + svGlow.xyz * 8.0 * glow_intensity;

    vec3 fog = mix(colA, colB, rd.y * 0.5 + 0.5);
    fog = mix(fog, fog.zyx, smoothstep(0.0, 0.35, suv.y - 0.35));
    float fogT = clamp(t * t / FAR / FAR * fog_amount, 0.0, 1.0);
    col = mix(col, fog / 1.5, smoothstep(0.0, 0.99, fogT));

    col = mix(col, vec3(1.0) * dot(col, vec3(0.299, 0.587, 0.114)), 0.75 * clamp(grayscale_amount, 0.0, 1.0));
    col = mix(col, col.zyx, clamp(palette_swap, 0.0, 1.0));

    col *= tint.rgb;

    fragColor = vec4(sqrt(max(col, 0.0)), 1.0);
}
