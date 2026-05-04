/*{
    "DESCRIPTION": "Block Distort - scrambles image in blocky chunks",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort", "Glitch"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "block_size", "LABEL": "Block Size", "TYPE": "float", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 32.0},
        {"NAME": "scramble", "LABEL": "Scramble", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "shift_amount", "LABEL": "Shift Amount", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "anim_speed", "LABEL": "Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 10.0},
        {"NAME": "color_split", "LABEL": "Color Split", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.05}
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
    float block_size;
    float scramble;
    float shift_amount;
    float anim_speed;
    float color_split;
};

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 blocks = floor(uv * block_size);
    float t = floor(TIME * anim_speed * 2.0); // Discrete time steps for glitchy feel

    float blockHash = hash(blocks + t * 0.1);

    vec2 offset = vec2(0.0);
    if (blockHash < scramble) {
        // Scramble this block
        float h1 = hash(blocks * 3.17 + t);
        float h2 = hash(blocks * 7.31 + t);
        offset = (vec2(h1, h2) - 0.5) * shift_amount * 2.0;
    }

    vec2 newUV = uv + offset;
    newUV = clamp(newUV, 0.0, 1.0);

    if (color_split > 0.001) {
        float r = texture(sampler2D(inputImage, texSampler), newUV + vec2(color_split, 0.0)).r;
        float g = texture(sampler2D(inputImage, texSampler), newUV).g;
        float b = texture(sampler2D(inputImage, texSampler), newUV - vec2(color_split, 0.0)).b;
        float a = texture(sampler2D(inputImage, texSampler), newUV).a;
        fragColor = vec4(r, g, b, a);
    } else {
        fragColor = texture(sampler2D(inputImage, texSampler), newUV);
    }
}
