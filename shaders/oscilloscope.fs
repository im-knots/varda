/*{
    "DESCRIPTION": "Oscilloscope / Lissajous - animated waveform and Lissajous curves",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "freq_x", "TYPE": "float", "DEFAULT": 3.0, "MIN": 0.5, "MAX": 12.0, "LABEL": "Frequency X"},
        {"NAME": "freq_y", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.5, "MAX": 12.0, "LABEL": "Frequency Y"},
        {"NAME": "phase", "TYPE": "float", "DEFAULT": 1.57, "MIN": 0.0, "MAX": 6.283, "LABEL": "Phase"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "line_width", "TYPE": "float", "DEFAULT": 0.008, "MIN": 0.002, "MAX": 0.03, "LABEL": "Line Width"},
        {"NAME": "glow_amount", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Glow"},
        {"NAME": "amplitude", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.1, "MAX": 1.0, "LABEL": "Amplitude"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.4, 1.0], "LABEL": "Line Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.02, 0.0, 1.0], "LABEL": "Background"}
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
    float freq_x;
    float freq_y;
    float phase;
    float anim_speed;
    float line_width;
    float glow_amount;
    float amplitude;
    vec4 color1;
    vec4 bg_color;
};

#define PI 3.14159265359

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = TIME * anim_speed;

    // Find closest point on Lissajous curve
    float minDist = 10.0;
    int samples = 512;
    for (int i = 0; i < 512; i++) {
        float s = float(i) / float(samples) * PI * 2.0;
        vec2 curvePoint = vec2(
            sin(freq_x * s + t) * amplitude,
            sin(freq_y * s + t + phase) * amplitude
        );
        float d = length(p - curvePoint);
        minDist = min(minDist, d);
    }

    // Line with glow
    float line = smoothstep(line_width, line_width * 0.3, minDist);
    float glow = exp(-minDist * minDist / (line_width * glow_amount * 2.0 + 0.001)) * glow_amount * 0.5;

    vec3 col = bg_color.rgb;
    col += color1.rgb * (line + glow);
    col = clamp(col, 0.0, 1.0);

    fragColor = vec4(col, 1.0);
}
