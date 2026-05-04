/*{
    "DESCRIPTION": "Pinch/Bulge - radial pinch or bulge distortion",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "pinch_amount", "LABEL": "Amount (-=Pinch +=Bulge)", "TYPE": "float", "DEFAULT": 0.5, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 1.5},
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float pinch_amount;
    float radius;
    float center_x;
    float center_y;
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
    float r = radius;

    if (dist < r) {
        float normalDist = dist / r;
        float factor;

        if (pinch_amount > 0.0) {
            // Bulge: push outward
            factor = mix(1.0, pow(normalDist, 1.0 + pinch_amount * 2.0) / normalDist, smoothstep(0.0, r, dist));
            factor = pow(normalDist, 1.0 - pinch_amount * 0.8) / (normalDist + 0.001);
        } else {
            // Pinch: pull inward
            factor = pow(normalDist, 1.0 + abs(pinch_amount) * 2.0) / (normalDist + 0.001);
        }

        p *= factor;
    }

    p.x /= aspect;
    vec2 newUV = p + center;

    fragColor = texture(sampler2D(inputImage, texSampler), clamp(newUV, 0.0, 1.0));
}
