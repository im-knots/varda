/*{
    "DESCRIPTION": "Particle system generator - procedural particle field",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "particle_count", "TYPE": "float", "DEFAULT": 40.0, "MIN": 5.0, "MAX": 100.0, "LABEL": "Particle Count"},
        {"NAME": "particle_size", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.005, "MAX": 0.08, "LABEL": "Particle Size"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "spread", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.1, "MAX": 1.5, "LABEL": "Spread"},
        {"NAME": "style", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Style (0=Float 1=Fountain 2=Orbit)"},
        {"NAME": "glow", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Glow"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [1.0, 0.6, 0.2, 1.0], "LABEL": "Color 1"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.2, 0.5, 1.0, 1.0], "LABEL": "Color 2"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
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
    float particle_count;
    float particle_size;
    float anim_speed;
    float spread;
    float style;
    float glow;
    vec4 color1;
    vec4 color2;
    vec4 bg_color;
};

float hash(float n) { return fract(sin(n) * 43758.5453); }

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv - 0.5;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float t = PHASE_TIME_0;
    int count = int(clamp(particle_count, 5.0, 100.0));
    int st = int(floor(style + 0.5));

    vec3 col = bg_color.rgb;

    for (int i = 0; i < 100; i++) {
        if (i >= count) break;
        float fi = float(i);
        float h1 = hash(fi * 1.731);
        float h2 = hash(fi * 2.519);
        float h3 = hash(fi * 3.147);

        vec2 pos;
        if (st == 0) {
            // Float: gentle random drift
            pos = vec2(
                sin(t * 0.3 * (0.5 + h1) + fi * 1.7) * spread * (0.3 + h2 * 0.7),
                cos(t * 0.25 * (0.5 + h2) + fi * 2.3) * spread * (0.3 + h1 * 0.7)
            );
        } else if (st == 1) {
            // Fountain: rise from bottom
            float life = fract(t * 0.2 * (0.5 + h1) + h3);
            pos = vec2(
                (h1 - 0.5) * spread * life,
                life * spread - spread * 0.5
            );
            pos.x += sin(life * 3.0 + fi) * 0.1;
        } else {
            // Orbit: circular paths
            float radius = (0.1 + h1 * 0.4) * spread;
            float speed = (0.5 + h2) * 0.5;
            float phase = fi * 2.399;
            pos = vec2(cos(t * speed + phase), sin(t * speed + phase)) * radius;
        }

        float d = length(p - pos);
        float sz = particle_size * (0.5 + h3 * 0.5);

        // Soft circle
        float particle = smoothstep(sz, sz * 0.3, d);
        // Glow halo
        float g = exp(-d * d / (sz * sz * glow * 4.0 + 0.001)) * glow * 0.3;

        // Color varies per particle
        vec3 pcol = mix(color1.rgb, color2.rgb, h2);
        col += pcol * (particle + g);
    }

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
