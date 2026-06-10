/*{
    "DESCRIPTION": "Big Bang — cyclical cosmic evolution with fluid-sim galaxy dust, stellar lifecycle, expansion/crunch",
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
        {"NAME": "gravity", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Gravity"},
        {"NAME": "gas_pressure", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Pressure"},
        {"NAME": "spin", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Spin"},
        {"NAME": "expansion", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Expansion"},
        {"NAME": "fg_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Foreground"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "INDEX": 0},
        {"PARAM": "star_speed", "INDEX": 1}
    ],
    "PASSES": [
        {"TARGET": "fluidBuffer", "PERSISTENT": true, "FLOAT": true},
        {}
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

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D fluidBuffer;

layout(set = 0, binding = 3) uniform UserParams {
    float speed;
    float star_speed;
    float intensity;
    float particle_count;
    float shockwave;
    float rays;
    float bloom;
    float streak_density;
    float dust;
    float gravity;
    float gas_pressure;
    float spin;
    float expansion;
    vec4 fg_color;
    vec4 bg_color;
};

#define PI 3.14159265359
#define TAU 6.28318530718
#define NUM_EXPLOSION_PARTICLES 48
#define NUM_STREAK_PARTICLES 24

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

// ── FBM — organic noise field (3 octaves for speed) ─────────────────
float fbm(vec2 p) {
    float v = 0.0, a = 0.5;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 3; i++) {
        v += a * noise(p);
        p = rot * p * 2.0;
        a *= 0.5;
    }
    return v;
}

// ── Shared phase computations ────────────────────────────────────────
float bangPhase() { return fract(PHASE_TIME_0 * 0.04); }
float bangT(float phase) { return 1.0 - abs(phase * 2.0 - 1.0); }

// ── Main ─────────────────────────────────────────────────────────────
void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float phase = bangPhase();
    float t = bangT(phase);
    float expandR = smoothstep(0.0, 1.0, t) * 3.0;
    float crunch = 1.0 - smoothstep(0.0, 0.5, t);
    float starClock = PHASE_TIME_1 * (0.5 + particle_count * 0.1);

    if (PASSINDEX == 0) {
        // ════════════════════════════════════════════════════════════
        // PASS 0: COSMOLOGICAL STRUCTURE FORMATION
        // Buffer: RG = velocity (px/frame), B = mass, A = temperature
        // Physics: reintegration tracking advection + multi-scale
        //          hierarchical gravity (Barnes-Hut style) + pressure
        // Result: Jeans instability → dust clumps into galaxies
        // ════════════════════════════════════════════════════════════
        vec2 p = (uv - 0.5) * 2.0;
        p.x *= RENDERSIZE.x / RENDERSIZE.y;
        float r = length(p);

        // Initialize: near-uniform gas with density perturbations
        if (FRAMEINDEX < 2u) {
            float n = fbm(p * 3.0 + vec2(1.7, 9.2));
            float n2 = noise(p * 6.0 + vec2(8.3, 2.8));
            // Density perturbations seed gravitational instability
            float density = (0.3 + 0.15 * n + 0.08 * n2) * smoothstep(1.8, 0.0, r);
            // Tiny random velocity (seeds angular momentum)
            vec2 vel = vec2(hash12(p * 7.1 + 3.2) - 0.5, hash12(p * 5.3 + 1.7) - 0.5) * 0.2;
            fragColor = vec4(vel, density, 0.2);
            return;
        }

        // ── Step 1: Reintegration tracking (mass-conserving advection) ──
        // Gather mass & momentum from neighbors whose particles moved here
        // Velocities in PIXELS PER FRAME — no conversion needed
        float massAcc = 0.0;
        vec2 momAcc = vec2(0.0);
        float tempAcc = 0.0;
        vec2 gravShort = vec2(0.0);

        // 5×5 neighborhood (R=2, Gaussian kernel negligible past ~2.5)
        for (int dy = -2; dy <= 2; dy++) {
            for (int dx = -2; dx <= 2; dx++) {
                vec2 iOff = vec2(float(dx), float(dy));
                vec2 nbUV = uv + iOff / RENDERSIZE;
                if (nbUV.x < 0.0 || nbUV.x > 1.0 || nbUV.y < 0.0 || nbUV.y > 1.0) continue;

                vec4 nb = texture(sampler2D(fluidBuffer, texSampler), nbUV);
                float mi = nb.z;
                if (mi < 0.0001) continue;

                // Reintegration: where this particle lands relative to us
                vec2 xi = iOff + nb.xy;
                // Gaussian kernel (sigma² = 0.5, sigma ≈ 0.707)
                float kern = exp(-dot(xi, xi)) / PI;

                massAcc += kern * mi;
                momAcc += kern * mi * nb.xy;
                tempAcc += kern * mi * nb.w;

                // Short-range gravity (from direct neighbors, NOT weighted by kernel)
                if (dx != 0 || dy != 0) {
                    float d2 = dot(iOff, iOff) + 0.25;
                    gravShort += (iOff / sqrt(d2)) * mi / d2;
                }
            }
        }

        // Recover velocity from momentum
        vec2 vel = vec2(0.0);
        float temp = 0.0;
        if (massAcc > 0.0001) {
            vel = momAcc / massAcc;
            temp = tempAcc / massAcc;
        }

        // ── Step 2: Multi-scale gravity (medium-range) ──
        // 2 scales × 4 directions — galaxy-scale attraction
        // Uses 1/r² falloff (no area weighting) → favors compact clumps
        vec2 gravLong = vec2(0.0);
        for (int s = 0; s < 2; s++) {
            // Distances: 5, 15 pixels — covers ~galaxy diameter
            float dist = 5.0 * pow(3.0, float(s));
            for (int d = 0; d < 4; d++) {
                float ang = float(d) * PI * 0.5;
                vec2 sDir = vec2(cos(ang), sin(ang));
                vec2 sUV = uv + sDir * dist / RENDERSIZE;
                if (sUV.x < 0.0 || sUV.x > 1.0 || sUV.y < 0.0 || sUV.y > 1.0) continue;

                float sMass = texture(sampler2D(fluidBuffer, texSampler), sUV).z;
                // 1/r² gravity — no area weight → compact clumps, not filaments
                float r2 = dist * dist + 4.0;
                gravLong += sDir * sMass / r2;
            }
        }

        // Combine: short-range (local) + medium-range (multi-scale)
        // gravity slider: 0 = no gravity, 0.5 = default, 1.0 = 2× gravity
        // Scale up gravLong to compensate for fewer samples (4 vs 8 directions)
        float gStr = 0.012 * gravity;
        vel += (gravShort + gravLong * 6.0) * gStr;

        // ── Step 3: Pressure + vorticity ──
        // Density gradient → pressure + tidal torque → spiral arms
        float rho_e = texture(sampler2D(fluidBuffer, texSampler), uv + vec2(1.0 / RENDERSIZE.x, 0.0)).z;
        float rho_w = texture(sampler2D(fluidBuffer, texSampler), uv - vec2(1.0 / RENDERSIZE.x, 0.0)).z;
        float rho_n = texture(sampler2D(fluidBuffer, texSampler), uv + vec2(0.0, 1.0 / RENDERSIZE.y)).z;
        float rho_s = texture(sampler2D(fluidBuffer, texSampler), uv - vec2(0.0, 1.0 / RENDERSIZE.y)).z;
        vec2 densGrad = vec2(rho_e - rho_w, rho_n - rho_s) * 0.5;

        // Pressure: F = -c_s²∇ρ/ρ  (c_s² sets Jeans length)
        // gas_pressure slider: 0 = no pressure (total collapse), 0.5 = default, 1.0 = stiff gas
        float cs2 = 0.024 * gas_pressure;
        vel -= densGrad * cs2 / max(massAcc, 0.01);

        // Tidal torque: perpendicular to density gradient → spiral arms
        // spin slider: 0 = no rotation, 0.5 = default, 1.0 = fast spirals
        vec2 curlKick = vec2(-densGrad.y, densGrad.x);
        vel += curlKick * 0.008 * spin * smoothstep(0.0, 0.3, massAcc);

        // ── Cosmic expansion (Hubble flow) ──
        // expansion slider: 0 = no expansion, 0.5 = default, 1.0 = fast expansion
        vec2 pixCenter = (uv - 0.5) * RENDERSIZE;
        float pixR = length(pixCenter);
        vec2 pixDir = pixCenter / max(pixR, 0.1);
        float hubble = smoothstep(0.0, 0.3, t) * (1.0 - crunch) * 0.024 * expansion;
        vel += pixDir * hubble * pixR / max(RENDERSIZE.y, 1.0);

        // Crunch: pull inward (also scaled by expansion)
        float crunchPull = crunch * 0.04 * expansion * pixR / max(RENDERSIZE.y, 1.0);
        vel -= pixDir * crunchPull;

        // Velocity damping (viscosity)
        vel *= 0.998;
        temp *= 0.997;

        // ── Supernova injection ──
        int pCount = int(clamp(particle_count * 8.0, 16.0, 96.0));
        for (int i = 0; i < NUM_EXPLOSION_PARTICLES; i++) {
            if (i >= pCount) break;
            float seed = float(i) + 1.0;
            vec2 polar = hashPolar(seed);
            vec2 dir = vec2(cos(polar.x), sin(polar.x));
            vec2 starPos = dir * polar.y * expandR;
            float d = length(p - starPos);
            float starOffset = hash(seed * 3.71) * 20.0;
            float lifespan = 1.5 + hash(seed * 5.53) * 2.0;
            float starAge = fract((starClock + starOffset) / lifespan);
            float novaInject = smoothstep(0.83, 0.88, starAge) * smoothstep(0.97, 0.90, starAge);
            float inject = novaInject * 0.02 * exp(-d * d * 15.0);
            massAcc += inject;
            temp += inject * 3.0;
        }

        // Big bang: seed gas from singularity
        if (t > 0.01 && t < 0.4) {
            float bangInject = smoothstep(0.0, 0.08, t) * smoothstep(0.4, 0.1, t);
            float radialDensity = bangInject * 0.012 * exp(-r * r * 2.0);
            massAcc += radialDensity;
            temp += radialDensity * 2.0;
        }

        // Mild mass dissipation (prevents runaway accumulation)
        massAcc *= 0.9995;

        // Clamp
        massAcc = clamp(massAcc, 0.0, 3.0);
        vel = clamp(vel, vec2(-2.0), vec2(2.0));
        temp = clamp(temp, 0.0, 5.0);

        fragColor = vec4(vel, massAcc, temp);

    } else {
        // ════════════════════════════════════════════════════════════
        // PASS 1: RENDER — read fluid density, combine with all visual layers
        // ════════════════════════════════════════════════════════════
        vec2 p = (uv - 0.5) * 2.0;
        p.x *= RENDERSIZE.x / RENDERSIZE.y;
        float r = length(p);
        float angle = atan(p.y, p.x);

        vec3 col = vec3(0.0);
        float hotWhite = 0.0;

        // ── Layer 1: Glowing core singularity ────────────────────────
        float coreStrength = crunch * crunch * intensity;
        float core = (0.008 / (r * r + 0.001)) * coreStrength;
        vec3 coreCol = mix(vec3(1.0, 0.6, 0.2), vec3(0.7, 0.85, 1.0), 1.0 / (1.0 + r * 8.0));
        col += coreCol * core;
        hotWhite += core * 0.5;

        // ── Read fluid density (used by stars + dust layers) ─────────
        vec4 fluid = texture(sampler2D(fluidBuffer, texSampler), uv);

        // ── Layer 2: Stars — gate brightness by local fluid density ──
        // Use the pixel's own fluid density instead of per-star texture reads
        int pCount = int(clamp(particle_count * 8.0, 16.0, 96.0));
        float particleBrightness = 0.0004 * intensity;
        vec3 blue  = vec3(0.4, 0.6, 1.0);
        vec3 white = vec3(1.0, 0.95, 0.9);
        vec3 red   = vec3(1.0, 0.25, 0.05);
        float localDust = fluid.z; // reuse already-fetched fluid data
        float dustGate = 0.1 + 0.9 * smoothstep(0.05, 0.4, localDust);
        for (int i = 0; i < NUM_EXPLOSION_PARTICLES; i++) {
            if (i >= pCount) break;
            float seed = float(i) + 1.0;
            vec2 polar = hashPolar(seed);
            vec2 dir = vec2(cos(polar.x), sin(polar.x));
            vec2 starPos = dir * polar.y * expandR;
            float d = length(p - starPos);
            float starAppear = smoothstep(0.15, 0.25, t);
            float starOffset = hash(seed * 3.71) * 20.0;
            float lifespan = 1.5 + hash(seed * 5.53) * 2.0;
            float starAge = fract((starClock + starOffset) / lifespan);
            vec3 pCol;
            float brightMult = 1.0;
            if (starAge < 0.2) {
                pCol = mix(blue, vec3(0.6, 0.8, 1.0), starAge / 0.2);
            } else if (starAge < 0.6) {
                pCol = mix(vec3(0.6, 0.8, 1.0), white, (starAge - 0.2) / 0.4);
            } else if (starAge < 0.85) {
                float rgPhase = (starAge - 0.6) / 0.25;
                pCol = mix(white, red, rgPhase);
                brightMult = 1.0 + rgPhase * 2.0;
            } else if (starAge < 0.95) {
                float novaPhase = (starAge - 0.85) / 0.1;
                float novaFlare = sin(novaPhase * PI);
                pCol = mix(red, vec3(1.0, 0.95, 0.8), novaFlare);
                brightMult = 1.0 + novaFlare * 8.0;
            } else {
                float fadePhase = (starAge - 0.95) / 0.05;
                pCol = mix(vec3(1.0, 0.4, 0.1), vec3(0.3, 0.1, 0.05), fadePhase);
                brightMult = max(1.0 - fadePhase * 0.9, 0.1);
            }
            float glow = particleBrightness * brightMult / (d + 0.001);
            float twinkle = sin(starClock * 6.0 + seed * 7.0) * 0.2 + 0.8;
            float alive = starAppear * dustGate * (1.0 - smoothstep(0.95, 1.0, starAge));
            col += pCol * glow * alive * twinkle;
            hotWhite += glow * alive * 0.15;
        }

        // ── Layer 3: Streak particles (orbital trails) ───────────────
        if (streak_density > 0.01 && expandR > 0.05) {
            int sCount = int(clamp(streak_density * float(NUM_STREAK_PARTICLES), 4.0, float(NUM_STREAK_PARTICLES)));
            float streakBright = 0.0003 * intensity * streak_density;
            for (int i = 0; i < NUM_STREAK_PARTICLES; i++) {
                if (i >= sCount) break;
                float seed = float(i) + 200.0;
                float sAngle = hash(seed * 1.13) * TAU;
                float sR = 0.05 + hash(seed * 2.71) * expandR * 0.9;
                float orbSpeed = 2.0 / (sqrt(sR) + 0.2);
                sAngle += starClock * orbSpeed * 0.5;
                vec2 sPos = vec2(cos(sAngle), sin(sAngle)) * sR;
                float d = length(p - sPos);
                float glow = streakBright / (d + 0.001);
                vec3 sCol = mix(vec3(1.0, 0.7, 0.3), vec3(0.5, 0.7, 1.0), hash(seed * 3.17));
                col += sCol * glow;
            }
        }

        // ── Layer 4: Shockwave ring ──────────────────────────────────
        if (shockwave > 0.01 && t > 0.02) {
            float ringR = expandR;
            float ringWidth = 0.1 + 0.15 * (1.0 - t);
            float ringDist = abs(r - ringR);
            float ring = exp(-ringDist * ringDist / (ringWidth * ringWidth * 0.5));
            ring *= shockwave * smoothstep(0.0, 0.05, t);
            float ringNoise = 0.8 + 0.2 * noise(vec2(angle * 8.0, t * 5.0));
            ring *= ringNoise;
            ring *= 1.0 / (1.0 + ringR * 0.15);
            vec3 ringCol = mix(vec3(0.8, 0.9, 1.0), vec3(1.0, 0.6, 0.2), smoothstep(0.0, 0.5, t));
            col += ringCol * ring * 0.8 * intensity;
            hotWhite += ring * 0.3;
        }

        // ── Layer 5: Radial rays ─────────────────────────────────────
        if (rays > 0.01) {
            float rayN1 = noise(vec2(angle * 3.0 + 0.5, 1.7));
            float rayN2 = noise(vec2(angle * 5.0 + 3.1, 2.3));
            float rayPattern = pow(abs(sin(angle * 5.0 + rayN1 * 2.5)), 3.0)
                              + pow(abs(sin(angle * 8.0 + rayN2 * 4.0)), 4.0) * 0.3;
            float rayFade = 1.0 / (1.0 + r * 3.0);
            rayFade *= smoothstep(expandR + 0.3, 0.0, r);
            rayFade *= crunch;
            float rayVal = rayPattern * rayFade * rays * intensity * 0.3;
            col += vec3(1.0, 0.85, 0.6) * rayVal;
            hotWhite += rayVal * 0.2;
        }

        // ── Layer 6: Fluid nebula dust (from simulation buffer) ──────
        if (dust > 0.01) {
            float gasDensity = fluid.z;
            float temperature = fluid.w;

            // Cool gas: warm brown/orange nebula with spatial color variation
            vec3 coolDust = mix(vec3(0.5, 0.25, 0.1), vec3(0.3, 0.45, 0.7), fbm(p * 4.0));
            // Hot gas: bright blue-white (recently ejected from supernovae)
            vec3 hotDust = mix(vec3(0.6, 0.7, 1.0), vec3(1.0, 0.9, 0.7), temperature * 0.3);
            float tempFactor = smoothstep(0.0, 2.0, temperature);
            vec3 dustCol = mix(coolDust, hotDust, tempFactor);
            dustCol = mix(dustCol, vec3(1.0, 0.5, 0.15), crunch * 0.3);

            float crunchBoost = 1.0 + crunch * 1.5;
            float dustVal = gasDensity * dust * intensity * 1.5 * crunchBoost;

            col += dustCol * dustVal;
            hotWhite += dustVal * 0.1;
        }

        // ── Layer 7: Bloom ───────────────────────────────────────────
        col += vec3(1.0, 0.95, 0.9) * hotWhite * bloom;

        // ── Audio reactivity ─────────────────────────────────────────
        col *= 1.0 + audio_bass * 0.3;

        // ── Filmic tonemap ───────────────────────────────────────────
        col = col / (1.0 + col);
        col = pow(col, vec3(0.9));

        // Mix with background/foreground colors
        float luminance = dot(col, vec3(0.299, 0.587, 0.114));
        vec3 final_col = mix(bg_color.rgb, fg_color.rgb * col / max(vec3(luminance), vec3(0.01)), clamp(luminance, 0.0, 1.0));
        final_col = mix(final_col, col, smoothstep(0.3, 0.8, luminance));

        fragColor = vec4(final_col, 1.0);
    }
}
