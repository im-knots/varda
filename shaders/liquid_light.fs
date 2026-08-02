/*{
    "DESCRIPTION": "1960s liquid light show - oil/water/dye overhead projector psychedelic visuals",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "flow_speed", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0, "LABEL": "Flow Speed"},
        {"NAME": "blob_scale", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0, "LABEL": "Blob Scale"},
        {"NAME": "color_intensity", "TYPE": "float", "DEFAULT": 1.2, "MIN": 0.3, "MAX": 2.0, "LABEL": "Color Intensity"},
        {"NAME": "edge_glow", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.5, "LABEL": "Edge Glow"},
        {"NAME": "dye_spread", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Dye Spread"},
        {"NAME": "warmth", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0, "LABEL": "Projector Warmth"},
        {"NAME": "vignette_amt", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Vignette"},
        {"NAME": "palette", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Palette (0=Classic 1=Acid 2=Sunset 3=Deep)"},
        {"NAME": "agitation", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Agitation"},
        {"NAME": "focus_soft", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Soft Focus"},
        {"NAME": "swirl", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "LABEL": "Dish Rotation"},
        {"NAME": "hue_cycle", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "LABEL": "Dye Hue Cycle"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "flow_speed", "INDEX": 0},
        {"PARAM": "flow_speed", "MULTIPLY_BY": "agitation", "INDEX": 1, "SCALE": 9.0},
        {"PARAM": "swirl", "INDEX": 2, "SCALE": 0.6},
        {"PARAM": "hue_cycle", "INDEX": 3, "SCALE": 0.4}
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
    float flow_speed;
    float blob_scale;
    float color_intensity;
    float edge_glow;
    float dye_spread;
    float warmth;
    float vignette_amt;
    float palette;
    float agitation;
    float focus_soft;
    float swirl;
    float hue_cycle;
};

// ---- Noise primitives (quintic-interpolated, no grid artifacts) ----

float hash21(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float noise2(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    float a = hash21(i);
    float b = hash21(i + vec2(1, 0));
    float c = hash21(i + vec2(0, 1));
    float d = hash21(i + vec2(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// FBM with per-octave rotation to break grid alignment
float fbm(vec2 p, int octaves) {
    float val = 0.0;
    float amp = 0.5;
    float freq = 1.0;
    for (int i = 0; i < 6; i++) {
        if (i >= octaves) break;
        val += amp * noise2(p * freq);
        freq *= 2.03;
        amp *= 0.49;
        p = vec2(p.x * 0.866 - p.y * 0.5, p.x * 0.5 + p.y * 0.866);
    }
    return val;
}

// ---- Inigo Quilez canonical domain warping ----
// fbm(p + fbm(p + fbm(p))) with q and r exposed for color mapping.
// This single technique creates all the organic, stretchy, liquid movement.
//
// Both `t` and `churn` are accumulated phases — see PHASE_INPUTS in the header
// — and nothing in here may multiply either by a live parameter, which is the
// discontinuity the accumulators exist to remove.
//
// `t` is the flow: how far the dye has travelled. `churn` is the mixing, and it
// advances the *inner* warp stages against the outer one, so the fine structure
// keeps reorganising relative to the coarse structure. That is what stirring
// looks like — the fluid does not go anywhere in particular, it just stops
// resembling its earlier self.
//
// Agitation drives `churn` as a rate, never as an amplitude. This is the whole
// point: an amplitude knob moves sample positions, so automating it slides the
// pattern one way and then back, which reads as sloshing or stutter rather than
// as mixing. A rate can only speed the churn up and slow it down, and no fader
// move — however fast, however often reversed — can relocate the fluid.
float pattern(vec2 p, float t, float churn, out vec2 q, out vec2 r) {
    q = vec2(
        fbm(p + vec2(0.0, 0.0) + vec2(t * 0.11, t * 0.07), 5),
        fbm(p + vec2(5.2, 1.3) + vec2(t * 0.08, -t * 0.06), 5)
    );

    r = vec2(
        fbm(p + 4.0 * q + vec2(1.7, 9.2) + vec2(t * 0.05 + churn * 0.31, churn * 0.22), 5),
        fbm(p + 4.0 * q + vec2(8.3, 2.8) + vec2(-churn * 0.27, -t * 0.04 + churn * 0.35), 5)
    );

    return fbm(p + 4.0 * r + vec2(churn * 0.14, -churn * 0.11), 5);
}

// Rotate a colour about the greyscale axis (Rodrigues about 1/√3). Linear in
// `c`, so rotating the blended dye is identical to rotating each dye first —
// which is why this is applied once rather than four times.
vec3 hueRotate(vec3 c, float a) {
    const vec3 k = vec3(0.577350269);
    float ca = cos(a);
    return c * ca + cross(k, c) * sin(a) + k * dot(k, c) * (1.0 - ca);
}

// ---- Dye palettes: vivid Flo-Master ink colors ----

vec3 dyeColor(int ci, int pal) {
    if (pal == 0) {
        if (ci == 0) return vec3(0.90, 0.05, 0.55);
        if (ci == 1) return vec3(0.05, 0.80, 0.35);
        if (ci == 2) return vec3(0.15, 0.10, 0.90);
        return vec3(0.95, 0.80, 0.10);
    } else if (pal == 1) {
        if (ci == 0) return vec3(0.95, 0.10, 0.70);
        if (ci == 1) return vec3(0.30, 0.95, 0.10);
        if (ci == 2) return vec3(0.05, 0.80, 0.95);
        return vec3(0.95, 0.50, 0.05);
    } else if (pal == 2) {
        if (ci == 0) return vec3(0.90, 0.10, 0.15);
        if (ci == 1) return vec3(0.95, 0.65, 0.05);
        if (ci == 2) return vec3(0.45, 0.10, 0.75);
        return vec3(0.90, 0.30, 0.50);
    } else {
        if (ci == 0) return vec3(0.20, 0.05, 0.65);
        if (ci == 1) return vec3(0.05, 0.60, 0.55);
        if (ci == 2) return vec3(0.85, 0.75, 0.10);
        return vec3(0.55, 0.05, 0.30);
    }
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    int pal = int(floor(palette + 0.5));

    // Flow phase: how far the dye has travelled, from Flow Speed alone.
    float t = PHASE_TIME_0;

    // Mixing phase: ∫ flow_speed·agitation dt. Agitation is orthogonal to Flow
    // Speed in what it does — one moves the dye, the other stirs it — but stays
    // multiplied by it, so Flow Speed remains the master: park it at zero and
    // the dish is genuinely still, agitated or not.
    float churn = PHASE_TIME_1;

    // Dish rotation — the operator turning the clock glass on the projector
    // bed. Only the liquid turns; the lens vignette below stays put, because
    // it belongs to the projector rather than to the dish.
    float dish = PHASE_TIME_2;
    float dc = cos(dish), ds = sin(dish);
    vec2 rp = vec2(p.x * dc - p.y * ds, p.x * ds + p.y * dc);

    // Scale coordinate space for blob size
    vec2 wp = rp * blob_scale * 0.7;

    // ---- Core: IQ double domain warp produces the organic liquid structure ----
    vec2 q, r;
    float f = pattern(wp, t, churn, q, r);

    // ---- Color mapping from warp intermediates (q and r) ----
    // This is how IQ gets color variation: the intermediate warp vectors
    // q and r reveal the internal structure of the fluid movement.
    // We use them to blend between dye colors, creating distinct territories
    // that push each other around — exactly like immiscible liquids.

    vec3 c0 = dyeColor(0, pal);
    vec3 c1 = dyeColor(1, pal);
    vec3 c2 = dyeColor(2, pal);
    vec3 c3 = dyeColor(3, pal);

    // Use q to create the primary color territories
    float qLen = length(q);
    float qAngle = atan(q.y, q.x) * 0.318; // normalize to ~[-1,1]

    // Use r to create secondary modulation
    float rLen = length(r);
    float rAngle = atan(r.y, r.x) * 0.318;

    // Blend factors: smooth territory boundaries from the warp field values
    float spread = 0.5 + dye_spread * 1.5;
    float w0 = smoothstep(0.0, spread, qLen + rAngle * 0.3);
    float w1 = smoothstep(0.0, spread, rLen + qAngle * 0.3);
    float w2 = smoothstep(0.0, spread, f + qLen * 0.2);
    float w3 = smoothstep(0.0, spread, 1.0 - f + rLen * 0.2);

    // Normalize to ensure smooth blending (soft territory boundaries)
    float wTotal = w0 + w1 + w2 + w3 + 0.001;
    w0 /= wTotal; w1 /= wTotal; w2 /= wTotal; w3 /= wTotal;

    vec3 liquid = c0 * w0 + c1 * w1 + c2 * w2 + c3 * w3;

    // ---- Dye hue cycle ----
    // Applied to the blended dye only, so the warm edge glow and the tungsten
    // tint added below keep their lamp colour. The angle is accumulated, so
    // returning the fader to zero parks the hue where it drifted to rather
    // than snapping it back.
    liquid = hueRotate(liquid, PHASE_TIME_3);

    // ---- Luminosity variation from the warp field ----
    // The field value f acts as "dye density": brighter where liquid is thin,
    // deeper/richer where it pools. This is the main depth cue.
    float luminosity = 0.6 + 0.5 * f;
    liquid *= luminosity;

    // ---- Edge glow at territory boundaries ----
    // Where two colors meet, the projector light catches the meniscus.
    // Detect by looking at how close the top two blend weights are.
    float maxW = max(max(w0, w1), max(w2, w3));
    float sorted0 = maxW;
    float sorted1 = 0.0;
    if (w0 < sorted0) sorted1 = max(sorted1, w0);
    if (w1 < sorted0) sorted1 = max(sorted1, w1);
    if (w2 < sorted0) sorted1 = max(sorted1, w2);
    if (w3 < sorted0) sorted1 = max(sorted1, w3);

    float edgeProximity = 1.0 - smoothstep(0.0, 0.15, sorted0 - sorted1);
    edgeProximity *= edgeProximity;
    // Bright warm glow at edges
    liquid += edgeProximity * edge_glow * vec3(0.25, 0.20, 0.15) * luminosity;

    // ---- Color intensity and saturation boost ----
    liquid *= color_intensity;
    float lum = dot(liquid, vec3(0.299, 0.587, 0.114));
    liquid = mix(vec3(lum), liquid, 1.62);

    // ---- Projector warmth: tungsten lamp tint ----
    liquid = mix(liquid, liquid * vec3(1.12, 0.98, 0.78), warmth);
    liquid += vec3(0.03, 0.015, 0.0) * warmth;

    // ---- Soft focus / bloom: projector optics halation ----
    if (focus_soft > 0.01) {
        float gl = dot(liquid, vec3(0.299, 0.587, 0.114));
        float glow = smoothstep(0.35, 0.9, gl);
        liquid += liquid * glow * focus_soft * 0.4;
        // Slight desaturation in bright glow areas
        float ld = dot(liquid, vec3(0.299, 0.587, 0.114));
        liquid = mix(liquid, vec3(ld), glow * focus_soft * 0.2);
    }

    // ---- Circular vignette: projector lens edge falloff ----
    float vig = 1.0 - smoothstep(0.4, 1.3, length(p * 0.7)) * vignette_amt;
    liquid *= vig;

    // ---- Final grade ----
    // Four dye territories that overlap across most of the frame average
    // towards their own mean, so the dish drifts to pastel however saturated
    // the individual dyes are. Pulling the level down and the contrast up about
    // a mid pivot re-separates them, and reads as dense aniline ink under a hot
    // lamp rather than as watercolour. The 0.92 gamma keeps the soft projected
    // falloff this shader is built around; it used to be the whole grade, which
    // is why the image was lifting rather than sitting down.
    const float BRIGHTNESS = 0.82;
    const float CONTRAST = 1.22;
    const float PIVOT = 0.42;
    liquid = pow(max(liquid, 0.0), vec3(0.92)) * BRIGHTNESS;
    liquid = (liquid - PIVOT) * CONTRAST + PIVOT;

    liquid = clamp(liquid, 0.0, 1.0);
    fragColor = vec4(liquid, 1.0);
}
