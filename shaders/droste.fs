/*{
    "DESCRIPTION": "Droste - the frame contains itself, nested forever, with an endless zoom and optional Escher spiral",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "scale", "LABEL": "Scale", "TYPE": "float", "DEFAULT": 2.5, "MIN": 1.5, "MAX": 8.0},
        {"NAME": "zoom_speed", "LABEL": "Zoom Speed", "TYPE": "float", "DEFAULT": 0.25, "MIN": -2.0, "MAX": 2.0},
        {"NAME": "spiral", "LABEL": "Spiral", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "strands", "LABEL": "Strands", "TYPE": "long", "DEFAULT": 1, "VALUES": [1, 2, 3, 4], "LABELS": ["1", "2", "3", "4"]},
        {"NAME": "rotation_speed", "LABEL": "Rotation Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0},
        {"NAME": "center_x", "LABEL": "Center X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.05, "MAX": 0.95},
        {"NAME": "center_y", "LABEL": "Center Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.05, "MAX": 0.95},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "zoom_speed", "INDEX": 0, "SCALE": 1.0},
        {"PARAM": "rotation_speed", "INDEX": 1, "SCALE": 1.0}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float scale;
    float zoom_speed;
    float spiral;
    int strands;
    float rotation_speed;
    float center_x;
    float center_y;
    float amount;
};

const float TAU = 6.28318530718;

// Distance from the center as a fraction of the way to the frame edge along q,
// so the nested copies keep the frame's shape even with an off-center center.
float frameNorm(vec2 q, vec2 c) {
    vec2 room = vec2(q.x > 0.0 ? 1.0 - c.x : c.x, q.y > 0.0 ? 1.0 - c.y : c.y);
    vec2 f = abs(q) / max(room, 1e-3);
    return max(max(f.x, f.y), 1e-6);
}

// Samples the self-similar image at log-polar position (lr, th) of the output.
vec4 drosteTap(float lr, float th, float beta, float logScale, vec2 c, vec2 aspect) {
    // Multiplying by (1 - i*beta) shears the log-polar lattice into a spiral.
    vec2 w = vec2(lr + beta * th, th - beta * lr);
    w.x = mod(w.x - PHASE_TIME_0 * logScale, logScale) - logScale;
    w.y += PHASE_TIME_1 * TAU;
    vec2 q = exp(w.x) * vec2(cos(w.y), sin(w.y)) / aspect;
    // Rescale by whole powers of `scale` until q lies between the frame and its copy.
    q *= exp(-ceil(log(frameNorm(q, c)) / logScale) * logScale);
    return texture(sampler2D(inputImage, texSampler), c + q);
}

void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec2 aspect = vec2(RENDERSIZE.x / RENDERSIZE.y, 1.0);
    vec2 c = vec2(center_x, center_y);
    vec2 p = (uv - c) * aspect;
    float lr = log(max(length(p), 1e-6));
    float th = atan(p.y, p.x);
    float logScale = log(scale);
    float twist = spiral * float(strands);
    float beta = twist * logScale / TAU;

    vec4 col = drosteTap(lr, th, beta, logScale, c, aspect);
    // A twist that is not a whole number leaves a seam on the angle's branch
    // cut. In a wedge around it, blend in a tap whose cut is on the other side.
    float wedge = smoothstep(2.5, 3.14159265, abs(th));
    if (wedge > 0.0 && abs(twist - round(twist)) > 1e-3) {
        float thB = th < 0.0 ? th + TAU : th;
        col = mix(col, drosteTap(lr, thB, beta, logScale, c, aspect), wedge);
    }

    vec4 res = mix(src, col, amount);
    fragColor = vec4(max(res.rgb, 0.0), clamp(res.a, 0.0, 1.0));
}
