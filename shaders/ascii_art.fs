/*{
    "DESCRIPTION": "ASCII Art - renders image using real font glyph atlases",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "cell_size", "LABEL": "Cell Size", "TYPE": "float", "DEFAULT": 10.0, "MIN": 4.0, "MAX": 30.0},
        {"NAME": "char_set", "LABEL": "Script", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 14.0},
        {"NAME": "color_mode", "LABEL": "Color (0=Green 1=Source 2=White)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "bg_brightness", "LABEL": "BG Brightness", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.3},
        {"NAME": "invert", "LABEL": "Invert", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0}
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
    }
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

// IMPORTED atlases — sorted alphabetically (15 textures, bindings 3-17)
layout(set = 0, binding = 3)  uniform texture2D arabic_font_atlas;
layout(set = 0, binding = 4)  uniform texture2D ascii_font_atlas;
layout(set = 0, binding = 5)  uniform texture2D binary_font_atlas;
layout(set = 0, binding = 6)  uniform texture2D chinese_font_atlas;
layout(set = 0, binding = 7)  uniform texture2D cuneiform_font_atlas;
layout(set = 0, binding = 8)  uniform texture2D devanagari_font_atlas;
layout(set = 0, binding = 9)  uniform texture2D ethiopic_font_atlas;
layout(set = 0, binding = 10) uniform texture2D hangul_font_atlas;
layout(set = 0, binding = 11) uniform texture2D hiero_font_atlas;
layout(set = 0, binding = 12) uniform texture2D katakana_font_atlas;
layout(set = 0, binding = 13) uniform texture2D linearb_font_atlas;
layout(set = 0, binding = 14) uniform texture2D phoenician_font_atlas;
layout(set = 0, binding = 15) uniform texture2D sanskrit_font_atlas;
layout(set = 0, binding = 16) uniform texture2D secretlanguage_font_atlas;
layout(set = 0, binding = 17) uniform texture2D witchy_font_atlas;

layout(set = 0, binding = 18) uniform UserParams {
    float cell_size;
    float char_set;
    float color_mode;
    float bg_brightness;
    float invert;
};

const float ATLAS_COLS = 8.0;
const float MSDF_PX_RANGE = 4.0;

float median3(float r, float g, float b) {
    return max(min(r, g), min(max(r, g), b));
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

float lumToIdx(float lum, float n) {
    return clamp(floor(lum * (n - 1.0) + 0.5), 0.0, n - 1.0);
}

//  0=ASCII:26  1=Binary:2  2=Arabic:37  3=Chinese:128  4=Cuneiform:128
//  5=Devanagari:65  6=Ethiopic:128  7=SecretLanguage:29  8=Hangul:128
//  9=Hieroglyphs:128  10=Katakana:93  11=LinearB:88  12=Phoenician:22
// 13=Sanskrit:65  14=Witchy:123

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 cellCount = RENDERSIZE / cell_size;
    vec2 cell = floor(uv * cellCount);
    vec2 cellUV = fract(uv * cellCount);

    vec2 sampleUV = (cell + 0.5) / cellCount;
    vec4 src = texture(sampler2D(inputImage, texSampler), sampleUV);
    float lum = dot(src.rgb, vec3(0.2126, 0.7152, 0.0722));

    float inv = step(0.5, invert);
    lum = mix(lum, 1.0 - lum, inv);

    int s = int(floor(char_set + 0.5));
    float glyph = 0.0;

    if      (s == 0)  { glyph = sampleGlyph(ascii_font_atlas,              26.0, lumToIdx(lum,  26.0), cellUV); }
    else if (s == 1)  { glyph = sampleGlyph(binary_font_atlas,              2.0, step(0.5, lum),      cellUV); }
    else if (s == 2)  { glyph = sampleGlyph(arabic_font_atlas,             37.0, lumToIdx(lum,  37.0), cellUV); }
    else if (s == 3)  { glyph = sampleGlyph(chinese_font_atlas,           128.0, lumToIdx(lum, 128.0), cellUV); }
    else if (s == 4)  { glyph = sampleGlyph(cuneiform_font_atlas,         128.0, lumToIdx(lum, 128.0), cellUV); }
    else if (s == 5)  { glyph = sampleGlyph(devanagari_font_atlas,         65.0, lumToIdx(lum,  65.0), cellUV); }
    else if (s == 6)  { glyph = sampleGlyph(ethiopic_font_atlas,          128.0, lumToIdx(lum, 128.0), cellUV); }
    else if (s == 7)  { glyph = sampleGlyph(secretlanguage_font_atlas,     29.0, lumToIdx(lum,  29.0), cellUV); }
    else if (s == 8)  { glyph = sampleGlyph(hangul_font_atlas,            128.0, lumToIdx(lum, 128.0), cellUV); }
    else if (s == 9)  { glyph = sampleGlyph(hiero_font_atlas,             128.0, lumToIdx(lum, 128.0), cellUV); }
    else if (s == 10) { glyph = sampleGlyph(katakana_font_atlas,           93.0, lumToIdx(lum,  93.0), cellUV); }
    else if (s == 11) { glyph = sampleGlyph(linearb_font_atlas,            88.0, lumToIdx(lum,  88.0), cellUV); }
    else if (s == 12) { glyph = sampleGlyph(phoenician_font_atlas,         22.0, lumToIdx(lum,  22.0), cellUV); }
    else if (s == 13) { glyph = sampleGlyph(sanskrit_font_atlas,           65.0, lumToIdx(lum,  65.0), cellUV); }
    else              { glyph = sampleGlyph(witchy_font_atlas,            123.0, lumToIdx(lum, 123.0), cellUV); }

    int cm = int(floor(color_mode + 0.5));
    vec3 charColor;
    if (cm == 0) charColor = vec3(0.0, 1.0, 0.3);
    else if (cm == 1) charColor = src.rgb;
    else charColor = vec3(1.0);

    vec3 result = mix(vec3(bg_brightness), charColor, clamp(glyph, 0.0, 1.0));
    fragColor = vec4(result, 1.0);
}