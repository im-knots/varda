/*{
    "DESCRIPTION": "Color gradient generator - linear, radial, or angular",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "gradient_type", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Type (0=Linear 1=Radial 2=Angular)"},
        {"NAME": "angle", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 6.283, "LABEL": "Angle"},
        {"NAME": "offset", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "LABEL": "Offset"},
        {"NAME": "repeat_mode", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Repeat"},
        {"NAME": "color_a", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Color A"},
        {"NAME": "color_b", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Color B"},
        {"NAME": "color_c", "TYPE": "color", "DEFAULT": [0.0, 0.5, 1.0, 1.0], "LABEL": "Color C"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Animation Speed"}
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

layout(set = 0, binding = 1) uniform UserParams {
    float gradient_type;
    float angle;
    float offset;
    float repeat_mode;
    vec4 color_a;
    vec4 color_b;
    vec4 color_c;
    float anim_speed;
};

#define PI 3.14159265359

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv - 0.5;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float t = 0.0;
    float gt = floor(gradient_type + 0.5);

    if (gt < 0.5) {
        // Linear gradient
        float ca = cos(angle), sa = sin(angle);
        t = dot(p, vec2(ca, sa)) + 0.5 + offset;
    } else if (gt < 1.5) {
        // Radial gradient
        t = length(p) * 2.0 + offset;
    } else {
        // Angular gradient
        t = (atan(p.y, p.x) + PI) / (2.0 * PI) + offset;
    }

    // Animation
    t += TIME * anim_speed * 0.1;

    // Repeat mode
    if (repeat_mode > 0.5) {
        t = fract(t);
    } else {
        t = clamp(t, 0.0, 1.0);
    }

    // 3-color gradient: A -> B -> C
    vec4 color;
    if (t < 0.5) {
        color = mix(color_a, color_b, t * 2.0);
    } else {
        color = mix(color_b, color_c, (t - 0.5) * 2.0);
    }

    fragColor = vec4(color.rgb * color.a, color.a);
}
