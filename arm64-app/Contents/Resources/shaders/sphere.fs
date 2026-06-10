/*{
    "DESCRIPTION": "Sphere - spherical/fisheye lens distortion",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "sphere_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.5, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.1, "MAX": 2.0},
        {"NAME": "center_x", "LABEL": "Center X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "center_y", "LABEL": "Center Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0}
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
    float sphere_amount;
    float radius;
    float center_x;
    float center_y;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 center = vec2(center_x, center_y);
    vec2 p = uv - center;
    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    p.x *= aspect;

    float dist = length(p);
    float r = radius;

    vec2 newP = p;
    if (dist < r) {
        float normalDist = dist / r;
        // Barrel (positive) or pincushion (negative) distortion
        float power = 1.0 + sphere_amount * 2.0;
        float newDist = pow(normalDist, power) * r;
        newP = normalize(p) * newDist;
    }

    newP.x /= aspect;
    vec2 newUV = newP + center;

    vec4 result = texture(sampler2D(inputImage, texSampler), clamp(newUV, 0.0, 1.0));
    fragColor = result;
}
