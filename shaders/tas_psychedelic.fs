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
        {"NAME": "anim_speed",      "TYPE": "float",  "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0}
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

layout(set = 0, binding = 1) uniform UserParams {
    vec4 bg_color;
    float snake_density;
    float snake_thickness;
    float snake_length;
    vec4 snake_color;
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
// High-resolution Mayan serpent head - smooth organic curves
// ============================================================
vec4 mayanHead(vec2 p, vec2 pivot, float facing, float size, float time, float seed) {
    vec2 hp = (p - pivot) / size;
    hp.x *= facing;
    hp.y *= -1.0; // Flip Y so fangs point down, plumes point up in screen space

    vec3 col = vec3(0.0);
    float alpha = 0.0;
    float glow = 0.0;

    // === ORGANIC HEAD SILHOUETTE using smooth ellipses ===
    // Main skull - ellipse
    float skull = length(hp * vec2(1.0, 1.3)) - 0.5;

    // Elongated snout - stretched ellipse
    vec2 snoutP = hp - vec2(0.4, 0.08);
    float snout = length(snoutP * vec2(0.7, 1.8)) - 0.35;

    // Upper jaw - curved wedge
    vec2 ujawP = hp - vec2(0.75, 0.12);
    float ujaw = length(ujawP * vec2(0.6, 2.0)) - 0.25;
    ujaw = max(ujaw, -hp.y + 0.0); // cut off bottom

    // Lower jaw - separate curved piece
    vec2 ljawP = hp - vec2(0.55, -0.22);
    float ljaw = length(ljawP * vec2(0.8, 2.5)) - 0.22;
    ljaw = max(ljaw, hp.y + 0.1); // cut off top

    // Brow ridge
    vec2 browP = hp - vec2(0.1, 0.35);
    float brow = length(browP * vec2(1.2, 3.0)) - 0.3;

    // Combine with smooth union
    float headSDF = min(skull, snout);
    headSDF = min(headSDF, ujaw);
    headSDF = min(headSDF, ljaw);
    headSDF = min(headSDF, brow);

    float headMask = smoothstep(0.015, -0.015, headSDF);
    float headEdge = smoothstep(0.03, 0.0, abs(headSDF));

    // === CONTOUR LINES - follow head shape ===
    float contours = 0.0;
    for (int i = 1; i < 6; i++) {
        float offset = float(i) * 0.06;
        float contour = smoothstep(0.012, 0.0, abs(headSDF + offset));
        contours += contour * (1.0 - float(i) * 0.15);
    }
    contours *= headMask;

    // === SCALES - flowing with head shape ===
    // Use polar-ish coordinates from head center for organic flow
    vec2 scaleCenter = hp - vec2(0.1, 0.05);
    float scaleAngle = atan(scaleCenter.y, scaleCenter.x);
    float scaleDist = length(scaleCenter);

    // Hex-like scale pattern
    vec2 scaleUV = vec2(scaleAngle * 3.0 + scaleDist * 8.0, scaleDist * 12.0);
    vec2 scaleId = floor(scaleUV);
    vec2 scaleF = fract(scaleUV) - 0.5;

    // Smooth hexagonal scales
    float hex = max(abs(scaleF.x) * 0.866 + abs(scaleF.y) * 0.5, abs(scaleF.y));
    float scales = smoothstep(0.45, 0.35, hex) - smoothstep(0.32, 0.22, hex);
    float scaleInner = smoothstep(0.2, 0.1, hex);
    scales *= headMask * smoothstep(0.1, 0.2, scaleDist); // fade near center
    scaleInner *= headMask * smoothstep(0.1, 0.2, scaleDist);

    // === EYE - large ornate Mayan style ===
    vec2 eyeP = hp - vec2(-0.05, 0.12);
    float eyeDist = length(eyeP * vec2(1.0, 0.8));
    float eyeAngle = atan(eyeP.y, eyeP.x);

    // Outer ornamental ring with notches
    float notches = 0.5 + 0.5 * sin(eyeAngle * 12.0 + time * 0.3);
    float outerEye = smoothstep(0.22, 0.18, eyeDist) * smoothstep(0.12, 0.16, eyeDist);
    outerEye *= (0.7 + notches * 0.3);

    // Diamond frame around eye
    float eyeDiamond = abs(eyeP.x) + abs(eyeP.y * 0.8);
    float diamondFrame = smoothstep(0.25, 0.22, eyeDiamond) - smoothstep(0.2, 0.17, eyeDiamond);

    // Iris with gradient
    float iris = smoothstep(0.14, 0.08, eyeDist);

    // Bright pupil core
    float pupil = smoothstep(0.05, 0.02, eyeDist);

    // Eye highlight
    vec2 highlightP = eyeP - vec2(-0.03, 0.03);
    float highlight = smoothstep(0.04, 0.01, length(highlightP));

    // === FANGS - curved and sharp ===
    float fangs = 0.0;
    float fangGlow = 0.0;

    // Large front fangs
    for (int i = 0; i < 2; i++) {
        float fi = float(i);
        vec2 fangBase = vec2(0.85 + fi * 0.12, 0.0 - fi * 0.05);
        vec2 fangP = hp - fangBase;

        // Curved fang using rotation along length
        float curve = fangP.y * 0.3;
        fangP.x += curve * curve * 2.0;

        // Tapered triangle
        float fangWidth = 0.04 * (1.0 - (-fangP.y) * 2.0);
        fangWidth = max(fangWidth, 0.01);
        float fang = smoothstep(fangWidth, fangWidth * 0.3, abs(fangP.x));
        fang *= smoothstep(0.0, -0.02, fangP.y) * smoothstep(-0.18, -0.12, fangP.y);

        fangs = max(fangs, fang * (1.0 - fi * 0.15));
        fangGlow += fang * 0.5;
    }

    // Smaller teeth row
    for (int i = 0; i < 4; i++) {
        float fi = float(i);
        vec2 toothP = hp - vec2(0.6 + fi * 0.06, -0.08 - fi * 0.02);
        float tooth = smoothstep(0.025, 0.01, abs(toothP.x));
        tooth *= smoothstep(0.0, -0.015, toothP.y) * smoothstep(-0.06, -0.04, toothP.y);
        fangs = max(fangs, tooth * 0.7);
    }

    // === FEATHERED CREST - flowing Quetzalcoatl plumes ===
    float plumes = 0.0;
    float plumeGlow = 0.0;

    for (int i = 0; i < 8; i++) {
        float fi = float(i);
        float baseAngle = 1.8 + fi * 0.18 + sin(time * 0.3 + fi) * 0.05;
        vec2 plumeStart = vec2(-0.25 - fi * 0.04, 0.3 + fi * 0.08);
        vec2 plumeP = hp - plumeStart;

        // Direction with gentle wave
        float wave = sin(fi * 1.5 + time * 0.5) * 0.1;
        vec2 plumeDir = vec2(cos(baseAngle + wave), sin(baseAngle + wave));
        vec2 plumePerp = vec2(-plumeDir.y, plumeDir.x);

        float along = dot(plumeP, plumeDir);
        float across = dot(plumeP, plumePerp);

        // Feather width varies - wider at base, pointed tip
        float plumeLen = 0.35 + fi * 0.03;
        float taper = smoothstep(0.0, 0.1, along) * smoothstep(plumeLen, plumeLen * 0.7, along);
        float width = 0.025 * taper + 0.008;

        // Main feather body
        float plume = smoothstep(width, width * 0.2, abs(across));
        plume *= smoothstep(-0.02, 0.02, along) * smoothstep(plumeLen + 0.02, plumeLen - 0.02, along);

        // Feather barbs - fine lines
        float barbs = sin(along * 60.0 - abs(across) * 30.0) * 0.5 + 0.5;
        plume *= 0.6 + barbs * 0.4;

        // Central rachis (spine)
        float rachis = smoothstep(0.008, 0.002, abs(across)) * plume;

        plumes = max(plumes, plume * (1.0 - fi * 0.08));
        plumeGlow += plume * 0.3;
    }

    // === NOSTRIL ===
    vec2 nostrilP = hp - vec2(0.7, 0.18);
    float nostril = smoothstep(0.04, 0.02, length(nostrilP * vec2(1.0, 1.5)));

    // === BROW DECORATION - stepped pattern ===
    vec2 browDecP = hp - vec2(0.0, 0.4);
    float browDec = 0.0;
    for (int i = 0; i < 3; i++) {
        float fi = float(i);
        float stepY = 0.04 + fi * 0.03;
        float stepW = 0.15 - fi * 0.03;
        float step = smoothstep(stepW + 0.01, stepW, abs(browDecP.x));
        step *= smoothstep(stepY + 0.015, stepY, abs(browDecP.y - fi * 0.04));
        step -= smoothstep(stepW - 0.02, stepW - 0.03, abs(browDecP.x))
              * smoothstep(stepY - 0.01, stepY - 0.015, abs(browDecP.y - fi * 0.04));
        browDec += step * (0.9 - fi * 0.2);
    }
    browDec *= headMask;

    // === COLOR COMPOSITION ===
    // Base gradient
    float headHue = 0.08 + hp.y * 0.08 + hp.x * 0.03;
    vec3 baseCol = hsv2rgb(vec3(fract(headHue + seed * 0.1), 0.6, 0.85)) * headMask * 0.5;

    // Contour lines - white/gold
    vec3 contourCol = hsv2rgb(vec3(0.12, 0.3, 1.0)) * contours * 0.8;

    // Scales - teal/cyan
    float scaleHue = 0.45 + hash(scaleId.x * 7.3 + scaleId.y * 11.1 + seed) * 0.15;
    vec3 scaleCol = hsv2rgb(vec3(scaleHue, 0.85, 0.95)) * scales;
    vec3 scaleInnerCol = hsv2rgb(vec3(scaleHue + 0.1, 0.9, 1.0)) * scaleInner * 0.7;
    glow += (scales + scaleInner) * 0.3;

    // Eye
    vec3 eyeCol = hsv2rgb(vec3(0.1, 0.7, 1.0)) * outerEye;
    eyeCol += hsv2rgb(vec3(0.55, 0.8, 1.0)) * diamondFrame;
    eyeCol += hsv2rgb(vec3(0.15, 0.9, 1.0)) * iris;
    eyeCol += vec3(1.0, 0.3, 0.1) * pupil * 2.0; // glowing red pupil
    eyeCol += vec3(1.0) * highlight;
    glow += (iris + pupil) * 0.6;

    // Fangs - bright white with glow
    vec3 fangCol = vec3(1.0, 0.98, 0.95) * fangs;
    glow += fangGlow;

    // Plumes - rainbow gradient along length
    float plumeHue = 0.3 + hp.y * 0.8 + time * 0.08;
    vec3 plumeCol = hsv2rgb(vec3(fract(plumeHue + seed * 0.1), 0.95, 1.0)) * plumes;
    glow += plumeGlow;

    // Nostril - dark
    vec3 nostrilCol = vec3(-0.3) * nostril;

    // Brow decoration - gold
    vec3 browCol = hsv2rgb(vec3(0.12, 0.9, 1.0)) * browDec;
    glow += browDec * 0.4;

    // Edge glow
    vec3 edgeGlowCol = hsv2rgb(vec3(fract(0.55 + seed * 0.2 + time * 0.03), 0.9, 1.0));
    glow += headEdge * 0.8;

    // Final composition
    col = baseCol;
    col += contourCol;
    col += scaleCol + scaleInnerCol;
    col += eyeCol;
    col += fangCol;
    col += plumeCol;
    col += nostrilCol;
    col += browCol;
    col += edgeGlowCol * glow * 0.6;

    alpha = max(max(max(headMask, plumes), fangs), glow * 0.5);
    alpha = clamp(alpha, 0.0, 1.0);

    return vec4(col, alpha);
}

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

    // Contour lines
    float contours = 0.0;
    for (int i = 1; i < 5; i++) {
        float offset = float(i) * 0.05;
        contours += smoothstep(0.01, 0.0, abs(headSDF + offset)) * (1.0 - float(i) * 0.18);
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

    // Plumes
    float plumes = 0.0;
    for (int i = 0; i < 6; i++) {
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

    // === LAYER 4: Eye/sun motifs ===
    vec2 grid4 = uv * vec2(4.0, 1.5);
    vec2 id4 = floor(grid4);
    vec2 f4 = fract(grid4) - 0.5;

    float eyeDist = length(f4 * vec2(1.0, 1.5));
    float eyeAngle = atan(f4.y, f4.x);

    // Sun rays
    float rays = abs(sin(eyeAngle * 6.0 + time + id4.x)) * 0.5 + 0.5;
    float sunRays = smoothstep(0.35, 0.25, eyeDist) * smoothstep(0.15, 0.2, eyeDist) * rays;

    // Central eye
    float eye = smoothstep(0.15, 0.08, eyeDist);
    float pupil = smoothstep(0.06, 0.03, eyeDist);

    float hue4 = hash(id4.x * 41.7 + id4.y * 19.3 + seed + 300.0);
    col += hsv2rgb(vec3(fract(hue4 + time * 0.02), 0.8, 1.0)) * sunRays * body;
    col += hsv2rgb(vec3(fract(hue4 + 0.5), 0.9, 1.0)) * eye * body;
    col -= vec3(0.5) * pupil * body;
    glow += (sunRays + eye) * body * 0.4;

    // === LAYER 5: Fine geometric line work ===
    vec2 grid5 = uv * vec2(40.0, 10.0);
    vec2 f5 = fract(grid5);

    // Cross-hatch
    float lines = smoothstep(0.08, 0.0, abs(f5.x - 0.5));
    lines += smoothstep(0.08, 0.0, abs(f5.y - 0.5));
    // Diagonal
    lines += smoothstep(0.06, 0.0, abs(f5.x - f5.y));
    lines += smoothstep(0.06, 0.0, abs(f5.x + f5.y - 1.0));
    lines = min(lines, 1.0);

    col += vec3(0.8, 0.85, 0.9) * lines * body * 0.15;

    // === NEON GLOW ===
    vec3 glowCol = hsv2rgb(vec3(fract(uv.x * 0.5 + time * 0.1 + seed * 0.1), 0.9, 1.0));
    col += glowCol * glow * 0.5;

    return vec4(col, 0.0);
}

// ============================================================
// Parametric snake path — bounded Lissajous-like curves
// Returns position at time t for a given snake seed
// ============================================================
vec2 snakePath(float t, float seed) {
    // 3 layers of sine/cosine at incommensurate frequencies
    // Amplitudes keep the snake within ~(-0.65, 0.65) on each axis
    float ax1 = 0.25 + hash(seed + 10.0) * 0.15;
    float ax2 = 0.12 + hash(seed + 20.0) * 0.10;
    float ax3 = 0.06 + hash(seed + 30.0) * 0.06;
    float ay1 = 0.20 + hash(seed + 40.0) * 0.15;
    float ay2 = 0.10 + hash(seed + 55.0) * 0.10;
    float ay3 = 0.05 + hash(seed + 65.0) * 0.06;

    // Frequencies — use irrational-ish ratios so paths don't repeat quickly
    float wx1 = 0.7  + hash(seed + 70.0) * 0.5;
    float wx2 = 1.3  + hash(seed + 80.0) * 0.6;
    float wx3 = 2.1  + hash(seed + 90.0) * 0.8;
    float wy1 = 0.5  + hash(seed + 110.0) * 0.5;
    float wy2 = 1.1  + hash(seed + 120.0) * 0.6;
    float wy3 = 1.9  + hash(seed + 130.0) * 0.8;

    // Phase offsets
    float px1 = hash(seed + 200.0) * TAU;
    float px2 = hash(seed + 210.0) * TAU;
    float px3 = hash(seed + 220.0) * TAU;
    float py1 = hash(seed + 230.0) * TAU;
    float py2 = hash(seed + 240.0) * TAU;
    float py3 = hash(seed + 250.0) * TAU;

    float x = ax1 * sin(wx1 * t + px1)
            + ax2 * sin(wx2 * t + px2)
            + ax3 * sin(wx3 * t + px3);
    float y = ay1 * cos(wy1 * t + py1)
            + ay2 * cos(wy2 * t + py2)
            + ay3 * cos(wy3 * t + py3);

    return vec2(x, y);
}

// ============================================================
// Single snake — returns premultiplied RGBA
// Uses parametric path for smooth on-screen movement
// ============================================================
vec4 drawSnake(vec2 p, float time, int i) {
    float fi   = float(i);
    float seed = fi * 127.1;

    // Snake parameters — randomized per snake
    float snakeLen = snake_length * (0.7 + hash(seed + 150.0) * 0.6);
    float freq1    = 4.0 + hash(seed + 100.0) * 3.0;
    float amp1     = 0.03 + hash(seed + 300.0) * 0.04;
    float phase    = hash(seed + 400.0) * TAU;
    float speed    = 0.8 + hash(seed + 500.0) * 0.8;

    // Time step between spine samples — controls how much path history the body covers
    float bodyTimeSpan = snakeLen / (speed * 0.6 + 0.3);

    // === BUILD SPINE by sampling parametric path at past times ===
    const int SPINE_N = 25;
    float dt = bodyTimeSpan / float(SPINE_N);

    // Find closest spine point to pixel
    float minDist = 1e6;
    float bestT = 0.0;        // normalized 0..1 along body
    float bestSigned = 0.0;   // signed perpendicular distance
    vec2 bestTangent = vec2(1.0, 0.0);

    // Head is at current time, tail is in the past
    float headTime = time * speed;

    vec2 prevPt = snakePath(headTime - float(SPINE_N) * dt, seed);
    vec2 headPos = prevPt;
    float headAngle = 0.0;

    for (int s = 0; s <= SPINE_N; s++) {
        float t = float(s) / float(SPINE_N); // 0=tail, 1=head

        // Sample path at this time (tail=oldest, head=newest)
        float sampleTime = headTime - float(SPINE_N - s) * dt;
        vec2 curPt = snakePath(sampleTime, seed);

        // Add sine undulation perpendicular to path for slithering feel
        if (s > 0 && s < SPINE_N) {
            vec2 tangent = normalize(curPt - prevPt);
            vec2 perp = vec2(-tangent.y, tangent.x);
            float wave = sin(t * snakeLen * freq1 + time * speed + phase) * amp1;
            curPt += perp * wave;
        }

        // Distance from pixel to this spine segment
        if (s > 0) {
            vec2 seg = curPt - prevPt;
            float segLen = length(seg);
            vec2 segDir = seg / max(segLen, 0.0001);
            vec2 toP = p - prevPt;
            float proj = clamp(dot(toP, segDir), 0.0, segLen);
            vec2 closest = prevPt + segDir * proj;
            float d = length(p - closest);

            if (d < minDist) {
                minDist = d;
                bestT = (float(s - 1) + proj / max(segLen, 0.0001)) / float(SPINE_N);
                bestTangent = segDir;
                vec2 norm = vec2(-segDir.y, segDir.x);
                bestSigned = dot(p - closest, norm);
            }
        }

        if (s == SPINE_N) {
            headPos = curPt;
            vec2 tangent = normalize(curPt - prevPt);
            headAngle = atan(tangent.y, tangent.x);
        }

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

    float time = TIME * anim_speed;

    vec4 result = vec4(bg_color.rgb * bg_color.a, bg_color.a);

    int numSnakes = int(clamp(snake_density, 1.0, 30.0));
    for (int i = 0; i < 30; i++) {
        if (i >= numSnakes) break;
        vec4 s = drawSnake(p, time, i);
        result = over(s, result);
    }

    fragColor = result;
}