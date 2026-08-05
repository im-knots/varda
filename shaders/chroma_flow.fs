/*{
    "DESCRIPTION": "Chroma Flow - warps the previous frame through a drifting, swirling camera and lets the source bleed back in, then grades the result into flat colour groups. The groups are level sets of a smoothly moving field, so they slither over one another and morph like a Deforum animation rather than blurring into smoke.",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "palette_mode", "LABEL": "Manual Palette", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "palette_size", "LABEL": "Palette Size", "TYPE": "float", "DEFAULT": 4.0, "MIN": 2.0, "MAX": 8.0},
        {"NAME": "palette_stability", "LABEL": "Palette Stability", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "regen_seconds", "LABEL": "Regen", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.02, "MAX": 4.0},
        {"NAME": "zoom", "LABEL": "Zoom", "TYPE": "float", "DEFAULT": 1.10, "MIN": 0.5, "MAX": 2.0},
        {"NAME": "rotate", "LABEL": "Rotate", "TYPE": "float", "DEFAULT": 8.0, "MIN": -180.0, "MAX": 180.0},
        {"NAME": "center_x", "LABEL": "Center X", "TYPE": "float", "DEFAULT": 0.5, "MIN": -0.5, "MAX": 1.5},
        {"NAME": "center_y", "LABEL": "Center Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": -0.5, "MAX": 1.5},
        {"NAME": "drift_x", "LABEL": "Drift X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -0.5, "MAX": 0.5},
        {"NAME": "drift_y", "LABEL": "Drift Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -0.5, "MAX": 0.5},
        {"NAME": "warp_amount", "LABEL": "Warp", "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "warp_scale", "LABEL": "Warp Scale", "TYPE": "float", "DEFAULT": 2.5, "MIN": 0.5, "MAX": 8.0},
        {"NAME": "flow_speed", "LABEL": "Warp Speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "group_shear", "LABEL": "Group Shear", "TYPE": "float", "DEFAULT": 0.6, "MIN": -2.0, "MAX": 2.0},
        {"NAME": "trail", "LABEL": "Trail", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.85, "MAX": 1.05},
        {"NAME": "edge_blend_width", "LABEL": "Edge Softness", "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "color_preservation", "LABEL": "Color Preservation", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "barrier_level", "LABEL": "Barrier Darkness", "TYPE": "float", "DEFAULT": 0.12, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "barrier_hardness", "LABEL": "Barrier Hardness", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "mask_radius", "LABEL": "Mask Radius", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.5},
        {"NAME": "mask_x", "LABEL": "Mask X", "TYPE": "float", "DEFAULT": 0.5, "MIN": -0.5, "MAX": 1.5},
        {"NAME": "mask_y", "LABEL": "Mask Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": -0.5, "MAX": 1.5},
        {"NAME": "mask_softness", "LABEL": "Mask Softness", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "mask_invert", "LABEL": "Invert Mask", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "palette_0", "LABEL": "Palette 1", "TYPE": "color", "DEFAULT": [1.0, 0.0, 0.0, 1.0]},
        {"NAME": "palette_1", "LABEL": "Palette 2", "TYPE": "color", "DEFAULT": [0.0, 0.5, 1.0, 1.0]},
        {"NAME": "palette_2", "LABEL": "Palette 3", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.2, 1.0]},
        {"NAME": "palette_3", "LABEL": "Palette 4", "TYPE": "color", "DEFAULT": [1.0, 1.0, 0.0, 1.0]},
        {"NAME": "palette_4", "LABEL": "Palette 5", "TYPE": "color", "DEFAULT": [1.0, 0.0, 1.0, 1.0]},
        {"NAME": "palette_5", "LABEL": "Palette 6", "TYPE": "color", "DEFAULT": [0.0, 1.0, 1.0, 1.0]},
        {"NAME": "palette_6", "LABEL": "Palette 7", "TYPE": "color", "DEFAULT": [1.0, 0.5, 0.0, 1.0]},
        {"NAME": "palette_7", "LABEL": "Palette 8", "TYPE": "color", "DEFAULT": [0.5, 0.0, 1.0, 1.0]},
        {"NAME": "background_fill", "LABEL": "Background Fill", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 0.0]}
    ],
    "PASSES": [
        {"TARGET": "paletteBuf", "PERSISTENT": true, "FLOAT": true, "WIDTH": "8", "HEIGHT": "1"},
        {"TARGET": "flowBuf", "PERSISTENT": true, "FLOAT": true},
        {}
    ],
    "PHASE_INPUTS": [{"PARAM": "flow_speed", "INDEX": 0}]
}*/

