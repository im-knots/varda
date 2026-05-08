/*{
    "DESCRIPTION": "Displace - luminance-based displacement mapping",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.0, "MAX": 0.3},
        {"NAME": "direction", "LABEL": "Direction (0=Both 1=Horiz 2=Vert)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "source", "LABEL": "Source (0=Self 1=Red 2=Luma)", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "smooth_amount", "LABEL": "Smooth", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0}
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
    float amount;
    float direction;
    float source;
    float smooth_amount;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 texel = smooth_amount / RENDERSIZE;

    // Sample neighbors for gradient
    float cL = dot(texture(sampler2D(inputImage, texSampler), uv - vec2(texel.x, 0.0)).rgb, vec3(0.299, 0.587, 0.114));
    float cR = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, 0.0)).rgb, vec3(0.299, 0.587, 0.114));
    float cU = dot(texture(sampler2D(inputImage, texSampler), uv + vec2(0.0, texel.y)).rgb, vec3(0.299, 0.587, 0.114));
    float cD = dot(texture(sampler2D(inputImage, texSampler), uv - vec2(0.0, texel.y)).rgb, vec3(0.299, 0.587, 0.114));

    vec2 offset = vec2(cR - cL, cU - cD) * amount;

    int dir = int(floor(direction + 0.5));
    if (dir == 1) offset.y = 0.0;
    if (dir == 2) offset.x = 0.0;

    vec4 result = texture(sampler2D(inputImage, texSampler), uv + offset);
    fragColor = result;
}
