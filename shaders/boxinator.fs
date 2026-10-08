/*{
    "DESCRIPTION": "Boxinator - grid of cells that churn simplex noise over the input",
    "CREDIT": "inspired by mojovideotech, simplex noise by Ian McEwan, Ashima Arts",
    "ISFVSN": "2",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "rate", "LABEL": "Rate", "TYPE": "float", "DEFAULT": 2.5, "MIN": 0.0, "MAX": 10.0},
        {"NAME": "edge", "LABEL": "Edge", "TYPE": "float", "DEFAULT": 0.001, "MIN": 0.0, "MAX": 0.01},
        {"NAME": "blend", "LABEL": "Blend", "TYPE": "float", "DEFAULT": 0.95, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "randomize", "LABEL": "Randomize", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "lift", "LABEL": "Lift", "TYPE": "float", "DEFAULT": -0.3, "MIN": -0.5, "MAX": 0.2},
        {"NAME": "grid_x", "LABEL": "Columns", "TYPE": "float", "DEFAULT": 64.0, "MIN": 1.5, "MAX": 900.0},
        {"NAME": "grid_y", "LABEL": "Rows", "TYPE": "float", "DEFAULT": 36.0, "MIN": 1.5, "MAX": 600.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "rate", "INDEX": 0, "SCALE": 1.0}
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
    float rate;
    float edge;
    float blend;
    float randomize;
    float lift;
    float grid_x;
    float grid_y;
};

// Simplex noise by Ian McEwan, Ashima Arts. (c) 2011 Ashima Arts, MIT License.
vec4 permute(vec4 x) { return mod(((x * 34.0) + 1.0) * x, 289.0); }

vec4 taylorInvSqrt(vec4 r) { return 1.79284291400159 - 0.85373472095314 * r; }

float snoise(vec3 v) {
    const vec2 C = vec2(1.0 / 6.0, 1.0 / 3.0);
    const vec4 D = vec4(0.0, 0.5, 1.0, 2.0);
    vec3 i = floor(v + dot(v, C.yyy));
    vec3 x0 = v - i + dot(i, C.xxx);
    vec3 g = step(x0.yzx, x0.xyz);
    vec3 l = 1.0 - g;
    vec3 i1 = min(g.xyz, l.zxy);
    vec3 i2 = max(g.xyz, l.zxy);
    vec3 x1 = x0 - i1 + C.xxx;
    vec3 x2 = x0 - i2 + 2.0 * C.xxx;
    vec3 x3 = x0 - 1.0 + 3.0 * C.xxx;
    i = mod(i, 289.0);
    vec4 p = permute(permute(permute(
                 i.z + vec4(0.0, i1.z, i2.z, 1.0))
             + i.y + vec4(0.0, i1.y, i2.y, 1.0))
             + i.x + vec4(0.0, i1.x, i2.x, 1.0));
    float n_ = 1.0 / 7.0;
    vec3 ns = n_ * D.wyz - D.xzx;
    vec4 j = p - 49.0 * floor(p * ns.z * ns.z);
    vec4 x_ = floor(j * ns.z);
    vec4 y_ = floor(j - 7.0 * x_);
    vec4 x = x_ * ns.x + ns.yyyy;
    vec4 y = y_ * ns.x + ns.yyyy;
    vec4 h = 1.0 - abs(x) - abs(y);
    vec4 b0 = vec4(x.xy, y.xy);
    vec4 b1 = vec4(x.zw, y.zw);
    vec4 s0 = floor(b0) * 2.0 + 1.0;
    vec4 s1 = floor(b1) * 2.0 + 1.0;
    vec4 sh = -step(h, vec4(0.0));
    vec4 a0 = b0.xzyw + s0.xzyw * sh.xxyy;
    vec4 a1 = b1.xzyw + s1.xzyw * sh.zzww;
    vec3 p0 = vec3(a0.xy, h.x);
    vec3 p1 = vec3(a0.zw, h.y);
    vec3 p2 = vec3(a1.xy, h.z);
    vec3 p3 = vec3(a1.zw, h.w);
    vec4 norm = taylorInvSqrt(vec4(dot(p0, p0), dot(p1, p1), dot(p2, p2), dot(p3, p3)));
    p0 *= norm.x;
    p1 *= norm.y;
    p2 *= norm.z;
    p3 *= norm.w;
    vec4 m = max(0.6 - vec4(dot(x0, x0), dot(x1, x1), dot(x2, x2), dot(x3, x3)), 0.0);
    m = m * m;
    return 42.0 * dot(m * m, vec4(dot(p0, x0), dot(p1, x1), dot(p2, x2), dot(p3, x3)));
}

float hash(float h) { return fract(sin(h) * 43758.5453123); }

float box(vec2 a, vec2 b) {
    vec2 o = step(b, a);
    return o.x * o.y;
}

void main() {
    vec2 p = vec2(uv.x, 1.0 - uv.y);
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    vec2 g = floor(vec2(grid_x, grid_y));
    float cells = g.x * g.y;
    float index = 1.0 + floor(p.x * g.x) + g.y * floor(p.y * g.y) + g.x;
    vec2 st = fract(p * g);

    // Each cell's value scales the phase, so cells churn at different speeds.
    float s = index / cells * box(st, vec2(edge * g));
    s = mix(s, hash(s), randomize);
    float n = snoise(vec3(s * PHASE_TIME_0) + src.rgb * blend);

    fragColor = vec4(max(vec3(n) + src.rgb + lift, 0.0), src.a);
}
