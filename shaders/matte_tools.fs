/*{
    "DESCRIPTION": "Matte Tools - erode, dilate, smooth and feather the alpha of a key",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "operation", "LABEL": "Operation", "TYPE": "long", "DEFAULT": 2, "VALUES": [0, 1, 2, 3, 4, 5], "LABELS": ["Erode", "Dilate", "Open", "Close", "Median", "Feather"]},
        {"NAME": "radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 2.0, "MIN": 1.0, "MAX": 8.0},
        {"NAME": "median_size", "LABEL": "Median Size", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1], "LABELS": ["3x3", "5x5"]},
        {"NAME": "shrink_grow", "LABEL": "Shrink / Grow", "TYPE": "float", "DEFAULT": 0.0, "MIN": -4.0, "MAX": 4.0},
        {"NAME": "alpha_gamma", "LABEL": "Alpha Gamma", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 5.0},
        {"NAME": "show_matte", "LABEL": "Show Matte", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PASSES": [
        {"TARGET": "stageA", "FORMAT": "r32float"},
        {"TARGET": "stageB", "FORMAT": "r32float"},
        {"TARGET": "stageC", "FORMAT": "r32float"},
        {"TARGET": "stageD", "FORMAT": "r32float"},
        {"TARGET": "stageE", "FORMAT": "r32float"},
        {"TARGET": "stageF"}
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
layout(set = 0, binding = 3) uniform texture2D stageA;
layout(set = 0, binding = 4) uniform texture2D stageB;
layout(set = 0, binding = 5) uniform texture2D stageC;
layout(set = 0, binding = 6) uniform texture2D stageD;
layout(set = 0, binding = 7) uniform texture2D stageE;
layout(set = 0, binding = 8) uniform texture2D stageF;

layout(set = 0, binding = 9) uniform UserParams {
    int operation;
    float radius;
    int median_size;
    float shrink_grow;
    float alpha_gamma;
    uint show_matte;
    float amount;
};

// Six one-dimensional passes on alpha, horizontal then vertical:
// A/B the operation, C/D the second half of Open or Close, E/F shrink or grow.
// Each pair is a separable min, max, median or Gaussian, or a plain copy.
const int COPY = 0;
const int MIN_OP = 1;
const int MAX_OP = 2;
const int MEDIAN = 3;
const int GAUSS = 4;

ivec2 gridSize;

// Alpha feeding the current pass, at cell p.
float alphaAt(ivec2 p) {
    p = clamp(p, ivec2(0), gridSize - 1);
    if (PASSINDEX == 0) return texture(sampler2D(inputImage, texSampler), (vec2(p) + 0.5) / vec2(gridSize)).a;
    if (PASSINDEX == 1) return texelFetch(sampler2D(stageA, texSampler), p, 0).r;
    if (PASSINDEX == 2) return texelFetch(sampler2D(stageB, texSampler), p, 0).r;
    if (PASSINDEX == 3) return texelFetch(sampler2D(stageC, texSampler), p, 0).r;
    if (PASSINDEX == 4) return texelFetch(sampler2D(stageD, texSampler), p, 0).r;
    return texelFetch(sampler2D(stageE, texSampler), p, 0).r;
}

float med3(float a, float b, float c) { return max(min(a, b), min(max(a, b), c)); }

// Min or max over a fractional radius: the outermost pair blends in by the
// fraction so the result moves smoothly as the radius changes.
float extremum(ivec2 p, ivec2 dir, float r, bool takeMax) {
    int n = int(floor(r));
    float m = alphaAt(p);
    for (int i = 1; i <= 8; i++) {
        if (i > n) break;
        float a = alphaAt(p + dir * i);
        float b = alphaAt(p - dir * i);
        m = takeMax ? max(m, max(a, b)) : min(m, min(a, b));
    }
    float f = r - float(n);
    if (f > 0.0 && n < 9) {
        float a = alphaAt(p + dir * (n + 1));
        float b = alphaAt(p - dir * (n + 1));
        float outer = takeMax ? max(m, max(a, b)) : min(m, min(a, b));
        m = mix(m, outer, f);
    }
    return m;
}

// A separable median: median along rows, then along columns. Close to the
// true 2D median for mattes, at a fraction of the cost.
float median1d(ivec2 p, ivec2 dir) {
    float c = alphaAt(p);
    float a1 = alphaAt(p - dir);
    float b1 = alphaAt(p + dir);
    if (median_size == 0) return med3(a1, c, b1);
    float a2 = alphaAt(p - dir * 2);
    float b2 = alphaAt(p + dir * 2);
    float f = max(min(a2, a1), min(b1, b2));
    float g = min(max(a2, a1), max(b1, b2));
    return med3(c, f, g);
}

float gauss1d(ivec2 p, ivec2 dir, float r) {
    float sigma = max(r * 0.5, 0.5);
    float sum = alphaAt(p);
    float total = 1.0;
    for (int i = 1; i <= 8; i++) {
        if (float(i) > r + 0.5) break;
        float w = exp(-float(i * i) / (2.0 * sigma * sigma));
        sum += (alphaAt(p + dir * i) + alphaAt(p - dir * i)) * w;
        total += 2.0 * w;
    }
    return sum / total;
}

void main() {
    gridSize = textureSize(sampler2D(stageA, texSampler), 0);

    if (PASSINDEX < 6) {
        ivec2 p = ivec2(gl_FragCoord.xy);
        ivec2 dir = (PASSINDEX & 1) == 0 ? ivec2(1, 0) : ivec2(0, 1);
        int stage = PASSINDEX / 2;
        int op = COPY;
        float r = radius;
        if (stage == 0) {
            op = operation == 0 || operation == 2 ? MIN_OP
               : operation == 1 || operation == 3 ? MAX_OP
               : operation == 4 ? MEDIAN : GAUSS;
        } else if (stage == 1) {
            op = operation == 2 ? MAX_OP : operation == 3 ? MIN_OP : COPY;
        } else {
            r = abs(shrink_grow);
            op = r < 1e-3 ? COPY : shrink_grow < 0.0 ? MIN_OP : MAX_OP;
        }

        float a;
        if (op == MIN_OP || op == MAX_OP) a = extremum(p, dir, r, op == MAX_OP);
        else if (op == MEDIAN) a = median1d(p, dir);
        else if (op == GAUSS) a = gauss1d(p, dir, r);
        else a = alphaAt(p);
        fragColor = vec4(a);
        return;
    }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    float matte = texture(sampler2D(stageF, texSampler), uv).r;
    matte = pow(clamp(matte, 0.0, 1.0), alpha_gamma);
    float alpha = clamp(mix(src.a, matte, amount), 0.0, 1.0);
    if (show_matte == 1u) {
        fragColor = vec4(vec3(alpha), 1.0);
        return;
    }
    fragColor = vec4(max(src.rgb, 0.0), alpha);
}
