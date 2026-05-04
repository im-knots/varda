/*{
    "DESCRIPTION": "Geometric shape generator - circle, triangle, square, star, polygon",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "shape", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 4.0, "LABEL": "Shape (0=Circle 1=Triangle 2=Square 3=Pentagon 4=Star)"},
        {"NAME": "shape_size", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.05, "MAX": 0.8, "LABEL": "Size"},
        {"NAME": "edge_width", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.1, "LABEL": "Edge Width"},
        {"NAME": "fill", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Fill"},
        {"NAME": "rotation_speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 5.0, "LABEL": "Rotation Speed"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [1.0, 0.3, 0.5, 1.0], "LABEL": "Shape Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
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
    float shape;
    float shape_size;
    float edge_width;
    float fill;
    float rotation_speed;
    vec4 color1;
    vec4 color2;
};

#define PI 3.14159265359
#define TAU 6.28318530718

// SDF for regular polygon with n sides
float sdPolygon(vec2 p, float r, int n) {
    float a = atan(p.y, p.x);
    float s = TAU / float(n);
    float d = cos(floor(0.5 + a / s) * s - a) * length(p);
    return d - r;
}

// SDF for star
float sdStar(vec2 p, float r, int n) {
    float a = atan(p.y, p.x);
    float s = TAU / float(n);
    float sa = mod(a, s) - s * 0.5;
    float l = length(p);
    float inner = r * 0.4;
    float d = cos(floor(0.5 + a / s) * s - a) * l;
    float d2 = cos(sa) * l;
    return mix(d - r, d2 - inner, 0.5);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    // Rotate
    float rot = TIME * rotation_speed;
    float ca = cos(rot), sa = sin(rot);
    p = vec2(p.x * ca - p.y * sa, p.x * sa + p.y * ca);

    float d;
    int st = int(floor(shape + 0.5));

    if (st == 0) {
        d = length(p) - shape_size;
    } else if (st == 1) {
        d = sdPolygon(p, shape_size, 3);
    } else if (st == 2) {
        d = sdPolygon(p, shape_size, 4);
    } else if (st == 3) {
        d = sdPolygon(p, shape_size, 5);
    } else {
        d = sdStar(p, shape_size, 5);
    }

    // Fill vs outline
    float shape_mask;
    if (fill > 0.5) {
        shape_mask = smoothstep(0.005, -0.005, d);
    } else {
        shape_mask = smoothstep(edge_width + 0.005, edge_width, abs(d));
    }

    // Edge glow
    float glow = exp(-abs(d) * 20.0) * 0.3;

    vec4 color = mix(color2, color1, shape_mask);
    color.rgb += color1.rgb * glow;
    color.rgb = clamp(color.rgb, 0.0, 1.0);
    fragColor = vec4(color.rgb * color.a, color.a);
}
