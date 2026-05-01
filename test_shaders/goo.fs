/*{
    "DESCRIPTION": "Goo / liquid distortion effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.03, "MIN": 0.0, "MAX": 0.15},
        {"NAME": "goo_scale", "LABEL": "Scale", "TYPE": "float", "DEFAULT": 3.0, "MIN": 0.5, "MAX": 15.0},
        {"NAME": "goo_speed", "LABEL": "Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0},
        {"NAME": "complexity", "LABEL": "Complexity", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 6.0},
        {"NAME": "audio_reactive", "LABEL": "Audio Reactive", "TYPE": "bool", "DEFAULT": true}
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
    float amount;
    float goo_scale;
    float goo_speed;
    float complexity;
    float audio_reactive;
};

// Simple hash noise
vec2 hash22(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
    return sin(p) * 43758.5453;
}

float noise2d(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);
    return mix(mix(sin(dot(hash22(i), f)),
                   sin(dot(hash22(i + vec2(1,0)), f - vec2(1,0))), u.x),
               mix(sin(dot(hash22(i + vec2(0,1)), f - vec2(0,1))),
                   sin(dot(hash22(i + vec2(1,1)), f - vec2(1,1))), u.x), u.y);
}

float fbm2(vec2 p, int oct) {
    float val = 0.0;
    float amp = 0.5;
    for (int i = 0; i < 6; i++) {
        if (i >= oct) break;
        val += amp * noise2d(p);
        p *= 2.0;
        amp *= 0.5;
    }
    return val;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float t = TIME * goo_speed * 0.3;
    int oct = int(clamp(complexity, 1.0, 6.0));

    float amt = amount;
    if (audio_reactive > 0.5) {
        amt *= (1.0 + audio_bass * 3.0);
    }

    // Distortion offset from noise
    float dx = fbm2(uv * goo_scale + vec2(t, 0.0), oct);
    float dy = fbm2(uv * goo_scale + vec2(0.0, t + 100.0), oct);

    vec2 distortedUV = uv + vec2(dx, dy) * amt;
    distortedUV = clamp(distortedUV, 0.0, 1.0);

    fragColor = texture(sampler2D(inputImage, texSampler), distortedUV);
}
