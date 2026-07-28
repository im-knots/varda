/*{
    "DESCRIPTION": "Mandelbrot fractal with decorative pattern overlay",
    "CREDIT": "Ported to Varda ISF",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "zoom_level", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 3.0, "LABEL": "Zoom Level"},
        {"NAME": "iterations", "TYPE": "float", "DEFAULT": 128.0, "MIN": 32.0, "MAX": 256.0, "LABEL": "Iterations"},
        {"NAME": "pattern_scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 4.0, "LABEL": "Pattern Scale"},
        {"NAME": "color_mode", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 4.0, "LABEL": "Color Mode"},
        {"NAME": "color_a", "TYPE": "color", "DEFAULT": [1.0, 0.5, 0.2, 1.0], "LABEL": "Custom Color A"},
        {"NAME": "color_b", "TYPE": "color", "DEFAULT": [0.2, 0.3, 1.0, 1.0], "LABEL": "Custom Color B"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 3.0, "LABEL": "Brightness"},
        {"NAME": "vignette_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Vignette"}
    ],
    "PHASE_INPUTS": [{"PARAM": "speed", "INDEX": 0, "SCALE": 0.125}]
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
    float zoom_level;
    float iterations;
    float pattern_scale;
    float color_mode;
    vec4 color_a;
    vec4 color_b;
    float brightness;
    float vignette_amount;
};

const int MAX_ITER = 256;

vec3 applyColorMode(float d, vec3 warmCol, int cm) {
    if (cm == 0) return warmCol;
    if (cm == 1) return pow(min(vec3(1.0, 1.0, 1.5) * min(d * 0.85, 0.96), vec3(1.0)), vec3(16.0, 3.0, 1.0)) * 1.15;
    if (cm == 2) return 0.5 + 0.5 * cos(6.283185 * (d + vec3(0.0, 0.33, 0.67)));
    if (cm == 3) { float g = dot(warmCol, vec3(0.299, 0.587, 0.114)); return vec3(g); }
    return mix(color_a.rgb, color_b.rgb, clamp(d, 0.0, 1.0));
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 vUv = vec2(uv.x, 1.0 - uv.y);
    float t = PHASE_TIME_0;
    int maxIter = clamp(int(iterations), 32, MAX_ITER);
    int cm = clamp(int(color_mode + 0.5), 0, 4);

    vec3 col = vec3(0.0);
    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) {
            vec2 fragCoord = vUv * RENDERSIZE + vec2(float(i), float(j)) / 2.0;
            vec2 p = (fragCoord - RENDERSIZE * 0.5) / RENDERSIZE.y;

            float ttm = cos(sin(t / 8.0)) * 6.2831;
            float ct = cos(ttm), st = sin(ttm);
            p = mat2(ct, st, -st, ct) * p;
            p -= vec2(cos(t / 2.0) / 2.0, sin(t / 3.0) / 5.0);

            float zm = (200.0 + sin(t / 7.0) * 50.0) * zoom_level;
            vec2 cc = vec2(-0.57735 + 0.004, 0.57735) + p / zm;
            vec2 z = vec2(0.0), dz = vec2(0.0);
            int ik = maxIter;

            for (int k = 0; k < MAX_ITER; k++) {
                if (k >= maxIter) break;
                dz = mat2(z, -z.y, z.x) * dz * 2.0 + vec2(1.0, 0.0);
                z = mat2(z, -z.y, z.x) * z + cc;
                if (dot(z, z) > 200.0) { ik = k; break; }
            }

            float d = sqrt(1.0 / max(length(dz), 0.0001)) * log(dot(z, z));
            d = clamp(d * 50.0, 0.0, 1.0);
            float ln = step(0.0, length(z) / 15.5 - 1.0);
            float dir = mod(float(ik), 2.0) < 0.5 ? -1.0 : 1.0;
            float sh = float(maxIter - ik) / float(maxIter);

            // Pattern overlay 1
            vec2 tuv = z / 320.0 * pattern_scale;
            float tm = -ttm * sh * sh * 16.0;
            float ctm = cos(tm), stm = sin(tm);
            tuv = mat2(ctm, stm, -stm, ctm) * tuv;
            tuv = abs(mod(tuv, 1.0 / 8.0) - 1.0 / 16.0);
            float invDz = 1.0 / max(length(dz), 0.0001);
            float pat = smoothstep(0.0, invDz, length(tuv) - 1.0 / 32.0);
            pat = min(pat, smoothstep(0.0, invDz, abs(max(tuv.x, tuv.y) - 1.0 / 16.0) - 0.04 / 16.0));

            // Base color
            vec3 warmCol = pow(min(vec3(1.5, 1.0, 1.0) * min(d * 0.85, 0.96), vec3(1.0)), vec3(1.0, 3.0, 16.0)) * 1.15;
            vec3 lCol = applyColorMode(d, warmCol, cm);

            lCol = dir < 0.0 ? lCol * min(pat, ln) : (sqrt(lCol) * 0.5 + 0.7) * max(1.0 - pat, 1.0 - ln);

            // Diffuse lighting
            vec3 rd = normalize(vec3(p, 1.0));
            rd = reflect(rd, vec3(0.0, 0.0, -1.0));
            float diff = clamp(dot(z * 0.5 + 0.5, rd.xy), 0.0, 1.0) * d;

            // Pattern overlay 2
            tuv = z / 200.0 * pattern_scale;
            tm = -tm / 1.5 + 0.5;
            ctm = cos(tm); stm = sin(tm);
            tuv = mat2(ctm, stm, -stm, ctm) * tuv;
            tuv = abs(mod(tuv, 1.0 / 8.0) - 1.0 / 16.0);
            pat = smoothstep(0.0, invDz, length(tuv) - 1.0 / 32.0);
            pat = min(pat, smoothstep(0.0, invDz, abs(max(tuv.x, tuv.y) - 1.0 / 16.0) - 0.04 / 16.0));

            lCol += mix(lCol, vec3(1.0) * ln, 0.5) * diff * diff * 0.5 * (pat * 0.6 + 0.6);

            if (mod(float(ik), 6.0) < 0.5) lCol = lCol.yxz;
            lCol = mix(lCol.xzy, lCol, d / 1.2);
            lCol = mix(lCol, vec3(0.0), (1.0 - step(0.0, -(length(z) * 0.05 * float(ik) / float(maxIter) - 1.0))) * 0.95);
            lCol = mix(vec3(0.0), lCol, sh * d);

            col += min(lCol, vec3(1.0));
        }
    }
    col /= 4.0;
    col *= brightness;

    // Vignette
    float vig = pow(16.0 * (1.0 - vUv.x) * (1.0 - vUv.y) * vUv.x * vUv.y, 1.0 / 8.0) * 1.15;
    col *= mix(1.0, vig, vignette_amount);

    fragColor = vec4(sqrt(max(col, vec3(0.0))), 1.0);
}