// The shape of this effect is MilkDrop's, which has been producing this look
// since 2001, and which Deforum arrived at independently.
//
// MilkDrop splits the work in two, and the split is the whole trick:
//
//   warp shader       samples the *previous frame* at a displaced coordinate and
//                     writes the result back. Its output is the next frame's
//                     input, so anything it does is baked in and compounds.
//   composite shader  draws that buffer to screen. Display only. Never feeds
//                     back, so nothing it does can accumulate.
//
// Its entire warp shader is two lines — sample the last frame slightly off, and
// decay a little. All the motion is in *where* it samples.
//
// Grading the picture into flat colour groups therefore has to happen in the
// composite half. Three earlier builds of this effect quantised inside the
// feedback loop, and every one of them failed the same way: re-quantising
// already-quantised data each frame destroys the smooth gradients that make a
// warp read as motion, and leaves every pixel near a decision boundary flipping
// on noise. That is the flicker.
//
// Kept on the display side, the groups are level sets of a smoothly moving
// continuous field. They slide over each other and change shape as the field
// warps, they are re-derived from scratch every frame so nothing compounds, and
// the field underneath never degrades because nothing ever writes a hard edge
// into it.

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
layout(set = 0, binding = 2) uniform texture2D inputImage;

// Pass 0 target, 8x1: one anchor colour per palette slot, carried between frames.
layout(set = 0, binding = 3) uniform texture2D paletteBuf;
// Pass 1 target, full resolution: the warped, continuous-toned field. Never
// graded, never snapped — only this buffer feeds the next frame.
layout(set = 0, binding = 4) uniform texture2D flowBuf;

layout(set = 0, binding = 5) uniform UserParams {
    uint palette_mode;      // bool stored as uint
    float palette_size;
    float palette_stability;
    float regen_seconds;
    float zoom;
    float rotate;
    float center_x;
    float center_y;
    float drift_x;
    float drift_y;
    float warp_amount;
    float warp_scale;
    float flow_speed;
    float group_shear;
    float trail;
    float edge_blend_width;
    float color_preservation;
    float barrier_level;
    float barrier_hardness;
    float mask_radius;
    float mask_x;
    float mask_y;
    float mask_softness;
    uint mask_invert;       // bool stored as uint
    vec4 palette_0;
    vec4 palette_1;
    vec4 palette_2;
    vec4 palette_3;
    vec4 palette_4;
    vec4 palette_5;
    vec4 palette_6;
    vec4 palette_7;
    vec4 background_fill;
};

#define PI 3.14159265359
#define TAU 6.28318530718
#define MAX_PALETTE 8

