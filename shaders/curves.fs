/*{
    "DESCRIPTION": "Curves - master and per-channel tone curves",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "master_25", "LABEL": "Shadows", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "master_50", "LABEL": "Midtones", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "master_75", "LABEL": "Highlights", "TYPE": "float", "DEFAULT": 0.75, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "red_25", "LABEL": "Red Shadows", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "red_50", "LABEL": "Red Midtones", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "red_75", "LABEL": "Red Highlights", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.75, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "green_25", "LABEL": "Green Shadows", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "green_50", "LABEL": "Green Midtones", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "green_75", "LABEL": "Green Highlights", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.75, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "blue_25", "LABEL": "Blue Shadows", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "blue_50", "LABEL": "Blue Midtones", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "blue_75", "LABEL": "Blue Highlights", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.75, "MIN": 0.0, "MAX": 1.0}
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
    float master_25;
    float master_50;
    float master_75;
    float amount;
    float red_25;
    float red_50;
    float red_75;
    float green_25;
    float green_50;
    float green_75;
    float blue_25;
    float blue_50;
    float blue_75;
};

// sRGB transfer with a linear extension above 1.0 so headroom survives.
vec3 encode(vec3 c) {
    vec3 lo = mix(c * 12.92, 1.055 * pow(clamp(c, 0.0, 1.0), vec3(1.0 / 2.4)) - 0.055, step(0.0031308, c));
    return mix(lo, 1.0 + (c - 1.0) * (1.055 / 2.4), step(1.0, c));
}

vec3 decode(vec3 e) {
    vec3 lo = mix(e / 12.92, pow((clamp(e, 0.0, 1.0) + 0.055) / 1.055, vec3(2.4)), step(0.04045, e));
    return mix(lo, 1.0 + (e - 1.0) * (2.4 / 1.055), step(1.0, e));
}

// Harmonic-mean tangent: zero at a local extremum, which keeps the curve
// monotone wherever its points are.
vec3 tangent(vec3 d0, vec3 d1) {
    vec3 p = d0 * d1;
    vec3 s = d0 + d1;
    // The step keeps the division finite where the result is discarded.
    vec3 m = 2.0 * p / (s + step(abs(s), vec3(1e-6)));
    return m * step(1e-12, p);
}

vec3 hermite(vec3 y0, vec3 y1, vec3 m0, vec3 m1, vec3 t) {
    vec3 t2 = t * t;
    vec3 t3 = t2 * t;
    return (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * 0.25 * m0
         + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * 0.25 * m1;
}

// A monotone cubic per channel through (0,0), (.25,a), (.5,b), (.75,c), (1,1).
// Past 1 it continues along its end slope.
vec3 curve(vec3 x, vec3 a, vec3 b, vec3 c) {
    vec3 d0 = a * 4.0;
    vec3 d1 = (b - a) * 4.0;
    vec3 d2 = (c - b) * 4.0;
    vec3 d3 = (1.0 - c) * 4.0;
    vec3 m0 = d0;
    vec3 m1 = tangent(d0, d1);
    vec3 m2 = tangent(d1, d2);
    vec3 m3 = tangent(d2, d3);
    vec3 m4 = d3;

    vec3 s0 = hermite(vec3(0.0), a, m0, m1, x * 4.0);
    vec3 s1 = hermite(a, b, m1, m2, x * 4.0 - 1.0);
    vec3 s2 = hermite(b, c, m2, m3, x * 4.0 - 2.0);
    vec3 s3 = hermite(c, vec3(1.0), m3, m4, x * 4.0 - 3.0);
    vec3 y = mix(s0, s1, step(0.25, x));
    y = mix(y, s2, step(0.5, x));
    y = mix(y, s3, step(0.75, x));
    return mix(y, 1.0 + (x - 1.0) * m4, step(1.0, x));
}

void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec3 e = encode(max(src.rgb, 0.0));
    e = curve(e, vec3(red_25, green_25, blue_25), vec3(red_50, green_50, blue_50), vec3(red_75, green_75, blue_75));
    e = curve(max(e, 0.0), vec3(master_25), vec3(master_50), vec3(master_75));
    vec3 col = mix(src.rgb, decode(max(e, 0.0)), amount);
    fragColor = vec4(max(col, 0.0), src.a);
}
