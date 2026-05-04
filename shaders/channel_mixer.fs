/*{
    "DESCRIPTION": "Channel Mixer - reroute and mix RGB channels",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "rr", "LABEL": "Red → Red", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "rg", "LABEL": "Green → Red", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "rb", "LABEL": "Blue → Red", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "gr", "LABEL": "Red → Green", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "gg", "LABEL": "Green → Green", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "gb", "LABEL": "Blue → Green", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "br", "LABEL": "Red → Blue", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "bg_param", "LABEL": "Green → Blue", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "bb", "LABEL": "Blue → Blue", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0}
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
    float rr;
    float rg;
    float rb;
    float gr;
    float gg;
    float gb;
    float br;
    float bg_param;
    float bb;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    mat3 mixMatrix = mat3(
        rr, gr, br,
        rg, gg, bg_param,
        rb, gb, bb
    );

    vec3 result = clamp(mixMatrix * src.rgb, 0.0, 1.0);
    fragColor = vec4(result, src.a);
}
