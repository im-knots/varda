/*{
    "DESCRIPTION": "Dark Matter - cosmic web filament network based on zozuar's neuro noise",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 4.0, "LABEL": "Zoom"},
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0, "LABEL": "Speed"},
        {"NAME": "rotation", "TYPE": "float", "DEFAULT": 0.01, "MIN": -0.2, "MAX": 0.2, "LABEL": "Rotation"},
        {"NAME": "warp", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0, "LABEL": "Warp"},
        {"NAME": "web_scale", "TYPE": "float", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 20.0, "LABEL": "Web Scale"},
        {"NAME": "scale_mult", "TYPE": "float", "DEFAULT": 1.2, "MIN": 1.05, "MAX": 1.5, "LABEL": "Scale Mult"},
        {"NAME": "contrast", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Contrast"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 1.2, "MIN": 0.1, "MAX": 4.0, "LABEL": "Brightness"},
        {"NAME": "seed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 100.0, "LABEL": "Seed"},
        {"NAME": "node_glow", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 3.0, "LABEL": "Node Glow"},
        {"NAME": "color_filament", "TYPE": "color", "DEFAULT": [0.7, 0.3, 0.9, 1.0], "LABEL": "Filament Color"},
        {"NAME": "color_node", "TYPE": "color", "DEFAULT": [1.0, 0.7, 0.15, 1.0], "LABEL": "Node Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "INDEX": 0},
        {"PARAM": "rotation", "INDEX": 1}
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

layout(set = 0, binding = 1) uniform UserParams {
    float zoom;
    float speed;
    float rotation;
    float warp;
    float web_scale;
    float scale_mult;
    float contrast;
    float brightness;
    float seed;
    float node_glow;
    vec4 color_filament;
    vec4 color_node;
    vec4 bg_color;
};

// 2D rotation
vec2 rot2(vec2 v, float a) {
    float c = cos(a), s = sin(a);
    return mat2(c, s, -s, c) * v;
}

// Neuro noise: iterative sine/cosine accumulation with rotation
float neuroWeb(vec2 p, float t, float sc, float sm, float w) {
    vec2 sine_acc = vec2(0.0);
    vec2 res = vec2(0.0);
    float scale = sc;

    for (int j = 0; j < 15; j++) {
        p = rot2(p, 1.0 + w * 0.02);
        sine_acc = rot2(sine_acc, 1.0);
        vec2 layer = p * scale + float(j) + sine_acc - t;
        sine_acc += sin(layer);
        res += (0.5 + 0.5 * cos(layer)) / scale;
        scale *= sm;
    }
    return res.x + res.y;
}

void main() {
    // Uniform guard
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Screen coords with aspect ratio
    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    // Zoom
    p *= 0.5 / max(zoom, 0.1);

    // Rotation (accumulated via PHASE_TIME_1)
    p = rot2(p, PHASE_TIME_1);

    // Warp: distort UVs for swirl/stretch effects
    float r = length(p);
    float angle = atan(p.y, p.x);
    p += vec2(cos(angle), sin(angle)) * warp * r * 0.15;

    // Animation time (accumulated via PHASE_TIME_0)
    float t = PHASE_TIME_0;

    // Seed offsets the noise space — different seed = completely different web
    p += vec2(seed * 1.7321, seed * 2.2361);

    // One neuro noise eval per pixel
    float noise = neuroWeb(p, t, web_scale, scale_mult, warp);

    // Shape: pow for contrast, threshold for dark voids
    noise = pow(noise, contrast);
    noise *= brightness;
    noise = max(0.0, noise - 0.05);

    // Node glow: bright convergence points
    float nodes = pow(max(0.0, noise - 0.3), 2.0) * node_glow * 4.0;

    // Color: bg → filament → node → white-hot cores
    vec3 col = bg_color.rgb;
    col = mix(col, color_filament.rgb, clamp(noise * 2.0, 0.0, 1.0));
    col = mix(col, color_node.rgb, clamp(nodes, 0.0, 1.0));
    col += vec3(1.0, 0.95, 0.85) * pow(clamp(nodes, 0.0, 1.0), 2.0) * 0.5;

    col = clamp(col, 0.0, 1.0);
    float alpha = clamp(noise * 3.0, 0.0, 1.0);
    fragColor = vec4(col, alpha);
}