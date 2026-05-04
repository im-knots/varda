/*{
    "DESCRIPTION": "Sepia - warm vintage sepia tone",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "warmth", "LABEL": "Warmth", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0}
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
    float warmth;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Sepia matrix
    vec3 sepia;
    sepia.r = dot(src.rgb, vec3(0.393, 0.769, 0.189));
    sepia.g = dot(src.rgb, vec3(0.349, 0.686, 0.168));
    sepia.b = dot(src.rgb, vec3(0.272, 0.534, 0.131));

    // Warmth adjustment
    sepia.r += warmth * 0.1;
    sepia.b -= warmth * 0.05;
    sepia = clamp(sepia, 0.0, 1.0);

    vec3 result = mix(src.rgb, sepia, amount);
    fragColor = vec4(result, src.a);
}
