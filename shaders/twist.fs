/*{
    "DESCRIPTION": "Twist/Twirl - rotational distortion from center",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "twist_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 2.0, "MIN": -10.0, "MAX": 10.0},
        {"NAME": "radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.05, "MAX": 1.5},
        {"NAME": "center_x", "LABEL": "Center X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "center_y", "LABEL": "Center Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "animate", "LABEL": "Animate", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0}
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

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float twist_amount;
    float radius;
    float center_x;
    float center_y;
    float animate;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 center = vec2(center_x, center_y);
    vec2 p = uv - center;
    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    p.x *= aspect;

    float dist = length(p);
    float t = twist_amount + sin(TIME * animate) * animate;

    if (dist < radius) {
        float factor = 1.0 - (dist / radius);
        factor = factor * factor; // Quadratic falloff
        float angle = factor * t;

        float ca = cos(angle), sa = sin(angle);
        p = vec2(p.x * ca - p.y * sa, p.x * sa + p.y * ca);
    }

    p.x /= aspect;
    vec2 newUV = p + center;
    vec4 result = texture(sampler2D(inputImage, texSampler), newUV);

    // Black outside bounds
    if (newUV.x < 0.0 || newUV.x > 1.0 || newUV.y < 0.0 || newUV.y > 1.0)
        result = vec4(0.0);

    fragColor = result;
}
