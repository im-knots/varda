/*{
    "DESCRIPTION": "Starfield - classic parallax star tunnel",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "star_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "star_density", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 1.0, "LABEL": "Density"},
        {"NAME": "layer_count", "TYPE": "float", "DEFAULT": 4.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Layers"},
        {"NAME": "star_size", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.005, "MAX": 0.05, "LABEL": "Star Size"},
        {"NAME": "trail_length", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Trails"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Star Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.05, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [{"PARAM": "star_speed", "INDEX": 0}]
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
    float star_speed;
    float star_density;
    float layer_count;
    float star_size;
    float trail_length;
    vec4 color1;
    vec4 bg_color;
};

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

float starLayer(vec2 p, float t, float size) {
    vec2 cell = floor(p);
    vec2 f = fract(p) - 0.5;
    float brightness = 0.0;

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 neighbor = vec2(float(x), float(y));
            vec2 id = cell + neighbor;
            float h = hash(id);
            if (h > star_density) continue;

            vec2 starPos = vec2(hash(id + 0.1), hash(id + 0.2)) - 0.5;
            vec2 diff = neighbor + starPos - f;

            // Twinkle
            float twinkle = 0.7 + 0.3 * sin(t * (1.0 + h * 3.0) + h * 6.283);

            // Trail stretch
            if (trail_length > 0.01) {
                diff.x /= (1.0 + trail_length * 3.0);
            }

            float d = length(diff);
            float star = smoothstep(size, size * 0.2, d) * twinkle;
            brightness += star;
        }
    }
    return brightness;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = PHASE_TIME_0;

    vec3 col = bg_color.rgb;
    int layers = int(clamp(layer_count, 1.0, 8.0));

    for (int i = 0; i < 8; i++) {
        if (i >= layers) break;
        float fi = float(i);
        float depth = 1.0 + fi * 0.5;
        float layerScale = 8.0 + fi * 6.0;
        vec2 lp = p * layerScale;
        // Parallax scroll from center
        lp += vec2(t * 0.5 * depth, 0.0);
        float sz = star_size * (1.0 + fi * 0.3);
        float stars = starLayer(lp, t, sz);
        float brightness = stars / depth;
        // Slight color variation per layer
        vec3 sc = color1.rgb * (0.7 + 0.3 * fract(fi * 0.37));
        col += sc * brightness;
    }

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
