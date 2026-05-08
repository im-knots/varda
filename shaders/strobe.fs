/*{
    "DESCRIPTION": "Strobe - flash to solid color on beat or timer",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "strobe_rate", "LABEL": "Rate (Hz)", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.5, "MAX": 30.0},
        {"NAME": "strobe_duty", "LABEL": "Duty Cycle", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.05, "MAX": 0.95},
        {"NAME": "strobe_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Flash Color"},
        {"NAME": "mix_mode", "LABEL": "Mode (0=Replace 1=Add 2=Invert)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0}
    ],
    "PHASE_INPUTS": [{"PARAM": "strobe_rate", "INDEX": 0}]
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
    float strobe_rate;
    float strobe_duty;
    vec4 strobe_color;
    float mix_mode;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Determine strobe phase
    float phase = fract(PHASE_TIME_0);

    float flash = step(phase, strobe_duty);
    int mode = int(floor(mix_mode + 0.5));

    vec4 result;
    if (mode == 0) {
        // Replace: show flash color or source
        result = mix(src, strobe_color, flash);
    } else if (mode == 1) {
        // Add: add flash color on top
        result = src + strobe_color * flash;
        result = clamp(result, 0.0, 1.0);
    } else {
        // Invert on flash
        vec4 inv = vec4(1.0 - src.rgb, src.a);
        result = mix(src, inv, flash);
    }

    fragColor = vec4(result.rgb * result.a, result.a);
}
