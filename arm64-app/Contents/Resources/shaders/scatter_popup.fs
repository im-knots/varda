/*{
    "DESCRIPTION": "Scatter Popup - shrinks input into small copies that pop up randomly outside a minimum radius",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "popup_count", "LABEL": "Popup Count", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 24.0},
        {"NAME": "popup_scale", "LABEL": "Popup Size", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.04, "MAX": 0.4},
        {"NAME": "min_radius", "LABEL": "Min Radius", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 0.8},
        {"NAME": "max_radius", "LABEL": "Max Radius", "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.1, "MAX": 0.9},
        {"NAME": "pop_speed", "LABEL": "Pop Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 5.0},
        {"NAME": "opacity", "LABEL": "Opacity", "TYPE": "float", "DEFAULT": 0.9, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "bg_mode", "LABEL": "BG (0=Black 1=Pass-through)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "pop_speed", "INDEX": 0}
    ]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME; float TIMEDELTA; uint FRAMEINDEX; int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level; float audio_bass; float audio_mid; float audio_treble;
    float audio_bpm; float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0; float PHASE_TIME_1; float PHASE_TIME_2; float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float popup_count;
    float popup_scale;
    float min_radius;
    float max_radius;
    float pop_speed;
    float opacity;
    float bg_mode;
};

float hash(float n) { return fract(sin(n) * 43758.5453); }

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    vec2 centered = uv - 0.5; // -0.5..0.5
    centered.x *= aspect;      // aspect-corrected

    // Background
    vec3 col = vec3(0.0);
    float bgAlpha = 0.0;
    if (bg_mode > 0.5) {
        vec4 bgSrc = texture(sampler2D(inputImage, texSampler), uv);
        col = bgSrc.rgb;
        bgAlpha = 1.0;
    }

    int nPop = int(clamp(popup_count, 1.0, 24.0));
    float t = PHASE_TIME_0;
    float cyclePeriod = 6.0; // each popup lives for 6 phase-time units
    float halfSize = popup_scale * 0.5;

    // Each popup gets a staggered phase offset so they don't all appear at once
    for (int i = 0; i < 24; i++) {
        if (i >= nPop) break;

        float pOff = float(i) / float(nPop);
        float phase = mod(t / cyclePeriod + pOff, 1.0);

        // Visibility envelope: fade in, hold, fade out
        float vis = smoothstep(0.0, 0.12, phase) * (1.0 - smoothstep(0.55, 0.7, phase));
        if (vis < 0.01) continue;

        // Scale pops up from 0, holds, then shrinks back
        float scaleMul = smoothstep(0.0, 0.15, phase) * (1.0 - smoothstep(0.5, 0.7, phase));

        // Per-popup random position seed (changes each cycle)
        float seed = floor(t / cyclePeriod + pOff) * 97.3 + float(i) * 53.7;
        float angle = hash(seed) * 6.28318;
        float rMin = min_radius;
        float rMax = max(max_radius, rMin + 0.05);
        float radius = rMin + hash(seed + 1.0) * (rMax - rMin);

        // Popup center in aspect-corrected space
        vec2 popCenter = vec2(cos(angle), sin(angle)) * radius;

        // Check if this pixel falls inside the popup rect
        float effSize = halfSize * scaleMul;
        vec2 delta = centered - popCenter;
        // Un-correct aspect for sampling: popup is square in screen space
        vec2 localUV = delta / (effSize * 2.0) + 0.5;

        if (localUV.x >= 0.0 && localUV.x <= 1.0 && localUV.y >= 0.0 && localUV.y <= 1.0 && effSize > 0.001) {
            // Sample the input image mapped to this popup
            vec4 src = texture(sampler2D(inputImage, texSampler), localUV);
            // Composite with opacity and visibility
            float alpha = vis * opacity * src.a;
            col = mix(col, src.rgb, alpha);
            bgAlpha = max(bgAlpha, alpha);
        }
    }

    fragColor = vec4(col, max(bgAlpha, step(0.5, bg_mode)));
}
