/*{
    "DESCRIPTION": "Fire - procedural animated fire effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "fire_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "fire_scale", "TYPE": "float", "DEFAULT": 4.0, "MIN": 1.0, "MAX": 12.0, "LABEL": "Scale"},
        {"NAME": "intensity", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 3.0, "LABEL": "Intensity"},
        {"NAME": "height", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.2, "MAX": 1.0, "LABEL": "Height"},
        {"NAME": "turbulence", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Turbulence"},
        {"NAME": "color_hot", "TYPE": "color", "DEFAULT": [1.0, 1.0, 0.6, 1.0], "LABEL": "Hot Color"},
        {"NAME": "color_mid", "TYPE": "color", "DEFAULT": [1.0, 0.4, 0.0, 1.0], "LABEL": "Mid Color"},
        {"NAME": "color_cool", "TYPE": "color", "DEFAULT": [0.3, 0.0, 0.0, 1.0], "LABEL": "Cool Color"}
    ],
    "PHASE_INPUTS": [{"PARAM": "fire_speed", "INDEX": 0}]
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
    float fire_speed;
    float fire_scale;
    float intensity;
    float height;
    float turbulence;
    vec4 color_hot;
    vec4 color_mid;
    vec4 color_cool;
};

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    for (int i = 0; i < 5; i++) {
        v += a * noise(p);
        p *= 2.0;
        a *= 0.5;
    }
    return v;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv;
    float t = PHASE_TIME_0;

    // Fire rises upward
    vec2 fireCoord = p * fire_scale;
    fireCoord.y -= t * 2.0; // Scroll up

    // Turbulent noise
    float n1 = fbm(fireCoord);
    float n2 = fbm(fireCoord * 2.0 + vec2(5.2, 1.3));
    float n = n1 + n2 * turbulence * 0.3;

    // Height gradient: fire is strongest at bottom
    float heightMask = 1.0 - smoothstep(0.0, height, p.y);
    heightMask *= heightMask;

    // Fire intensity
    float fire = n * heightMask * intensity;
    fire = clamp(fire, 0.0, 1.0);

    // Horizontal taper
    float xFade = 1.0 - pow(abs(p.x - 0.5) * 2.0, 2.0);
    fire *= clamp(xFade, 0.0, 1.0);

    // Color mapping: cool -> mid -> hot
    vec3 col;
    if (fire < 0.4) {
        col = mix(vec3(0.0), color_cool.rgb, fire * 2.5);
    } else if (fire < 0.7) {
        col = mix(color_cool.rgb, color_mid.rgb, (fire - 0.4) * 3.33);
    } else {
        col = mix(color_mid.rgb, color_hot.rgb, (fire - 0.7) * 3.33);
    }

    fragColor = vec4(col, clamp(fire * 2.0, 0.0, 1.0));
}
