/*{
    "DESCRIPTION": "Hilbert curve - space-filling fractal growing outward from center",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "iterations", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 7.0, "LABEL": "Iterations"},
        {"NAME": "line_width", "TYPE": "float", "DEFAULT": 0.012, "MIN": 0.002, "MAX": 0.05, "LABEL": "Line Width"},
        {"NAME": "growth", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Growth"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 2.0, "LABEL": "Growth Speed"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 3.0, "LABEL": "Zoom"},
        {"NAME": "curve_color", "TYPE": "color", "DEFAULT": [0.2, 0.8, 1.0, 1.0], "LABEL": "Curve Color"},
        {"NAME": "glow_color", "TYPE": "color", "DEFAULT": [0.0, 0.3, 0.8, 1.0], "LABEL": "Glow Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.02, 1.0], "LABEL": "Background Color"},
        {"NAME": "glow_intensity", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Glow Intensity"},
        {"NAME": "rainbow", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Rainbow Amount"}
    ],
    "PHASE_INPUTS": [{"PARAM": "anim_speed", "INDEX": 0, "SCALE": 0.08}]
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
    float iterations;
    float line_width;
    float growth;
    float anim_speed;
    float zoom;
    vec4 curve_color;
    vec4 glow_color;
    vec4 bg_color;
    float glow_intensity;
    float rainbow;
};

#define PI 3.14159265359

vec3 hsv2rgb(vec3 c) {
    vec3 rgb = clamp(abs(mod(c.x * 6.0 + vec3(0.0, 4.0, 2.0), 6.0) - 3.0) - 1.0, 0.0, 1.0);
    return c.z * mix(vec3(1.0), rgb, c.y);
}

// Convert Hilbert curve index d to (x,y) coordinates at order n
vec2 hilbertD2XY(int n, int d) {
    int x = 0;
    int y = 0;
    for (int s = 1; s < 128; s *= 2) {
        if (s >= n) break;
        int rx = (d / 2) & 1;
        int ry = ((d & 1) ^ rx);
        if (ry == 0) {
            if (rx == 1) {
                x = s - 1 - x;
                y = s - 1 - y;
            }
            int tmp = x;
            x = y;
            y = tmp;
        }
        x += s * rx;
        y += s * ry;
        d /= 4;
    }
    return vec2(float(x), float(y));
}

// Distance from point p to line segment a-b
float distToSegment(vec2 p, vec2 a, vec2 b) {
    vec2 ab = b - a;
    float t = clamp(dot(p - a, ab) / dot(ab, ab), 0.0, 1.0);
    return length(p - (a + ab * t));
}

void main() {
    // Uniform preservation
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Aspect-corrected centered coordinates
    vec2 p = (gl_FragCoord.xy - 0.5 * RENDERSIZE) / min(RENDERSIZE.x, RENDERSIZE.y);
    p /= max(zoom, 0.1);

    // Determine curve order
    int order = int(clamp(iterations, 1.0, 7.0));
    int n = 1;
    for (int i = 0; i < 7; i++) {
        if (i >= order) break;
        n *= 2;
    }
    int totalPoints = n * n;
    float nf = float(n - 1);

    // Animated growth: expand from center outward
    float growRadius = growth;
    if (anim_speed > 0.01) {
        growRadius = fract(PHASE_TIME_0) * growth;
    }
    float growOuter = growRadius * 1.15 + 0.05;

    // Map pixel to grid space (curve lives in [0, n-1]^2, centered)
    float scale = 0.45;
    vec2 gridP = (p / scale + 1.0) * 0.5 * nf;
    vec2 gridCenter = vec2(nf * 0.5);

    // Early out: if pixel is far outside the grid, skip the loop
    float pixelRadial = length(gridP - gridCenter) / (nf * 0.707);
    if (pixelRadial > growOuter + 0.15) {
        fragColor = vec4(bg_color.rgb * bg_color.a, bg_color.a);
        return;
    }

    // Walk the curve, only test segments within the growth radius
    float minDist = 1e6;
    float closestT = 0.0;
    float closestRadial = 0.0;

    vec2 prevPt = hilbertD2XY(n, 0);
    for (int d = 1; d < 16384; d++) {
        if (d >= totalPoints) break;

        vec2 curPt = hilbertD2XY(n, d);

        // Segment midpoint radial distance from center, normalized
        vec2 mid = (prevPt + curPt) * 0.5;
        float radial = length(mid - gridCenter) / (nf * 0.707);

        if (radial < growOuter) {
            float dist = distToSegment(gridP, prevPt, curPt);
            if (dist < minDist) {
                minDist = dist;
                closestT = float(d) / float(totalPoints);
                closestRadial = radial;
            }
        }

        prevPt = curPt;
    }

    // Convert distance from grid space to NDC
    minDist *= scale * 2.0 / nf;

    float lw = line_width;

    // Soft fade at the growth front
    float frontFade = 1.0 - smoothstep(growRadius * 0.85, growOuter, closestRadial);

    // Curve rendering with antialiased edge
    float curve = smoothstep(lw * 1.2, lw * 0.5, minDist) * frontFade;

    // Glow
    float glow = exp(-minDist * minDist / (lw * lw * 25.0)) * glow_intensity * frontFade;
    glow += exp(-minDist * minDist / (lw * lw * 100.0)) * glow_intensity * 0.3 * frontFade;

    // Color
    vec3 cColor = curve_color.rgb;
    vec3 gColor = glow_color.rgb;

    if (rainbow > 0.01) {
        float hue = fract(closestT + TIME * 0.1);
        vec3 rainbowCol = hsv2rgb(vec3(hue, 0.9, 1.0));
        cColor = mix(cColor, rainbowCol, rainbow);
        gColor = mix(gColor, rainbowCol * 0.5, rainbow);
    }

    vec3 color = bg_color.rgb * bg_color.a;
    color += gColor * glow;
    color += cColor * curve;

    float alpha = clamp(max(curve, glow * 0.5) * curve_color.a + bg_color.a, 0.0, 1.0);

    fragColor = vec4(color * alpha, alpha);
}
