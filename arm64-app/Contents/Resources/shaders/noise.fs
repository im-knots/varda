/*{
    "DESCRIPTION": "Procedural noise generator - simplex-style animated noise",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "scale", "TYPE": "float", "DEFAULT": 4.0, "MIN": 0.5, "MAX": 20.0, "LABEL": "Scale"},
        {"NAME": "octaves", "TYPE": "float", "DEFAULT": 4.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Octaves"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "contrast", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Contrast"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Color 1"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Color 2"},
        {"NAME": "color_mode", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Color Mode (0=Gradient 1=RGB)"}
    ],
    "PHASE_INPUTS": [{"PARAM": "anim_speed", "INDEX": 0}]
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

layout(set = 0, binding = 1) uniform UserParams {
    float scale;
    float octaves;
    float anim_speed;
    float contrast;
    vec4 color1;
    vec4 color2;
    float color_mode;
};

// Hash-based pseudo-random
vec3 hash33(vec3 p) {
    p = vec3(dot(p, vec3(127.1, 311.7, 74.7)),
             dot(p, vec3(269.5, 183.3, 246.1)),
             dot(p, vec3(113.5, 271.9, 124.6)));
    return fract(sin(p) * 43758.5453123) * 2.0 - 1.0;
}

// 3D value noise
float noise3d(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    vec3 u = f * f * (3.0 - 2.0 * f);

    float n000 = dot(hash33(i), f);
    float n100 = dot(hash33(i + vec3(1,0,0)), f - vec3(1,0,0));
    float n010 = dot(hash33(i + vec3(0,1,0)), f - vec3(0,1,0));
    float n110 = dot(hash33(i + vec3(1,1,0)), f - vec3(1,1,0));
    float n001 = dot(hash33(i + vec3(0,0,1)), f - vec3(0,0,1));
    float n101 = dot(hash33(i + vec3(1,0,1)), f - vec3(1,0,1));
    float n011 = dot(hash33(i + vec3(0,1,1)), f - vec3(0,1,1));
    float n111 = dot(hash33(i + vec3(1,1,1)), f - vec3(1,1,1));

    return mix(mix(mix(n000, n100, u.x), mix(n010, n110, u.x), u.y),
               mix(mix(n001, n101, u.x), mix(n011, n111, u.x), u.y), u.z);
}

float fbm(vec3 p, int oct) {
    float val = 0.0;
    float amp = 0.5;
    float freq = 1.0;
    for (int i = 0; i < 8; i++) {
        if (i >= oct) break;
        val += amp * noise3d(p * freq);
        freq *= 2.0;
        amp *= 0.5;
    }
    return val;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float t = PHASE_TIME_0;
    int oct = int(clamp(octaves, 1.0, 8.0));

    float n = fbm(vec3(p * scale, t), oct);
    n = n * 0.5 + 0.5; // normalize to 0-1
    n = clamp((n - 0.5) * contrast + 0.5, 0.0, 1.0);

    vec3 col;
    if (color_mode > 0.5) {
        // RGB noise - independent channels
        float nr = fbm(vec3(p * scale, t + 100.0), oct) * 0.5 + 0.5;
        float ng = fbm(vec3(p * scale, t + 200.0), oct) * 0.5 + 0.5;
        float nb = fbm(vec3(p * scale, t + 300.0), oct) * 0.5 + 0.5;
        col = vec3(nr, ng, nb) * contrast;
    } else {
        col = mix(color1.rgb, color2.rgb, n);
    }

    fragColor = vec4(col, 1.0);
}
