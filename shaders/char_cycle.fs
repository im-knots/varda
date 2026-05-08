/*{
    "DESCRIPTION": "Character Cycle - cycles through glyphs from a selected script",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "char_set", "LABEL": "Script", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 14.0},
        {"NAME": "speed", "LABEL": "Speed", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.1, "MAX": 30.0},
        {"NAME": "grid_size", "LABEL": "Grid", "TYPE": "float", "DEFAULT": 1.0, "MIN": 1.0, "MAX": 16.0},
        {"NAME": "offset_amount", "LABEL": "Cell Offset", "TYPE": "float", "DEFAULT": 3.0, "MIN": 0.0, "MAX": 20.0},
        {"NAME": "speed_variation", "LABEL": "Speed Variation", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "fg_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Foreground"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
    ],
    "IMPORTED": {
        "arabic_font_atlas":          { "PATH": "character_atlases/arabic_font_atlas.png" },
        "ascii_font_atlas":           { "PATH": "character_atlases/ascii_font_atlas.png" },
        "binary_font_atlas":          { "PATH": "character_atlases/binary_font_atlas.png" },
        "chinese_font_atlas":         { "PATH": "character_atlases/chinese_font_atlas.png" },
        "cuneiform_font_atlas":       { "PATH": "character_atlases/cuneiform_font_atlas.png" },
        "devanagari_font_atlas":      { "PATH": "character_atlases/devanagari_font_atlas.png" },
        "ethiopic_font_atlas":        { "PATH": "character_atlases/ethiopic_font_atlas.png" },
        "hangul_font_atlas":          { "PATH": "character_atlases/hangul_font_atlas.png" },
        "hiero_font_atlas":           { "PATH": "character_atlases/hiero_font_atlas.png" },
        "katakana_font_atlas":        { "PATH": "character_atlases/katakana_font_atlas.png" },
        "linearb_font_atlas":         { "PATH": "character_atlases/linearb_font_atlas.png" },
        "phoenician_font_atlas":      { "PATH": "character_atlases/phoenician_font_atlas.png" },
        "sanskrit_font_atlas":        { "PATH": "character_atlases/sanskrit_font_atlas.png" },
        "secretlanguage_font_atlas":  { "PATH": "character_atlases/secretlanguage_font_atlas.png" },
        "witchy_font_atlas":          { "PATH": "character_atlases/witchy_font_atlas.png" }
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

// IMPORTED atlases — sorted alphabetically (bindings 2-16)
layout(set = 0, binding = 2)  uniform texture2D arabic_font_atlas;
layout(set = 0, binding = 3)  uniform texture2D ascii_font_atlas;
layout(set = 0, binding = 4)  uniform texture2D binary_font_atlas;
layout(set = 0, binding = 5)  uniform texture2D chinese_font_atlas;
layout(set = 0, binding = 6)  uniform texture2D cuneiform_font_atlas;
layout(set = 0, binding = 7)  uniform texture2D devanagari_font_atlas;
layout(set = 0, binding = 8)  uniform texture2D ethiopic_font_atlas;
layout(set = 0, binding = 9)  uniform texture2D hangul_font_atlas;
layout(set = 0, binding = 10) uniform texture2D hiero_font_atlas;
layout(set = 0, binding = 11) uniform texture2D katakana_font_atlas;
layout(set = 0, binding = 12) uniform texture2D linearb_font_atlas;
layout(set = 0, binding = 13) uniform texture2D phoenician_font_atlas;
layout(set = 0, binding = 14) uniform texture2D sanskrit_font_atlas;
layout(set = 0, binding = 15) uniform texture2D secretlanguage_font_atlas;
layout(set = 0, binding = 16) uniform texture2D witchy_font_atlas;

layout(set = 0, binding = 17) uniform UserParams {
    float char_set;
    float speed;
    float grid_size;
    float offset_amount;
    float speed_variation;
    vec4 fg_color;
    vec4 bg_color;
};

// Char counts per atlas (must match generated MSDF PNGs)
// 0=ASCII:26 1=Binary:2 2=Arabic:37 3=Chinese:128 4=Cuneiform:128
// 5=Devanagari:65 6=Ethiopic:128 7=SecretLanguage:29 8=Hangul:128
// 9=Hieroglyphs:128 10=Katakana:93 11=LinearB:88 12=Phoenician:22
// 13=Sanskrit:65 14=Witchy:123

const float ATLAS_COLS = 8.0;
const float MSDF_PX_RANGE = 4.0;

float median3(float r, float g, float b) {
    return max(min(r, g), min(max(r, g), b));
}

float getNumChars(int s) {
    if      (s == 0)  return 26.0;
    else if (s == 1)  return 2.0;
    else if (s == 2)  return 37.0;
    else if (s == 3)  return 128.0;
    else if (s == 4)  return 128.0;
    else if (s == 5)  return 65.0;
    else if (s == 6)  return 128.0;
    else if (s == 7)  return 29.0;
    else if (s == 8)  return 128.0;
    else if (s == 9)  return 128.0;
    else if (s == 10) return 93.0;
    else if (s == 11) return 88.0;
    else if (s == 12) return 22.0;
    else if (s == 13) return 65.0;
    else              return 123.0;
}

float sampleGlyph(texture2D atlas, float numChars, float charIdx, vec2 cellUV) {
    float col = mod(charIdx, ATLAS_COLS);
    float row = floor(charIdx / ATLAS_COLS);
    float numRows = ceil(numChars / ATLAS_COLS);

    // Clamp cellUV inward by half a texel to avoid sampling neighboring cells' MSDF data
    vec2 atlasSize = vec2(textureSize(sampler2D(atlas, texSampler), 0));
    vec2 cellSize = atlasSize / vec2(ATLAS_COLS, numRows);
    vec2 halfTexel = 0.5 / cellSize;
    vec2 clamped = clamp(cellUV, halfTexel, vec2(1.0) - halfTexel);

    float u = (col + clamped.x) / ATLAS_COLS;
    float v = (row + clamped.y) / numRows;

    // Sample returns linear (sRGB-decoded) values; undo gamma to recover raw MSDF data
    vec3 msdLinear = texture(sampler2D(atlas, texSampler), vec2(u, v)).rgb;
    vec3 msd = pow(msdLinear, vec3(1.0 / 2.2));
    float sd = median3(msd.r, msd.g, msd.b);

    vec2 unitRange = MSDF_PX_RANGE / atlasSize;
    vec2 screenTexSize = vec2(1.0) / fwidth(vec2(u, v));
    float screenPxRange = max(0.5 * dot(unitRange, screenTexSize), 1.0);

    return clamp((sd - 0.5) * screenPxRange + 0.5, 0.0, 1.0);
}

float sampleSet(int s, float idx, vec2 cellUV) {
    if      (s == 0)  return sampleGlyph(ascii_font_atlas,              26.0, idx, cellUV);
    else if (s == 1)  return sampleGlyph(binary_font_atlas,              2.0, idx, cellUV);
    else if (s == 2)  return sampleGlyph(arabic_font_atlas,             37.0, idx, cellUV);
    else if (s == 3)  return sampleGlyph(chinese_font_atlas,           128.0, idx, cellUV);
    else if (s == 4)  return sampleGlyph(cuneiform_font_atlas,         128.0, idx, cellUV);
    else if (s == 5)  return sampleGlyph(devanagari_font_atlas,         65.0, idx, cellUV);
    else if (s == 6)  return sampleGlyph(ethiopic_font_atlas,          128.0, idx, cellUV);
    else if (s == 7)  return sampleGlyph(secretlanguage_font_atlas,     29.0, idx, cellUV);
    else if (s == 8)  return sampleGlyph(hangul_font_atlas,            128.0, idx, cellUV);
    else if (s == 9)  return sampleGlyph(hiero_font_atlas,             128.0, idx, cellUV);
    else if (s == 10) return sampleGlyph(katakana_font_atlas,           93.0, idx, cellUV);
    else if (s == 11) return sampleGlyph(linearb_font_atlas,            88.0, idx, cellUV);
    else if (s == 12) return sampleGlyph(phoenician_font_atlas,         22.0, idx, cellUV);
    else if (s == 13) return sampleGlyph(sanskrit_font_atlas,           65.0, idx, cellUV);
    else              return sampleGlyph(witchy_font_atlas,            123.0, idx, cellUV);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    int s = int(floor(char_set + 0.5));
    float numChars = getNumChars(s);
    int g = int(floor(grid_size + 0.5));

    // Aspect-correct UV
    float aspect = RENDERSIZE.x / RENDERSIZE.y;

    if (g <= 1) {
        // Single big character centered
        float charIdx = floor(mod(PHASE_TIME_0, numChars));

        // Scale UV to fill ~80% of screen, aspect-corrected
        vec2 charUV = uv;
        charUV.x = (uv.x - 0.5) * aspect + 0.5;
        // Center and scale to 80%
        charUV = (charUV - 0.5) / 0.8 + 0.5;

        if (charUV.x < 0.0 || charUV.x > 1.0 || charUV.y < 0.0 || charUV.y > 1.0) {
            fragColor = bg_color;
            return;
        }

        float glyph = sampleSet(s, charIdx, charUV);
        vec3 result = mix(bg_color.rgb, fg_color.rgb, clamp(glyph, 0.0, 1.0));
        fragColor = vec4(result, 1.0);
    } else {
        // Grid mode: g x g grid with offset cycling
        float gf = float(g);

        // Compute grid cell
        vec2 cellCoord = floor(uv * gf);
        vec2 cellUV = fract(uv * gf);

        // Aspect correction within cell
        cellUV.x = (cellUV.x - 0.5) * min(aspect, 1.0) / max(aspect, 1.0) * aspect + 0.5;

        // Per-cell offset and speed variation
        float cellId = cellCoord.y * gf + cellCoord.x;
        float cellOffset = cellId * offset_amount;

        // Hash cellId for pseudo-random per-cell speed multiplier
        float h = fract(sin(cellId * 127.1 + 311.7) * 43758.5453);
        float cellSpeed = speed * (1.0 + (h * 2.0 - 1.0) * speed_variation);

        float charIdx = floor(mod(PHASE_TIME_0 * (1.0 + (h * 2.0 - 1.0) * speed_variation) + cellOffset, numChars));

        // Clamp UV to avoid sampling neighboring cells
        if (cellUV.x < 0.05 || cellUV.x > 0.95 || cellUV.y < 0.05 || cellUV.y > 0.95) {
            fragColor = bg_color;
            return;
        }
        // Remap 0.05..0.95 to 0..1
        vec2 glyphUV = (cellUV - 0.05) / 0.9;

        float glyph = sampleSet(s, charIdx, glyphUV);
        vec3 result = mix(bg_color.rgb, fg_color.rgb, clamp(glyph, 0.0, 1.0));
        fragColor = vec4(result, 1.0);
    }
}
