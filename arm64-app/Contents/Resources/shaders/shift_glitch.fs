/*{
    "DESCRIPTION": "Digital glitch / shift glitch effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Glitch"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "glitch_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "block_size", "LABEL": "Block Size", "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.01, "MAX": 0.3},
        {"NAME": "shift_intensity", "LABEL": "Shift Intensity", "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.0, "MAX": 0.2},
        {"NAME": "color_shift", "LABEL": "Color Shift", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "glitch_rate", "LABEL": "Rate", "TYPE": "float", "DEFAULT": 5.0, "MIN": 0.5, "MAX": 30.0}
    ],
    "PHASE_INPUTS": [{"PARAM": "glitch_rate", "INDEX": 0}]
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
    float glitch_amount;
    float block_size;
    float shift_intensity;
    float color_shift;
    float glitch_rate;
};

// Pseudo-random
float hash(float n) { return fract(sin(n) * 43758.5453); }
float hash2(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float amt = glitch_amount;

    // Time-quantized seed for glitch blocks
    float timeSeed = floor(PHASE_TIME_0);

    // Block row
    float blockY = floor(uv.y / block_size);
    float blockRand = hash2(vec2(blockY, timeSeed));

    vec2 shiftedUV = uv;

    // Only glitch some rows
    if (blockRand < amt) {
        // Horizontal shift
        float shiftAmount = (hash(blockY + timeSeed * 13.7) - 0.5) * shift_intensity * 2.0;
        shiftedUV.x += shiftAmount;

        // Occasional vertical jump
        if (hash(blockY + timeSeed * 7.3) > 0.8) {
            shiftedUV.y += (hash(blockY + timeSeed * 3.1) - 0.5) * block_size * 2.0;
        }
    }

    shiftedUV = fract(shiftedUV);

    vec4 color = texture(sampler2D(inputImage, texSampler), shiftedUV);

    // Color channel separation on glitched blocks
    if (blockRand < amt && color_shift > 0.01) {
        float cs = color_shift * 0.02;
        color.r = texture(sampler2D(inputImage, texSampler), fract(shiftedUV + vec2(cs, 0.0))).r;
        color.b = texture(sampler2D(inputImage, texSampler), fract(shiftedUV - vec2(cs, 0.0))).b;
    }

    fragColor = color;
}
