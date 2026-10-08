/*{
    "DESCRIPTION": "Pixel Sort - sorts runs of pixels inside a brightness band into streaks",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Glitch"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "direction", "LABEL": "Direction", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["Horizontal", "Vertical"]},
        {"NAME": "order", "LABEL": "Order", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["Ascending", "Descending"]},
        {"NAME": "sort_by", "LABEL": "Sort By", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2], "LABELS": ["Luma", "Hue", "Saturation"]},
        {"NAME": "threshold_low", "LABEL": "Threshold Low", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "threshold_high", "LABEL": "Threshold High", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "max_span", "LABEL": "Max Span", "TYPE": "float", "DEFAULT": 64.0, "MIN": 8.0, "MAX": 96.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PASSES": [
        {"TARGET": "keys", "FORMAT": "rgba32float"},
        {"TARGET": "hist", "FORMAT": "rgba32float"}
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
layout(set = 0, binding = 3) uniform texture2D keys;
layout(set = 0, binding = 4) uniform texture2D hist;

layout(set = 0, binding = 5) uniform UserParams {
    int direction;
    int order;
    int sort_by;
    float threshold_low;
    float threshold_high;
    float max_span;
    float amount;
};

// A bounded-span approximation: runs are cut into blocks of max_span, each
// block's 12-bin key histogram is built once at its first pixel, and every
// pixel takes the span pixel whose key is nearest the one its rank implies.

const int MAX_SPAN = 96;
const int BINS = 12;

ivec2 gridSize;
int axisLen;

// Cell `along` on the sort axis, on the same line as p.
ivec2 cellAt(ivec2 p, int along) {
    return direction == 0 ? ivec2(along, p.y) : ivec2(p.x, along);
}

int alongOf(ivec2 p) { return direction == 0 ? p.x : p.y; }

vec3 fetchColor(ivec2 p) {
    return texture(sampler2D(inputImage, texSampler), (vec2(p) + 0.5) / vec2(gridSize)).rgb;
}

vec3 encode(vec3 c) {
    c = clamp(c, 0.0, 1.0);
    return mix(c * 12.92, 1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
}

// The sort key of a color in [0, 1], or -1 outside the threshold band.
float keyOf(vec3 c) {
    vec3 e = encode(c);
    float l = dot(e, vec3(0.2126, 0.7152, 0.0722));
    if (l < threshold_low || l > threshold_high) return -1.0;
    if (sort_by == 0) return l;
    float hi = max(e.r, max(e.g, e.b));
    float lo = min(e.r, min(e.g, e.b));
    float chroma = hi - lo;
    if (sort_by == 2) return hi > 0.0 ? chroma / hi : 0.0;
    if (chroma <= 0.0) return 0.0;
    float h;
    if (hi == e.r) h = mod((e.g - e.b) / chroma, 6.0);
    else if (hi == e.g) h = (e.b - e.r) / chroma + 2.0;
    else h = (e.r - e.g) / chroma + 4.0;
    return h / 6.0;
}

// Keys of cells along..along+3 on p's line. `along` must be in the grid.
vec4 keys4(ivec2 p, int along) {
    return texelFetch(sampler2D(keys, texSampler), cellAt(p, along), 0);
}

int binOf(float key) { return clamp(int(key * float(BINS)), 0, BINS - 1); }

// One count for key's bin, in the packed histogram layout.
vec4 binSlot(float key) {
    int b = binOf(key);
    return vec4(equal(ivec4(b / 3), ivec4(0, 1, 2, 3))) * pow(128.0, float(b % 3));
}

float hash(float n) { return fract(sin(n * 12.9898) * 43758.5453); }

// The first and last cells, along the sort axis, of the block holding p.
ivec2 blockRange(ivec2 p) {
    int line = direction == 0 ? p.y : p.x;
    int block = int(max_span);
    int offset = int(hash(float(line)) * float(block));
    int start = ((alongOf(p) + offset) / block) * block - offset;
    return ivec2(max(start, 0), min(start + block - 1, axisLen - 1));
}

void main() {
    // Effect passes see the deck's RENDERSIZE, so take the grid from a buffer.
    gridSize = textureSize(sampler2D(keys, texSampler), 0);
    axisLen = direction == 0 ? gridSize.x : gridSize.y;

    // Each texel packs the keys of four consecutive cells so the span loops
    // fetch a quarter as often.
    if (PASSINDEX == 0) {
        ivec2 p = ivec2(gl_FragCoord.xy);
        int along = alongOf(p);
        fragColor = vec4(
            keyOf(fetchColor(p)),
            along + 1 < axisLen ? keyOf(fetchColor(cellAt(p, along + 1))) : -1.0,
            along + 2 < axisLen ? keyOf(fetchColor(cellAt(p, along + 2))) : -1.0,
            along + 3 < axisLen ? keyOf(fetchColor(cellAt(p, along + 3))) : -1.0);
        return;
    }

    if (PASSINDEX == 1) {
        ivec2 p = ivec2(gl_FragCoord.xy);
        ivec2 range = blockRange(p);
        int along = alongOf(p);
        bool start = keys4(p, along).x >= 0.0
            && (along == range.x || keys4(p, along - 1).x < 0.0);
        if (!start) {
            fragColor = vec4(0.0);
            return;
        }
        // Find the span length, then count keys per bin. Bin b lives in word
        // b / 3 at weight 128^(b % 3); spans are under 128 cells. The span
        // length is the sum of the counts.
        int len = 0;
        for (int j = 0; j < MAX_SPAN / 4; j++) {
            int k0 = 4 * j;
            vec4 kv = keys4(p, along + k0);
            int room = range.y - along - k0 + 1;
            if (room < 1 || kv.x < 0.0) { len = k0; break; }
            if (room < 2 || kv.y < 0.0) { len = k0 + 1; break; }
            if (room < 3 || kv.z < 0.0) { len = k0 + 2; break; }
            if (room < 4 || kv.w < 0.0) { len = k0 + 3; break; }
            len = k0 + 4;
        }
        vec4 packed = vec4(0.0);
        for (int j = 0; j < MAX_SPAN / 4; j++) {
            int k0 = 4 * j;
            if (k0 >= len) break;
            vec4 kv = keys4(p, along + k0);
            vec4 valid = vec4(lessThan(ivec4(k0) + ivec4(0, 1, 2, 3), ivec4(len)));
            packed += binSlot(kv.x) * valid.x + binSlot(kv.y) * valid.y
                + binSlot(kv.z) * valid.z + binSlot(kv.w) * valid.w;
        }
        fragColor = packed;
        return;
    }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    ivec2 p = clamp(ivec2(uv * vec2(gridSize)), ivec2(0), gridSize - 1);
    int along = alongOf(p);
    if (amount <= 0.0 || keys4(p, along).x < 0.0) {
        fragColor = vec4(max(src.rgb, 0.0), src.a);
        return;
    }

    // Walk back to the span start, four cells per fetch.
    ivec2 range = blockRange(p);
    int index = 0;
    for (int j = 0; j < MAX_SPAN / 4; j++) {
        int k0 = 4 * j;
        int base = along - k0 - 4;
        // Near the start of the line, fetch the cells one at a time.
        vec4 kv = base >= 0 ? keys4(p, base) : vec4(
            -1.0,
            base + 1 >= 0 ? keys4(p, base + 1).x : -1.0,
            base + 2 >= 0 ? keys4(p, base + 2).x : -1.0,
            base + 3 >= 0 ? keys4(p, base + 3).x : -1.0);
        if (base + 3 < range.x || kv.w < 0.0) { index = k0; break; }
        if (base + 2 < range.x || kv.z < 0.0) { index = k0 + 1; break; }
        if (base + 1 < range.x || kv.y < 0.0) { index = k0 + 2; break; }
        if (base < range.x || kv.x < 0.0) { index = k0 + 3; break; }
    }
    int start = along - index;
    vec4 packed = texelFetch(sampler2D(hist, texSampler), cellAt(p, start), 0);
    vec4 c0 = mod(packed, 128.0);
    vec4 c1 = mod(floor(packed / 128.0), 128.0);
    vec4 c2 = floor(packed / 16384.0);
    int len = int(dot(c0 + c1 + c2, vec4(1.0)));
    if (len < 2) {
        fragColor = vec4(max(src.rgb, 0.0), src.a);
        return;
    }

    float rank = float(order == 0 ? index : len - 1 - index);
    float below = 0.0;
    int bin = BINS - 1;
    float inBin = 1.0;
    for (int w = 0; w < BINS / 3; w++) {
        vec4 pick = vec4(equal(ivec4(w), ivec4(0, 1, 2, 3)));
        vec3 cnt = vec3(dot(c0, pick), dot(c1, pick), dot(c2, pick));
        if (rank < below + cnt.x) { bin = 3 * w; inBin = cnt.x; break; }
        below += cnt.x;
        if (rank < below + cnt.y) { bin = 3 * w + 1; inBin = cnt.y; break; }
        below += cnt.y;
        if (rank < below + cnt.z) { bin = 3 * w + 2; inBin = cnt.z; break; }
        below += cnt.z;
    }
    // Assume keys spread evenly inside the bin to place the rank within it.
    float target = (float(bin) + (rank - below + 0.5) / max(inBin, 1.0)) / float(BINS);

    // Distance of each key to the target, with keys outside the target bin
    // ranked after every key inside it. Cells past the span end score 1e9.
    float binLo = float(bin) / float(BINS);
    float binHi = bin == BINS - 1 ? 2.0 : float(bin + 1) / float(BINS);
    int bestK = index;
    float bestDiff = 1e9;
    for (int j = 0; j < MAX_SPAN / 4; j++) {
        int k0 = 4 * j;
        if (k0 >= len) break;
        vec4 kv = keys4(p, start + k0);
        vec4 inside = step(vec4(binLo), kv) * (1.0 - step(vec4(binHi), kv));
        vec4 d = abs(kv - target) + 1.0 - inside;
        d = mix(d, vec4(1e9), greaterThanEqual(ivec4(k0) + ivec4(0, 1, 2, 3), ivec4(len)));
        if (d.x < bestDiff) { bestDiff = d.x; bestK = k0; }
        if (d.y < bestDiff) { bestDiff = d.y; bestK = k0 + 1; }
        if (d.z < bestDiff) { bestDiff = d.z; bestK = k0 + 2; }
        if (d.w < bestDiff) { bestDiff = d.w; bestK = k0 + 3; }
    }
    vec3 best = fetchColor(cellAt(p, start + bestK));

    fragColor = vec4(max(mix(src.rgb, best, amount), 0.0), src.a);
}
