/*{
    "DESCRIPTION": "Big Brother surveillance overlay — face detection with dossier info boxes",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Analysis"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "overlay_opacity", "LABEL": "Overlay Opacity", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "text_opacity", "LABEL": "Text Opacity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "text_size", "LABEL": "Text Size", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 2.0},
        {"NAME": "tint_r", "LABEL": "Tint Red", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "tint_g", "LABEL": "Tint Green", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "tint_b", "LABEL": "Tint Blue", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "bracket_thickness", "LABEL": "Bracket Thickness", "TYPE": "float", "DEFAULT": 2.0, "MIN": 1.0, "MAX": 5.0},
        {"NAME": "scanline_intensity", "LABEL": "Scanlines", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.5}
    ],
    "IMPORTED": {
        "bigbro_font_atlas": {"PATH": "character_atlases/bigbro_font_atlas.png"}
    },
    "PREPROCESSORS": [
        {"NAME": "landmarks", "TYPE": "face_detect"},
        {"NAME": "face_data", "TYPE": "face_detect"},
        {"NAME": "dossier_text", "TYPE": "face_detect"}
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
layout(set = 0, binding = 2) uniform texture2D inputImage;

// IMPORTED atlas
layout(set = 0, binding = 3) uniform texture2D bigbro_font_atlas;

// PREPROCESSOR textures from face_detect analyzer
layout(set = 0, binding = 4) uniform texture2D landmarks;
layout(set = 0, binding = 5) uniform texture2D face_data;
layout(set = 0, binding = 6) uniform texture2D dossier_text;

layout(set = 0, binding = 7) uniform UserParams {
    float overlay_opacity;
    float text_opacity;
    float text_size;
    float tint_r;
    float tint_g;
    float tint_b;
    float bracket_thickness;
    float scanline_intensity;
};

// --- MSDF text rendering constants ---

const float ATLAS_COLS = 8.0;
const float NUM_CHARS = 45.0;
const float MSDF_PX_RANGE = 4.0;

// --- Data texture constants ---

const int MAX_FACES = 10;
const int FACE_DATA_W = 480;
const int DOSSIER_TEX_W = 48;
const int CHARS_PER_PIXEL = 4;

// --- Helper functions ---

float median3(float r, float g, float b) {
    return max(min(r, g), min(max(r, g), b));
}

float sampleGlyph(float charIdx, vec2 cellUV) {
    float numRows = ceil(NUM_CHARS / ATLAS_COLS);
    float col = mod(charIdx, ATLAS_COLS);
    float row = floor(charIdx / ATLAS_COLS);

    vec2 atlasSize = vec2(textureSize(sampler2D(bigbro_font_atlas, texSampler), 0));
    vec2 cellSize = atlasSize / vec2(ATLAS_COLS, numRows);
    vec2 halfTexel = 0.5 / cellSize;
    vec2 clamped = clamp(cellUV, halfTexel, vec2(1.0) - halfTexel);

    float u = (col + clamped.x) / ATLAS_COLS;
    float v = (row + clamped.y) / numRows;

    // Sample returns linear (sRGB-decoded) values; undo gamma to recover raw MSDF data
    vec3 msdLinear = texture(sampler2D(bigbro_font_atlas, texSampler), vec2(u, v)).rgb;
    vec3 msd = pow(msdLinear, vec3(1.0 / 2.2));
    float sd = median3(msd.r, msd.g, msd.b);

    vec2 unitRange = MSDF_PX_RANGE / atlasSize;
    vec2 screenTexSize = vec2(1.0) / fwidth(vec2(u, v));
    float screenPxRange = max(0.5 * dot(unitRange, screenTexSize), 1.0);

    return clamp((sd - 0.5) * screenPxRange + 0.5, 0.0, 1.0);
}

vec4 readFaceData(int col, int row) {
    vec2 texCoord = (vec2(float(col) + 0.5, float(row) + 0.5))
                  / vec2(float(FACE_DATA_W), float(MAX_FACES));
    return texture(sampler2D(face_data, texSampler), texCoord);
}

vec4 readDossierText(int col, int row) {
    vec2 texCoord = (vec2(float(col) + 0.5, float(row) + 0.5))
                  / vec2(float(DOSSIER_TEX_W), float(MAX_FACES));
    return texture(sampler2D(dossier_text, texSampler), texCoord);
}

// --- Corner bracket rendering ---

vec3 renderBrackets(vec3 color, vec4 bbox, vec2 pixelUV) {
    float x0 = bbox.r, y0 = bbox.g, x1 = bbox.b, y1 = bbox.a;
    float w = x1 - x0, h = y1 - y0;
    float arm = min(w, h) * 0.2;
    float thick = bracket_thickness / RENDERSIZE.y;
    float accentLen = arm * 0.33;

    vec3 darkColor = vec3(0.12);
    vec3 accentColor = vec3(0.86, 0.16, 0.16);

    // Top-left corner
    bool inTL = (pixelUV.x >= x0 && pixelUV.x <= x0 + arm && pixelUV.y >= y0 && pixelUV.y <= y0 + thick) ||
                (pixelUV.x >= x0 && pixelUV.x <= x0 + thick && pixelUV.y >= y0 && pixelUV.y <= y0 + arm);
    bool inTLaccent = (pixelUV.x >= x0 && pixelUV.x <= x0 + accentLen && pixelUV.y >= y0 && pixelUV.y <= y0 + thick) ||
                      (pixelUV.x >= x0 && pixelUV.x <= x0 + thick && pixelUV.y >= y0 && pixelUV.y <= y0 + accentLen);

    // Top-right corner
    bool inTR = (pixelUV.x >= x1 - arm && pixelUV.x <= x1 && pixelUV.y >= y0 && pixelUV.y <= y0 + thick) ||
                (pixelUV.x >= x1 - thick && pixelUV.x <= x1 && pixelUV.y >= y0 && pixelUV.y <= y0 + arm);
    bool inTRaccent = (pixelUV.x >= x1 - accentLen && pixelUV.x <= x1 && pixelUV.y >= y0 && pixelUV.y <= y0 + thick) ||
                      (pixelUV.x >= x1 - thick && pixelUV.x <= x1 && pixelUV.y >= y0 && pixelUV.y <= y0 + accentLen);

    // Bottom-left corner
    bool inBL = (pixelUV.x >= x0 && pixelUV.x <= x0 + arm && pixelUV.y >= y1 - thick && pixelUV.y <= y1) ||
                (pixelUV.x >= x0 && pixelUV.x <= x0 + thick && pixelUV.y >= y1 - arm && pixelUV.y <= y1);
    bool inBLaccent = (pixelUV.x >= x0 && pixelUV.x <= x0 + accentLen && pixelUV.y >= y1 - thick && pixelUV.y <= y1) ||
                      (pixelUV.x >= x0 && pixelUV.x <= x0 + thick && pixelUV.y >= y1 - accentLen && pixelUV.y <= y1);

    // Bottom-right corner
    bool inBR = (pixelUV.x >= x1 - arm && pixelUV.x <= x1 && pixelUV.y >= y1 - thick && pixelUV.y <= y1) ||
                (pixelUV.x >= x1 - thick && pixelUV.x <= x1 && pixelUV.y >= y1 - arm && pixelUV.y <= y1);
    bool inBRaccent = (pixelUV.x >= x1 - accentLen && pixelUV.x <= x1 && pixelUV.y >= y1 - thick && pixelUV.y <= y1) ||
                      (pixelUV.x >= x1 - thick && pixelUV.x <= x1 && pixelUV.y >= y1 - accentLen && pixelUV.y <= y1);

    bool inAnyBracket = inTL || inTR || inBL || inBR;
    bool inAnyAccent = inTLaccent || inTRaccent || inBLaccent || inBRaccent;

    if (inAnyAccent) {
        color = accentColor;
    } else if (inAnyBracket) {
        color = darkColor;
    }

    return color;
}

// --- Dossier text rendering ---

vec3 renderDossierText(vec3 color, vec4 bbox, vec2 pixelUV, int faceIdx) {
    // Text box: right of face bbox
    float boxX = bbox.b + 0.01;
    float boxY = bbox.g;

    float charH = text_size * 0.018;
    float charW = charH * 0.6;
    float lineSpacing = charH * 1.4;
    float pad = 0.005;

    float maxCharsPerLine = 25.0;
    float numLines = 7.0;
    float boxW = maxCharsPerLine * charW + 2.0 * pad;
    float boxH = numLines * lineSpacing + 2.0 * pad;

    // Bounds check
    if (pixelUV.x < boxX || pixelUV.x > boxX + boxW) return color;
    if (pixelUV.y < boxY || pixelUV.y > boxY + boxH) return color;

    // Semi-transparent background
    color = mix(color, vec3(0.0), 0.7);

    // Draw thin border
    float borderT = 1.0 / RENDERSIZE.y;
    if (abs(pixelUV.x - boxX) < borderT || abs(pixelUV.x - (boxX + boxW)) < borderT ||
        abs(pixelUV.y - boxY) < borderT || abs(pixelUV.y - (boxY + boxH)) < borderT) {
        color = vec3(0.0, 0.7, 0.2);
        return color;
    }

    // Local coords within text area
    float lx = pixelUV.x - boxX - pad;
    float ly = pixelUV.y - boxY - pad;
    if (lx < 0.0 || ly < 0.0) return color;
    if (lx > maxCharsPerLine * charW || ly > numLines * lineSpacing) return color;

    // Which character cell?
    int charCol = int(floor(lx / charW));
    int charRow = int(floor(ly / lineSpacing));

    // cellUV within the glyph cell
    float cellX = fract(lx / charW);
    float cellY = fract(ly / lineSpacing);
    // Scale Y to account for line spacing vs char height ratio
    cellY = cellY * (lineSpacing / charH);
    if (cellY > 1.0) return color; // inter-line gap
    vec2 cellUV = vec2(cellX, cellY);

    // Walk through packed dossier_text data to find the target line start
    int globalIdx = 0;
    int currentLine = 0;

    if (charRow > 0) {
        for (int pix = 0; pix < DOSSIER_TEX_W; pix++) {
            vec4 texel = readDossierText(pix, faceIdx);
            float vals[4] = float[4](texel.r, texel.g, texel.b, texel.a);
            for (int ch = 0; ch < CHARS_PER_PIXEL; ch++) {
                int v = int(vals[ch] * 255.0 + 0.5);
                if (v == 255) {
                    return color; // end of all text
                }
                if (v == 254) {
                    currentLine++;
                    globalIdx++;
                    if (currentLine == charRow) {
                        break;
                    }
                    continue;
                }
                globalIdx++;
            }
            if (currentLine == charRow) break;
        }
        if (currentLine < charRow) return color;
    }

    // Advance by charCol characters within this line
    int targetIdx = globalIdx + charCol;
    int pixelIdx = targetIdx / CHARS_PER_PIXEL;
    int channelIdx = targetIdx % CHARS_PER_PIXEL;

    if (pixelIdx >= DOSSIER_TEX_W) return color;

    vec4 charData = readDossierText(pixelIdx, faceIdx);
    float charVal;
    if      (channelIdx == 0) charVal = charData.r;
    else if (channelIdx == 1) charVal = charData.g;
    else if (channelIdx == 2) charVal = charData.b;
    else                      charVal = charData.a;

    int atlasIdx = int(charVal * 255.0 + 0.5);
    if (atlasIdx >= 254) return color; // sentinel
    if (atlasIdx >= 45) return color;  // out of range

    // MSDF sample
    float glyph = sampleGlyph(float(atlasIdx), cellUV);

    vec3 textColor = vec3(tint_r, tint_g, tint_b);
    color = mix(color, textColor, glyph * text_opacity);

    return color;
}

// --- Main ---

void main() {
    // Keep all uniforms alive to satisfy validation
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 base = texture(sampler2D(inputImage, texSampler), uv);
    vec3 result = base.rgb;

    // 1. Composite wireframe overlay
    vec4 wireframe = texture(sampler2D(landmarks, texSampler), uv);
    vec3 tinted_wire = wireframe.rgb * vec3(tint_r, tint_g, tint_b);
    result = mix(result, tinted_wire, wireframe.a * overlay_opacity);

    // 2. Read face count from face_data (pixel 1, row 0)
    vec4 header = readFaceData(1, 0);
    int faceCount = int(header.r * 255.0 + 0.5);

    // 3. For each face: render brackets + dossier text
    for (int f = 0; f < MAX_FACES; f++) {
        if (f >= faceCount) break;

        vec4 bbox = readFaceData(0, f);
        result = renderBrackets(result, bbox, uv);
        result = renderDossierText(result, bbox, uv, f);
    }

    // 4. Scanline effect
    float scanline = 1.0 - scanline_intensity * (0.5 + 0.5 * sin(uv.y * RENDERSIZE.y * 3.14159));
    result *= scanline;

    fragColor = vec4(result, base.a);
}