/*{
    "DESCRIPTION": "Chroma Key - picks a target color and sets matching pixels to a given opacity",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "target_color", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.0, 1.0], "LABEL": "Target Color"},
        {"NAME": "opacity", "LABEL": "Opacity", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "tolerance", "LABEL": "Tolerance", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "softness", "LABEL": "Edge Softness", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.5}
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
    vec4 target_color;
    float opacity;
    float tolerance;
    float softness;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Distance between pixel color and target color in RGB space
    float dist = distance(src.rgb, target_color.rgb);

    // How much this pixel matches: 1.0 = exact match, 0.0 = no match
    float match_amount = 1.0 - smoothstep(tolerance, tolerance + softness, dist);

    // For matching pixels, blend alpha toward the opacity setting
    // Non-matching pixels keep their original alpha
    float new_alpha = mix(src.a, opacity, match_amount);

    fragColor = vec4(src.rgb, new_alpha);
}
