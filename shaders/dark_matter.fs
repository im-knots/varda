/*{
    "DESCRIPTION": "Dark Matter - 3D cosmic web / dark matter filament simulation with rotation and zoom",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D"],
    "INPUTS": [
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.3, "MAX": 5.0, "LABEL": "Zoom"},
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0, "LABEL": "Drift Speed"},
        {"NAME": "rot_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate X"},
        {"NAME": "rot_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate Y"},
        {"NAME": "rot_z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate Z"},
        {"NAME": "rot_speed_x", "TYPE": "float", "DEFAULT": 0.02, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin X"},
        {"NAME": "rot_speed_y", "TYPE": "float", "DEFAULT": 0.03, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin Y"},
        {"NAME": "rot_speed_z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin Z"},
        {"NAME": "web_scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 3.0, "LABEL": "Web Scale"},
        {"NAME": "filament_sharpness", "TYPE": "float", "DEFAULT": 12.0, "MIN": 2.0, "MAX": 30.0, "LABEL": "Filament Sharpness"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 2.5, "MIN": 0.5, "MAX": 6.0, "LABEL": "Brightness"},
        {"NAME": "color_void", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.02, 1.0], "LABEL": "Void Color"},
        {"NAME": "color_filament", "TYPE": "color", "DEFAULT": [0.08, 0.12, 0.5, 1.0], "LABEL": "Filament Color"},
        {"NAME": "color_warm", "TYPE": "color", "DEFAULT": [0.7, 0.75, 1.0, 1.0], "LABEL": "Dense Filament"},
        {"NAME": "color_cluster", "TYPE": "color", "DEFAULT": [1.0, 0.35, 0.1, 1.0], "LABEL": "Cluster Color"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "INDEX": 0},
        {"PARAM": "rot_speed_x", "INDEX": 1},
        {"PARAM": "rot_speed_y", "INDEX": 2},
        {"PARAM": "rot_speed_z", "INDEX": 3}
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
    float rot_x;
    float rot_y;
    float rot_z;
    float rot_speed_x;
    float rot_speed_y;
    float rot_speed_z;
    float web_scale;
    float filament_sharpness;
    float brightness;
    vec4 color_void;
    vec4 color_filament;
    vec4 color_warm;
    vec4 color_cluster;
};

// Fast 3D hash — no sin(), just integer-style bit mixing
vec3 hash3(vec3 p) {
    p = vec3(dot(p, vec3(127.1, 311.7, 74.7)),
             dot(p, vec3(269.5, 183.3, 246.1)),
             dot(p, vec3(113.5, 271.9, 124.6)));
    return fract(sin(p) * 43758.5453);
}

// Single Voronoi call: returns (edge_dist, center_dist)
// edge_dist = distance to nearest cell boundary → filaments
// center_dist = distance to nearest cell center → cluster nodes
vec2 voronoi(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    float d1 = 100.0;
    float d2 = 100.0;

    for (int x = -1; x <= 1; x++)
    for (int y = -1; y <= 1; y++)
    for (int z = -1; z <= 1; z++) {
        vec3 nb = vec3(float(x), float(y), float(z));
        vec3 diff = nb + hash3(i + nb) * 0.85 + 0.075 - f;
        float dist = dot(diff, diff);
        if (dist < d1) { d2 = d1; d1 = dist; }
        else if (dist < d2) { d2 = dist; }
    }

    return vec2(sqrt(d2) - sqrt(d1), sqrt(d1));
}

// Rotation
mat3 rotX(float a) { float c=cos(a),s=sin(a); return mat3(1,0,0,0,c,-s,0,s,c); }
mat3 rotY(float a) { float c=cos(a),s=sin(a); return mat3(c,0,s,0,1,0,-s,0,c); }
mat3 rotZ(float a) { float c=cos(a),s=sin(a); return mat3(c,-s,0,s,c,0,0,0,1); }

void main() {
    // Uniform guard
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float t = PHASE_TIME_0;

    // Rotation: manual + auto-spin
    float ax = rot_x + PHASE_TIME_1;
    float ay = rot_y + PHASE_TIME_2;
    float az = rot_z + PHASE_TIME_3;
    mat3 rot = rotZ(az) * rotY(ay) * rotX(ax);

    // Camera
    vec3 ro = vec3(t * 0.4, t * 0.15, t * 0.25);
    vec3 forward = rot * vec3(0.0, 0.0, 1.0);
    vec3 up = rot * vec3(0.0, 1.0, 0.0);
    vec3 right = normalize(cross(forward, up));
    up = cross(right, forward);

    float fov = 0.9 / max(zoom, 0.3);
    vec3 rd = normalize(forward + p.x * right * fov + p.y * up * fov);

    // Volumetric raymarch — 24 steps, single voronoi per step
    vec3 col = vec3(0.0);
    float totalA = 0.0;
    float maxDist = 8.0;
    float step = maxDist / 24.0;
    float sharp = filament_sharpness;

    for (int i = 0; i < 24; i++) {
        if (totalA > 0.85) break;

        float dist = (float(i) + 0.5) * step;
        vec3 pos = (ro + rd * dist) * web_scale * 0.6;

        // Single voronoi sample
        vec2 v = voronoi(pos);
        float edge = v.x;   // 0 at cell boundary (filament), large in void
        float center = v.y; // 0 at cell center (cluster node)

        // Filament intensity: sharp falloff from edge boundary
        // Small edge = on a filament, large edge = deep in void
        float filament = 1.0 / (1.0 + pow(edge * sharp, 3.0));

        // Cluster glow: bright where close to Voronoi center
        float cluster = 1.0 / (1.0 + pow(center * 4.0, 4.0));

        // Combined density for opacity
        float density = filament * 0.7 + cluster * 0.5;
        density = max(density - 0.08, 0.0); // cut the faint background haze

        if (density > 0.0) {
            // Color: void → blue filament → white dense → orange/red cluster
            // This matches the ESA cosmic web color scheme
            vec3 sc = color_void.rgb;
            sc = mix(sc, color_filament.rgb, smoothstep(0.0, 0.2, filament));
            sc = mix(sc, color_warm.rgb, smoothstep(0.3, 0.7, filament));
            sc = mix(sc, color_cluster.rgb, smoothstep(0.15, 0.6, cluster));
            // Hot white core at cluster centers
            sc += vec3(1.0, 0.9, 0.7) * pow(cluster, 3.0) * 0.6;

            // Depth fade
            sc *= exp(-dist * 0.12);

            // Front-to-back accumulation
            float a = density * step * brightness * 0.3;
            a = min(a, 0.5);
            col += sc * a * (1.0 - totalA);
            totalA += a * (1.0 - totalA);
        }
    }

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, clamp(totalA * 2.5, 0.0, 1.0));
}
