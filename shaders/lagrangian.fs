/*{
    "DESCRIPTION": "Lagrangian - Standard Model Lagrangian typed terminal-style with parallax layers",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "speed",       "LABEL": "Speed",       "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.1, "MAX": 10.0},
        {"NAME": "text_size",   "LABEL": "Text Size",   "TYPE": "float", "DEFAULT": 0.06, "MIN": 0.02, "MAX": 0.15},
        {"NAME": "glow",        "LABEL": "Glow",        "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0, "MAX": 1.0},
        {"NAME": "depth_layers","LABEL": "Depth Layers","TYPE": "float", "DEFAULT": 3.0,  "MIN": 1.0, "MAX": 5.0},
        {"NAME": "spread",      "LABEL": "Layer Spread","TYPE": "float", "DEFAULT": 0.3,  "MIN": 0.0, "MAX": 1.0},
        {"NAME": "h_position",  "LABEL": "H Position",  "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0, "MAX": 1.0},
        {"NAME": "rotation",    "LABEL": "Rotation",    "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0, "MAX": 1.0},
        {"NAME": "fg_color",    "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Foreground"},
        {"NAME": "bg_color",    "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 0.0], "LABEL": "Background"}
    ],
    "IMPORTED": {
        "physics_font_atlas": { "PATH": "character_atlases/physics_font_atlas.png" }
    },
    "PHASE_INPUTS": [{"PARAM": "speed", "INDEX": 0}]
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
layout(set = 0, binding = 2) uniform texture2D physics_font_atlas;

layout(set = 0, binding = 3) uniform UserParams {
    float speed;
    float text_size;
    float glow;
    float depth_layers;
    float spread;
    float h_position;
    float rotation;
    vec4 fg_color;
    vec4 bg_color;
};

const float ATLAS_COLS = 8.0;
const float NUM_CHARS = 110.0;
const float MSDF_PX_RANGE = 4.0;
const int SP = 200;

// ── Standard Model Lagrangian (Bradley Hand Bold atlas, 110 glyphs) ──
// Index map: 0=( 1=) 2=+ 3=- 4=/ 5=0 6=1 7=2 8=3 9=4 10=5 15==
// 17=B 19=D 20=F 21=G 23=L 24=M 28=R 29=S 31=U 32=V 33=W
// 40=e 41=g 42=h 43=i 44=l 46=q 47=r 49=t 50=u
// 51=× 54=Γ 76=γ 84=λ 85=μ 86=ν 95=φ 97=ψ 99=† 100=∂ 103=√
const int EQ_COUNT = 16;
const int EQ_LEN = 24;
const int EQ_DATA[EQ_COUNT * EQ_LEN] = int[](
    23,200,15,200,3,6,4,9,200,17,85,86,17,85,86,200,200,200,200,200,200,200,200,200, // L = -1/4 BμνBμν
    3,6,4,7,200,49,47,0,33,85,86,33,85,86,1,200,200,200,200,200,200,200,200,200,     // -1/2 tr(WμνWμν)
    3,6,4,7,200,49,47,0,21,85,86,21,85,86,1,200,200,200,200,200,200,200,200,200,     // -1/2 tr(GμνGμν)
    2,43,97,76,85,19,85,97,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200, // +iψγμDμψ
    2,0,19,85,95,1,99,19,85,95,200,200,200,200,200,200,200,200,200,200,200,200,200,200,     // +(Dμφ)†Dμφ
    3,85,7,95,99,95,2,84,0,95,99,95,1,7,200,200,200,200,200,200,200,200,200,200,     // -μ2φ†φ+λ(φ†φ)2
    3,54,40,44,95,40,28,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200, // -ΓelφeR
    3,54,50,46,95,50,28,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200, // -ΓuqφuR
    3,54,39,46,95,39,28,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200,200, // -ΓdqφdR
    17,85,86,15,100,85,17,86,3,100,86,17,85,200,200,200,200,200,200,200,200,200,200,200,     // Bμν=∂μBν-∂νBμ
    33,85,86,15,100,85,33,86,3,100,86,33,85,200,200,200,200,200,200,200,200,200,200,200,     // Wμν=∂μWν-∂νWμ
    19,85,15,100,85,2,43,41,33,85,200,200,200,200,200,200,200,200,200,200,200,200,200,200,   // Dμ=∂μ+igWμ
    32,0,95,1,15,85,7,95,99,95,2,84,0,95,99,95,1,7,200,200,200,200,200,200,           // V(φ)=μ2φ†φ+λ(φ†φ)2
    29,31,0,8,1,51,29,31,0,7,1,51,31,0,6,1,200,200,200,200,200,200,200,200,           // SU(3)×SU(2)×U(1)
    100,23,4,100,95,3,100,85,0,100,23,4,100,0,100,85,95,1,1,15,5,200,200,200,         // ∂L/∂φ-∂μ(∂L/∂(∂μφ))=0
    0,43,76,85,100,85,3,24,1,97,15,5,200,200,200,200,200,200,200,200,200,200,200,200  // (iγμ∂μ-M)ψ=0
);

const int EQ_LENS[EQ_COUNT] = int[](
    15, 15, 15, 8, 10, 14, 7, 7, 7, 13, 13, 10, 18, 16, 21, 12
);

// Cumulative character counts for mapping global char index → (equation, column)
// EQ_CUM[i] = sum of EQ_LENS[0..i-1]. EQ_CUM[0] = 0, EQ_CUM[16] = TOTAL_CHARS
const int TOTAL_CHARS = 201;
const int EQ_CUM[EQ_COUNT + 1] = int[](
    0, 15, 30, 45, 53, 63, 77, 84, 91, 98, 111, 124, 134, 152, 168, 189, 201
);

// Pause (in chars worth of time) after all equations before looping
const int LOOP_PAUSE = 40;

float median3(float r, float g, float b) {
    return max(min(r, g), min(max(r, g), b));
}

// Single MSDF sample — returns signed-distance opacity
float sampleGlyph(float charIdx, vec2 cellUV) {
    float numRows = ceil(NUM_CHARS / ATLAS_COLS);
    float col = mod(charIdx, ATLAS_COLS);
    float row = floor(charIdx / ATLAS_COLS);

    vec2 atlasSize = vec2(textureSize(sampler2D(physics_font_atlas, texSampler), 0));
    vec2 cellSize = atlasSize / vec2(ATLAS_COLS, numRows);
    vec2 halfTexel = 0.5 / cellSize;
    vec2 clamped = clamp(cellUV, halfTexel, vec2(1.0) - halfTexel);

    float u = (col + clamped.x) / ATLAS_COLS;
    float v = (row + clamped.y) / numRows;

    vec3 msdLinear = texture(sampler2D(physics_font_atlas, texSampler), vec2(u, v)).rgb;
    vec3 msd = pow(msdLinear, vec3(1.0 / 2.2));
    float sd = median3(msd.r, msd.g, msd.b);

    vec2 unitRange = MSDF_PX_RANGE / atlasSize;
    vec2 screenTexSize = vec2(1.0) / fwidth(vec2(u, v));
    float screenPxRange = max(0.5 * dot(unitRange, screenTexSize), 1.0);

    return (sd - 0.5) * screenPxRange + 0.5;
}

void main() {
    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    int numLayers = int(clamp(depth_layers, 1.0, 5.0));
    vec3 color = bg_color.rgb;
    float alpha = bg_color.a;

    // Typing speed: characters per time unit
    float charsPerSec = speed * 8.0;

    for (int layer = 0; layer < 5; layer++) {
        if (layer >= numLayers) break;

        float layerF = float(layer);
        float depthT = layerF / max(float(numLayers) - 1.0, 1.0);

        // Depth: back layers smaller, dimmer
        float scale = mix(0.5, 1.0, 1.0 - depthT);
        float opacity = mix(0.15, 1.0, pow(1.0 - depthT, 1.5));

        // Each layer is offset in time so they type at different phases
        float layerTimeOffset = layerF * float(TOTAL_CHARS + LOOP_PAUSE) / charsPerSec * 0.3;
        float t = PHASE_TIME_0 + layerTimeOffset;

        // Global cursor: how many chars have been typed so far (loops)
        int loopLen = TOTAL_CHARS + LOOP_PAUSE;
        float rawCursor = t * charsPerSec;
        int globalCursor = int(mod(rawCursor, float(loopLen)));

        // Character cell dimensions
        float charHeight = text_size * scale;
        float lineSpacing = charHeight * 2.0;
        float charWidth = charHeight * 0.6;
        // h_position: 0 = far left, 0.5 = centered, 1.0 = far right
        float maxEqWidth = 21.0 * charWidth; // widest equation is 21 chars
        float availableWidth = aspect - maxEqWidth;
        float leftMargin = max(charWidth * 0.5, availableWidth * h_position);

        // Transform pixel position
        vec2 p = vec2(uv.x * aspect, (1.0 - uv.y)); // flip Y so row 0 is at top

        // Optional per-layer rotation
        if (rotation > 0.01) {
            float angle = rotation * (layerF - float(numLayers) * 0.5) * 0.08;
            vec2 center = vec2(aspect * 0.5, 0.5);
            vec2 d = p - center;
            float ca = cos(angle); float sa = sin(angle);
            p = center + vec2(ca * d.x - sa * d.y, sa * d.x + ca * d.y);
        }

        // Per-layer horizontal offset for parallax spread (controlled by spread param)
        float layerXOffset = spread * (layerF - float(numLayers) * 0.5) * charWidth * 8.0;
        p.x -= layerXOffset;

        // How many visible rows fit on screen
        float visibleRows = 1.0 / lineSpacing;

        // Find which equation the cursor is currently in (for auto-scroll)
        int cursorEqRow = 0;
        for (int e = 0; e < EQ_COUNT; e++) {
            if (globalCursor >= EQ_CUM[e]) cursorEqRow = e;
        }

        // Scroll offset: once cursor passes visible area, scroll up
        float scrollOffset = 0.0;
        float cursorScreenY = float(cursorEqRow) * lineSpacing + lineSpacing * 0.5;
        float bottomMargin = lineSpacing * 2.0;
        if (cursorScreenY > (1.0 - bottomMargin)) {
            scrollOffset = cursorScreenY - (1.0 - bottomMargin);
        }

        // Which row does this pixel fall in?
        float py = p.y + scrollOffset;
        int eqRow = int(floor(py / lineSpacing));
        float rowFrac = fract(py / lineSpacing);

        // Out of range
        if (eqRow < 0 || eqRow >= EQ_COUNT) continue;

        // Vertical position within character cell
        float cellV = rowFrac * lineSpacing / charHeight;
        if (cellV < 0.2 || cellV > 1.2) continue; // center the glyph in the row
        cellV = (cellV - 0.2);

        // Horizontal: which column?
        float px = p.x - leftMargin;
        if (px < 0.0) continue;
        int charCol = int(floor(px / charWidth));
        float cellU = fract(px / charWidth);

        int eqLen = EQ_LENS[eqRow];
        if (charCol >= eqLen) continue;

        // Global index of this character
        int globalIdx = EQ_CUM[eqRow] + charCol;

        // Only show characters that have been "typed" already
        if (globalIdx >= globalCursor) {
            // Blinking cursor: show a block cursor at the current position
            if (globalIdx == globalCursor) {
                // Cursor blink (on/off at ~2Hz)
                float blink = step(0.0, sin(PHASE_TIME_0 * 12.566));
                if (blink > 0.5 && cellU > 0.1 && cellU < 0.9 && cellV > 0.2 && cellV < 0.8) {
                    float cursorContrib = 0.6 * opacity;
                    color = mix(color, fg_color.rgb, cursorContrib);
                    alpha = max(alpha, cursorContrib * fg_color.a);
                }
            }
            continue;
        }

        // Fade-in: recently typed characters are brighter
        float charAge = float(globalCursor - globalIdx);
        float fadeIn = smoothstep(0.0, 4.0, charAge); // fade in over ~4 chars

        int charVal = EQ_DATA[eqRow * EQ_LEN + charCol];
        if (charVal >= int(NUM_CHARS)) continue;

        // MSDF sample
        float dist = sampleGlyph(float(charVal), vec2(cellU, cellV));

        // Sharp glyph edge
        float glyphMask = clamp(dist, 0.0, 1.0);

        // Analytical glow
        float glowMask = smoothstep(-0.4, 0.3, dist) * glow * 0.6;

        float combined = max(glyphMask, glowMask) * fadeIn;

        float contrib = combined * opacity;
        color = mix(color, fg_color.rgb, contrib);
        alpha = max(alpha, contrib * fg_color.a);
    }

    fragColor = vec4(color, alpha);
}
