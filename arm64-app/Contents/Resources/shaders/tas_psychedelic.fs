/*{
    "DESCRIPTION": "TAS Visuals - layered psychedelic bilateral ornamental art",
    "CREDIT": "Inspired by TAS Visuals aesthetic",
    "ISFVSN": "2",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "bg_color",        "TYPE": "color",  "DEFAULT": [0.0, 0.0, 0.0, 0.0]},
        {"NAME": "snake_density",   "TYPE": "float",  "DEFAULT": 8.0, "MIN": 1.0, "MAX": 30.0},
        {"NAME": "snake_thickness", "TYPE": "float",  "DEFAULT": 0.02, "MIN": 0.005, "MAX": 0.1},
        {"NAME": "snake_length",    "TYPE": "float",  "DEFAULT": 0.6, "MIN": 0.2, "MAX": 1.5},
        {"NAME": "snake_color",     "TYPE": "color",  "DEFAULT": [1.0, 1.0, 1.0, 1.0]},
        {"NAME": "move_speed",      "TYPE": "float",  "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "move_range",      "TYPE": "float",  "DEFAULT": 1.0, "MIN": 0.1, "MAX": 3.0},
        {"NAME": "slither_freq",    "TYPE": "float",  "DEFAULT": 1.0, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "slither_amp",     "TYPE": "float",  "DEFAULT": 1.0, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "anim_speed",      "TYPE": "float",  "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0}
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
    vec4 bg_color;
    float snake_density;
    float snake_thickness;
    float snake_length;
    vec4 snake_color;
    float move_speed;
    float move_range;
    float slither_freq;
    float slither_amp;
    float anim_speed;
};

#define PI  3.14159265359
#define TAU 6.28318530718

// ============================================================
// Utilities
// ============================================================
float hash(float n) { return fract(sin(n) * 43758.5453); }
float hash2(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

vec3 hsv2rgb(vec3 c) {
    vec3 rgb = clamp(abs(mod(c.x * 6.0 + vec3(0.0, 4.0, 2.0), 6.0) - 3.0) - 1.0, 0.0, 1.0);
    return c.z * mix(vec3(1.0), rgb, c.y);
}

// Porter-Duff over (premultiplied)
vec4 over(vec4 src, vec4 dst) {
    float a = src.a + dst.a * (1.0 - src.a);
    if (a < 0.0001) return vec4(0.0);
    return vec4((src.rgb * src.a + dst.rgb * dst.a * (1.0 - src.a)) / a, a);
}

// Smooth noise
float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(hash2(i), hash2(i + vec2(1.0, 0.0)), f.x),
        mix(hash2(i + vec2(0.0, 1.0)), hash2(i + vec2(1.0, 1.0)), f.x),
        f.y
    );
}



// ============================================================
// Mayan serpent head — optimized (removed unused mayanHead,
// only mayanHeadRotated is called)
// ============================================================

// ============================================================
// Rotated Mayan head - handles arbitrary direction angles
// ============================================================
vec4 mayanHeadRotated(vec2 p, vec2 pivot, float angle, float size, float time, float seed) {
    // Transform point to head-local coordinates with rotation
    vec2 hp = (p - pivot) / size;

    // Rotate to align with snake direction (head faces along +X in local space)
    float c = cos(-angle);
    float s = sin(-angle);
    hp = vec2(hp.x * c - hp.y * s, hp.x * s + hp.y * c);

    // Flip Y for correct orientation
    hp.y *= -1.0;

    vec3 col = vec3(0.0);
    float alpha = 0.0;
    float glow = 0.0;

    // === ORGANIC HEAD SILHOUETTE using smooth ellipses ===
    float skull = length(hp * vec2(1.0, 1.3)) - 0.5;
    vec2 snoutP = hp - vec2(0.4, 0.08);
    float snout = length(snoutP * vec2(0.7, 1.8)) - 0.35;
    vec2 ujawP = hp - vec2(0.75, 0.12);
    float ujaw = length(ujawP * vec2(0.6, 2.0)) - 0.25;
    ujaw = max(ujaw, -hp.y + 0.0);
    vec2 ljawP = hp - vec2(0.55, -0.22);
    float ljaw = length(ljawP * vec2(0.8, 2.5)) - 0.22;
    ljaw = max(ljaw, hp.y + 0.1);
    vec2 browP = hp - vec2(0.1, 0.35);
    float brow = length(browP * vec2(1.2, 3.0)) - 0.3;

    float headSDF = min(min(min(skull, snout), min(ujaw, ljaw)), brow);
    float headMask = smoothstep(0.015, -0.015, headSDF);
    float headEdge = smoothstep(0.03, 0.0, abs(headSDF));

    // Early-out: skip detail if pixel is far from head
    if (headSDF > 0.5) return vec4(0.0);

    // Contour lines (reduced from 5 to 3)
    float contours = 0.0;
    for (int i = 1; i < 4; i++) {
        float offset = float(i) * 0.06;
        contours += smoothstep(0.01, 0.0, abs(headSDF + offset)) * (1.0 - float(i) * 0.22);
    }
    contours *= headMask;

    // Flowing scales
    vec2 scaleCenter = hp - vec2(0.1, 0.05);
    float scaleAngle = atan(scaleCenter.y, scaleCenter.x);
    float scaleDist = length(scaleCenter);
    vec2 scaleUV = vec2(scaleAngle * 3.0 + scaleDist * 8.0, scaleDist * 12.0);
    vec2 scaleF = fract(scaleUV) - 0.5;
    float hex = max(abs(scaleF.x) * 0.866 + abs(scaleF.y) * 0.5, abs(scaleF.y));
    float scales = (smoothstep(0.45, 0.35, hex) - smoothstep(0.32, 0.22, hex)) * headMask * smoothstep(0.1, 0.2, scaleDist);

    // Eye
    vec2 eyeP = hp - vec2(-0.05, 0.12);
    float eyeDist = length(eyeP * vec2(1.0, 0.8));
    float eyeAngle = atan(eyeP.y, eyeP.x);
    float notches = 0.5 + 0.5 * sin(eyeAngle * 12.0 + time * 0.3);
    float outerEye = smoothstep(0.22, 0.18, eyeDist) * smoothstep(0.12, 0.16, eyeDist) * (0.7 + notches * 0.3);
    float eyeDiamond = abs(eyeP.x) + abs(eyeP.y * 0.8);
    float diamondFrame = smoothstep(0.25, 0.22, eyeDiamond) - smoothstep(0.2, 0.17, eyeDiamond);
    float iris = smoothstep(0.14, 0.08, eyeDist);
    float pupil = smoothstep(0.05, 0.02, eyeDist);

    // Fangs
    float fangs = 0.0;
    for (int i = 0; i < 2; i++) {
        float fi = float(i);
        vec2 fangBase = vec2(0.85 + fi * 0.12, 0.0 - fi * 0.05);
        vec2 fangP = hp - fangBase;
        fangP.x += fangP.y * fangP.y * 2.0;
        float fangWidth = max(0.04 * (1.0 - (-fangP.y) * 2.0), 0.01);
        float fang = smoothstep(fangWidth, fangWidth * 0.3, abs(fangP.x));
        fang *= smoothstep(0.0, -0.02, fangP.y) * smoothstep(-0.18, -0.12, fangP.y);
        fangs = max(fangs, fang * (1.0 - fi * 0.15));
    }

    // Plumes (reduced from 6 to 4)
    float plumes = 0.0;
    for (int i = 0; i < 4; i++) {
        float fi = float(i);
        float baseAngle = 1.8 + fi * 0.18 + sin(time * 0.3 + fi) * 0.05;
        vec2 plumeStart = vec2(-0.25 - fi * 0.04, 0.3 + fi * 0.08);
        vec2 plumeP = hp - plumeStart;
        vec2 plumeDir = vec2(cos(baseAngle), sin(baseAngle));
        vec2 plumePerp = vec2(-plumeDir.y, plumeDir.x);
        float along = dot(plumeP, plumeDir);
        float across = dot(plumeP, plumePerp);
        float plumeLen = 0.35 + fi * 0.03;
        float taper = smoothstep(0.0, 0.1, along) * smoothstep(plumeLen, plumeLen * 0.7, along);
        float width = 0.025 * taper + 0.008;
        float plume = smoothstep(width, width * 0.2, abs(across));
        plume *= smoothstep(-0.02, 0.02, along) * smoothstep(plumeLen + 0.02, plumeLen - 0.02, along);
        plume *= 0.6 + (sin(along * 60.0 - abs(across) * 30.0) * 0.5 + 0.5) * 0.4;
        plumes = max(plumes, plume * (1.0 - fi * 0.08));
    }

    // Colors
    vec3 baseCol = hsv2rgb(vec3(fract(0.08 + hp.y * 0.08 + seed * 0.1), 0.6, 0.85)) * headMask * 0.5;
    vec3 contourCol = hsv2rgb(vec3(0.12, 0.3, 1.0)) * contours * 0.8;
    vec3 scaleCol = hsv2rgb(vec3(0.45 + seed * 0.1, 0.85, 0.95)) * scales;
    glow += scales * 0.3;

    vec3 eyeCol = hsv2rgb(vec3(0.1, 0.7, 1.0)) * outerEye;
    eyeCol += hsv2rgb(vec3(0.55, 0.8, 1.0)) * diamondFrame;
    eyeCol += hsv2rgb(vec3(0.15, 0.9, 1.0)) * iris;
    eyeCol += vec3(1.0, 0.3, 0.1) * pupil * 2.0;
    glow += (iris + pupil) * 0.6;

    vec3 fangCol = vec3(1.0, 0.98, 0.95) * fangs;
    glow += fangs * 0.5;

    vec3 plumeCol = hsv2rgb(vec3(fract(0.3 + hp.y * 0.8 + time * 0.08 + seed * 0.1), 0.95, 1.0)) * plumes;
    glow += plumes * 0.4;

    vec3 edgeGlowCol = hsv2rgb(vec3(fract(0.55 + seed * 0.2 + time * 0.03), 0.9, 1.0));
    glow += headEdge * 0.8;

    col = baseCol + contourCol + scaleCol + eyeCol + fangCol + plumeCol + edgeGlowCol * glow * 0.6;
    alpha = clamp(max(max(max(headMask, plumes), fangs), glow * 0.5), 0.0, 1.0);

    return vec4(col, alpha);
}

// ============================================================
// Complex layered body patterns
// ============================================================
vec4 bodyPatterns(vec2 uv, float body, float time, float seed) {
    vec3 col = vec3(0.0);
    float glow = 0.0;

    // Multiple pattern layers at different scales

    // === LAYER 1: Large stepped diamonds ===
    vec2 grid1 = uv * vec2(8.0, 3.0);
    vec2 id1 = floor(grid1);
    vec2 f1 = fract(grid1) - 0.5;

    // Nested diamonds
    float diamond1 = abs(f1.x) + abs(f1.y);
    float outerDiamond = smoothstep(0.5, 0.45, diamond1) - smoothstep(0.4, 0.35, diamond1);
    float midDiamond = smoothstep(0.35, 0.3, diamond1) - smoothstep(0.25, 0.2, diamond1);
    float innerDiamond = smoothstep(0.2, 0.15, diamond1);

    float hue1 = hash(id1.x * 13.7 + id1.y * 7.3 + seed);
    col += hsv2rgb(vec3(fract(hue1 + time * 0.03), 0.9, 1.0)) * outerDiamond * body;
    col += hsv2rgb(vec3(fract(hue1 + 0.33), 0.95, 1.0)) * midDiamond * body;
    col += hsv2rgb(vec3(fract(hue1 + 0.66), 1.0, 1.0)) * innerDiamond * body;
    glow += (outerDiamond + midDiamond * 1.5 + innerDiamond * 2.0) * body * 0.3;

    // === LAYER 2: Small triangular teeth pattern ===
    vec2 grid2 = uv * vec2(24.0, 6.0);
    vec2 id2 = floor(grid2);
    vec2 f2 = fract(grid2);

    // Alternating up/down triangles
    float flip = mod(id2.x + id2.y, 2.0);
    vec2 tf = flip > 0.5 ? vec2(f2.x, 1.0 - f2.y) : f2;
    float tri = tf.y - abs(tf.x - 0.5) * 2.0;
    float triangle = smoothstep(0.0, 0.1, tri) * smoothstep(0.8, 0.5, tf.y);

    float hue2 = hash(id2.x * 23.1 + id2.y * 17.9 + seed + 100.0);
    col += hsv2rgb(vec3(fract(hue2 + time * 0.05), 0.85, 0.9)) * triangle * body * 0.6;

    // === LAYER 3: Stepped pyramid / ziggurat pattern ===
    vec2 grid3 = uv * vec2(5.0, 1.0);
    vec2 id3 = floor(grid3);
    vec2 f3 = fract(grid3) - 0.5;

    float steps = 0.0;
    for (int s = 0; s < 4; s++) {
        float fs = float(s);
        float stepH = 0.4 - fs * 0.1;
        float stepW = 0.4 - fs * 0.08;
        float step = smoothstep(stepW + 0.02, stepW, abs(f3.x))
                   * smoothstep(stepH + 0.02, stepH, abs(f3.y));
        step -= smoothstep(stepW - 0.03, stepW - 0.05, abs(f3.x))
              * smoothstep(stepH - 0.03, stepH - 0.05, abs(f3.y));
        steps += step * (0.6 + fs * 0.15);
    }

    float hue3 = hash(id3.x * 31.3 + seed + 200.0);
    col += hsv2rgb(vec3(fract(hue3 + 0.1), 0.9, 1.0)) * steps * body * 0.5;
    glow += steps * body * 0.2;

    // === NEON GLOW ===
    vec3 glowCol = hsv2rgb(vec3(fract(uv.x * 0.5 + time * 0.1 + seed * 0.1), 0.9, 1.0));
    col += glowCol * glow * 0.5;

    return vec4(col, 0.0);
}

// ============================================================
// Analytical snake path — deterministic position for any time t
// Pure Lissajous with phase offset for smooth speed control
// ============================================================
vec2 snakePath(float t, float seed, float rangeScale, float phaseOff) {
    float ax1 = (0.28 + hash(seed + 10.0) * 0.12) * rangeScale;
    float ax2 = (0.10 + hash(seed + 20.0) * 0.08) * rangeScale;
    float ay1 = (0.22 + hash(seed + 40.0) * 0.12) * rangeScale;
    float ay2 = (0.08 + hash(seed + 55.0) * 0.08) * rangeScale;

    float wx1 = 0.30 + hash(seed + 70.0) * 0.20;
    float wx2 = 0.55 + hash(seed + 80.0) * 0.30;
    float wy1 = 0.25 + hash(seed + 110.0) * 0.20;
    float wy2 = 0.50 + hash(seed + 120.0) * 0.30;

    float px1 = hash(seed + 200.0) * TAU;
    float px2 = hash(seed + 210.0) * TAU;
    float py1 = hash(seed + 230.0) * TAU;
    float py2 = hash(seed + 240.0) * TAU;

    return vec2(
        ax1 * sin(wx1 * t + px1 + phaseOff) + ax2 * sin(wx2 * t + px2 + phaseOff),
        ay1 * cos(wy1 * t + py1 + phaseOff) + ay2 * cos(wy2 * t + py2 + phaseOff)
    );
}

// Velocity of snakePath (analytical derivative)
vec2 snakePathVel(float t, float seed, float rangeScale, float phaseOff) {
    float ax1 = (0.28 + hash(seed + 10.0) * 0.12) * rangeScale;
    float ax2 = (0.10 + hash(seed + 20.0) * 0.08) * rangeScale;
    float ay1 = (0.22 + hash(seed + 40.0) * 0.12) * rangeScale;
    float ay2 = (0.08 + hash(seed + 55.0) * 0.08) * rangeScale;

    float wx1 = 0.30 + hash(seed + 70.0) * 0.20;
    float wx2 = 0.55 + hash(seed + 80.0) * 0.30;
    float wy1 = 0.25 + hash(seed + 110.0) * 0.20;
    float wy2 = 0.50 + hash(seed + 120.0) * 0.30;

    float px1 = hash(seed + 200.0) * TAU;
    float px2 = hash(seed + 210.0) * TAU;
    float py1 = hash(seed + 230.0) * TAU;
    float py2 = hash(seed + 240.0) * TAU;

    return vec2(
        ax1 * wx1 * cos(wx1 * t + px1 + phaseOff) + ax2 * wx2 * cos(wx2 * t + px2 + phaseOff),
       -ay1 * wy1 * sin(wy1 * t + py1 + phaseOff) - ay2 * wy2 * sin(wy2 * t + py2 + phaseOff)
    );
}

// ============================================================
// Single snake — returns premultiplied RGBA
// Body trails behind head by sampling past positions
// ============================================================
vec4 drawSnake(vec2 p, float time, int i) {
    float fi   = float(i);
    float seed = fi * 127.1;

    // Per-snake randomized params
    float snakeLen = snake_length * (0.7 + hash(seed + 150.0) * 0.6);
    float mr = move_range;
    float phaseOff = move_speed * TAU * 2.0;
    float sFreqBase = 2.0 + hash(seed + 160.0) * 1.5;
    float sAmpBase  = 0.03 + hash(seed + 170.0) * 0.02;

    const int SPINE_N = 20; // reduced from 40

    float headTime = time;
    vec2 headPos = snakePath(headTime, seed, mr, phaseOff);

    // Bounding-box early-out: skip snake if pixel is far from head
    float maxExtent = snakeLen + snake_thickness * 4.0;
    if (length(p - headPos) > maxExtent) return vec4(0.0);

    vec2 headVel = snakePathVel(headTime, seed, mr, phaseOff);
    float headAngle = atan(headVel.y, headVel.x);

    // Estimate speed (reduced from 3 samples to 2)
    float speedEst = (length(headVel)
                    + length(snakePathVel(headTime - 1.5, seed, mr, phaseOff))) * 0.5;
    speedEst = max(speedEst, 0.03);
    float stepLen = snakeLen / float(SPINE_N);
    float dt = stepLen / speedEst;

    float minDist = 1e6;
    float bestT = 0.0;
    float bestSigned = 0.0;
    vec2 bestTangent = vec2(1.0, 0.0);

    vec2 prevBasePt = headPos;
    vec2 prevPt = headPos;
    for (int s = 1; s <= 20; s++) {
        float bodyFrac = float(s) / float(SPINE_N);
        vec2 basePt = snakePath(headTime - float(s) * dt, seed, mr, phaseOff);

        vec2 tangent = prevBasePt - basePt;
        float tanLen = length(tangent);
        vec2 tanDir = tangent / max(tanLen, 0.0001);
        vec2 perp = vec2(-tanDir.y, tanDir.x);
        float bodyTaper = smoothstep(0.0, 0.1, bodyFrac) * smoothstep(1.0, 0.85, bodyFrac);
        float slitherOffset = sin(bodyFrac * sFreqBase * slither_freq * TAU)
                            * sAmpBase * slither_amp * bodyTaper;
        vec2 curPt = basePt + perp * slitherOffset;

        vec2 seg = prevPt - curPt;
        float segLen = length(seg);
        vec2 segDir = seg / max(segLen, 0.0001);
        vec2 toP = p - curPt;
        float proj = clamp(dot(toP, segDir), 0.0, segLen);
        vec2 closest = curPt + segDir * proj;
        float d = length(p - closest);

        if (d < minDist) {
            minDist = d;
            float prevFrac = float(s - 1) / float(SPINE_N);
            bestT = 1.0 - prevFrac - (bodyFrac - prevFrac) * (1.0 - proj / max(segLen, 0.0001));
            bestTangent = segDir;
            vec2 norm = vec2(-segDir.y, segDir.x);
            bestSigned = dot(p - closest, norm);
        }

        prevBasePt = basePt;
        prevPt = curPt;
    }

    float alongBody = clamp(bestT, 0.0, 1.0);
    float dist = minDist;

    // Taper at head and tail
    float taper = smoothstep(0.0, 0.25, alongBody) * smoothstep(1.0, 0.85, alongBody);
    float thick = snake_thickness * (0.35 + taper * 0.65);

    // Body mask
    float inBody = smoothstep(thick * 1.5, thick, dist);
    float body = smoothstep(thick, thick * 0.15, dist) * smoothstep(0.0, 0.12, alongBody);

    // Outer glow
    float outerGlow = smoothstep(thick * 2.5, thick, dist) * 0.3;

    // Outline
    float outline = smoothstep(thick * 1.15, thick * 0.95, dist)
                  - smoothstep(thick * 0.95, thick * 0.7, dist);
    outline *= smoothstep(0.0, 0.12, alongBody);

    // Pattern UV — use signed distance for cross-body coordinate
    float normDist = bestSigned / max(thick, 0.001) * 0.5 + 0.5;
    vec2 patternUV = vec2(alongBody, normDist);
    vec4 patterns = bodyPatterns(patternUV, body, time, seed);

    // === HEAD ===
    vec4 headResult = mayanHeadRotated(p, headPos, headAngle, snake_thickness * 3.0, time, seed);

    // === COMBINE ===
    vec3 col = vec3(0.0);

    vec3 glowHue = hsv2rgb(vec3(fract(seed * 0.01 + time * 0.05), 0.9, 1.0));
    col += glowHue * outerGlow;
    col += snake_color.rgb * body * 0.2;
    col += patterns.rgb;

    vec3 outlineCol = hsv2rgb(vec3(fract(alongBody + time * 0.1), 0.8, 1.0));
    col += outlineCol * outline * 0.8;
    col += snake_color.rgb * outline * 0.4;
    col += headResult.rgb;

    float a = clamp(max(max(max(body, outline), headResult.a), outerGlow) * snake_color.a, 0.0, 1.0);
    return vec4(col * a, a);
}

// ============================================================
// Main
// ============================================================
void main() {
    // Uniform preservation
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + DATE.x;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Aspect-corrected centered coordinates
    vec2 p = (gl_FragCoord.xy - 0.5 * RENDERSIZE) / min(RENDERSIZE.x, RENDERSIZE.y);

    float time = PHASE_TIME_0;

    vec4 result = vec4(bg_color.rgb * bg_color.a, bg_color.a);

    int numSnakes = int(clamp(snake_density, 1.0, 30.0));
    for (int i = 0; i < 30; i++) {
        if (i >= numSnakes) break;
        vec4 s = drawSnake(p, time, i);
        result = over(s, result);
    }

    fragColor = result;
}