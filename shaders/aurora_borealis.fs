/*{
    "DESCRIPTION": "Aurora Borealis - dense, drifting curtains rendered as a fake-volumetric raymarch through folded noise sheets (technique in the spirit of nimitz's 'Auroras', Shadertoy XtGGRt)",
    "CREDIT": "Varda VJ (aurora raymarch adapted from nimitz's Shadertoy 'Auroras', https://www.shadertoy.com/view/XtGGRt)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Speed"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 1.3, "MIN": 0.3, "MAX": 3.0, "LABEL": "Brightness"},
        {"NAME": "density", "TYPE": "float", "DEFAULT": 45.0, "MIN": 10.0, "MAX": 60.0, "LABEL": "Raymarch Steps"},
        {"NAME": "detail", "TYPE": "float", "DEFAULT": 5.0, "MIN": 2.0, "MAX": 6.0, "LABEL": "Curtain Fold Detail"},
        {"NAME": "tilt", "TYPE": "float", "DEFAULT": 0.15, "MIN": -0.3, "MAX": 0.6, "LABEL": "Camera Tilt"},
        {"NAME": "pan", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 2.0, "LABEL": "Camera Sway"},
        {"NAME": "hue_shift", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Hue Shift"},
        {"NAME": "saturation", "TYPE": "float", "DEFAULT": 1.1, "MIN": 0.0, "MAX": 2.0, "LABEL": "Saturation"},
        {"NAME": "star_density", "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.0, "MAX": 1.0, "LABEL": "Star Density"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.03, 0.05, 0.12, 1.0], "LABEL": "Sky Zenith"},
        {"NAME": "bg_color2", "TYPE": "color", "DEFAULT": [0.05, 0.03, 0.10, 1.0], "LABEL": "Sky Horizon"},
        {"NAME": "star_color", "TYPE": "color", "DEFAULT": [0.90, 0.95, 1.00, 1.0], "LABEL": "Star Color"}
    ],
    "PHASE_INPUTS": [{"PARAM": "speed", "INDEX": 0, "SCALE": 1.0}]
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
    float speed;
    float brightness;
    float density;
    float detail;
    float tilt;
    float pan;
    float hue_shift;
    float saturation;
    float star_density;
    vec4 bg_color;
    vec4 bg_color2;
    vec4 star_color;
};

// ---- Rotation / fold helpers ----

mat2 mm2(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, s, -s, c);
}

const mat2 M2 = mat2(0.95534, 0.29552, -0.29552, 0.95534);

float triWave(float x) {
    return clamp(abs(fract(x) - 0.5), 0.01, 0.49);
}

vec2 triWave2(vec2 p) {
    return vec2(triWave(p.x) + triWave(p.y), triWave(p.y + triWave(p.x)));
}

// Iterative fold-and-rotate triangle-wave noise. Repeatedly warps the
// sample point through a rotated, rescaled triangle-wave lattice; the
// accumulated ridge value reads as fibrous, draped curtain fabric rather
// than a smooth blob — this is what makes the aurora look "complex"
// instead of a soft gradient blur.
float triNoise2d(vec2 p, float spd, float t, int oct) {
    float z = 1.8;
    float z2 = 2.5;
    float rz = 0.0;
    p *= mm2(p.x * 0.06);
    vec2 bp = p;
    for (int i = 0; i < 6; i++) {
        if (i >= oct) break;
        vec2 dg = triWave2(bp * 1.85) * 0.75;
        dg *= mm2(t * spd);
        p -= dg / z2;

        bp *= 1.3;
        z2 *= 0.45;
        z *= 0.42;
        p *= 1.21 + (rz - 1.0) * 0.02;

        rz += triWave(p.x + triWave(p.y)) * z;
        p *= -M2;
    }
    return clamp(1.0 / pow(rz * 29.0, 1.3), 0.0, 0.55);
}

float hash21(vec2 n) {
    return fract(sin(dot(n, vec2(12.9898, 4.1414))) * 43758.5453);
}

// Fake-volumetric raymarch through a stack of horizontal noise sheets.
// Each step samples triNoise2d on an increasingly distant plane; blending
// consecutive samples and accumulating with exponential depth falloff
// produces soft draped curtains, while the per-step hue cycle
// (sin(... + i*0.043)) gives the natural green -> teal -> violet color
// progression with depth, exactly like the real thing.
vec4 aurora(vec3 ro, vec3 rd, float t, int steps, int oct, vec2 fragXY) {
    vec4 col = vec4(0.0);
    vec4 avgCol = vec4(0.0);

    for (int i = 0; i < 60; i++) {
        if (i >= steps) break;
        float fi = float(i);
        float of = 0.006 * hash21(fragXY) * smoothstep(0.0, 15.0, fi);
        float pt = ((0.8 + pow(fi, 1.4) * 0.002) - ro.y) / (rd.y * 2.0 + 0.4);
        pt -= of;
        vec3 bpos = ro + pt * rd;
        vec2 p = bpos.zx;
        float rzt = triNoise2d(p, 0.06, t, oct);
        vec4 col2 = vec4(0.0, 0.0, 0.0, rzt);
        col2.rgb = (sin(1.0 - vec3(2.15, -0.5, 1.2) + hue_shift * 6.283 + fi * 0.043) * 0.5 + 0.5) * rzt;
        avgCol = mix(avgCol, col2, 0.5);
        col += avgCol * exp2(-fi * 0.065 - 2.5) * smoothstep(0.0, 5.0, fi);
    }

    col *= clamp(rd.y * 15.0 + 0.4, 0.0, 1.0);
    return clamp(col * 1.5, 0.0, 1.0);
}

float starHash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float starField(vec2 p, float density_, float t) {
    vec2 cell = floor(p);
    vec2 f = fract(p) - 0.5;
    float h = starHash(cell);
    if (h > density_) return 0.0;
    vec2 starPos = vec2(starHash(cell + 0.17), starHash(cell + 0.41)) - 0.5;
    float d = length(f - starPos);
    float twinkle = 0.55 + 0.45 * sin(t * (1.5 + h * 4.0) + h * 6.2831);
    return smoothstep(0.06, 0.0, d) * twinkle;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); this shader's
    // raymarch was ported from Shadertoy's bottom-left/y-up convention,
    // so flip y here to keep "up" on screen mapped to positive rd.y.
    vec2 p = vec2(uv.x, 1.0 - uv.y) - 0.5;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = PHASE_TIME_0;

    vec3 ro = vec3(0.0, 0.0, -6.7);
    vec3 rd = normalize(vec3(p, 1.3));

    // Slow camera tilt + horizontal sway so the curtains drift across
    // frame over time instead of sitting in a perfectly static crop.
    rd.yz *= mm2(tilt);
    rd.xz *= mm2((sin(t * 0.05) * 0.2 - 0.1) * pan);

    int steps = int(clamp(density, 10.0, 60.0));
    int oct = int(clamp(detail, 2.0, 6.0));

    vec4 aur = smoothstep(0.0, 1.5, aurora(ro, rd, t, steps, oct, uv * RENDERSIZE));

    // Simple night-sky gradient keyed off the (post-tilt) ray direction.
    float skyGrad = clamp(rd.y * 0.6 + 0.4, 0.0, 1.0);
    vec3 col = mix(bg_color2.rgb, bg_color.rgb, skyGrad);

    float starMask = starField(p * 220.0, star_density * 0.5, t);
    starMask *= 1.0 - clamp(aur.a * 1.2, 0.0, 1.0);
    col += star_color.rgb * starMask;

    col = col * (1.0 - aur.a) + aur.rgb * brightness;

    float lum = dot(col, vec3(0.299, 0.587, 0.114));
    col = mix(vec3(lum), col, saturation);

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
