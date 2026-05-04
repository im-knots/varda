/*{
    "DESCRIPTION": "Tunnel - infinite zoom tunnel distortion",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "zoom_speed", "LABEL": "Zoom Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0},
        {"NAME": "twist_amount", "LABEL": "Twist", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "repeat_count", "LABEL": "Repeats", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 4.0},
        {"NAME": "center_x", "LABEL": "Center X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "center_y", "LABEL": "Center Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0}
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
    float zoom_speed;
    float twist_amount;
    float repeat_count;
    float center_x;
    float center_y;
};

#define PI 3.14159265359
#define TAU 6.28318530718

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 center = vec2(center_x, center_y);
    vec2 p = uv - center;
    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    p.x *= aspect;

    float dist = length(p);
    float angle = atan(p.y, p.x);

    float t = TIME * zoom_speed;

    // Tunnel mapping: polar to cartesian with depth
    float depth = repeat_count / (dist + 0.01);
    float tunnelU = angle / TAU + twist_amount * depth * 0.05;
    float tunnelV = depth + t * 0.5;

    // Map back to texture coordinates
    vec2 newUV = fract(vec2(tunnelU, tunnelV) * 0.5 + 0.5);

    vec4 result = texture(sampler2D(inputImage, texSampler), newUV);

    // Darken at edges (far from center = close in tunnel)
    float fade = smoothstep(0.0, 0.3, dist);
    result.rgb *= mix(1.0, 0.3, fade);

    fragColor = result;
}
