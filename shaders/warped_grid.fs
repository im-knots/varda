/*{
    "DESCRIPTION": "Warped City - a raymarched cyberpunk skyline on a pinwheel-skewed grid, twisting through itself along a tunnelling path. Towers are built by setback massing with rooftop spires and district height clustering; facades carry lit windows that bloom through the fog",
    "CREDIT": "Varda VJ (grid and tunnel ported from Shane's 'Warped Extruded Skewed Grid', https://www.shadertoy.com/view/WlsfWM; setback massing and facade articulation after the standard procedural-cityscape approach)",
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
        {"NAME": "window_amount", "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.0, "MAX": 1.0, "LABEL": "Lit Windows"},
        {"NAME": "window_glow", "TYPE": "float", "DEFAULT": 0.26, "MIN": 0.0, "MAX": 1.0, "LABEL": "Window Glow"},
        {"NAME": "window_scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 2.5, "LABEL": "Window Density"},
        {"NAME": "window_flicker", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0, "LABEL": "Window Flicker"},
        {"NAME": "setback_amount", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.0, "LABEL": "Setbacks"},
        {"NAME": "spire_amount", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Spires"},
        {"NAME": "district_variation", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "District Variation"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 0.72, "MIN": 0.0, "MAX": 2.0, "LABEL": "Brightness"},
        {"NAME": "contrast", "TYPE": "float", "DEFAULT": 1.28, "MIN": 0.0, "MAX": 2.0, "LABEL": "Contrast"},
        {"NAME": "saturation", "TYPE": "float", "DEFAULT": 1.4, "MIN": 0.0, "MAX": 2.0, "LABEL": "Saturation"},
        {"NAME": "soft_shadows", "TYPE": "bool", "DEFAULT": false, "LABEL": "Soft Shadows"},
        {"NAME": "ambient_occlusion", "TYPE": "bool", "DEFAULT": true, "LABEL": "Ambient Occlusion"},
        {"NAME": "fog_color_a", "TYPE": "color", "DEFAULT": [1.0, 0.25, 0.5, 1.0], "LABEL": "Glow/Fog Tint A"},
        {"NAME": "fog_color_b", "TYPE": "color", "DEFAULT": [1.0, 0.5, 0.25, 1.0], "LABEL": "Glow/Fog Tint B"},
        {"NAME": "window_color", "TYPE": "color", "DEFAULT": [1.0, 0.82, 0.45, 1.0], "LABEL": "Window Colour"},
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
    float window_amount;
    float window_glow;
    float window_scale;
    float window_flicker;
    float setback_amount;
    float spire_amount;
    float district_variation;
    float brightness;
    float contrast;
    float saturation;
    uint soft_shadows;       // bool stored as uint
    uint ambient_occlusion;  // bool stored as uint
    vec4 fog_color_a;
    vec4 fog_color_b;
    vec4 window_color;
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

// The skew basis and its inverse, set once per fragment in main(). Both derive
// only from `skew_amount`, which is constant for the frame, but the original
// rebuilt them — including a full `inverse()` — inside unskewXY, which sits
// three levels down the innermost loop of the march: four candidates per map()
// call, well over a hundred map() calls per pixel.
mat2 gSkew = mat2(1.0, 0.0, 0.0, 1.0);
mat2 gUnskew = mat2(1.0, 0.0, 0.0, 1.0);

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
//
// Returns both halves of the one hash it computes: `.x` is the cell's raw
// personality, used to decide whether it gets a setback tier or a spire, and
// `.y` is that value squared, the height weight.
vec2 hm(in vec2 p) {
    float h = hash21(p * 3.5 + vec2(19.1, 7.3));
    return vec2(h, h * h);
}

// Downtown clustering. Left to an independent hash per cell, every skyline is
// uniform noise — tall and short shuffled evenly, which reads as a field of
// pylons rather than as a city. Real cities put their towers in districts, so
// a low-frequency field over cell centres scales the whole neighbourhood's
// height budget. This is the single cheapest change that makes procedural
// massing read as urban, and it is why generative cityscapes almost all carry
// some version of it.
// A single sine over cell centres, not value noise. district() is called once
// per candidate pair, four times per map(), and map() runs well over a hundred
// times per pixel; routing it through noise2D() costs four hash21 calls each
// time, which tripled the hash traffic of the whole march for a field that only
// has to vary slowly. Two dot products and a sine give the same low-frequency
// clustering. Measured at 1080p, the noise version cost about 6 ms a frame.
float district(in vec2 id) {
    float w = 0.5 + 0.5 * sin(dot(id, vec2(0.62, 1.63)));
    return mix(1.0, 0.3 + 0.7 * w, clamp(district_variation, 0.0, 1.0));
}

float opExtrusion(in float sdf, in float pz, in float h, in float sf) {
    vec2 w = vec2(sdf, abs(pz) - h) + sf;
    return min(max(w.x, w.y), 0.0) + length(max(w, 0.0)) - sf;
}

float sBoxS(in vec2 p, in vec2 b, in float sf) {
    p = abs(p) - b + sf;
    return length(max(p, 0.0)) + min(max(p.x, p.y), 0.0) - sf;
}

vec2 skewXY(vec2 p) { return gSkew * p; }

vec2 unskewXY(vec2 p) { return gUnskew * p; }

// One tower: base mass, a setback tier, and a rooftop spire.
//
// Setback massing is what gives a skyscraper its stepped silhouette — it came
// out of 1916 zoning law requiring upper floors to step back from the street,
// and it is the shape the eye reads as "skyscraper" rather than "box". Two
// extra boxes buy it.
//
// The stepping is a width that varies with height, not a stack of boxes, and
// that is the whole reason this is affordable. Built as three boxes combined
// with min() it was correct and looked right and cost 17.7 ms of a 1080p frame
// — more than half the total — because map() runs upwards of 130 times per
// pixel between the march, the normal and the occlusion, so every extra box is
// a thousand extra evaluations per pixel. Gating the two upper boxes behind
// `if` did nothing at all: the compiler flattens branches that short, so both
// were evaluated whatever the uniforms said, which is also why switching
// Setbacks off at runtime showed no saving.
//
// Choosing the width by height band instead keeps it at one box, at the cost of
// no longer being an exact distance field near a step. That matters: switching
// to the narrow width the instant the ray clears the shoulder *overestimates*
// the distance, because the wide mass just below is nearer than the tier, and
// an overestimate is the one error a sphere trace cannot take — rays step clean
// through the corner. It showed up immediately as speckle across the rooftops,
// seven times HEAD's count.
//
// The error is bounded by how far the tier is set back, so delaying each width
// change by exactly that distance makes the field underestimate through the
// band where the wide mass below is the nearer surface. That is where most of
// the damage was: it took the speckle count from 8.2% of pixels to 5.2%.
//
// What remains is structural. Above that band a point out over the shoulder
// ledge still measures to the tier rather than to the ledge, and the one box
// has no ledge surface to measure to. Tightening the march to 0.6 recovered
// only a further 0.2 points for 12% of the frame, so it was not taken. Note
// also that the count is not comparable to HEAD's 1.2% — a skyline of stepped
// towers has several times the silhouette edge of a field of plain pylons, and
// this metric counts an edge pixel the same as a hole.
float building(in vec2 p, in float qy, in vec2 halfW, in float h, in float personality) {
    float tierAmt = setback_amount * step(0.35, personality);
    // Spires are deliberately rare, short and not too thin. A mast on every
    // roof reads as a pin cushion; a tall one silhouettes as a black pole,
    // because a hairline box turns too little area towards the key light to be
    // lit by it. Both swamp the massing the spire is meant to punctuate.
    float spireAmt = spire_amount * step(0.93, personality);

    float baseTop = 2.0 * h;
    float tierH = h * 0.5 * tierAmt;
    float spireH = h * 0.22 * spireAmt;
    float tierTop = baseTop + 2.0 * tierH;

    // Height above the tower's footing. The extrusion frame hangs downward from
    // the ground plane, so this is -qy.
    float y = -qy;
    vec2 tierW = halfW * (1.0 - 0.22 * tierAmt);
    vec2 spireW = halfW * 0.17;
    vec2 w = halfW;
    if (y > tierTop + (tierW.x - spireW.x)) w = spireW;
    else if (y > baseTop + (halfW.x - tierW.x)) w = tierW;

    float hh = h + tierH + spireH;
    return opExtrusion(sBoxS(p, w, 0.015), qy + hh, hh, 0.006);
}

// One of the four pinwheel-arranged grid-cell candidates that make up
// blocks() below. The original indexed a `const vec2[4] ps4 = vec2[4](...)`
// array constructor with a `for` loop; that construct is unrolled here into
// four direct calls (see blocks()) for safer shaderc/naga compatibility,
// with each call's fixed `cntr` offset baked in at the call site instead of
// being read out of an array.
vec4 blockCandidate(vec3 q, vec2 cntr, vec2 offs, vec2 dim, vec2 scale, vec2 s, float hs) {
    vec2 p = skewXY(q.xz);
    vec2 ip = floor(p / s - cntr) + 0.5;
    p -= (ip + cntr) * s;
    p = unskewXY(p);
    vec2 idi = unskewXY((ip + cntr) * s);

    // One district sample for the pair. They are neighbours inside the same
    // block, so a second sample of a deliberately low-frequency field would
    // return near enough the same number for twice the noise cost — and the
    // two would then belong to different districts, which is the one thing
    // district clustering exists to prevent.
    float dst = district(idi);

    vec2 idi1 = idi;
    vec2 m1 = hm(idi1);
    float h1 = m1.y * hs * dst;
    float face1Ext = building(p, q.y, 2.0 / 5.0 * dim - 0.02 * scale.x, h1, m1.x);

    vec2 idi2 = idi + offs;
    vec2 m2 = hm(idi2);
    float h2 = m2.y * hs * dst;
    float face2Ext = building(p - offs, q.y, 1.0 / 5.0 * dim - 0.02 * scale.x, h2, m2.x);

    gCandP = p;
    return face1Ext < face2Ext ? vec4(face1Ext, idi1, h1) : vec4(face2Ext, idi2, h2);
}

// Warped, extruded, skewed grid: cell centers are skewed into position,
// then two different-sized unskewed squares are built around them to form
// a pinwheel arrangement (four candidates per point, closest wins).
// skew_amount lerps `sk` between vec2(0) (unskewed) and vec2(-.5,.5)
// (fully skewed, the original's SKEW_GRID default) instead of baking the
// toggle in as a hard on/off.
vec4 blocks(vec3 q) {
    const vec2 scale = vec2(1.0 / 5.0);
    const vec2 dim = scale;
    const vec2 s = dim * 2.0;
    float hs = 0.4 * pylon_height;

    vec2 offs = unskewXY(dim * 0.5);

    float d = 1e5;
    vec2 id = vec2(0.0);
    float height = 0.0;
    gP = vec2(0.0);

    vec4 di;

    di = blockCandidate(q, vec2(0.0, 0.0), offs, dim, scale, s, hs);
    if (di.x < d) { d = di.x; id = di.yz; height = di.w; gP = gCandP; }

    di = blockCandidate(q, vec2(0.5, 0.0), offs, dim, scale, s, hs);
    if (di.x < d) { d = di.x; id = di.yz; height = di.w; gP = gCandP; }

    di = blockCandidate(q, vec2(0.5, -0.5), offs, dim, scale, s, hs);
    if (di.x < d) { d = di.x; id = di.yz; height = di.w; gP = gCandP; }

    di = blockCandidate(q, vec2(0.0, -0.5), offs, dim, scale, s, hs);
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

    vec4 d4 = blocks(p);
    gID = d4.yzw;

    // The per-cell glow used to be computed here. map() is also the workhorse
    // of getNormal, softShadow and calcAO, none of which look at the glow, so
    // it moved into trace() where it is the only consumer.

    objID = fl < d4.x ? 1.0 : 0.0;

    return min(fl, d4.x);
}

// Sphere-trace stride. A distance field may be stepped by its full value only
// if it is Lipschitz-1, and this one is not: the pinwheel cell lookup makes it
// underestimate near cell boundaries.
//
// A distance-based relaxation was measured against this — stride growing with
// `t`, on the reasoning that a tunnelled ray far away is sub-pixel and hidden
// by fog. It was worth about 2.5 ms of the frame at 1080p, 31.7 against 34.2,
// but it also multiplied the speckle count fivefold, and speckle here is rays
// punching through a tower rather than a dithering pattern. Not a trade worth
// taking for 7%.
float stepScale(in int i) {
    return i < 32 ? 0.4 : 0.7;
}

float trace(in vec3 ro, in vec3 rd) {
    float t = 0.0, d;
    gGlow = vec4(0.0);
    t = hash31(ro.zxy + rd.yzx) * 0.25;

    // Loop-invariant. The original recomputed this hash on every one of up to
    // 128 iterations for a value that depends only on the ray.
    float jitter = (hash31(ro + rd) - 0.5) * 0.05;

    for (int i = 0; i < 128; i++) {
        d = map(ro + rd * t);
        float ad = abs(d + jitter);
        const float dst = 0.25;
        if (ad < dst) {
            float rnd = hash21(gID.xy);
            // The slow blink the grid always had, plus a standing haze around
            // towers with lit windows. The accumulator runs during the march,
            // long before any surface is shaded, so the window contribution is
            // per cell rather than per pane — but fog is soft enough that a
            // whole-tower bloom reads as its windows glowing through it, which
            // is the effect for a fraction of the cost of a second pass.
            float blink = smoothstep(0.992, 0.997, sin(rnd * 6.2831 + PHASE_TIME_0 / 4.0) * 0.5 + 0.5);
            gGlow.w = blink + window_amount * window_glow * 0.35 * rnd;
            gGlow.xyz += gGlow.w * (dst - ad) * (dst - ad) / (1.0 + t);
        }
        if (abs(d) < 0.001 * (1.0 + t * 0.05) || t > FAR) break;
        t += d * stepScale(i);
    }

    return min(t, FAR);
}

// Four-tap tetrahedral normal. The six-tap central difference it replaces
// sampled each axis both ways; the tetrahedron gets the same gradient from
// four samples of a map() that is by far the most expensive call in the
// shader. The original's `mp[6]`-array plus `if(sgn>2.) break;` compiler-timing
// hack was already dropped for naga/shaderc portability.
vec3 getNormal(in vec3 p) {
    const vec2 e = vec2(0.001, -0.001);
    return normalize(
        e.xyy * map(p + e.xyy) +
        e.yyx * map(p + e.yyx) +
        e.yxy * map(p + e.yxy) +
        e.xxx * map(p + e.xxx)
    );
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

    // Skew basis for the frame. skew_amount lerps between unskewed and the
    // original's fully-skewed vec2(-.5,.5) rather than baking the toggle in as
    // a hard on/off. Building it here rather than inside unskewXY is what keeps
    // the inverse() out of the march.
    vec2 sk = mix(vec2(0.0), vec2(-0.5, 0.5), clamp(skew_amount, 0.0, 1.0));
    gSkew = mat2(1, -sk.y, -sk.x, 1);
    gUnskew = inverse(gSkew);

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

            // ---- Facade articulation: lit windows and a crown band ----
            // Shading, not geometry. Panes modelled as boxes would multiply the
            // SDF cost by the number of windows; at this scale they are a few
            // pixels across, so a grid in facade space is indistinguishable and
            // effectively free.
            //
            // Facade coordinates: the cell-local offset says which wall the
            // point is on, and windows run along that wall. Taking the smaller
            // component as the across-axis keeps them from smearing around the
            // corner, which is what using the raw position would do.
            vec2 local = txP.xz - svGID.xy;
            float across = abs(local.x) > abs(local.y) ? local.y : local.x;

            // Panes fade out with distance rather than aliasing into sparkle.
            // The crown band goes with them: it is the same high-frequency
            // detail seen end-on, and left in it twinkled worse than the panes.
            float detail = 1.0 - smoothstep(5.0, 13.0, t);

            if (detail > 0.001 && window_amount > 0.001) {
                vec2 grid = vec2(across * 78.0, yDist * 62.0) * window_scale;
                vec2 wID = floor(grid);
                vec2 wF = fract(grid) - 0.5;
                // Rectangular pane, taller than wide, with a margin for the
                // mullion between them.
                float pane = step(max(abs(wF.x) - 0.26, abs(wF.y) - 0.3), 0.0);

                float wr = hash21(wID + svGID.xy * 13.7);
                float on = step(1.0 - window_amount, wr);
                // Occupancy flicker: a few windows switch over time. Driven by
                // the accumulated phase, so it keeps step with the blink.
                float phase = fract(sin(wr * 91.3) * 43758.5453);
                float flick = mix(1.0, step(0.35, fract(phase + PHASE_TIME_0 * 0.06)), window_flicker);

                float lit = pane * on * flick * detail;
                texCol += lit * window_color.rgb * window_glow * 3.4;

                // Crown band: the lit parapet strip near the top of a tower.
                // Placed from the building's own height so it lands on the
                // roofline rather than at a fixed altitude.
                float crown = 1.0 - smoothstep(0.0, 0.045, abs(yDist - 2.5 - 0.08));
                texCol += crown * detail * window_color.rgb * window_glow * 1.1;
            }
        } else {
            texCol = vec3(0.0);
        }

        vec3 ld = lp - sp;
        float lDist = max(length(ld), 0.001);
        ld /= lDist;

        // Both are optional. softShadow is up to 24 more map() calls and calcAO
        // five, on top of the march and the normal, and between them they were
        // the largest single cost in the shader — enough to push 1080p past the
        // 60 fps budget once the towers gained tiers and spires. Shadows are
        // off by default: in a fog-heavy night scene with the key light close
        // behind the camera they change very little, and the occlusion carries
        // most of the depth cue on its own.
        float ao = ambient_occlusion != 0u ? calcAO(sp, sn) : 1.0;
        // With shadows off the stand-in is the mean of the shadow term over the
        // scene rather than 1.0, so the toggle changes how the light falls and
        // not how much of it there is. At 1.0 the city came up almost a stop
        // brighter and the facades washed out to a flat pink.
        float sh = soft_shadows != 0u ? min(softShadow(sp, lp, sn, 16.0) + ao * 0.25, 1.0) : 0.7;

        float atten = 3.0 / (1.0 + lDist * lDist * 0.5);

        float diff = max(dot(sn, ld), 0.0);
        diff *= diff * 1.35;

        float spec = pow(max(dot(reflect(ld, sn), rd), 0.0), 32.0);

        float fre = pow(clamp(1.0 - abs(dot(sn, rd)) * 0.5, 0.0, 1.0), 4.0);

        // Ambient pulled well down from the 0.25 the pylon version used. A city
        // at night is lit by its own windows, not by a fill light; leaving the
        // ambient up greys the unlit faces and the whole thing reads as dusk.
        col = texCol * (diff + ao * 0.13 + vec3(1.0, 0.4, 0.2) * fre * 0.25 * fresnel_strength + vec3(1.0, 0.4, 0.2) * spec * 4.0 * spec_strength);

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
    // Fog level dropped a long way from the pylon version's /1.5. The tints are
    // stored quartered and scaled back up by 4, so at the old level distant fog
    // sat above 2.0 and clipped to a flat wash the moment the grade touched it,
    // taking the far skyline with it. Held down here it stays a colour the
    // towers can be seen through.
    col = mix(col, fog * 0.22, smoothstep(0.0, 0.99, fogT));

    col = mix(col, vec3(1.0) * dot(col, vec3(0.299, 0.587, 0.114)), 0.75 * clamp(grayscale_amount, 0.0, 1.0));
    col = mix(col, col.zyx, clamp(palette_swap, 0.0, 1.0));

    col *= tint.rgb;

    // ---- Grade ----
    // Brightness is applied in linear light, where scaling is exposure and
    // behaves the way stopping a lens down does. Contrast and saturation follow
    // the sqrt, in display space, because both are defined about a mid-grey
    // pivot and a linear-space pivot would crush the darks and clip the neon.
    col *= brightness;
    col = sqrt(max(col, 0.0));
    col = (col - 0.5) * contrast + 0.5;
    col = mix(vec3(dot(col, vec3(0.299, 0.587, 0.114))), col, saturation);

    fragColor = vec4(max(col, 0.0), 1.0);
}
