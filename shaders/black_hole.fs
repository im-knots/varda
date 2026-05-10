/*{
    "DESCRIPTION": "Black Hole — Particle-streak shell with emergent accretion disk, jets, orbiting crystals. Black & white.",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D"],
    "INPUTS": [
        {"NAME": "camera_speed",    "TYPE": "float", "DEFAULT": 0.3,  "MIN": 0.0, "MAX": 2.0,  "LABEL": "Camera Speed"},
        {"NAME": "orbit_radius",   "TYPE": "float", "DEFAULT": 5.0,  "MIN": 2.0, "MAX": 10.0, "LABEL": "Orbit Radius"},
        {"NAME": "orbit_tilt",     "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.0, "MAX": 6.283, "LABEL": "Orbit Tilt"},
        {"NAME": "spin_speed",     "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0, "MAX": 3.0,  "LABEL": "Spin Speed"},
        {"NAME": "hole_size",      "TYPE": "float", "DEFAULT": 0.7,  "MIN": 0.2, "MAX": 2.0,  "LABEL": "Black Hole Size"},
        {"NAME": "shell_radius",   "TYPE": "float", "DEFAULT": 3.5,  "MIN": 1.0, "MAX": 6.0,  "LABEL": "Shell Radius"},
        {"NAME": "density",        "TYPE": "float", "DEFAULT": 1.2,  "MIN": 0.0, "MAX": 3.0,  "LABEL": "Density"},
        {"NAME": "streak_count",   "TYPE": "float", "DEFAULT": 60.0, "MIN": 10.0,"MAX": 150.0,"LABEL": "Streak Count"},
        {"NAME": "streak_thin",    "TYPE": "float", "DEFAULT": 0.75, "MIN": 0.1, "MAX": 1.0,  "LABEL": "Streak Thinness"},
        {"NAME": "equator_bias",   "TYPE": "float", "DEFAULT": 0.6,  "MIN": 0.0, "MAX": 3.0,  "LABEL": "Equator Bias"},
        {"NAME": "scatter",        "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0, "MAX": 1.0,  "LABEL": "Particle Scatter"},
        {"NAME": "cloud_density",  "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0, "MAX": 1.5,  "LABEL": "Cloud Density"},
        {"NAME": "jet_power",      "TYPE": "float", "DEFAULT": 0.7,  "MIN": 0.0, "MAX": 1.0,  "LABEL": "Jet Power"},
        {"NAME": "jet_width",      "TYPE": "float", "DEFAULT": 0.3,  "MIN": 0.05,"MAX": 1.0,  "LABEL": "Jet Width"},
        {"NAME": "jet_height",     "TYPE": "float", "DEFAULT": 6.0,  "MIN": 1.0, "MAX": 12.0, "LABEL": "Jet Height"},
        {"NAME": "jet_speed",      "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.0, "MAX": 4.0,  "LABEL": "Jet Speed"},
        {"NAME": "crystal_count",  "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0, "MAX": 12.0, "LABEL": "Crystal Count"},
        {"NAME": "crystal_size",   "TYPE": "float", "DEFAULT": 0.3,  "MIN": 0.1, "MAX": 0.8,  "LABEL": "Crystal Size"},
        {"NAME": "crystal_dist",   "TYPE": "float", "DEFAULT": 2.5,  "MIN": 1.0, "MAX": 5.0,  "LABEL": "Crystal Distance"},
        {"NAME": "crystal_glow",   "TYPE": "float", "DEFAULT": 0.6,  "MIN": 0.0, "MAX": 1.0,  "LABEL": "Crystal Glow"},
        {"NAME": "pulse_rate",     "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.0, "MAX": 4.0,  "LABEL": "Pulse Rate"},
        {"NAME": "distortion",     "TYPE": "float", "DEFAULT": 0.4,  "MIN": 0.0, "MAX": 1.5,  "LABEL": "Gravitational Lensing"},
        {"NAME": "bloom",          "TYPE": "float", "DEFAULT": 0.2,  "MIN": 0.0, "MAX": 1.0,  "LABEL": "Bloom"},
        {"NAME": "contrast",       "TYPE": "float", "DEFAULT": 1.3,  "MIN": 0.5, "MAX": 3.0,  "LABEL": "Contrast"},
        {"NAME": "invert",         "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0, "MAX": 1.0,  "LABEL": "Invert"},
        {"NAME": "color_mode",     "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0, "MAX": 1.0,  "LABEL": "Color Mode"},
        {"NAME": "hue_shift",      "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0, "MAX": 1.0,  "LABEL": "Hue Shift"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "camera_speed", "INDEX": 0, "SCALE": 1.0},
        {"PARAM": "pulse_rate",   "INDEX": 1, "SCALE": 1.0},
        {"PARAM": "spin_speed",   "INDEX": 2, "SCALE": 1.0},
        {"PARAM": "jet_speed",    "INDEX": 3, "SCALE": 1.0}
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

layout(set = 0, binding = 1) uniform UserParams {
    float camera_speed; float orbit_radius; float orbit_tilt; float spin_speed;
    float hole_size; float shell_radius; float density; float streak_count; float streak_thin;
    float equator_bias; float scatter; float cloud_density;
    float jet_power; float jet_width; float jet_height; float jet_speed;
    float crystal_count; float crystal_size; float crystal_dist; float crystal_glow;
    float pulse_rate;
    float distortion; float bloom; float contrast; float invert;
    float color_mode; float hue_shift;
};

const float PI  = 3.14159265;
const float TAU = 6.28318530;
#define EH hole_size

float hash(float n) { return fract(sin(n) * 43758.5453); }
vec2 hash2(float n) { return vec2(hash(n), hash(n + 71.37)); }

vec3 hsv2rgb(vec3 c) {
    vec3 p = abs(fract(c.xxx + vec3(1.0, 2.0/3.0, 1.0/3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3(1.0), clamp(p - 1.0, 0.0, 1.0), c.y);
}

float sdOctahedron(vec3 p, float s) {
    p = abs(p);
    return (p.x + p.y + p.z - s) * 0.57735027;
}



float raySphere(vec3 ro, vec3 rd, float r) {
    float b = dot(ro, rd);
    float c = dot(ro, ro) - r * r;
    float h = b * b - c;
    if (h < 0.0) return -1.0;
    return -b - sqrt(h);
}

// Gravitational lensing — iterative ray deflection toward the black hole
// Simulates photon path bending in Schwarzschild metric (approximation)
vec3 lensRay(vec3 ro, vec3 rd) {
    if (distortion < 0.01) return rd;
    vec3 p = ro;
    vec3 d = rd;
    float rs = EH * 2.0; // Schwarzschild radius
    float stepLen = 0.3;
    // Integrate ray deflection over several steps
    for (int i = 0; i < 8; i++) {
        vec3 toCenter = -p;
        float r = length(toCenter);
        if (r < EH * 0.5) break;
        // Deflection strength: proportional to rs/r^2 (Newtonian approx)
        // Stronger closer to the hole
        float strength = distortion * rs * stepLen / (r * r + 0.01);
        d = normalize(d + normalize(toCenter) * strength);
        p += d * stepLen;
    }
    return d;
}

mat3 rotAxis(vec3 axis, float a) {
    float c=cos(a), s=sin(a), t=1.0-c;
    return mat3(
        t*axis.x*axis.x+c,       t*axis.x*axis.y-s*axis.z, t*axis.x*axis.z+s*axis.y,
        t*axis.x*axis.y+s*axis.z, t*axis.y*axis.y+c,       t*axis.y*axis.z-s*axis.x,
        t*axis.x*axis.z-s*axis.y, t*axis.y*axis.z+s*axis.x, t*axis.z*axis.z+c
    );
}

// Streaks on a single orbital plane — very thin lines via sharp falloff
float streaksOnPlane(vec2 q, float t, float nStreaks, float planeId) {
    float r = length(q);
    float angle = atan(q.y, q.x);

    // Radial envelope: fade in from EH, extend to 2x shell_radius for wide orbits
    float outerR = shell_radius * 2.0;
    float rEnv = smoothstep(EH - 0.02, EH + 0.15, r) * smoothstep(outerR, shell_radius * 0.3, r);
    if (rEnv < 0.001) return 0.0;

    // Keplerian orbital speed — inner orbits faster
    float angSpeed = 1.8 / (sqrt(r) + 0.15);
    // Work in world-space angle so arcs stay fixed to their orbital plane
    float acc = 0.0;
    float lineW = mix(0.018, 0.005, streak_thin);

    // Slot lookup in rotating frame to find nearby streaks
    float rotAngle = angle - t * angSpeed;
    float slotW = TAU / nStreaks;

    for (int si = 0; si < 4; si++) {
        float slot = floor(rotAngle / slotW) + float(si) - 1.0;
        float sid = slot + planeId * 500.0;
        vec2 h = hash2(sid * 7.13);
        float h3 = hash(sid * 23.71);
        float h4 = hash(sid * 41.03);
        float streakR = EH + 0.1 + h.x * (outerR - EH - 0.1);
        float brightness = 0.3 + 0.7 * h.y;
        float dr = abs(r - streakR);
        float line = exp(-dr * dr / (lineW * lineW));

        // Arc extent: 60%-100% of full circle, biased long (h3^0.3 clusters near 1.0)
        float arcTotal = (0.6 + 0.4 * pow(h3, 0.3)) * TAU;
        // Arc center is fixed in rotating frame (orbits with the streak)
        float arcCenter = (slot + 0.5) * slotW + h4 * TAU;
        // Distance from arc center in rotating frame, wrapped to [-PI, PI]
        float angDist = abs(mod(rotAngle - arcCenter + PI, TAU) - PI);
        float arcHalf = arcTotal * 0.5;
        // Smooth fade at arc tips only
        float arcFade = smoothstep(arcHalf, arcHalf - 0.15, angDist);
        acc += line * brightness * rEnv * arcFade;
    }

    // Inner bright band — denser accumulation near event horizon
    float innerBand = smoothstep(EH + 0.6, EH + 0.1, r) * 0.4;
    acc += innerBand * rEnv;

    return acc;
}

// Full particle shell: 20 tilted orbital planes covering the sphere
float particleShell(vec3 ro, vec3 rd, float t) {
    float acc = 0.0;
    const int NUM_PLANES = 20;

    // Pre-compute EH occlusion once
    float tEH = raySphere(ro, rd, EH);

    for (int i = 0; i < NUM_PLANES; i++) {
        float fi = float(i);
        // Inclination: both positive and negative tilts for full sphere
        // Use golden-angle-like distribution for even coverage
        float rawT = fi / float(NUM_PLANES);
        // Map to inclination: -PI/2 to PI/2 with equator bias
        float normT = rawT * 2.0 - 1.0; // -1 to 1
        float inc = normT * abs(normT) * 1.3; // quadratic bias toward equator
        inc = mix(normT * 1.3, inc, equator_bias);

        // Azimuth: golden angle spacing for non-repeating pattern
        float azim = fi * 2.39996323 + hash(fi * 3.17) * 0.3;

        // Build plane normal
        vec3 tiltAxis = vec3(cos(azim), 0.0, sin(azim));
        mat3 tilt = rotAxis(tiltAxis, inc);
        vec3 normal = tilt * vec3(0.0, 1.0, 0.0);

        // Ray-plane intersection
        float denom = dot(rd, normal);
        if (abs(denom) < 1e-5) continue;
        float tHit = -dot(ro, normal) / denom;
        if (tHit < 0.0) continue;

        vec3 hitP = ro + rd * tHit;
        float r = length(hitP);
        if (r < EH * 0.6 || r > shell_radius * 2.2) continue;

        // Occlusion check
        if (tEH > 0.0 && tHit > tEH) continue;

        // Project to plane-local 2D
        vec3 localHit = transpose(tilt) * hitP;
        vec2 q = localHit.xz;

        // Streak count: more for equatorial planes
        float absInc = abs(inc);
        float weight = 1.0 - absInc / 1.4;
        weight = max(weight, 0.1);
        float nStr = streak_count * (0.5 + 0.5 * weight);
        if (nStr < 3.0) continue;

        float planeT = t + hash(fi * 19.7) * TAU;
        float streaks = streaksOnPlane(q, planeT, nStr, fi);
        acc += streaks * density * weight * 0.35;
    }
    return acc;
}

// Scattered particle dots — tiny pinpoints, denser grid
float scatteredDots(vec3 ro, vec3 rd) {
    float acc = 0.0;
    for (int i = 0; i < 12; i++) {
        float tt = 0.5 + float(i) * 1.0;
        vec3 p = ro + rd * tt;
        float r = length(p);
        if (r < EH || r > shell_radius + 2.5) continue;

        // Finer grid for smaller dots
        vec3 cell = floor(p * 10.0);
        float h = hash(dot(cell, vec3(127.1, 311.7, 74.7)));
        float threshold = 0.94 - scatter * 0.12;
        if (h > threshold) {
            vec3 dotPos = (cell + 0.5) / 10.0;
            float d = length(p - dotPos);
            float rEnv = smoothstep(shell_radius + 2.0, EH + 0.2, r);
            // Very sharp falloff for pinpoint dots
            acc += exp(-d * 300.0) * rEnv * 0.25;
        }
    }
    return acc;
}

// Smooth 3D value noise
float vnoise(vec3 p) {
    vec3 i = floor(p);
    vec3 f = fract(p);
    f = f * f * (3.0 - 2.0 * f); // smoothstep interpolation
    float a = hash(dot(i, vec3(127.1, 311.7, 74.7)));
    float b = hash(dot(i + vec3(1,0,0), vec3(127.1, 311.7, 74.7)));
    float c = hash(dot(i + vec3(0,1,0), vec3(127.1, 311.7, 74.7)));
    float d = hash(dot(i + vec3(1,1,0), vec3(127.1, 311.7, 74.7)));
    float e = hash(dot(i + vec3(0,0,1), vec3(127.1, 311.7, 74.7)));
    float g = hash(dot(i + vec3(1,0,1), vec3(127.1, 311.7, 74.7)));
    float h = hash(dot(i + vec3(0,1,1), vec3(127.1, 311.7, 74.7)));
    float k = hash(dot(i + vec3(1,1,1), vec3(127.1, 311.7, 74.7)));
    return mix(mix(mix(a,b,f.x), mix(c,d,f.x), f.y),
               mix(mix(e,g,f.x), mix(h,k,f.x), f.y), f.z);
}

// ── Orbiting dust clouds — disk-shaped, respects equator_bias ────────
float dustClouds(vec3 ro, vec3 rd, float t) {
    if (cloud_density < 0.01) return 0.0;
    float acc = 0.0;
    for (int i = 0; i < 16; i++) {
        float tt = 1.0 + float(i) * 0.8;
        vec3 p = ro + rd * tt;
        float r = length(p.xz);
        float h = abs(p.y);
        if (r < EH || r > shell_radius * 1.8) continue;

        // Disk thickness — thinner with more equator_bias
        float diskH = 0.6 / (1.0 + equator_bias * 1.5);
        float yFade = exp(-h * h / (diskH * diskH));

        // Orbital rotation — clouds orbit at Keplerian speed
        float angSpeed = 1.8 / (sqrt(r) + 0.15);
        float ang = atan(p.z, p.x) - t * angSpeed;

        // Smooth wispy noise in polar coords
        vec3 noiseP = vec3(ang * 2.0, r * 3.0, p.y * 4.0);
        float n = vnoise(noiseP) * 0.6 + vnoise(noiseP * 2.5) * 0.3 + vnoise(noiseP * 6.0) * 0.1;

        // Radial density — denser near inner edge
        float rFade = smoothstep(EH, EH + 0.5, r) * smoothstep(shell_radius * 1.8, shell_radius * 0.5, r);
        float innerBoost = 1.0 + 2.0 * smoothstep(EH + 1.0, EH + 0.2, r);

        float cloudShape = smoothstep(0.35, 0.55, n) * rFade * yFade * innerBoost;
        acc += cloudShape * cloud_density * 0.04;
    }
    return acc;
}


// ── Magnetic field line particle streams ──────────────────────────────
// Particles flow as 1D chains along dipole field lines: r = L * sin²(θ).
// Each strand is a string of beads — dots spaced along arc length.
// jet_power controls bead density, jet_width controls strand thickness.
float fieldLineJets(vec3 ro, vec3 rd) {
    float acc = 0.0;
    float maxT = length(ro) + jet_height * 2.0;
    float step = maxT / 32.0;
    float strandW = jet_width * 0.15;
    float nStrands = 6.0 + jet_power * 16.0;
    float slotW = TAU / nStrands;
    float flow = PHASE_TIME_3 * TAU * 3.0;
    // Bead spacing along arc — denser with higher jet_power
    float beadSpacing = mix(0.5, 0.12, jet_power);

    for (int i = 0; i < 32; i++) {
        float tt = float(i) * step;
        vec3 p = ro + rd * tt;
        float r = length(p);
        if (r < EH * 0.8 || r > jet_height * 2.0) continue;

        // Spherical coords
        float cosT = clamp(p.y / r, -1.0, 1.0);
        float theta = acos(cosT);
        float sinT = max(sin(theta), 0.01);
        float phi = atan(p.z, p.x);

        // Dipole field line shell: L = r / sin²(θ)
        float L = r / (sinT * sinT);
        if (L > jet_height * 1.5 || L < EH * 1.0) continue;

        for (int di = -1; di <= 1; di++) {
            float slot = floor(phi / slotW) + float(di);
            float sid = slot * 137.5;
            float h1 = hash(sid * 7.13);
            float h2 = hash(sid * 13.37);

            float strandL = EH * 1.2 + h1 * (jet_height - EH);

            // Cross-section distance to this strand's field line
            float dL = (L - strandL) * sinT * sinT;
            float slotCenter = (slot + 0.5) * slotW;
            float dPhi = mod(phi - slotCenter + PI, TAU) - PI;
            float dPhiWorld = dPhi * r * sinT;
            float crossDist = sqrt(dL * dL + dPhiWorld * dPhiWorld);

            if (crossDist > strandW * 2.0) continue;

            // Arc length along this field line (θ as parameter)
            float arc = theta * strandL * 4.0;

            // 1D bead chain: find nearest bead along the arc
            float beadArc = arc - flow; // animate flow from pole
            float beadIdx = floor(beadArc / beadSpacing);
            float beadFrac = fract(beadArc / beadSpacing) - 0.5; // -0.5 to 0.5
            float beadAlong = beadFrac * beadSpacing; // distance along arc to nearest bead

            // Each bead has a tiny random cross-section offset
            float bh = hash(beadIdx * 71.37 + sid);
            float bh2 = hash(beadIdx * 113.0 + sid);
            float crossOffset = (bh - 0.5) * strandW * 0.3;

            // Total distance to bead center (arc + cross-section)
            float beadDist = sqrt(beadAlong * beadAlong
                                + (crossDist + crossOffset) * (crossDist + crossOffset));

            // Sharp tiny dot per bead
            float beadR = strandW * 0.25;
            float dot = exp(-beadDist * beadDist / (beadR * beadR));

            // Thin connecting line between beads (faint thread)
            float threadEnv = exp(-crossDist * crossDist / (strandW * strandW * 0.1));
            float thread = threadEnv * 0.15;

            // Brighter near poles, fade at extent
            float poleBright = 0.3 + 0.7 * pow(max(1.0 - sinT, 0.0), 0.4);
            float rFade = smoothstep(jet_height * 2.0, EH * 1.2, r);
            float brightness = 0.4 + 0.6 * h2;

            acc += (dot + thread) * poleBright * rFade * brightness * jet_power * 3.0;
        }
    }
    return acc;
}

// ── Crystals ─────────────────────────────────────────────────────────
float crystalShape(vec3 ro, vec3 rd, vec3 cp, float t, float fi, float pulse) {
    float tRay = max(dot(cp - ro, rd), 0.0);
    vec3 lp = ro + rd * tRay - cp;
    // Spin around Y axis only (pole axis)
    float ra = t * 0.5 + fi;
    float c = cos(ra), s = sin(ra);
    lp.xz = mat2(c, -s, s, c) * lp.xz;
    float d = sdOctahedron(lp, crystal_size);
    float acc = exp(-d * (5.0 + pulse * 8.0 * crystal_glow)) * 0.35;
    if (d < 0.02) acc += 0.7;
    return acc;
}

float crystalsAnalytic(vec3 ro, vec3 rd, float t) {
    int count = int(crystal_count + 0.5);
    if (count < 1) return 0.0;
    float acc = 0.0;
    float pulse = sin(PHASE_TIME_1 * TAU) * 0.5 + 0.5;

    if (count == 1) {
        // Single pair: directly at the poles, spinning in place
        for (float side = -1.0; side <= 1.0; side += 2.0) {
            vec3 cp = vec3(0.0, side * crystal_dist, 0.0);
            acc += crystalShape(ro, rd, cp, t, 0.0, pulse);
        }
        return acc;
    }

    // Multiple: ring around the equator with mirrored pairs above/below
    float nf = float(count);
    for (int i = 0; i < 12; i++) {
        if (i >= count) break;
        float fi = float(i);
        float a = fi / nf * TAU + t * 0.2;
        for (float side = -1.0; side <= 1.0; side += 2.0) {
            vec3 cp = vec3(cos(a) * crystal_dist * 0.4, side * crystal_dist, sin(a) * crystal_dist * 0.4);
            acc += crystalShape(ro, rd, cp, t, fi, pulse);
        }
    }
    return acc;
}

// ── Main ─────────────────────────────────────────────────────────────
void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum  = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float camT  = PHASE_TIME_0;  // camera orbit
    float spinT = PHASE_TIME_2;  // object/streak spin

    vec3 ro = vec3(cos(camT) * orbit_radius,
                   sin(orbit_tilt) * orbit_radius,
                   sin(camT) * orbit_radius);
    vec3 fw = normalize(-ro);
    vec3 ri = normalize(cross(fw, vec3(0.0, 1.0, 0.0)));
    vec3 up = cross(ri, fw);
    vec3 rd = normalize(p.x * ri + p.y * up + fw * 1.8);
    rd = lensRay(ro, rd);

    float lum = 0.0;
    float bloomAcc = 0.0;

    // Event horizon — pure black, with a thin bright photon ring at the edge
    float tEH = raySphere(ro, rd, EH);
    bool hitHorizon = (tEH > 0.0);
    // Photon ring: bright thin line just outside the event horizon
    float tPhoton = raySphere(ro, rd, EH * 1.15);
    if (tPhoton > 0.0 && !hitHorizon) {
        float grazeR = length(ro + rd * tPhoton);
        float ring = exp(-(grazeR - EH) * (grazeR - EH) * 200.0) * 0.4;
        lum += ring;
        bloomAcc += ring * 0.5;
    }

    // Particle shell (tilted orbital planes)
    float shell = particleShell(ro, rd, spinT);
    lum += shell;
    bloomAcc += shell * 0.25;

    // Scattered dots
    float dots = scatteredDots(ro, rd);
    lum += dots;

    // Dust clouds (disk-shaped)
    float clouds = dustClouds(ro, rd, spinT);
    lum += clouds;
    bloomAcc += clouds * 0.4;

    // Jets
    float jetVal = fieldLineJets(ro, rd);
    lum += jetVal;
    bloomAcc += jetVal * 0.6;

    // Crystals
    float crysVal = crystalsAnalytic(ro, rd, spinT);
    lum += crysVal;
    bloomAcc += crysVal * 0.3;

    // Black hole is BLACK — nothing escapes
    if (hitHorizon) { lum = 0.0; bloomAcc = 0.0; }

    lum *= 1.0 + audio_bass * 0.5;
    bloomAcc *= 1.0 + audio_beat_phase * 0.4;

    lum += bloomAcc * bloom;
    lum = pow(clamp(lum, 0.0, 1.0), 1.0 / contrast);
    lum = mix(lum, 1.0 - lum, invert);

    // Color output
    if (color_mode < 0.01) {
        fragColor = vec4(vec3(lum), 1.0);
    } else {
        // Map different elements to different hues
        float totalE = shell + dots + clouds + jetVal + crysVal + 0.001;
        // Streaks/clouds: warm (orange/gold), jets: cool (blue/cyan), crystals: purple
        float hStreaks = hue_shift + 0.08;          // orange
        float hClouds = hue_shift + 0.05;           // warm amber
        float hJets   = hue_shift + 0.55;           // cyan-blue
        float hCrystals = hue_shift + 0.75;         // purple
        float hDots   = hue_shift + 0.12;           // yellow
        // Weighted hue blend
        float hue = (shell * hStreaks + clouds * hClouds + jetVal * hJets
                    + crysVal * hCrystals + dots * hDots) / totalE;
        // Saturation: stronger color for brighter elements
        float sat = color_mode * (0.5 + 0.5 * lum);
        vec3 rgb = hsv2rgb(vec3(fract(hue), sat, lum));
        fragColor = vec4(rgb, 1.0);
    }
}
