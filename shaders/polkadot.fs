/*{
    "DESCRIPTION": "Polka Dot - circular dot pattern overlay",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "dot_density", "LABEL": "Density", "TYPE": "float", "DEFAULT": 20.0, "MIN": 4.0, "MAX": 60.0},
        {"NAME": "dot_radius", "LABEL": "Dot Size", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.05, "MAX": 0.5},
        {"NAME": "dot_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Dot Color"},
        {"NAME": "use_source", "LABEL": "Use Source Color", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    float dot_density;
    float dot_radius;
    vec4 dot_color;
    float use_source;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    vec2 coord = uv * dot_density;
    coord.x *= RENDERSIZE.x / RENDERSIZE.y;
    vec2 cell = floor(coord) + 0.5;
    vec2 f = fract(coord) - 0.5;

    float dist = length(f);
    float dot = smoothstep(dot_radius, dot_radius - 0.02, dist);

    // Sample source at cell center for color
    vec2 cellUV = cell / dot_density;
    cellUV.x /= RENDERSIZE.x / RENDERSIZE.y;
    vec4 cellColor = texture(sampler2D(inputImage, texSampler), cellUV);

    vec3 dColor = mix(dot_color.rgb, cellColor.rgb, use_source);
    vec3 result = mix(src.rgb, dColor, dot);

    fragColor = vec4(result, src.a);
}
