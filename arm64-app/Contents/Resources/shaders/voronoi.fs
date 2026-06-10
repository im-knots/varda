/*{
    "DESCRIPTION": "Voronoi - animated cellular/organic Voronoi pattern",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "cell_count", "TYPE": "float", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 30.0, "LABEL": "Cell Count"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "edge_width", "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.0, "MAX": 0.2, "LABEL": "Edge Width"},
        {"NAME": "style", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Style (0=Cells 1=Edges 2=Distance)"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.1, 0.4, 0.9, 1.0], "LABEL": "Color 1"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.9, 0.2, 0.3, 1.0], "LABEL": "Color 2"},
        {"NAME": "edge_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Edge Color"}
    ],
    "PHASE_INPUTS": [{"PARAM": "anim_speed", "INDEX": 0, "SCALE": 0.5}]
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
    float cell_count;
    float anim_speed;
    float edge_width;
    float style;
    vec4 color1;
    vec4 color2;
    vec4 edge_color;
};

vec2 hash2(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv * cell_count;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = PHASE_TIME_0;

    float minDist = 10.0;
    float secondDist = 10.0;
    float cellID = 0.0;

    for (int y = -1; y <= 1; y++) {
        for (int x = -1; x <= 1; x++) {
            vec2 neighbor = vec2(float(x), float(y));
            vec2 cell = floor(p) + neighbor;
            vec2 point = hash2(cell);
            point = 0.5 + 0.5 * sin(t + 6.283 * point); // Animate
            vec2 diff = cell + point - p;
            float d = length(diff);
            if (d < minDist) {
                secondDist = minDist;
                minDist = d;
                cellID = dot(cell, vec2(7.0, 113.0));
            } else if (d < secondDist) {
                secondDist = d;
            }
        }
    }

    int st = int(floor(style + 0.5));
    vec3 col;

    if (st == 0) {
        // Cells: color based on cell ID
        float h = fract(cellID * 0.1);
        col = mix(color1.rgb, color2.rgb, h);
        // Edge highlight
        float edge = smoothstep(edge_width + 0.01, edge_width, secondDist - minDist);
        col = mix(col, edge_color.rgb, edge);
    } else if (st == 1) {
        // Edges only
        float edge = smoothstep(edge_width + 0.01, edge_width - 0.01, secondDist - minDist);
        col = mix(vec3(0.0), edge_color.rgb, edge);
    } else {
        // Distance field
        col = mix(color1.rgb, color2.rgb, minDist);
    }

    fragColor = vec4(col, 1.0);
}
