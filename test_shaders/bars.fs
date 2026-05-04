/*{
    "DESCRIPTION": "Animated bars/stripes generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "bar_count", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 64.0, "LABEL": "Bar Count"},
        {"NAME": "bar_width", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.01, "MAX": 1.0, "LABEL": "Bar Width"},
        {"NAME": "angle", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.14159, "LABEL": "Angle"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Bar Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Gap Color"},
        {"NAME": "softness", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.5, "LABEL": "Softness"}
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
    float bar_count;
    float bar_width;
    float angle;
    float anim_speed;
    vec4 color1;
    vec4 color2;
    float softness;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv - 0.5;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    // Rotate
    float ca = cos(angle), sa = sin(angle);
    float coord = p.x * ca + p.y * sa;

    // Animate
    coord += TIME * anim_speed * 0.1;

    // Create bars
    float pattern = fract(coord * bar_count);
    float bar;
    if (softness > 0.001) {
        bar = smoothstep(0.5 - bar_width * 0.5 - softness, 0.5 - bar_width * 0.5, pattern)
            - smoothstep(0.5 + bar_width * 0.5, 0.5 + bar_width * 0.5 + softness, pattern);
    } else {
        bar = step(0.5 - bar_width * 0.5, pattern) * step(pattern, 0.5 + bar_width * 0.5);
    }

    vec4 color = mix(color2, color1, bar);
    fragColor = vec4(color.rgb * color.a, color.a);
}
