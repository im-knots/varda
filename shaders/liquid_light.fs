/*{
    "DESCRIPTION": "1960s liquid light show - oil/water/dye overhead projector psychedelic visuals",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "flow_speed", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0, "LABEL": "Flow Speed"},
        {"NAME": "blob_scale", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0, "LABEL": "Blob Scale"},
        {"NAME": "color_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 2.0, "LABEL": "Color Intensity"},
        {"NAME": "edge_glow", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.5, "LABEL": "Edge Glow"},
        {"NAME": "dye_spread", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Dye Spread"},
        {"NAME": "warmth", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Projector Warmth"},
        {"NAME": "vignette_amt", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Vignette"},
        {"NAME": "palette", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Palette (0=Classic 1=Acid 2=Sunset 3=Deep)"},
        {"NAME": "agitation", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Agitation"},
        {"NAME": "focus_soft", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 1.0, "LABEL": "Soft Focus"}
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
};

#define PI  3.14159265359
#define TAU 6.28318530718

// ---- Noise primitives ----

vec3 hash33(vec3 p3) {
    p3 = fract(p3 * vec3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yxz + 33.33);
    return fract((p3.xxy + p3.yxx) * p3.zyx);
}

vec2 hash22(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * vec3(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

float hash21(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

// Smooth 2D noise
float noise2(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    float a = hash21(i);
    float b = hash21(i + vec2(1, 0));
    float c = hash21(i + vec2(0, 1));
    float d = hash21(i + vec2(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// FBM with domain warping for organic flow
float fbm(vec2 p, int octaves) {
    float val = 0.0;
    float amp = 0.5;
    float freq = 1.0;
    for (int i = 0; i < 7; i++) {
        if (i >= octaves) break;
        val += amp * noise2(p * freq);
        freq *= 2.01;
        amp *= 0.49;
        // Slight rotation per octave for organic feel
        p = vec2(p.x * 0.866 - p.y * 0.5, p.x * 0.5 + p.y * 0.866);
    }
    return val;
}

// Domain-warped FBM: the key to the liquid oil look
float warpedFbm(vec2 p, float t) {
    // First warp layer
    vec2 q = vec2(fbm(p + vec2(0.0, 0.0), 5),
                  fbm(p + vec2(5.2, 1.3), 5));

    // Second warp layer (warp the warp)
    vec2 r = vec2(fbm(p + 4.0 * q + vec2(1.7, 9.2) + t * 0.15, 5),
                  fbm(p + 4.0 * q + vec2(8.3, 2.8) + t * 0.126, 5));

    return fbm(p + 4.0 * r, 6);
}

// Metaball field for large organic blobs
float metaballs(vec2 p, float t) {
    float field = 0.0;
    for (int i = 0; i < 8; i++) {
        float fi = float(i);
        // Each blob has its own slow orbit
        vec2 center = vec2(
            sin(t * 0.17 + fi * 1.73) * 0.35 + sin(t * 0.09 + fi * 2.51) * 0.2,
            cos(t * 0.13 + fi * 2.17) * 0.35 + cos(t * 0.11 + fi * 1.37) * 0.2
        );
        float radius = 0.08 + 0.05 * sin(t * 0.23 + fi * 3.14);
        float d = length(p - center);
        field += (radius * radius) / (d * d + 0.001);
    }
    return field;
}

// Color palettes inspired by actual liquid light show dyes
vec3 getPaletteColor(float t, float pal) {
    int p = int(floor(pal + 0.5));
    vec3 a, b, c, d;
    if (p == 0) {
        // Classic: magenta, green, blue, yellow
        a = vec3(0.5, 0.5, 0.5);
        b = vec3(0.5, 0.5, 0.5);
        c = vec3(1.0, 0.7, 0.4);
        d = vec3(0.0, 0.15, 0.20);
    } else if (p == 1) {
        // Acid: neon green, hot pink, electric blue
        a = vec3(0.5, 0.5, 0.5);
        b = vec3(0.5, 0.5, 0.5);
        c = vec3(1.0, 1.0, 0.5);
        d = vec3(0.80, 0.90, 0.30);
    } else if (p == 2) {
        // Sunset: deep reds, oranges, purples
        a = vec3(0.5, 0.5, 0.5);
        b = vec3(0.5, 0.5, 0.5);
        c = vec3(1.0, 0.5, 0.2);
        d = vec3(0.0, 0.25, 0.45);
    } else {
        // Deep: indigo, teal, gold
        a = vec3(0.5, 0.5, 0.5);
        b = vec3(0.5, 0.5, 0.5);
        c = vec3(0.5, 1.0, 0.7);
        d = vec3(0.15, 0.40, 0.65);
    }
    return a + b * cos(TAU * (c * t + d));
}

void main() {
    // Uniform preservation
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Centered, aspect-corrected coordinates
    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float t = TIME * flow_speed;

    float agit = agitation;

    // ---- Layer 1: Large-scale oil blobs via metaballs ----
    vec2 mp = p * blob_scale * 0.7;
    float meta = metaballs(mp, t * (1.0 + agit));

    // Threshold metaball field into distinct oil regions
    float oilMask = smoothstep(1.2, 2.5, meta);
    float oilEdge = smoothstep(1.0, 1.3, meta) - smoothstep(2.3, 2.8, meta);

    // ---- Layer 2: Domain-warped noise for organic dye flow ----
    vec2 wp = p * blob_scale * 1.3;
    float warp1 = warpedFbm(wp + vec2(t * 0.05, 0.0), t);
    float warp2 = warpedFbm(wp + vec2(100.0, t * 0.04), t * 0.8);
    float warp3 = warpedFbm(wp * 0.8 + vec2(200.0, 50.0), t * 1.1);

    // ---- Compose color layers like actual dye in water ----
    // Each warp channel drives a different dye color
    float dyeA = smoothstep(0.3, 0.7, warp1) * (0.5 + dye_spread * 0.5);
    float dyeB = smoothstep(0.35, 0.65, warp2) * (0.5 + dye_spread * 0.5);
    float dyeC = smoothstep(0.4, 0.6, warp3) * (0.5 + dye_spread * 0.5);

    // Get palette colors at different phase offsets
    vec3 colA = getPaletteColor(warp1 * 0.5 + t * 0.02, palette);
    vec3 colB = getPaletteColor(warp2 * 0.5 + 0.33 + t * 0.015, palette);
    vec3 colC = getPaletteColor(warp3 * 0.5 + 0.66 + t * 0.01, palette);

    // Immiscible layers: colors don't blend, they push each other
    // Simulate surface tension between dyes
    vec3 liquid = vec3(0.02, 0.01, 0.02); // dark base (water)

    // Layer the dyes with priority/dominance
    float dominance1 = dyeA * (1.0 + oilMask * 0.5);
    float dominance2 = dyeB * (1.0 - oilMask * 0.3);
    float dominance3 = dyeC * smoothstep(0.0, 0.5, 1.0 - oilMask);

    // Weighted blend that respects immiscibility
    float totalWeight = dominance1 + dominance2 + dominance3 + 0.001;
    liquid = (colA * dominance1 + colB * dominance2 + colC * dominance3) / totalWeight;

    // Fade toward dark base where no dye is strong
    float dyePresence = max(max(dominance1, dominance2), dominance3);
    liquid = mix(vec3(0.02, 0.01, 0.03), liquid, smoothstep(0.1, 0.5, dyePresence));

    // ---- Edge glow: bright lines where oil regions meet ----
    float edgeBright = oilEdge * edge_glow * 2.0;
    // Iridescent edge color
    vec3 edgeCol = getPaletteColor(warp1 + warp2 + t * 0.05, palette) * 1.5;
    liquid += edgeCol * edgeBright;

    // ---- Surface tension boundaries between dye regions ----
    float boundary = abs(dyeA - dyeB) + abs(dyeB - dyeC) + abs(dyeA - dyeC);
    boundary = smoothstep(0.1, 0.5, boundary) * edge_glow * 0.5;
    liquid += vec3(boundary) * 0.3;

    // ---- Color intensity & saturation ----
    liquid *= color_intensity;

    // Boost saturation (liquid light shows are VIVID)
    float lum = dot(liquid, vec3(0.299, 0.587, 0.114));
    liquid = mix(vec3(lum), liquid, 1.3);

    // ---- Projector warmth (incandescent lamp cast) ----
    liquid = mix(liquid, liquid * vec3(1.1, 1.0, 0.8), warmth);
    liquid += vec3(0.03, 0.015, 0.0) * warmth; // slight warm fog

    // ---- Soft focus (overhead projector optics) ----
    // Simulate by gently blending with a slightly offset sample
    if (focus_soft > 0.01) {
        float sf = focus_soft * 0.003;
        // Average nearby noise evaluations for a fake blur
        float w2a = warpedFbm(wp + vec2(sf, 0.0), t);
        float w2b = warpedFbm(wp + vec2(0.0, sf), t);
        vec3 softCol = getPaletteColor((w2a + w2b) * 0.25 + t * 0.02, palette);
        liquid = mix(liquid, (liquid + softCol * dyePresence) * 0.5, focus_soft * 0.4);
    }

    // ---- Vignette (projector edge falloff) ----
    float vig = 1.0 - smoothstep(0.5, 1.4, length(p * 0.7)) * vignette_amt;
    liquid *= vig;

    liquid = clamp(liquid, 0.0, 1.0);
    fragColor = vec4(liquid, 1.0);
}
