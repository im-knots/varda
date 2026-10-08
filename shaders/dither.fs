/*{
    "DESCRIPTION": "Dither - ordered and noise dithering with retro palettes",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "method", "LABEL": "Method", "TYPE": "long", "DEFAULT": 2, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Bayer 2x2", "Bayer 4x4", "Bayer 8x8", "Noise", "Lines"]},
        {"NAME": "palette", "LABEL": "Palette", "TYPE": "long", "DEFAULT": 2, "VALUES": [0, 1, 2, 3, 4, 5], "LABELS": ["Mono", "Two Color", "Game Boy", "CGA", "PICO-8", "Source Quantized"]},
        {"NAME": "levels", "LABEL": "Levels", "TYPE": "float", "DEFAULT": 4.0, "MIN": 2.0, "MAX": 16.0},
        {"NAME": "pixel_size", "LABEL": "Pixel Size", "TYPE": "float", "DEFAULT": 2.0, "MIN": 1.0, "MAX": 8.0},
        {"NAME": "spread", "LABEL": "Spread", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "color_a", "LABEL": "Dark Color", "TYPE": "color", "DEFAULT": [0.08, 0.06, 0.2, 1.0]},
        {"NAME": "color_b", "LABEL": "Light Color", "TYPE": "color", "DEFAULT": [0.98, 0.86, 0.62, 1.0]},
        {"NAME": "animate", "LABEL": "Animate Noise", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    int method;
    int palette;
    float levels;
    float pixel_size;
    float spread;
    vec4 color_a;
    vec4 color_b;
    uint animate;
    float amount;
};

// Nearest palette entry so far. Entries are given in sRGB and in Oklab, the
// space the search runs in. Unrolled calls, because indexing a const array
// copies it into per-pixel memory on some backends.
vec3 searchLab;
float bestDist;
vec3 bestColor;

void entry(vec3 srgb, vec3 lab) {
    vec3 d = lab - searchLab;
    float dist = dot(d, d);
    if (dist < bestDist) {
        bestDist = dist;
        bestColor = srgb;
    }
}

void gameBoy() {
    entry(vec3(0.059, 0.220, 0.059), vec3(0.3015, -0.0653, 0.0488));
    entry(vec3(0.188, 0.384, 0.188), vec3(0.4478, -0.0771, 0.0565));
    entry(vec3(0.545, 0.675, 0.059), vec3(0.6948, -0.0893, 0.1401));
    entry(vec3(0.608, 0.737, 0.059), vec3(0.7441, -0.0926, 0.1506));
}

void cga() {
    entry(vec3(0.0), vec3(0.0));
    entry(vec3(0.333, 1.0, 1.0), vec3(0.9150, -0.1318, -0.0354));
    entry(vec3(1.0, 0.333, 1.0), vec3(0.7401, 0.2303, -0.1445));
    entry(vec3(1.0), vec3(1.0, 0.0, 0.0));
}

void pico8() {
    entry(vec3(0.000, 0.000, 0.000), vec3(0.0000, 0.0000, 0.0000));
    entry(vec3(0.114, 0.169, 0.325), vec3(0.2997, -0.0035, -0.0748));
    entry(vec3(0.494, 0.145, 0.325), vec3(0.4169, 0.1300, -0.0172));
    entry(vec3(0.000, 0.529, 0.318), vec3(0.5486, -0.1203, 0.0511));
    entry(vec3(0.671, 0.322, 0.212), vec3(0.5438, 0.0978, 0.0766));
    entry(vec3(0.373, 0.341, 0.310), vec3(0.4619, 0.0063, 0.0152));
    entry(vec3(0.761, 0.765, 0.780), vec3(0.8176, 0.0005, -0.0056));
    entry(vec3(1.000, 0.945, 0.910), vec3(0.9667, 0.0111, 0.0159));
    entry(vec3(1.000, 0.000, 0.302), vec3(0.6340, 0.2420, 0.0769));
    entry(vec3(1.000, 0.639, 0.000), vec3(0.7892, 0.0596, 0.1606));
    entry(vec3(1.000, 0.925, 0.153), vec3(0.9297, -0.0434, 0.1838));
    entry(vec3(0.000, 0.894, 0.212), vec3(0.7979, -0.2083, 0.1499));
    entry(vec3(0.161, 0.678, 1.000), vec3(0.7178, -0.0737, -0.1425));
    entry(vec3(0.514, 0.463, 0.612), vec3(0.5915, 0.0297, -0.0509));
    entry(vec3(1.000, 0.467, 0.659), vec3(0.7421, 0.1721, -0.0016));
    entry(vec3(1.000, 0.800, 0.667), vec3(0.8816, 0.0419, 0.0604));
}

// sRGB transfer with a linear extension above 1.0 so headroom survives.
vec3 encode(vec3 c) {
    vec3 lo = mix(c * 12.92, 1.055 * pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
    return mix(lo, 1.0 + (c - 1.0) * (1.055 / 2.4), step(1.0, c));
}

vec3 decode(vec3 e) {
    vec3 lo = mix(e / 12.92, pow((clamp(e, 0.0, 1.0) + 0.055) / 1.055, vec3(2.4)), step(0.04045, e));
    return mix(lo, 1.0 + (e - 1.0) * (2.4 / 1.055), step(1.0, e));
}

vec3 oklab(vec3 srgb) {
    vec3 c = decode(clamp(srgb, 0.0, 1.0));
    vec3 lms = mat3(0.4122214708, 0.2119034982, 0.0883024619,
                    0.5363325363, 0.6806995451, 0.2817188376,
                    0.0514459929, 0.1073969566, 0.6299787005) * c;
    lms = pow(max(lms, 0.0), vec3(1.0 / 3.0));
    return mat3(0.2104542553, 1.9779984951, 0.0259040371,
                0.7936177850, -2.4285922050, 0.7827717662,
                -0.0040720468, 0.4505937099, -0.8086757660) * lms;
}

float bayer(ivec2 p, int bits) {
    int v = 0;
    for (int i = 0; i < bits; i++) {
        int x = (p.x >> i) & 1;
        int y = (p.y >> i) & 1;
        v |= (((x ^ y) << 1) | y) << (2 * (bits - 1 - i));
    }
    float n = float(1 << (2 * bits));
    return (float(v) + 0.5) / n;
}

// Threshold in [-0.5, 0.5) for the cell.
float threshold(ivec2 cell) {
    float t;
    if (method <= 2) {
        t = bayer(cell & ((2 << method) - 1), method + 1);
    } else if (method == 3) {
        vec2 q = vec2(cell);
        if (animate == 1u) q += 5.588238 * float(FRAMEINDEX % 64u);
        t = fract(52.9829189 * fract(dot(q, vec2(0.06711056, 0.00583715))));
    } else {
        t = (float(cell.y & 3) + 0.5) / 4.0;
    }
    return (t - 0.5) * spread;
}

vec3 quantize(vec3 e, float steps, float t) {
    float n = max(steps - 1.0, 1.0);
    return floor(e * n + 0.5 + t) / n;
}

void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    float ps = max(floor(pixel_size), 1.0);
    ivec2 cell = ivec2(floor(gl_FragCoord.xy / ps));
    vec4 cellSrc = texture(sampler2D(inputImage, texSampler), (vec2(cell) + 0.5) * ps / RENDERSIZE);
    vec3 e = encode(max(cellSrc.rgb, 0.0));
    float t = threshold(cell);
    float luma = dot(e, vec3(0.2126, 0.7152, 0.0722));
    float lv = floor(levels);

    vec3 outE;
    if (palette == 0) {
        outE = vec3(quantize(vec3(luma), lv, t).x);
    } else if (palette == 1) {
        float k = clamp(quantize(vec3(luma), 2.0, t).x, 0.0, 1.0);
        outE = encode(mix(color_a.rgb, color_b.rgb, k));
    } else if (palette == 5) {
        outE = quantize(e, lv, t);
    } else {
        // Offset by the threshold scaled to the palette's spacing, then take
        // the nearest entry in Oklab.
        float spacing = palette == 2 ? 0.33 : (palette == 3 ? 0.5 : 0.25);
        searchLab = oklab(clamp(e + t * spacing, 0.0, 1.0));
        bestDist = 1e9;
        bestColor = vec3(0.0);
        if (palette == 2) gameBoy();
        else if (palette == 3) cga();
        else pico8();
        outE = bestColor;
    }

    vec3 col = mix(src.rgb, decode(max(outE, 0.0)), amount);
    fragColor = vec4(max(col, 0.0), src.a);
}
