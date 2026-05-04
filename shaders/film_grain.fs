/*{
    "DESCRIPTION": "Film Grain - analog film grain noise overlay",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "grain_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "grain_size", "LABEL": "Size", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 3.0},
        {"NAME": "grain_color", "LABEL": "Color Grain", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "luma_response", "LABEL": "Luminance Response", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0}
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
    float grain_amount;
    float grain_size;
    float grain_color;
    float luma_response;
};

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    vec2 grainCoord = uv * RENDERSIZE / grain_size;
    float frame = float(FRAMEINDEX);

    // Per-frame noise
    float n = hash(grainCoord + frame * 0.37) * 2.0 - 1.0;

    // Color grain
    vec3 grainVec;
    if (grain_color > 0.01) {
        grainVec = vec3(
            hash(grainCoord + frame * 0.37),
            hash(grainCoord + frame * 0.73),
            hash(grainCoord + frame * 1.13)
        ) * 2.0 - 1.0;
        grainVec = mix(vec3(n), grainVec, grain_color);
    } else {
        grainVec = vec3(n);
    }

    // More grain in shadows (film characteristic)
    float lum = dot(src.rgb, vec3(0.299, 0.587, 0.114));
    float lumaFactor = mix(1.0, 1.0 - lum, luma_response);

    vec3 result = src.rgb + grainVec * grain_amount * lumaFactor;
    result = clamp(result, 0.0, 1.0);

    fragColor = vec4(result, src.a);
}
