/*{
    "DESCRIPTION": "Shape Mask - mask area with selectable shape, position, size, feather, and fill color/opacity",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "shape", "LABEL": "Shape (0=Circle 1=Rect 2=Diamond 3=Star 4=Heart 5=Hex)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 5.0},
        {"NAME": "pos_x", "LABEL": "Position X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "pos_y", "LABEL": "Position Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "size", "LABEL": "Size", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.01, "MAX": 1.5},
        {"NAME": "aspect", "LABEL": "Aspect Ratio", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 5.0},
        {"NAME": "rotation", "LABEL": "Rotation", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 6.283},
        {"NAME": "feather", "LABEL": "Feather", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.3},
        {"NAME": "invert_mask", "LABEL": "Invert Mask", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "fill_opacity", "LABEL": "Fill Opacity", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "fill_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Fill Color"}
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

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float shape;
    float pos_x;
    float pos_y;
    float size;
    float aspect;
    float rotation;
    float feather;
    float invert_mask;
    float fill_opacity;
    vec4 fill_color;
};

// Signed distance functions (negative = inside shape)
float sdCircle(vec2 p, float r) {
    return length(p) - r;
}

float sdBox(vec2 p, vec2 b) {
    vec2 d = abs(p) - b;
    return length(max(d, 0.0)) + min(max(d.x, d.y), 0.0);
}

float sdDiamond(vec2 p, float r) {
    return (abs(p.x) + abs(p.y)) - r;
}

float sdStar(vec2 p, float r) {
    // 5-pointed star
    float a = atan(p.y, p.x) + 1.5708;
    float seg = 6.2832 / 5.0;
    a = abs(mod(a, seg) - seg * 0.5);
    float d1 = length(p) * cos(a) - r * 0.5;
    float d2 = length(p) - r;
    return max(d1, d2 * 0.6);
}

float sdHeart(vec2 p, float r) {
    p.y -= r * 0.3;
    p.x = abs(p.x);
    float b = sqrt(2.0) / 3.0;
    if (p.x + p.y > r * b) {
        return length(p - r * vec2(b, b) * 0.5) - r * 0.35;
    }
    return max(length(p - r * vec2(0.0, 0.35)) - r * 0.55,
               -(p.x + p.y));
}

float sdHexagon(vec2 p, float r) {
    vec2 q = abs(p);
    return max(q.x * 0.866 + q.y * 0.5, q.y) - r;
}

vec2 rotate2d(vec2 p, float a) {
    float c = cos(a);
    float s = sin(a);
    return vec2(c * p.x - s * p.y, s * p.x + c * p.y);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Transform UV to shape space: center on position, correct aspect ratio, apply rotation
    vec2 p = uv - vec2(pos_x, pos_y);
    float screenAspect = RENDERSIZE.x / RENDERSIZE.y;
    p.x *= screenAspect;
    p = rotate2d(p, rotation);
    p.x *= aspect;

    // Compute signed distance for selected shape
    int shapeIdx = int(floor(shape + 0.5));
    float d;
    if (shapeIdx == 0) {
        d = sdCircle(p, size);
    } else if (shapeIdx == 1) {
        d = sdBox(p, vec2(size, size));
    } else if (shapeIdx == 2) {
        d = sdDiamond(p, size);
    } else if (shapeIdx == 3) {
        d = sdStar(p, size);
    } else if (shapeIdx == 4) {
        d = sdHeart(p, size);
    } else {
        d = sdHexagon(p, size);
    }

    // Feathered mask: 1.0 inside shape, 0.0 outside
    float f = max(feather, 0.001);
    float mask = 1.0 - smoothstep(-f, f, d);

    if (invert_mask > 0.5) mask = 1.0 - mask;

    // Outside the mask: blend toward fill_color at fill_opacity
    // Inside the mask: show source
    vec4 filled = vec4(mix(src.rgb, fill_color.rgb, fill_opacity), mix(src.a, fill_color.a * fill_opacity, fill_opacity));
    vec4 result = mix(filled, src, mask);

    fragColor = result;
}
