/*{
    "DESCRIPTION": "Wave warp distortion effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "amplitude", "LABEL": "Amplitude", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.1},
        {"NAME": "frequency", "LABEL": "Frequency", "TYPE": "float", "DEFAULT": 10.0, "MIN": 1.0, "MAX": 50.0},
        {"NAME": "wave_speed", "LABEL": "Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0},
        {"NAME": "direction", "LABEL": "Direction (0=H 1=V 2=Both)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "wave_type", "LABEL": "Type (0=Sine 1=Triangle 2=Square)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0}
    ],
    "PHASE_INPUTS": [{"PARAM": "wave_speed", "INDEX": 0}]
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
    float amplitude;
    float frequency;
    float wave_speed;
    float direction;
    float wave_type;
};

#define PI 3.14159265359

float waveFunc(float x, float wt) {
    float t = floor(wt + 0.5);
    if (t < 0.5) {
        return sin(x); // Sine
    } else if (t < 1.5) {
        return abs(fract(x / (2.0 * PI)) * 2.0 - 1.0) * 2.0 - 1.0; // Triangle
    } else {
        return sign(sin(x)); // Square
    }
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float amp = amplitude;

    float t = PHASE_TIME_0;
    float dir = floor(direction + 0.5);

    vec2 warpedUV = uv;

    if (dir < 0.5) {
        // Horizontal waves
        warpedUV.x += waveFunc(uv.y * frequency + t, wave_type) * amp;
    } else if (dir < 1.5) {
        // Vertical waves
        warpedUV.y += waveFunc(uv.x * frequency + t, wave_type) * amp;
    } else {
        // Both
        warpedUV.x += waveFunc(uv.y * frequency + t, wave_type) * amp;
        warpedUV.y += waveFunc(uv.x * frequency * 0.7 + t * 1.3, wave_type) * amp * 0.7;
    }

    warpedUV = clamp(warpedUV, 0.0, 1.0);
    fragColor = texture(sampler2D(inputImage, texSampler), warpedUV);
}
