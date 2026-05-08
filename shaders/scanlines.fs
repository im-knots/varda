/*{
    "DESCRIPTION": "Scan Lines - CRT-style horizontal scan lines",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "line_density", "LABEL": "Line Density", "TYPE": "float", "DEFAULT": 400.0, "MIN": 50.0, "MAX": 1000.0},
        {"NAME": "line_darkness", "LABEL": "Darkness", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "line_width", "LABEL": "Line Width", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 0.9},
        {"NAME": "flicker", "LABEL": "Flicker", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.3},
        {"NAME": "scroll_speed", "LABEL": "Scroll Speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 5.0},
        {"NAME": "horizontal", "LABEL": "Horizontal", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [{"PARAM": "scroll_speed", "INDEX": 0, "SCALE": 0.01}]
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
    float line_density;
    float line_darkness;
    float line_width;
    float flicker;
    float scroll_speed;
    float horizontal;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    float coord = (horizontal > 0.5) ? uv.y : uv.x;
    coord += PHASE_TIME_0;

    float scanline = fract(coord * line_density);
    float line = smoothstep(line_width, line_width + 0.1, scanline);

    // Flicker
    float flick = 1.0 - flicker * sin(TIME * 60.0) * 0.5;

    float mask = mix(1.0, 1.0 - line_darkness, 1.0 - line);
    mask *= flick;

    fragColor = vec4(src.rgb * mask, src.a);
}