float lum(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

vec3 srcAt(vec2 p) { return texture(sampler2D(inputImage, texSampler), p).rgb; }

vec3 fieldAt(vec2 p) { return texture(sampler2D(flowBuf, texSampler), p).rgb; }

// --- Noise primitives ---

vec2 hash2(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}

// Unit-length gradients. Hashing straight into the [0,1) corner values biases
// every gradient into one quadrant, which shows up as a visible diagonal grain
// and gives the warp a boxy, mechanical drift. Taking an angle instead spreads
// the directions evenly.
vec2 grad2(vec2 p) {
    float a = hash2(p).x * TAU;
    return vec2(cos(a), sin(a));
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = dot(grad2(i), f);
    float b = dot(grad2(i + vec2(1.0, 0.0)), f - vec2(1.0, 0.0));
    float c = dot(grad2(i + vec2(0.0, 1.0)), f - vec2(0.0, 1.0));
    float d = dot(grad2(i + vec2(1.0, 1.0)), f - vec2(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm(vec2 p) {
    float v = 0.0, a = 0.5, norm = 0.0;
    for (int i = 0; i < 4; i++) {
        v += a * noise(p);
        norm += a;
        p = p * 2.0 + vec2(100.0);
        a *= 0.5;
    }
    return v / norm;
}

// Warp time accumulates without bound, and feeding it straight into a noise
// coordinate eventually walks the sample point out to where float32 can no
// longer resolve one grid cell from the next, degrading the field into hash
// noise over a long set. Orbiting the sample point instead of translating it
// keeps the coordinate bounded forever. Two incommensurate rates keep the orbit
// from ever exactly repeating.
vec2 timeOrbit(float t) {
    return vec2(cos(t * 0.31), sin(t * 0.37)) * 1.7;
}

// Divergence-free noise. Curl has no sources or sinks, so it stirs the picture
// without pumping material into or out of any region — a plain noise offset
// would pile the image up wherever the field happens to converge.
vec2 flowCurl(vec2 p, float t) {
    vec2 o = timeOrbit(t);
    float e = 0.01;
    float n  = fbm(p + o);
    float nx = fbm(p + vec2(e, 0.0) + o);
    float ny = fbm(p + vec2(0.0, e) + o);
    vec2 v = vec2(ny - n, -(nx - n)) / e;
    // The finite difference divides by its own epsilon, so this comes out around
    // a hundred times longer than a unit vector without the limiter.
    return v / (1.0 + length(v));
}

vec2 rotateVec(vec2 v, float radians_) {
    float c = cos(radians_), s = sin(radians_);
    return vec2(v.x * c - v.y * s, v.x * s + v.y * c);
}

// --- Accessors ---

vec4 getManualPalette(int i) {
    if (i == 0) return palette_0; if (i == 1) return palette_1;
    if (i == 2) return palette_2; if (i == 3) return palette_3;
    if (i == 4) return palette_4; if (i == 5) return palette_5;
    if (i == 6) return palette_6; return palette_7;
}

vec4 readPaletteSlot(int slot) {
    float u = (float(slot) + 0.5) / float(MAX_PALETTE);
    return texture(sampler2D(paletteBuf, texSampler), vec2(u, 0.5));
}

void loadPalette(out vec3 pal[MAX_PALETTE]) {
    // Automatic anchors come from the persistent buffer, which is what keeps
    // them continuous. Manual anchors are read straight from the parameters:
    // they only move when the performer moves them, so they have nothing to
    // settle, and easing them would lag an edit and damp any modulation routed
    // at a palette colour.
    if (palette_mode == 0u) {
        for (int i = 0; i < MAX_PALETTE; i++) pal[i] = readPaletteSlot(i).rgb;
    } else {
        for (int i = 0; i < MAX_PALETTE; i++) pal[i] = getManualPalette(i).rgb;
    }
}

// --- Auto palette extraction ---

#define NUM_CANDIDATES 25

// A candidate stands for a whole region of the picture, so estimating it from a
// single texel makes it track that texel's noise: on live video the value moves
// every frame and the palette inherits the flicker. A short cross average costs
// four extra taps on 25 candidates, once per frame on a tiny target.
vec3 sampleRegion(vec2 c) {
    const float r = 0.045;
    vec3 s = srcAt(c);
    s += srcAt(clamp(c + vec2(r, 0.0), 0.0, 1.0));
    s += srcAt(clamp(c - vec2(r, 0.0), 0.0, 1.0));
    s += srcAt(clamp(c + vec2(0.0, r), 0.0, 1.0));
    s += srcAt(clamp(c - vec2(0.0, r), 0.0, 1.0));
    return s / 5.0;
}

// Greedy farthest-point selection over a 5x5 grid, for maximum colour diversity.
void extractAutoPalette(int numGroups, out vec3 pal[MAX_PALETTE]) {
    vec3 candidates[NUM_CANDIDATES];
    for (int y = 0; y < 5; y++) {
        for (int x = 0; x < 5; x++) {
            vec2 samplePos = vec2(float(x) + 0.5, float(y) + 0.5) / 5.0;
            candidates[y * 5 + x] = sampleRegion(samplePos);
        }
    }

    bool used[NUM_CANDIDATES];
    for (int i = 0; i < NUM_CANDIDATES; i++) used[i] = false;

    // First pick: candidate nearest to image center (index 12 = (2,2) in 5x5)
    pal[0] = candidates[12];
    used[12] = true;

    for (int g = 1; g < MAX_PALETTE; g++) {
        if (g >= numGroups) { pal[g] = vec3(0.0); continue; }
        float bestMinDist = -1.0;
        int bestIdx = 0;
        for (int c = 0; c < NUM_CANDIDATES; c++) {
            if (used[c]) continue;
            float minDist = 1e10;
            for (int p = 0; p < g; p++) {
                vec3 d = candidates[c] - pal[p];
                minDist = min(minDist, dot(d, d));
            }
            if (minDist > bestMinDist) {
                bestMinDist = minDist;
                bestIdx = c;
            }
        }
        pal[g] = candidates[bestIdx];
        used[bestIdx] = true;
    }
}

/// Pass 0. One anchor colour per slot, carried between frames.
///
/// Extraction is a selection — greedy farthest-point over a grid of samples —
/// and a selection is discontinuous by construction. When two candidates are
/// near-tied, a change too small to see swaps which one wins and the anchor for
/// that slot jumps to an unrelated colour, regrading the whole frame at once. So
/// the palette is state, not a per-frame derivation: each slot keeps the anchor
/// nearest to where it already sits, which absorbs a reordered search, and then
/// eases toward it, which turns a genuine jump into a glide.
vec4 palettePass(int numGroups) {
    int slot = clamp(int(floor(uv.x * float(MAX_PALETTE))), 0, MAX_PALETTE - 1);

    vec3 targets[MAX_PALETTE];
    extractAutoPalette(numGroups, targets);

    // Alpha doubles as the "this buffer holds a palette" flag. A persistent
    // target starts cleared, so the first frame has to seed rather than ease out
    // of black.
    vec4 mine = readPaletteSlot(slot);
    if (mine.a < 0.5) return vec4(targets[slot], 1.0);

    vec3 prev[MAX_PALETTE];
    for (int i = 0; i < MAX_PALETTE; i++) prev[i] = readPaletteSlot(i).rgb;

    // Confident pairs first, rather than slot 0 first.
    //
    // Walking the slots in index order lets slot 0 take whichever anchor it
    // likes and leaves the last slot with whatever is left, which may be nothing
    // like where it currently sits. That mis-pairing is worst exactly when the
    // anchors lag furthest behind the candidates, so the smoothing meant to
    // steady the palette was instead handing it a target that jumped. Taking the
    // closest available pair each round makes the assignment independent of slot
    // order, so a lagging slot can no longer be displaced by a luckier one.
    bool claimed[MAX_PALETTE];
    bool assigned[MAX_PALETTE];
    for (int i = 0; i < MAX_PALETTE; i++) { claimed[i] = false; assigned[i] = false; }

    vec3 chosen = targets[slot];
    for (int round = 0; round < MAX_PALETTE; round++) {
        if (round >= numGroups) break;
        int bestSlot = -1;
        int bestCand = -1;
        float bestDist = 1e10;
        for (int s = 0; s < MAX_PALETTE; s++) {
            if (s >= numGroups || assigned[s]) continue;
            for (int c = 0; c < MAX_PALETTE; c++) {
                if (c >= numGroups || claimed[c]) continue;
                vec3 d = targets[c] - prev[s];
                float dd = dot(d, d);
                if (dd < bestDist) { bestDist = dd; bestSlot = s; bestCand = c; }
            }
        }
        if (bestSlot < 0) break;
        assigned[bestSlot] = true;
        claimed[bestCand] = true;
        if (bestSlot == slot) chosen = targets[bestCand];
    }

    // Ease in real time, so the settle looks the same at any frame rate.
    float tau = mix(0.08, 0.5, clamp(palette_stability, 0.0, 1.0));
    float rate = 1.0 - exp(-max(TIMEDELTA, 0.0) / tau);
    return vec4(mix(mine.rgb, chosen, clamp(rate, 0.0, 1.0)), 1.0);
}

// --- The warp ---

/// Where this pixel's colour came from one step ago.
///
/// This is MilkDrop's per-vertex motion written directly per-pixel: a zoom and
/// a rotation about a movable centre, a constant drift, and a noise warp on top.
/// Every rate is per second and raised to the frame's own timestep, so the
/// motion is identical at any frame rate rather than running at whatever speed
/// the machine happens to manage.
///
/// It reads as *backward* motion because it is a lookup, not a push: to make the
/// picture travel one way, each pixel fetches from the other way. Sampling
/// backward is also what keeps it stable — every output pixel gets exactly one
/// value, with no gaps to fill and nothing piling up.
vec2 warpSource(vec2 p, float t, float dt) {
    vec2 c = vec2(center_x, center_y);
    vec2 d = p - c;

    // Zoom in means each pixel shows what was nearer the centre last frame.
    float z = pow(max(zoom, 0.01), dt);
    d /= z;

    d = rotateVec(d, -radians(rotate) * dt);
    p = c + d - vec2(drift_x, drift_y) * dt;

    if (warp_amount > 0.0) {
        p += warp_amount * 0.35 * flowCurl(p * warp_scale, t) * dt;
    }
    return p;
}

/// How strongly dark ground holds the picture at a point: 1 is a wall the flow
/// cannot cross, 0 lets it run straight through.
///
/// Hardness does two things at once, because they are the same thing to look at.
/// It sets how much of the picture the barrier holds *and* how abruptly it lets
/// go: at 1 the boundary is a step, so regions stop dead against the dark and
/// keep a hard edge; winding it down both weakens the hold and widens the
/// threshold into a ramp, so colour bleeds further past the boundary the lower it
/// goes. On a modulator that reads as the flow bursting its banks rather than
/// as a wall being switched off.
float barrierHold(vec2 at) {
    float hard = clamp(barrier_hardness, 0.0, 1.0);
    if (barrier_level <= 0.0 || hard <= 0.0) return 0.0;

    // A step at full hardness, widening to a ramp across the whole range as it
    // falls.
    float feather = (1.0 - hard) * barrier_level;
    float below = 1.0 - smoothstep(barrier_level - feather, barrier_level + feather, lum(srcAt(at)));
    return below * hard;
}

/// How much of the effect applies at a point: 0 leaves the source untouched, 1
/// is the full effect.
///
/// A radius of zero disables it, so the mask costs nothing until it is reached
/// for. By default the inside of the circle is held still and everything outside
/// flows, which is the way round you want for holding a subject steady while the
/// frame moves around it; Invert swaps that.
///
/// The distance is measured in screen proportions rather than in texture
/// coordinates. Those are only the same on a square frame — left alone, the
/// "circle" would come out an ellipse on any normal output.
float effectMask(vec2 p) {
    if (mask_radius <= 0.0) return 1.0;

    vec2 d = p - vec2(mask_x, mask_y);
    d.x *= RENDERSIZE.x / max(RENDERSIZE.y, 1.0);
    float r = length(d);

    float feather = max(mask_softness * mask_radius, 1e-4);
    float outside = smoothstep(mask_radius - feather, mask_radius + feather, r);
    return mask_invert != 0u ? 1.0 - outside : outside;
}

/// Pass 1: the warp. This is the only thing that carries between frames.
///
/// Continuous-toned throughout, deliberately. Nothing here snaps to a palette or
/// writes a hard edge, because whatever this pass writes is what the next frame
/// reads, and a hard edge fed back through a warp is what turns into flicker.
///
/// The source bleeds back in at a fixed rate rather than being blended in a
/// fixed proportion, which is Deforum's strength schedule in another form: how
/// long the picture is allowed to drift before the source reasserts itself. Long
/// enough and the frame is almost entirely its own history, so it flows freely
/// and drifts far from the input; short enough and the source dominates and the
/// motion is a shimmer over the live picture.
vec4 flowPass(float t) {
    float dt = clamp(TIMEDELTA, 0.0, 0.05);

    vec3 src = srcAt(uv);

    // Dark ground is floor, and a camera warp has no notion of that — it carries
    // the whole frame indiscriminately, shadows included. Holding the dark parts
    // to the live source is what keeps the picture in the lit areas and makes
    // shapes crawl around the dark ones instead of washing over them. It cannot
    // build up, because it is re-decided from the source every frame rather than
    // accumulated.
    float hold = barrierHold(uv);
    if (hold >= 1.0) return vec4(src, 1.0);

    vec3 prev = fieldAt(warpSource(uv, t, dt));

    // Groups slide over one another because brighter material is pulled through
    // the warp harder than dark material, so a bright patch crossing a dark
    // field genuinely travels across it. Sampling the shear from the previous
    // frame rather than the source ties it to what is actually being carried.
    if (group_shear != 0.0) {
        float bias = 1.0 + group_shear * (lum(prev) - 0.5);
        prev = fieldAt(warpSource(uv, t, dt * bias));
    }

    prev *= pow(max(trail, 0.01), dt);

    // First frame: a persistent target starts cleared, so seed rather than fade
    // up out of black.
    if (FRAMEINDEX < 2u) return vec4(src, 1.0);

    float regen = 1.0 - exp(-dt / max(regen_seconds, 1e-3));
    vec3 flowed = mix(prev, src, regen);

    // Held ground has to be held here, in the buffer, not just hidden at the
    // end. Warping it and covering it up would leave the motion running
    // underneath, so material would still be carried across the boundary and
    // reappear the moment the mask moved.
    return vec4(mix(src, flowed, effectMask(uv) * (1.0 - hold)), 1.0);
}

// --- Grouping, display side only ---

/// How far the group decision is nudged per pixel.
#define GROUP_DITHER 0.35

/// Nearest anchor, with the decision boundary nudged a fixed amount per pixel.
///
/// Membership is a hard pick, which is what gives the flat poster-like regions.
/// A hard pick has a cliff in it: when an anchor drifts across the midpoint of a
/// broad flat area, every pixel there is equidistant at the same instant and the
/// whole region changes colour in a single frame.
///
/// Biasing each pixel's comparison by a hash of its own position moves that
/// midpoint slightly for every pixel, so they cross at different moments and the
/// change sweeps through as a dissolve. The hash is over position only, never
/// time, so the bias is fixed for the life of a pixel and cannot itself shimmer.
int ditheredGroup(vec3 c, vec3 pal[MAX_PALETTE], int numGroups, vec2 at) {
    vec2 cell = floor(at * 1024.0);
    int best = 0;
    float bestDist = 1e10;
    for (int i = 0; i < MAX_PALETTE; i++) {
        if (i >= numGroups) continue;
        vec3 d = c - pal[i];
        float dd = dot(d, d);
        float j = hash2(cell + vec2(float(i) * 37.0, float(i) * 91.0)).x;
        dd *= 1.0 + GROUP_DITHER * (j - 0.5);
        if (dd < bestDist) { bestDist = dd; best = i; }
    }
    return best;
}

/// Grade one point of the field into its group colour.
///
/// Material below the barrier is left alone. The nearest anchor to something
/// near-black is very often something bright, so grading the shadows would turn
/// the darkest parts of the frame into the loudest.
vec3 gradeAt(vec2 at, vec3 pal[MAX_PALETTE], int numGroups) {
    vec3 field = fieldAt(at);
    vec3 graded = pal[ditheredGroup(field, pal, numGroups, at)];
    // Weighed on the source, the same quantity the warp holds on. Asking the
    // field instead lets the two disagree along the edge of a lit region, which
    // showed up as a handful of dark pixels next to bright ones coming out
    // graded when they should have been left alone.
    vec3 held = mix(field, background_fill.rgb, background_fill.a);
    return mix(graded, held, barrierHold(at));
}

// --- Main ---

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIME + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    int numGroups = clamp(int(floor(palette_size + 0.5)), 2, MAX_PALETTE);
    float t = PHASE_TIME_0;

    if (PASSINDEX == 0) {
        fragColor = palettePass(numGroups);
        return;
    }
    if (PASSINDEX == 1) {
        fragColor = flowPass(t);
        return;
    }

    // Composite pass. Display only: nothing below is ever read back.
    vec3 pal[MAX_PALETTE];
    loadPalette(pal);

    // Fully held ground leaves the pass untouched, rather than being reassembled
    // out of a grade and a shade that happen to cancel.
    float hold = barrierHold(uv);
    if (hold >= 1.0) {
        fragColor = texture(sampler2D(inputImage, texSampler), uv);
        return;
    }

    vec3 colour = gradeAt(uv, pal, numGroups);

    // Softening happens here, on the way out, where it cannot feed back and
    // accumulate. Doing it in the warp pass instead is a per-frame blur, and a
    // per-frame blur is what compounds into smoke.
    if (edge_blend_width > 0.0) {
        float e = edge_blend_width * 0.01;
        vec3 around = gradeAt(uv + vec2(e, 0.0), pal, numGroups)
                    + gradeAt(uv - vec2(e, 0.0), pal, numGroups)
                    + gradeAt(uv + vec2(0.0, e), pal, numGroups)
                    + gradeAt(uv - vec2(0.0, e), pal, numGroups);
        colour = mix(colour, around * 0.25, 0.5);
    }

    // Shade the flat group colour by the field's own brightness, so the texture
    // inside a region survives being posterised and travels with it.
    vec3 field = fieldAt(uv);
    float shade = lum(field) / max(lum(colour), 0.04);
    vec3 detailed = colour * clamp(shade, 0.0, 2.0);

    vec3 graded = mix(colour, detailed, color_preservation);

    // Partially held ground fades back toward the source in step with the hold,
    // so winding hardness down reads as the flow spilling past the boundary
    // rather than as the boundary vanishing in one step.
    graded = mix(graded, srcAt(uv), hold);

    // The buffer already holds the source inside the mask, but grading would
    // still posterise it. Fading back to the source here is what makes held
    // ground read as untouched picture rather than as a frozen version of the
    // effect.
    graded = mix(srcAt(uv), graded, effectMask(uv));

    fragColor = vec4(graded, texture(sampler2D(inputImage, texSampler), uv).a);
}
