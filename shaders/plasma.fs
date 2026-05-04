/*{
    "DESCRIPTION": "Simple plasma effect",
    "CREDIT": "Varda Test",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {
            "NAME": "speed",
            "TYPE": "float",
            "DEFAULT": 1.0,
            "MIN": 0.0,
            "MAX": 5.0
        },
        {
            "NAME": "color1",
            "TYPE": "color",
            "DEFAULT": [1.0, 0.0, 0.5, 1.0]
        },
        {
            "NAME": "color2",
            "TYPE": "color",
            "DEFAULT": [0.0, 0.5, 1.0, 1.0]
        }
    ]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

// ISF automatic uniforms - matches Rust ISFUniforms struct
layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
    // Audio
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4 DATE;
};

// User parameters - matches ShaderParams buffer layout
layout(set = 0, binding = 1) uniform UserParams {
    float speed;
    vec4 color1;
    vec4 color2;
};

void main() {
    // Use all ISF uniforms to prevent SPIRV from stripping them
    // Conditional based on UV - compiler cannot prove it's always false
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0 && uv.y < -1.0) {
        fragColor = vec4(audioSum, timeSum, 0.0, 1.0);
        return;
    }

    vec2 p = uv * 2.0 - 1.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float t = TIME * speed;

    // Plasma effect
    float v = 0.0;
    v += sin(p.x * 10.0 + t);
    v += sin(p.y * 10.0 + t);
    v += sin((p.x + p.y) * 10.0 + t);
    v += sin(length(p) * 10.0 + t);
    v /= 4.0;

    // Map to colors
    vec4 color = mix(color1, color2, (v + 1.0) * 0.5);

    fragColor = vec4(color.rgb * color.a, color.a);
}

