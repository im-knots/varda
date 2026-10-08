/*{
    "DESCRIPTION": "Mosaic - square, hexagon, Voronoi, triangle and diamond cell mosaics with grout and bevel",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "shape", "LABEL": "Shape", "TYPE": "long", "DEFAULT": 1, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Square", "Hexagon", "Voronoi", "Triangle", "Diamond"]},
        {"NAME": "cell_size", "LABEL": "Cell Size", "TYPE": "float", "DEFAULT": 24.0, "MIN": 2.0, "MAX": 120.0},
        {"NAME": "jitter", "LABEL": "Jitter", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "drift_speed", "LABEL": "Drift Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0},
        {"NAME": "edge_width", "LABEL": "Edge Width", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "edge_color", "LABEL": "Edge Color", "TYPE": "color", "DEFAULT": [0.05, 0.05, 0.05, 1.0]},
        {"NAME": "sampling", "LABEL": "Sample", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["Center", "Average"]},
        {"NAME": "bevel", "LABEL": "Bevel", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "drift_speed", "INDEX": 0, "SCALE": 1.0}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    int shape;
    float cell_size;
    float jitter;
    float drift_speed;
    float edge_width;
    vec4 edge_color;
    int sampling;
    float bevel;
    float amount;
};

const float SQRT3 = 1.7320508;

vec2 hash2(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}

// Each shape returns the cell center in pixels and, in z, the distance from
// the pixel to the cell's edge in pixels.

vec3 squareCell(vec2 p, float s) {
    vec2 i = floor(p / s);
    vec2 f = abs(p / s - i - 0.5);
    return vec3((i + 0.5) * s, (0.5 - max(f.x, f.y)) * s);
}

vec3 hexCell(vec2 p, float s) {
    // Flat-top hexagons s apart: two offset rectangular lattices, nearest
    // center wins. Edges face 30, 90 and 150 degrees, s / 2 from the center.
    vec2 r = vec2(SQRT3, 1.0) * s;
    vec2 a = (floor(p / r) + 0.5) * r;
    vec2 b = (floor((p - r * 0.5) / r) + 1.0) * r;
    vec2 c = dot(p - a, p - a) < dot(p - b, p - b) ? a : b;
    vec2 q = abs(p - c);
    float hexDist = max(q.y, dot(q, vec2(0.5 * SQRT3, 0.5)));
    return vec3(c, 0.5 * s - hexDist);
}

vec3 voronoiCell(vec2 p, float s) {
    vec2 g = p / s;
    vec2 base = floor(g);
    float f1 = 1e9, f2 = 1e9;
    vec2 best = base;
    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 cell = base + vec2(x, y);
            vec2 h = hash2(cell);
            // Seeds orbit inside their cell, so drift never piles up and the
            // 3x3 search always finds the nearest one.
            vec2 wander = 0.2 * vec2(sin(PHASE_TIME_0 + h.x * 6.2832), cos(PHASE_TIME_0 * 0.87 + h.y * 6.2832));
            vec2 seed = cell + 0.5 + ((h - 0.5) * 0.6 + wander) * jitter;
            float d = dot(g - seed, g - seed);
            if (d < f1) {
                f2 = f1;
                f1 = d;
                best = seed;
            } else if (d < f2) {
                f2 = d;
            }
        }
    }
    // F2 - F1 is about twice the distance to the shared edge, in cell units.
    return vec3(best * s, 0.5 * (sqrt(f2) - sqrt(f1)) * s);
}

vec3 triangleCell(vec2 p, float s) {
    // Lattice with basis (1, 0) and (0.5, sqrt3/2), each cell split in two.
    float h = 0.5 * SQRT3 * s;
    vec2 l = vec2(p.x / s - p.y / (SQRT3 * s), p.y / h);
    vec2 i = floor(l);
    vec2 f = l - i;
    bool upper = f.x + f.y > 1.0;
    vec2 centerL = i + (upper ? vec2(2.0 / 3.0) : vec2(1.0 / 3.0));
    vec2 center = vec2((centerL.x + 0.5 * centerL.y) * s, centerL.y * h);
    vec3 bary = upper ? vec3(1.0 - f.x, 1.0 - f.y, f.x + f.y - 1.0) : vec3(f.x, f.y, 1.0 - f.x - f.y);
    return vec3(center, min(bary.x, min(bary.y, bary.z)) * h);
}

vec3 diamondCell(vec2 p, float s) {
    // A square grid turned 45 degrees.
    vec2 r = vec2(p.x + p.y, p.y - p.x) * 0.70710678 / s;
    vec2 i = floor(r) + 0.5;
    vec2 f = abs(r - i);
    vec2 center = vec2(i.x - i.y, i.x + i.y) * 0.70710678 * s;
    return vec3(center, (0.5 - max(f.x, f.y)) * s);
}

void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec2 p = gl_FragCoord.xy;
    float s = max(cell_size, 1.0);

    vec3 cell;
    if (shape == 0) cell = squareCell(p, s);
    else if (shape == 2) cell = voronoiCell(p, s);
    else if (shape == 3) cell = triangleCell(p, s);
    else if (shape == 4) cell = diamondCell(p, s);
    else cell = hexCell(p, s);

    vec2 c = cell.xy / RENDERSIZE;
    vec3 col = texture(sampler2D(inputImage, texSampler), c).rgb;
    if (sampling == 1) {
        vec2 o = 0.25 * s / RENDERSIZE;
        col += texture(sampler2D(inputImage, texSampler), c + vec2(o.x, 0.0)).rgb;
        col += texture(sampler2D(inputImage, texSampler), c - vec2(o.x, 0.0)).rgb;
        col += texture(sampler2D(inputImage, texSampler), c + vec2(0.0, o.y)).rgb;
        col += texture(sampler2D(inputImage, texSampler), c - vec2(0.0, o.y)).rgb;
        col *= 0.2;
    }

    // Relief: tilt each tile's rim toward a light at the top left.
    float d = cell.z;
    vec2 grad = vec2(dFdx(d), dFdy(d));
    float rim = 1.0 - smoothstep(0.0, 0.3 * s, d);
    float lit = dot(-grad / max(length(grad), 1e-4), vec2(-0.7071));
    col *= max(1.0 + bevel * lit * rim, 0.0);

    float edge = edge_width > 0.0 ? 1.0 - smoothstep(0.5 * edge_width - 0.5, 0.5 * edge_width + 0.5, d) : 0.0;
    col = mix(col, edge_color.rgb, edge);

    fragColor = vec4(max(mix(src.rgb, col, amount), 0.0), src.a);
}
