/*{
    "DESCRIPTION": "Vignette effect - darkens edges of frame",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "intensity", "LABEL": "Intensity", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 0.75, "MIN": 0.1, "MAX": 1.5},
        {"NAME": "softness", "LABEL": "Softness", "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.01, "MAX": 1.0},
        {"NAME": "vignette_color", "LABEL": "Vignette Color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0]},
        {"NAME": "roundness", "LABEL": "Roundness", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
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
    float intensity;
    float radius;
    float softness;
    vec4 vignette_color;
    float roundness;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 color = texture(sampler2D(inputImage, texSampler), uv);

    vec2 p = uv - 0.5;

    // Aspect ratio correction blended with roundness
    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    p.x *= mix(1.0, aspect, roundness);

    float dist = length(p);

    // Vignette falloff
    float vignette = smoothstep(radius, radius - softness, dist);
    vignette = pow(vignette, intensity);

    // Apply vignette
    color.rgb = mix(vignette_color.rgb, color.rgb, vignette);

    fragColor = color;
}
