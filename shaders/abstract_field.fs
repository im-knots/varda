/*{
    "DESCRIPTION": "Abstract generative field - flowing organic patterns",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "field_scale", "TYPE": "float", "DEFAULT": 3.0, "MIN": 0.5, "MAX": 10.0, "LABEL": "Scale"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "complexity", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Complexity"},
        {"NAME": "color_shift", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Color Shift"},
        {"NAME": "contrast", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 3.0, "LABEL": "Contrast"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.1, 0.3, 0.8, 1.0], "LABEL": "Color 1"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.9, 0.2, 0.5, 1.0], "LABEL": "Color 2"},
        {"NAME": "color3", "TYPE": "color", "DEFAULT": [0.1, 0.8, 0.4, 1.0], "LABEL": "Color 3"}
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
    float field_scale;
    float anim_speed;
    float complexity;
    float color_shift;
    float contrast;
    vec4 color1;
    vec4 color2;
    vec4 color3;
};

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

float fbm(vec2 p, int oct) {
    float val = 0.0, amp = 0.5;
    for (int i = 0; i < 8; i++) {
        if (i >= oct) break;
        val += amp * noise(p);
        p *= 2.0;
        amp *= 0.5;
    }
    return val;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv * field_scale;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = TIME * anim_speed * 0.3;
    int oct = int(clamp(complexity, 1.0, 8.0));

    // Domain warping for organic flow
    vec2 q = vec2(fbm(p + vec2(0.0, 0.0) + t * 0.1, oct),
                  fbm(p + vec2(5.2, 1.3) + t * 0.12, oct));
    vec2 r = vec2(fbm(p + 4.0 * q + vec2(1.7, 9.2) + t * 0.08, oct),
                  fbm(p + 4.0 * q + vec2(8.3, 2.8) + t * 0.09, oct));
    float f = fbm(p + 4.0 * r, oct);

    // Color mapping using domain warp coordinates
    float c1 = clamp((f - 0.3) * contrast, 0.0, 1.0);
    float c2 = clamp((q.x - 0.2) * contrast * 0.8, 0.0, 1.0);
    float c3 = clamp((r.y - 0.1) * contrast * 0.6, 0.0, 1.0);

    // Blend three colors based on warp layers
    float phase = color_shift;
    vec3 col = color1.rgb * c1;
    col = mix(col, color2.rgb, c2 * phase);
    col = mix(col, color3.rgb, c3 * phase * 0.7);

    // Boost saturation
    float lum = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(lum), col, 1.4);
    col = clamp(col, 0.0, 1.0);

    fragColor = vec4(col, 1.0);
}
