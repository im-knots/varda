/*{
    "DESCRIPTION": "Transform - 2D translate, rotate, scale effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "pos_x", "LABEL": "Position X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "pos_y", "LABEL": "Position Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "rotation", "LABEL": "Rotation", "TYPE": "float", "DEFAULT": 0.0, "MIN": -6.283, "MAX": 6.283},
        {"NAME": "scale_x", "LABEL": "Scale X", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 5.0},
        {"NAME": "scale_y", "LABEL": "Scale Y", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 5.0},
        {"NAME": "wrap", "LABEL": "Wrap (0=Clamp 1=Repeat 2=Mirror)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0}
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
    float pos_x;
    float pos_y;
    float rotation;
    float scale_x;
    float scale_y;
    float wrap;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Center, then apply transforms in reverse order
    vec2 p = uv - 0.5;

    // Scale
    p /= vec2(scale_x, scale_y);

    // Rotate
    float ca = cos(-rotation), sa = sin(-rotation);
    p = vec2(p.x * ca - p.y * sa, p.x * sa + p.y * ca);

    // Translate
    p -= vec2(pos_x, pos_y);

    // Back to UV space
    p += 0.5;

    // Wrap mode
    int w = int(floor(wrap + 0.5));
    if (w == 1) {
        p = fract(p);
    } else if (w == 2) {
        p = abs(mod(p, 2.0) - 1.0);
    } else {
        // Clamp — black outside
        if (p.x < 0.0 || p.x > 1.0 || p.y < 0.0 || p.y > 1.0) {
            fragColor = vec4(0.0);
            return;
        }
    }

    fragColor = texture(sampler2D(inputImage, texSampler), p);
}
