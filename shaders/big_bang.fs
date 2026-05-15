/*{
    "DESCRIPTION": "Big Bang — cyclical cosmic evolution: singularity, expansion, stellar lifecycle (blue→white→red→supernova), nebula dust, crunch",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 5.0, "LABEL": "Cycle Speed"},
        {"NAME": "star_speed", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 3.0, "LABEL": "Star Speed"},
        {"NAME": "intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 3.0, "LABEL": "Intensity"},
        {"NAME": "particle_count", "TYPE": "float", "DEFAULT": 6.0, "MIN": 2.0, "MAX": 12.0, "LABEL": "Stars"},
        {"NAME": "shockwave", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.05, "MAX": 1.0, "LABEL": "Shockwave"},
        {"NAME": "rays", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Rays"},
        {"NAME": "bloom", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Bloom"},
        {"NAME": "streak_density", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Streak Density"},
        {"NAME": "dust", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0, "LABEL": "Dust"},
        {"NAME": "fg_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Foreground"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "INDEX": 0},
        {"PARAM": "star_speed", "INDEX": 1}
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
    float speed;
    float star_speed;
    float intensity;
    float particle_count;
    float shockwave;
    float rays;
    float bloom;
    float streak_density;
    float dust;
    vec4 fg_color;
    vec4 bg_color;
};

#define PI 3.14159265359
#define TAU 6.28318530718
#define NUM_EXPLOSION_PARTICLES 80
#define NUM_STREAK_PARTICLES 40

// ── Hash helpers (pseudo-random, deterministic per-particle) ────────
float hash(float n) { return fract(sin(n) * 43758.5453); }
vec2 hash2(float n) { return vec2(hash(n), hash(n + 71.37)); }
float hash12(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

// ── Polar hash: returns (angle, distance) for particle direction ────
vec2 hashPolar(float seed) {
    float angle = fract(sin(seed * 674.3) * 453.2) * TAU;
    float dist  = fract(sin((seed + angle) * 724.3) * 341.2);
    return vec2(angle, dist);
}

// ── Value noise ─────────────────────────────────────────────────────
float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash12(i);
    float b = hash12(i + vec2(1.0, 0.0));
    float c = hash12(i + vec2(0.0, 1.0));
    float d = hash12(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

// ── FBM — organic noise field ───────────────────────────────────────
float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 5; i++) {
        v += a * noise(p);
        p = rot * p * 2.0;
        a *= 0.5;
    }
    return v;
}

// ── Main ─────────────────────────────────────────────────────────────
void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float r = length(p);
    float angle = atan(p.y, p.x);

    // ── Cyclical phase: triangle wave 0→1→0 (bang→crunch) ───────────
    // PHASE_TIME_0 driven by cycle_speed (slow expansion/contraction)
    float phase = fract(PHASE_TIME_0 * 0.04);
    float t = 1.0 - abs(phase * 2.0 - 1.0);
    // expandR goes to 3.0 so shockwave travels well past frame edges
    float expandR = smoothstep(0.0, 1.0, t) * 3.0;
    // crunch factor: 1 when collapsed, 0 when fully expanded, back to 1
    float crunch = 1.0 - smoothstep(0.0, 0.5, t);

    // PHASE_TIME_1 driven by star_speed — independent stellar evolution clock
    // More stars = faster lifecycles (generations cycle quicker)
    float starClock = PHASE_TIME_1 * (0.5 + particle_count * 0.1);

    vec3 col = vec3(0.0);
    float hotWhite = 0.0;

    // ── Layer 1: Glowing core singularity ────────────────────────────
    // Fades to near-zero during expansion, re-intensifies during crunch
    float coreStrength = crunch * crunch * intensity;
    float core = (0.008 / (r * r + 0.001)) * coreStrength;
    // Hot white-blue core, orange halo
    vec3 coreCol = mix(vec3(1.0, 0.6, 0.2), vec3(0.7, 0.85, 1.0), 1.0 / (1.0 + r * 8.0));
    col += coreCol * core;
    hotWhite += core * 0.5;

    // ── Layer 2: Stars — born in nebula, live blue→white→red, die as supernovae
    // Stars only appear where dust exists, embedded in the nebula
    int pCount = int(clamp(particle_count * 8.0, 16.0, 96.0));
    float particleBrightness = 0.0004 * intensity;
    vec3 blue  = vec3(0.4, 0.6, 1.0);
    vec3 white = vec3(1.0, 0.95, 0.9);
    vec3 red   = vec3(1.0, 0.25, 0.05);
    for (int i = 0; i < NUM_EXPLOSION_PARTICLES; i++) {
        if (i >= pCount) break;
        float seed = float(i) + 1.0;
        vec2 polar = hashPolar(seed);
        float pAngle = polar.x;
        float pDist = polar.y;

        // Stars ride with the expanding/contracting matter
        vec2 dir = vec2(cos(pAngle), sin(pAngle));
        vec2 starPos = dir * pDist * expandR;
        float d = length(p - starPos);

        // Stars only appear once nebula has expanded enough
        float starAppear = smoothstep(0.15, 0.25, t);

        // Stellar lifecycle driven by starClock (independent of bang/crunch)
        // Each star has a unique phase offset so they're born at different times
        float starOffset = hash(seed * 3.71) * 20.0;
        // Lifespan in starClock units — each star cycles through its life
        float lifespan = 1.5 + hash(seed * 5.53) * 2.0;
        float starAge = fract((starClock + starOffset) / lifespan);

        // HR diagram color evolution:
        // 0.0–0.2: blue (hot young O/B star, forming from nebula)
        // 0.2–0.6: white-yellow (main sequence)
        // 0.6–0.85: red giant (swells, brightens)
        // 0.85–0.95: supernova flash (brief intense white)
        // 0.95–1.0: fades to ember/nothing
        vec3 pCol;
        float brightMult = 1.0;
        if (starAge < 0.2) {
            pCol = mix(blue, vec3(0.6, 0.8, 1.0), starAge / 0.2);
        } else if (starAge < 0.6) {
            pCol = mix(vec3(0.6, 0.8, 1.0), white, (starAge - 0.2) / 0.4);
        } else if (starAge < 0.85) {
            // Red giant phase — star swells and brightens
            float rgPhase = (starAge - 0.6) / 0.25;
            pCol = mix(white, red, rgPhase);
            brightMult = 1.0 + rgPhase * 2.0;
        } else if (starAge < 0.95) {
            // Supernova — brief intense white-hot flash
            float novaPhase = (starAge - 0.85) / 0.1;
            float novaFlare = sin(novaPhase * PI);
            pCol = mix(red, vec3(1.0, 0.95, 0.8), novaFlare);
            brightMult = 1.0 + novaFlare * 8.0;
        } else {
            // Remnant — dims rapidly to nothing
            float fadePhase = (starAge - 0.95) / 0.05;
            pCol = mix(vec3(1.0, 0.4, 0.1), vec3(0.3, 0.1, 0.05), fadePhase);
            brightMult = max(1.0 - fadePhase * 0.9, 0.1);
        }

        // Additive glow: brightness / distance
        float glow = particleBrightness * brightMult / (d + 0.001);

        // Twinkle — subtle scintillation (uses starClock for independent rate)
        float twinkle = sin(starClock * 6.0 + seed * 7.0) * 0.2 + 0.8;

        // Only visible while alive (starAppear gates birth, starAge < 1 gates death)
        float alive = starAppear * (1.0 - smoothstep(0.95, 1.0, starAge));

        col += pCol * glow * alive * twinkle;
        hotWhite += glow * alive * 0.15;
    }

    // ── Layer 3: Streak particles (orbital trails) ───────────────────
    if (streak_density > 0.01 && expandR > 0.05) {
        int sCount = int(clamp(streak_density * float(NUM_STREAK_PARTICLES), 4.0, float(NUM_STREAK_PARTICLES)));
        float streakBright = 0.0003 * intensity * streak_density;
        for (int i = 0; i < NUM_STREAK_PARTICLES; i++) {
            if (i >= sCount) break;
            float seed = float(i) + 200.0;
            float sAngle = hash(seed * 1.13) * TAU;
            float sR = 0.05 + hash(seed * 2.71) * expandR * 0.9;
            // Orbital rotation
            float orbSpeed = 2.0 / (sqrt(sR) + 0.2);
            sAngle += starClock * orbSpeed * 0.5;
            vec2 sPos = vec2(cos(sAngle), sin(sAngle)) * sR;
            float d = length(p - sPos);
            float glow = streakBright / (d + 0.001);
            // Warm color for streaks
            vec3 sCol = mix(vec3(1.0, 0.7, 0.3), vec3(0.5, 0.7, 1.0), hash(seed * 3.17));
            col += sCol * glow;
        }
    }

    // ── Layer 4: Shockwave ring ──────────────────────────────────────
    // Ring expands past frame edges (expandR up to 3.0) and contracts back
    if (shockwave > 0.01 && t > 0.02) {
        float ringR = expandR;
        float ringWidth = 0.1 + 0.15 * (1.0 - t);
        float ringDist = abs(r - ringR);
        // Soft glow ring
        float ring = exp(-ringDist * ringDist / (ringWidth * ringWidth * 0.5));
        ring *= shockwave * smoothstep(0.0, 0.05, t);
        // Noise variation around the ring
        float ringNoise = 0.8 + 0.2 * noise(vec2(angle * 8.0, t * 5.0));
        ring *= ringNoise;
        // Fade slightly as it gets very far from center
        ring *= 1.0 / (1.0 + ringR * 0.15);
        vec3 ringCol = mix(vec3(0.8, 0.9, 1.0), vec3(1.0, 0.6, 0.2), smoothstep(0.0, 0.5, t));
        col += ringCol * ring * 0.8 * intensity;
        hotWhite += ring * 0.3;
    }

    // ── Layer 5: Radial rays ─────────────────────────────────────────
    // Rays fade with expansion like the core, re-intensify during crunch
    if (rays > 0.01) {
        float rayN1 = noise(vec2(angle * 3.0 + 0.5, 1.7));
        float rayN2 = noise(vec2(angle * 5.0 + 3.1, 2.3));
        float rayPattern = pow(abs(sin(angle * 5.0 + rayN1 * 2.5)), 3.0)
                          + pow(abs(sin(angle * 8.0 + rayN2 * 4.0)), 4.0) * 0.3;
        // Rays fade outward from center
        float rayFade = 1.0 / (1.0 + r * 3.0);
        rayFade *= smoothstep(expandR + 0.3, 0.0, r);
        // Fade with expansion, return during crunch (like core)
        rayFade *= crunch;
        float rayVal = rayPattern * rayFade * rays * intensity * 0.3;
        vec3 rayCol = vec3(1.0, 0.85, 0.6);
        col += rayCol * rayVal;
        hotWhite += rayVal * 0.2;
    }

    // ── Layer 6: Nebula dust (volumetric FBM noise) ──────────────────
    // Dust re-intensifies during crunch as matter collapses back
    if (dust > 0.01 && expandR > 0.05) {
        float swirl = starClock * 0.3;
        float cs = cos(swirl), sn = sin(swirl);
        vec2 sp = vec2(p.x * cs - p.y * sn, p.x * sn + p.y * cs);

        float n1 = fbm(sp * 3.0 + vec2(1.7, 9.2));
        float n2 = fbm(sp * 5.0 + vec2(8.3, 2.8) + starClock * 0.15);
        // Ridged noise for filaments
        float filament = 1.0 - abs(n1 * 2.0 - 1.0);
        filament = filament * filament;
        float detail = 0.5 + 0.5 * n2;

        // Envelope: visible within expansion radius
        float dustEnv = smoothstep(expandR + 0.1, expandR * 0.3, r)
                      * smoothstep(0.02, expandR * 0.15, r);
        // Brighten during crunch: dust glows as it compresses back
        float crunchBoost = 1.0 + crunch * 1.5;
        float dustVal = filament * detail * dustEnv * dust * intensity * 0.6 * crunchBoost;
        vec3 dustCol = mix(vec3(0.6, 0.3, 0.1), vec3(0.3, 0.5, 0.8), n2);
        // Shift warmer during crunch (compressing = heating)
        dustCol = mix(dustCol, vec3(1.0, 0.5, 0.15), crunch * 0.4);
        col += dustCol * dustVal;
    }

    // ── Layer 7: Bloom (re-add hot areas with spread) ────────────────
    col += vec3(1.0, 0.95, 0.9) * hotWhite * bloom;

    // ── Audio reactivity ─────────────────────────────────────────────
    col *= 1.0 + audio_bass * 0.3;

    // ── Filmic tonemap to handle HDR from additive blending ──────────
    col = col / (1.0 + col);
    col = pow(col, vec3(0.9));

    // Mix with background color
    float luminance = dot(col, vec3(0.299, 0.587, 0.114));
    vec3 final_col = mix(bg_color.rgb, fg_color.rgb * col / max(vec3(luminance), vec3(0.01)), clamp(luminance, 0.0, 1.0));
    // For very bright areas, let the actual color through
    final_col = mix(final_col, col, smoothstep(0.3, 0.8, luminance));

    fragColor = vec4(final_col, 1.0);
}
