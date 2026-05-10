/*{
    "DESCRIPTION": "True Oscilloscope - audio-reactive waveform and shape visualizer with 2D/3D modes",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Audio"],
    "INPUTS": [
        {"NAME": "mode", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 4.0, "LABEL": "Mode (0=Wave 1=Lissajous 2=Circular 3=Spectrum 4=Mesh)"},
        {"NAME": "gain", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 4.0, "LABEL": "Gain"},
        {"NAME": "line_width", "TYPE": "float", "DEFAULT": 0.006, "MIN": 0.001, "MAX": 0.03, "LABEL": "Line Width"},
        {"NAME": "glow", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.0, "MAX": 5.0, "LABEL": "Glow"},
        {"NAME": "persistence", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Persistence"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.2, 1.0, 0.4, 1.0], "LABEL": "Primary Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.1, 0.4, 1.0, 1.0], "LABEL": "Secondary Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.02, 0.04, 1.0], "LABEL": "Background"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "complexity", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Complexity"}
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
    float mode;
    float gain;
    float line_width;
    float glow;
    float persistence;
    vec4 color1;
    vec4 color2;
    vec4 bg_color;
    float anim_speed;
    float complexity;
};

#define PI 3.14159265359
#define TAU 6.28318530718
#define SAMPLES 128

// Synthesize audio-driven waveform value at position t (0..1)
// Uses real audio uniforms to shape the wave
float audioWave(float t, float phase) {
    float g = gain;
    float bass = audio_bass * g;
    float mid = audio_mid * g;
    float treble = audio_treble * g;
    float lvl = audio_level * g;
    float bp = audio_beat_phase;
    float comp = complexity;

    // Base wave driven by bass (low freq oscillation)
    float wave = sin(t * TAU * comp + phase) * bass;
    // Mid frequencies add harmonics
    wave += sin(t * TAU * comp * 2.0 + phase * 1.5) * mid * 0.7;
    wave += sin(t * TAU * comp * 3.0 - phase * 0.8) * mid * 0.4;
    // Treble adds high-frequency detail
    wave += sin(t * TAU * comp * 5.0 + phase * 2.3) * treble * 0.5;
    wave += sin(t * TAU * comp * 8.0 - phase * 3.1) * treble * 0.25;
    // Beat pulse — sharp transient on beat
    float beatPulse = exp(-bp * 6.0);
    wave += sin(t * TAU * comp * 1.5) * beatPulse * lvl * 0.6;
    // Overall level envelope
    wave *= 0.3 + lvl * 0.7;
    return wave;
}

// Distance from point p to the waveform curve (mode 0: classic oscilloscope)
float dWaveform(vec2 p, float phase) {
    float minD = 1e6;
    float prevY = audioWave(0.0, phase);
    for (int i = 1; i <= SAMPLES; i++) {
        float t = float(i) / float(SAMPLES);
        float x = t * 2.0 - 1.0;
        float y = audioWave(t, phase);
        // Closest point on segment from prev to current
        float prevX = (float(i - 1) / float(SAMPLES)) * 2.0 - 1.0;
        vec2 a = vec2(prevX, prevY);
        vec2 b = vec2(x, y);
        vec2 pa = p - a, ba = b - a;
        float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
        float d = length(pa - ba * h);
        minD = min(minD, d);
        prevY = y;
    }
    return minD;
}

// Lissajous figure (mode 1): two audio-driven oscillators create 2D/3D shapes
float dLissajous(vec2 p, float phase) {
    float minD = 1e6;
    float g = gain;
    float bass = audio_bass * g;
    float mid = audio_mid * g;
    float treble = audio_treble * g;
    float lvl = max(audio_level * g, 0.05);
    float bp = audio_beat_phase;
    float comp = complexity;
    float beatPulse = exp(-bp * 4.0);

    // Lissajous ratios shift with audio
    float ratioX = comp + bass * 2.0;
    float ratioY = comp * 1.5 + mid * 2.0;
    float phaseShift = treble * PI + phase * 0.3;

    vec2 prev = vec2(0.0);
    for (int i = 0; i <= SAMPLES; i++) {
        float t = float(i) / float(SAMPLES);
        float angle = t * TAU;
        float x = sin(angle * ratioX + phaseShift) * lvl;
        float y = sin(angle * ratioY + phase) * lvl;
        // Beat distortion
        x += sin(angle * 3.0) * beatPulse * 0.15;
        y += cos(angle * 2.0) * beatPulse * 0.15;
        // Treble shimmer
        x += sin(angle * comp * 5.0 + phase * 3.0) * treble * 0.1;
        y += cos(angle * comp * 7.0 - phase * 2.0) * treble * 0.1;
        vec2 cur = vec2(x, y) * 0.8;
        if (i > 0) {
            vec2 pa = p - prev, ba = cur - prev;
            float h = clamp(dot(pa, ba) / (dot(ba, ba) + 1e-8), 0.0, 1.0);
            minD = min(minD, length(pa - ba * h));
        }
        prev = cur;
    }
    return minD;
}

// Circular oscilloscope (mode 2): waveform wrapped around a circle
float dCircular(vec2 p, float phase) {
    float minD = 1e6;
    float radius = 0.35;
    vec2 prev = vec2(0.0);
    for (int i = 0; i <= SAMPLES; i++) {
        float t = float(i) / float(SAMPLES);
        float angle = t * TAU;
        float wave = audioWave(t, phase);
        float r = radius + wave * 0.25;
        vec2 cur = vec2(cos(angle), sin(angle)) * r;
        if (i > 0) {
            vec2 pa = p - prev, ba = cur - prev;
            float h = clamp(dot(pa, ba) / (dot(ba, ba) + 1e-8), 0.0, 1.0);
            minD = min(minD, length(pa - ba * h));
        }
        prev = cur;
    }
    return minD;
}

// Spectrum analyzer bars (mode 3): frequency bands as vertical bars
float dSpectrum(vec2 p, float phase) {
    float minD = 1e6;
    float g = gain;
    float lvl = audio_level * g;
    float bp = audio_beat_phase;
    float beatPulse = exp(-bp * 5.0);
    // Simulate 8 frequency bands from bass/mid/treble
    float bands[8];
    bands[0] = audio_bass * g * 1.2;
    bands[1] = audio_bass * g * 0.9 + audio_mid * g * 0.1;
    bands[2] = audio_bass * g * 0.4 + audio_mid * g * 0.6;
    bands[3] = audio_mid * g * 1.0;
    bands[4] = audio_mid * g * 0.7 + audio_treble * g * 0.3;
    bands[5] = audio_mid * g * 0.3 + audio_treble * g * 0.7;
    bands[6] = audio_treble * g * 1.0;
    bands[7] = audio_treble * g * 0.7;

    float barW = 1.8 / 8.0;
    float gap = barW * 0.15;
    for (int i = 0; i < 8; i++) {
        float cx = -0.9 + (float(i) + 0.5) * barW;
        float h = bands[i] * 0.7 + beatPulse * 0.05;
        h = clamp(h, 0.01, 0.9);
        // Bar rectangle distance
        float halfW = (barW - gap) * 0.5;
        float dx = abs(p.x - cx) - halfW;
        float dy = abs(p.y + 0.5 - h * 0.5) - h * 0.5;
        float d = length(max(vec2(dx, dy), 0.0)) + min(max(dx, dy), 0.0);
        minD = min(minD, d);
        // Peak dot above bar
        float peakY = -0.5 + h + 0.03;
        float peakD = length(p - vec2(cx, peakY)) - 0.008;
        minD = min(minD, max(peakD, 0.0));
    }
    return minD;
}

// 3D wireframe mesh (mode 4): audio-deformed mesh rendered with projection
float dMesh(vec2 p, float phase) {
    float minD = 1e6;
    float g = gain;
    float bass = audio_bass * g;
    float mid = audio_mid * g;
    float treble = audio_treble * g;
    float lvl = max(audio_level * g, 0.05);
    float bp = audio_beat_phase;
    float beatPulse = exp(-bp * 4.0);
    float comp = complexity;

    // Rotation from audio and time
    float rotY = phase * 0.3 + bass * 0.5;
    float rotX = phase * 0.2 + mid * 0.3;
    float cy = cos(rotY), sy = sin(rotY);
    float cx = cos(rotX), sx = sin(rotX);

    // Grid resolution
    int gridN = 12;
    float gridSize = 0.6;

    for (int gy = 0; gy < 12; gy++) {
        for (int gx = 0; gx < 12; gx++) {
            if (gx >= gridN || gy >= gridN) continue;
            float fx = (float(gx) / float(gridN - 1) - 0.5) * 2.0 * gridSize;
            float fy = (float(gy) / float(gridN - 1) - 0.5) * 2.0 * gridSize;
            // Audio displacement on Z
            float dist = length(vec2(fx, fy));
            float z = sin(dist * comp * 3.0 - phase * 2.0) * bass * 0.3;
            z += sin(fx * comp * 5.0 + phase) * mid * 0.15;
            z += cos(fy * comp * 7.0 - phase * 1.5) * treble * 0.1;
            z += beatPulse * 0.1 * sin(dist * 8.0);
            z *= lvl;
            // Rotate Y then X
            float x2 = fx * cy + z * sy;
            float z2 = -fx * sy + z * cy;
            float y2 = fy * cx - z2 * sx;
            float z3 = fy * sx + z2 * cx;
            // Perspective projection
            float persp = 1.5 / (1.5 + z3 + 0.5);
            vec2 proj = vec2(x2, y2) * persp;
            // Point distance
            float d = length(p - proj) - 0.004 * persp;
            minD = min(minD, max(d, 0.0));
        }
    }
    return minD;
}

void main() {
    // Uniform guard
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float phase = PHASE_TIME_0;

    // Compute distance to shape based on mode
    float d;
    int m = int(floor(mode + 0.5));
    if (m <= 0) {
        d = dWaveform(p, phase);
    } else if (m == 1) {
        d = dLissajous(p, phase);
    } else if (m == 2) {
        d = dCircular(p, phase);
    } else if (m == 3) {
        d = dSpectrum(p, phase);
    } else {
        d = dMesh(p, phase);
    }

    // Render: crisp line + phosphor glow (oscilloscope aesthetic)
    float lw = line_width;
    float line = smoothstep(lw, lw * 0.15, d);

    // Phosphor glow — exponential falloff
    float glowFalloff = lw * glow * 4.0 + 0.001;
    float glowVal = exp(-d * d / (glowFalloff * glowFalloff)) * glow * 0.35;

    // Color: primary on line, secondary blended into glow
    // Audio level drives color intensity and hue shift
    float lvl = audio_level * gain;
    float hueShift = audio_beat_phase * 0.3;
    vec3 lineCol = color1.rgb * (1.0 + lvl * 0.5);
    vec3 glowCol = mix(color1.rgb, color2.rgb, 0.5 + hueShift);

    // Persistence: scanline fade for CRT look
    float scanline = 1.0 - persistence * 0.3 * (0.5 + 0.5 * sin(uv.y * RENDERSIZE.y * PI));

    // Compose
    vec3 col = bg_color.rgb;
    col += glowCol * glowVal * scanline;
    col += lineCol * line * scanline;

    // Beat flash — subtle background pulse on beat
    float beatFlash = exp(-audio_beat_phase * 8.0) * audio_level * gain * 0.08;
    col += color1.rgb * beatFlash;

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
