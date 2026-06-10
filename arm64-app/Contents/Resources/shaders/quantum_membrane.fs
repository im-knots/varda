/*{
    "DESCRIPTION": "Quantum Membrane - Rolling wave-mesh terrain with rainbow grid flyover",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D"],
    "INPUTS": [
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.3, "MAX": 5.0, "LABEL": "Zoom"},
        {"NAME": "cam_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Camera X"},
        {"NAME": "cam_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Camera Y"},
        {"NAME": "fly_speed", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0, "LABEL": "Fly Speed"},
        {"NAME": "spin_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin X"},
        {"NAME": "spin_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin Y"},
        {"NAME": "membrane_freq", "TYPE": "float", "DEFAULT": 3.0, "MIN": 0.5, "MAX": 10.0, "LABEL": "Wave Freq"},
        {"NAME": "membrane_amp", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 3.0, "LABEL": "Wave Amp"},
        {"NAME": "wave_speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Wave Speed"},
        {"NAME": "grid_density", "TYPE": "float", "DEFAULT": 10.0, "MIN": 1.0, "MAX": 20.0, "LABEL": "Grid Density"},
        {"NAME": "grid_thickness", "TYPE": "float", "DEFAULT": 0.03, "MIN": 0.005, "MAX": 0.15, "LABEL": "Grid Thickness"},
        {"NAME": "glow_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 4.0, "LABEL": "Glow Intensity"},
        {"NAME": "glow_radius", "TYPE": "float", "DEFAULT": 0.08, "MIN": 0.01, "MAX": 0.3, "LABEL": "Glow Radius"},
        {"NAME": "color_a", "TYPE": "color", "DEFAULT": [0.1, 0.4, 0.9, 1.0], "LABEL": "Primary"},
        {"NAME": "color_b", "TYPE": "color", "DEFAULT": [0.9, 0.2, 0.4, 1.0], "LABEL": "Accent"},
        {"NAME": "color_glow", "TYPE": "color", "DEFAULT": [1.0, 0.8, 0.2, 1.0], "LABEL": "Glow Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.01, 0.01, 0.03, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "fly_speed", "INDEX": 0},
        {"PARAM": "spin_x", "INDEX": 1},
        {"PARAM": "spin_y", "INDEX": 2},
        {"PARAM": "wave_speed", "INDEX": 3}
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
    float cam_x;
    float cam_y;
    float fly_speed;
    float spin_x;
    float spin_y;
    float membrane_freq;
    float membrane_amp;
    float wave_speed;
    float grid_density;
    float grid_thickness;
    float glow_intensity;
    float glow_radius;
    vec4 color_a;
    vec4 color_b;
    vec4 color_glow;
    vec4 bg_color;
};

// --- Rotation helpers ---
mat3 rotX(float a) { float c=cos(a),s=sin(a); return mat3(1,0,0,0,c,-s,0,s,c); }
mat3 rotY(float a) { float c=cos(a),s=sin(a); return mat3(c,0,s,0,1,0,-s,0,c); }

// --- Smooth noise for spatial randomization ---
float hash1(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash1(i);
    float b = hash1(i + vec2(1.0, 0.0));
    float c = hash1(i + vec2(0.0, 1.0));
    float d = hash1(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// --- Heightfield: waves + noise so terrain never repeats ---
float terrain(vec2 p, float t) {
    float h = 0.0;
    float freq = membrane_freq;
    float amp = 0.5;
    float cs = cos(0.7);
    float sn = sin(0.7);
    mat2 twist = mat2(cs, sn, -sn, cs);
    for (int i = 0; i < 4; i++) {
        float fi = float(i);
        // Sine waves for the rolling shape
        float wx = sin(p.x * freq + t * (0.8 + fi * 0.4) + fi * 2.3);
        float wy = cos(p.y * freq + t * (0.6 + fi * 0.3) + fi * 1.7);
        // Noise to randomize freq/amp spatially — breaks tiling
        float n = noise(p * freq * 0.3 + fi * 5.1) * 2.0 - 1.0;
        h += (wx * wy + n * 0.5) * amp;
        p = twist * p;
        freq *= 1.9;
        amp *= 0.45;
    }
    return h * membrane_amp;
}

// --- HSV to RGB ---
vec3 hsv2rgb(vec3 c) {
    vec3 p = abs(fract(c.xxx + vec3(0.0, 2.0/3.0, 1.0/3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3(1.0), clamp(p - 1.0, 0.0, 1.0), c.y);
}

// --- Height-driven color ---
// Normalized height (0=valley, 1=peak) drives the hue.
// color_a = valley color, color_b = peak color, rainbow blend via color_glow.a
vec3 surfaceColor(float height) {
    // Normalize: terrain range is roughly [-1,1] * membrane_amp
    float norm = clamp(height / max(membrane_amp, 0.01) * 0.5 + 0.5, 0.0, 1.0);
    vec3 userBlend = mix(color_a.rgb, color_b.rgb, norm);
    vec3 rainbow = hsv2rgb(vec3(norm * 0.8, 0.85, 1.0));
    return mix(userBlend, rainbow, color_glow.a);
}

// --- Grid lines with thickness control, returns distance to nearest line ---
float gridDist(vec2 p) {
    vec2 g = abs(fract(p * grid_density) - 0.5);
    return min(g.x, g.y);
}

// --- Ray-heightfield intersection (double-sided) ---
float traceHeightfield(vec3 ro, vec3 rd, float t, out vec3 hitPos) {
    float maxDist = 30.0;
    float dt = maxDist / 48.0;
    float lastSign = ro.y - terrain(ro.xz, t);
    float lastT = 0.0;
    for (int i = 0; i < 48; i++) {
        float d = (float(i) + 0.5) * dt;
        vec3 pos = ro + rd * d;
        float h = terrain(pos.xz, t);
        float sn = pos.y - h;
        if (sn * lastSign < 0.0) {
            float frac = abs(lastSign) / (abs(lastSign) + abs(sn));
            float hitD = mix(lastT, d, frac);
            hitPos = ro + rd * hitD;
            hitPos.y = terrain(hitPos.xz, t);
            return hitD;
        }
        lastSign = sn;
        lastT = d;
    }
    return -1.0;
}

void main() {
    // Uniform guard
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    p.y = -p.y;

    float t = PHASE_TIME_3;

    // Camera
    float flyZ = PHASE_TIME_0 * 4.0;
    float camHeight = zoom * 2.0;
    float ax = cam_x + PHASE_TIME_1;
    float ay = cam_y + PHASE_TIME_2;
    vec3 ro = vec3(0.0, camHeight, flyZ);

    mat3 rot = rotY(ay) * rotX(ax - 0.6);
    vec3 fwd = rot * vec3(0.0, 0.0, 1.0);
    vec3 right = normalize(cross(fwd, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(right, fwd);
    vec3 rd = normalize(fwd + p.x * right * 0.9 + p.y * up * 0.9);

    vec3 col = bg_color.rgb;
    float alpha = 0.0;

    vec3 hitPos;
    float d = traceHeightfield(ro, rd, t, hitPos);
    if (d > 0.0) {
        vec3 sc = surfaceColor(hitPos.y);
        float fog = exp(-d * 0.08);

        // Grid: sharp line + soft glow halo
        float gd = gridDist(hitPos.xz);
        float line = smoothstep(grid_thickness, 0.0, gd);
        float glow = exp(-gd * gd / (glow_radius * glow_radius)) * glow_intensity;

        // Combine: bright grid lines + colored glow around them
        col = sc * line * (1.0 + glow_intensity * 0.5);
        col += color_glow.rgb * glow * 0.5;
        col += sc * 0.03; // faint surface fill
        col *= fog;
        alpha = clamp((line + glow * 0.4 + 0.05) * fog * 2.0, 0.0, 1.0);
    }

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, alpha);
}