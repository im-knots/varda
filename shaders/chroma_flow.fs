/*{
    "DESCRIPTION": "Chroma Flow - color-grouped displacement that makes regions of similar color flow and separate",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "palette_mode", "LABEL": "Manual Palette", "TYPE": "bool", "DEFAULT": false},
        {"NAME": "palette_size", "LABEL": "Palette Size", "TYPE": "float", "DEFAULT": 4.0, "MIN": 2.0, "MAX": 8.0},
        {"NAME": "snap_hardness", "LABEL": "Snap Hardness", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "flow_speed", "LABEL": "Flow Speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "flow_scale", "LABEL": "Flow Scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 4.0},
        {"NAME": "flow_type", "LABEL": "Flow Type (0=Curl 1=Sine 2=Radial 3=Vortex)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "displacement_amount", "LABEL": "Displacement Amount", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "displacement_falloff", "LABEL": "Displacement Falloff", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "edge_blend_width", "LABEL": "Edge Blend Width", "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "color_preservation", "LABEL": "Color Preservation", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "group_speed_0", "LABEL": "Group 1 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_speed_1", "LABEL": "Group 2 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_speed_2", "LABEL": "Group 3 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_speed_3", "LABEL": "Group 4 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_speed_4", "LABEL": "Group 5 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_speed_5", "LABEL": "Group 6 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_speed_6", "LABEL": "Group 7 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_speed_7", "LABEL": "Group 8 Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": -1.0, "MAX": 2.0},
        {"NAME": "group_angle_0", "LABEL": "Group 1 Angle", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "group_angle_1", "LABEL": "Group 2 Angle", "TYPE": "float", "DEFAULT": 45.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "group_angle_2", "LABEL": "Group 3 Angle", "TYPE": "float", "DEFAULT": 90.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "group_angle_3", "LABEL": "Group 4 Angle", "TYPE": "float", "DEFAULT": 135.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "group_angle_4", "LABEL": "Group 5 Angle", "TYPE": "float", "DEFAULT": 180.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "group_angle_5", "LABEL": "Group 6 Angle", "TYPE": "float", "DEFAULT": 225.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "group_angle_6", "LABEL": "Group 7 Angle", "TYPE": "float", "DEFAULT": 270.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "group_angle_7", "LABEL": "Group 8 Angle", "TYPE": "float", "DEFAULT": 315.0, "MIN": 0.0, "MAX": 360.0},
        {"NAME": "palette_0", "LABEL": "Palette 1", "TYPE": "color", "DEFAULT": [1.0, 0.0, 0.0, 1.0]},
        {"NAME": "palette_1", "LABEL": "Palette 2", "TYPE": "color", "DEFAULT": [0.0, 0.5, 1.0, 1.0]},
        {"NAME": "palette_2", "LABEL": "Palette 3", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.2, 1.0]},
        {"NAME": "palette_3", "LABEL": "Palette 4", "TYPE": "color", "DEFAULT": [1.0, 1.0, 0.0, 1.0]},
        {"NAME": "palette_4", "LABEL": "Palette 5", "TYPE": "color", "DEFAULT": [1.0, 0.0, 1.0, 1.0]},
        {"NAME": "palette_5", "LABEL": "Palette 6", "TYPE": "color", "DEFAULT": [0.0, 1.0, 1.0, 1.0]},
        {"NAME": "palette_6", "LABEL": "Palette 7", "TYPE": "color", "DEFAULT": [1.0, 0.5, 0.0, 1.0]},
        {"NAME": "palette_7", "LABEL": "Palette 8", "TYPE": "color", "DEFAULT": [0.5, 0.0, 1.0, 1.0]},
        {"NAME": "background_fill", "LABEL": "Background Fill", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 0.0]}
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

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    uint palette_mode;  // bool stored as uint
    float palette_size;
    float snap_hardness;
    float flow_speed;
    float flow_scale;
    float flow_type;
    float displacement_amount;
    float displacement_falloff;
    float edge_blend_width;
    float color_preservation;
    float group_speed_0;
    float group_speed_1;
    float group_speed_2;
    float group_speed_3;
    float group_speed_4;
    float group_speed_5;
    float group_speed_6;
    float group_speed_7;
    float group_angle_0;
    float group_angle_1;
    float group_angle_2;
    float group_angle_3;
    float group_angle_4;
    float group_angle_5;
    float group_angle_6;
    float group_angle_7;
    vec4 palette_0;
    vec4 palette_1;
    vec4 palette_2;
    vec4 palette_3;
    vec4 palette_4;
    vec4 palette_5;
    vec4 palette_6;
    vec4 palette_7;
    vec4 background_fill;
};

#define PI 3.14159265359
#define MAX_PALETTE 8

// --- Noise primitives ---

vec2 hash2(vec2 p) {
    p = vec2(dot(p, vec2(127.1, 311.7)), dot(p, vec2(269.5, 183.3)));
    return fract(sin(p) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = dot(hash2(i), f);
    float b = dot(hash2(i + vec2(1.0, 0.0)), f - vec2(1.0, 0.0));
    float c = dot(hash2(i + vec2(0.0, 1.0)), f - vec2(0.0, 1.0));
    float d = dot(hash2(i + vec2(1.0, 1.0)), f - vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y) + 0.5;
}

float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    vec2 shift = vec2(100.0);
    for (int i = 0; i < 4; i++) {
        v += a * noise(p);
        p = p * 2.0 + shift;
        a *= 0.5;
    }
    return v;
}

// --- Flow field functions ---

vec2 rotateVec(vec2 v, float angle_deg) {
    float r = angle_deg * PI / 180.0;
    float c = cos(r), s = sin(r);
    return vec2(v.x * c - v.y * s, v.x * s + v.y * c);
}

vec2 flowCurl(vec2 p, float t) {
    float e = 0.01;
    float n  = fbm(p + t * 0.3);
    float nx = fbm(p + vec2(e, 0.0) + t * 0.3);
    float ny = fbm(p + vec2(0.0, e) + t * 0.3);
    return vec2(ny - n, -(nx - n)) / e;
}

vec2 flowSine(vec2 p, float t) {
    return vec2(sin(p.y * PI * 2.0 + t), cos(p.x * PI * 2.0 + t * 1.3));
}

vec2 flowRadial(vec2 p, float t) {
    vec2 dir = p - vec2(0.5);
    float dist = length(dir);
    if (dist < 0.001) return vec2(0.0);
    return normalize(dir) * (sin(dist * PI * 4.0 - t * 2.0) * 0.5 + 0.5);
}

vec2 flowVortex(vec2 p, float t) {
    vec2 dir = p - vec2(0.5);
    float dist = length(dir);
    if (dist < 0.001) return vec2(0.0);
    vec2 tangent = vec2(-dir.y, dir.x) / dist;
    float strength = smoothstep(0.0, 0.3, dist) * smoothstep(0.8, 0.3, dist);
    return tangent * strength * (1.0 + 0.3 * sin(dist * 8.0 - t));
}

vec2 evalFlow(vec2 p, float t, int type_id) {
    if (type_id == 1) return flowSine(p, t);
    if (type_id == 2) return flowRadial(p, t);
    if (type_id == 3) return flowVortex(p, t);
    return flowCurl(p, t);
}

// --- Accessor helpers ---

vec4 getManualPalette(int i) {
    if (i == 0) return palette_0; if (i == 1) return palette_1;
    if (i == 2) return palette_2; if (i == 3) return palette_3;
    if (i == 4) return palette_4; if (i == 5) return palette_5;
    if (i == 6) return palette_6; return palette_7;
}

float getGroupSpeed(int i) {
    if (i == 0) return group_speed_0; if (i == 1) return group_speed_1;
    if (i == 2) return group_speed_2; if (i == 3) return group_speed_3;
    if (i == 4) return group_speed_4; if (i == 5) return group_speed_5;
    if (i == 6) return group_speed_6; return group_speed_7;
}

float getGroupAngle(int i) {
    if (i == 0) return group_angle_0; if (i == 1) return group_angle_1;
    if (i == 2) return group_angle_2; if (i == 3) return group_angle_3;
    if (i == 4) return group_angle_4; if (i == 5) return group_angle_5;
    if (i == 6) return group_angle_6; return group_angle_7;
}

// --- Auto palette extraction ---
// Samples image at 25 positions, greedily selects N most color-diverse samples

#define NUM_CANDIDATES 25

void extractAutoPalette(int numGroups, out vec3 pal[MAX_PALETTE]) {
    // Sample candidates on a 5x5 grid offset from edges
    vec3 candidates[NUM_CANDIDATES];
    for (int y = 0; y < 5; y++) {
        for (int x = 0; x < 5; x++) {
            vec2 samplePos = vec2(float(x) + 0.5, float(y) + 0.5) / 5.0;
            candidates[y * 5 + x] = texture(sampler2D(inputImage, texSampler), samplePos).rgb;
        }
    }

    // Greedy farthest-point selection for maximum color diversity
    bool used[NUM_CANDIDATES];
    for (int i = 0; i < NUM_CANDIDATES; i++) used[i] = false;

    // First pick: candidate nearest to image center (index 12 = (2,2) in 5x5)
    pal[0] = candidates[12];
    used[12] = true;

    for (int g = 1; g < MAX_PALETTE; g++) {
        if (g >= numGroups) { pal[g] = vec3(0.0); continue; }
        float bestMinDist = -1.0;
        int bestIdx = 0;
        for (int c = 0; c < NUM_CANDIDATES; c++) {
            if (used[c]) continue;
            // Minimum distance from this candidate to all already-chosen palette entries
            float minDist = 1e10;
            for (int p = 0; p < g; p++) {
                vec3 d = candidates[c] - pal[p];
                minDist = min(minDist, dot(d, d));
            }
            if (minDist > bestMinDist) {
                bestMinDist = minDist;
                bestIdx = c;
            }
        }
        pal[g] = candidates[bestIdx];
        used[bestIdx] = true;
    }
}

// --- Main ---

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    int numGroups = clamp(int(floor(palette_size + 0.5)), 2, MAX_PALETTE);
    int ft = int(floor(flow_type + 0.5));
    float t = TIME * flow_speed;
    bool autoMode = palette_mode == 0u;

    // Build active palette (auto-detect or manual)
    vec3 activePalette[MAX_PALETTE];
    if (autoMode) {
        extractAutoPalette(numGroups, activePalette);
    } else {
        for (int i = 0; i < MAX_PALETTE; i++) {
            activePalette[i] = getManualPalette(i).rgb;
        }
    }

    // Step 1: Sample source color
    vec4 srcColor = texture(sampler2D(inputImage, texSampler), uv);

    // Step 2: Color grouping — compute weighted membership per palette anchor
    float weights[MAX_PALETTE];
    float totalWeight = 0.0;
    int bestGroup = 0;
    float bestDist = 1e10;
    float sharpness = mix(1.0, 32.0, snap_hardness);

    for (int i = 0; i < MAX_PALETTE; i++) {
        if (i >= numGroups) { weights[i] = 0.0; continue; }
        vec3 diff = srcColor.rgb - activePalette[i];
        float dist = dot(diff, diff);
        if (dist < bestDist) { bestDist = dist; bestGroup = i; }
        weights[i] = exp(-dist * sharpness);
        totalWeight += weights[i];
    }
    if (totalWeight > 0.0) {
        for (int i = 0; i < MAX_PALETTE; i++) weights[i] /= totalWeight;
    } else {
        weights[bestGroup] = 1.0;
    }

    // Step 3: Compute weighted displacement from per-group flow
    vec2 baseFlow = evalFlow(uv * flow_scale, t, ft);
    vec2 displacement = vec2(0.0);
    for (int i = 0; i < MAX_PALETTE; i++) {
        if (i >= numGroups) break;
        if (weights[i] < 0.001) continue;
        displacement += rotateVec(baseFlow, getGroupAngle(i)) * getGroupSpeed(i) * weights[i];
    }
    displacement *= displacement_amount;

    // Edge falloff
    if (displacement_falloff > 0.0) {
        vec2 ed = min(uv, 1.0 - uv);
        displacement *= smoothstep(0.0, displacement_falloff, min(ed.x, ed.y));
    }

    // Step 4: Re-sample at displaced position
    vec2 sampleUV = uv + displacement;
    bool oob = sampleUV.x < 0.0 || sampleUV.x > 1.0 || sampleUV.y < 0.0 || sampleUV.y > 1.0;

    vec4 outputColor;
    if (oob) {
        outputColor = background_fill;
    } else {
        vec4 displaced = texture(sampler2D(inputImage, texSampler), sampleUV);

        // Edge blending: soften at group boundaries
        if (edge_blend_width > 0.0) {
            float dominance = weights[bestGroup];
            float blendZone = 1.0 - smoothstep(0.5 - edge_blend_width, 0.5 + edge_blend_width, dominance);
            displaced = mix(displaced, srcColor, blendZone * 0.3);
        }

        // Color preservation: 1.0 = image color, 0.0 = flat palette anchor
        vec4 flatColor = vec4(activePalette[bestGroup], 1.0);
        outputColor = mix(flatColor, displaced, color_preservation);
        outputColor.a = displaced.a;
    }

    fragColor = outputColor;
}