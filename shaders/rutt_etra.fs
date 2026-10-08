/*{
    "DESCRIPTION": "Rutt-Etra - scanlines lifted by brightness and drawn as glowing lines on black",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "lines", "LABEL": "Lines", "TYPE": "float", "DEFAULT": 90.0, "MIN": 20.0, "MAX": 300.0},
        {"NAME": "displacement", "LABEL": "Displacement", "TYPE": "float", "DEFAULT": 0.12, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "line_width", "LABEL": "Line Width", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0},
        {"NAME": "brightness", "LABEL": "Brightness", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "color_mix", "LABEL": "Tint", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "line_color", "LABEL": "Line Color", "TYPE": "color", "DEFAULT": [0.55, 1.0, 0.6, 1.0]},
        {"NAME": "perspective", "LABEL": "Perspective", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "background", "LABEL": "Background", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "scroll_speed", "LABEL": "Scroll Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -8.0, "MAX": 8.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "scroll_speed", "INDEX": 0, "SCALE": 1.0}
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
    float lines;
    float displacement;
    float line_width;
    float brightness;
    float color_mix;
    vec4 line_color;
    float perspective;
    float background;
    float scroll_speed;
};

const int MAX_LINES = 48;

float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

// Lines are laid out on a ground plane parameter s in [0, 1] (top to bottom)
// and projected to the screen by g. Perspective packs the top rows together.
float persp() { return 0.7 * perspective; }
float g(float s) { float a = persp(); return mix(s, s * s, a); }
float gSlope(float s) { float a = persp(); return 1.0 - a + 2.0 * a * s; }
float gInverse(float y) {
    float a = persp();
    if (a < 1e-4) return y;
    float b = 1.0 - a;
    return (-b + sqrt(b * b + 4.0 * a * max(y, 0.0))) / (2.0 * a);
}

void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    float px = 1.0 / RENDERSIZE.y;
    float dx = 1.0 / RENDERSIZE.x;
    float n = max(lines, 1.0);
    // Displacement is measured along the ground plane; the loop covers every
    // line that can reach this pixel, so cap it at MAX_LINES spacings.
    float reach = min(displacement, float(MAX_LINES - 2) / n);
    float bottomSlope = 1.0 + persp();

    float sy = gInverse(uv.y);
    float margin = (line_width + 2.0) * px / gSlope(sy);
    float phase = fract(PHASE_TIME_0);
    float first = ceil((sy - margin) * n - phase);

    vec3 glow = vec3(0.0);
    float coverage = 0.0;
    for (int i = 0; i < MAX_LINES + 4; i++) {
        float s = (first + float(i) + phase) / n;
        if (s > sy + reach + margin) break;
        if (s < 0.0 || s > 1.0) continue;
        float baseY = g(s);
        vec3 c0 = texture(sampler2D(inputImage, texSampler), vec2(uv.x, baseY)).rgb;
        float l1 = luma(texture(sampler2D(inputImage, texSampler), vec2(uv.x + dx, baseY)).rgb);
        float l0 = luma(c0);
        float y0 = g(s - min(l0, 1.0) * reach);
        float y1 = g(s - min(l1, 1.0) * reach);
        // Vertical distance corrected by the line's slope so steep runs keep their width.
        float slope = (y1 - y0) / px;
        float dist = abs(uv.y - y0) / px / sqrt(1.0 + slope * slope);
        float halfWidth = 0.5 * line_width * gSlope(s) / bottomSlope;
        float cover = 1.0 - smoothstep(halfWidth - 0.5, halfWidth + 0.5, dist);
        glow += cover * mix(c0, line_color.rgb, color_mix);
        coverage += cover;
    }
    // Where lines overlap, blend their colors instead of summing them to white.
    glow /= max(coverage, 1.0);

    vec3 col = src.rgb * background + glow * brightness;
    fragColor = vec4(max(col, 0.0), src.a);
}
