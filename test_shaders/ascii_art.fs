/*{
    "DESCRIPTION": "ASCII Art - renders image as character-sized blocks",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "cell_size", "LABEL": "Cell Size", "TYPE": "float", "DEFAULT": 10.0, "MIN": 4.0, "MAX": 30.0},
        {"NAME": "char_set", "LABEL": "Style (0=Blocks 1=Lines 2=Dots)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "color_mode", "LABEL": "Color (0=Green 1=Source 2=White)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "bg_brightness", "LABEL": "BG Brightness", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 0.3}
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

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float cell_size;
    float char_set;
    float color_mode;
    float bg_brightness;
};

// Procedural "character" patterns based on luminance
float charPattern(vec2 cellUV, float lum, int style) {
    if (lum < 0.05) return 0.0;

    if (style == 0) {
        // Blocks: fill proportional to luminance
        float size = lum;
        vec2 d = abs(cellUV - 0.5);
        return step(d.x, size * 0.45) * step(d.y, size * 0.45);
    } else if (style == 1) {
        // Lines: horizontal/vertical/diagonal based on luminance
        if (lum < 0.25) {
            return step(abs(cellUV.y - 0.5), 0.05); // -
        } else if (lum < 0.5) {
            return step(abs(cellUV.x - 0.5), 0.05); // |
        } else if (lum < 0.75) {
            return step(abs(cellUV.x - 0.5), 0.05) + step(abs(cellUV.y - 0.5), 0.05); // +
        } else {
            float size = 0.4;
            vec2 d = abs(cellUV - 0.5);
            return step(d.x, size) * step(d.y, size); // filled
        }
    } else {
        // Dots: circle size based on luminance
        float r = lum * 0.45;
        return smoothstep(r, r - 0.03, length(cellUV - 0.5));
    }
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 cellCount = RENDERSIZE / cell_size;
    vec2 cell = floor(uv * cellCount);
    vec2 cellUV = fract(uv * cellCount);

    // Sample source at cell center
    vec2 sampleUV = (cell + 0.5) / cellCount;
    vec4 src = texture(sampler2D(inputImage, texSampler), sampleUV);
    float lum = dot(src.rgb, vec3(0.299, 0.587, 0.114));

    int style = int(floor(char_set + 0.5));
    float pattern = charPattern(cellUV, lum, style);

    int cm = int(floor(color_mode + 0.5));
    vec3 charColor;
    if (cm == 0) charColor = vec3(0.0, 1.0, 0.3); // Terminal green
    else if (cm == 1) charColor = src.rgb;
    else charColor = vec3(1.0);

    vec3 result = mix(vec3(bg_brightness), charColor, clamp(pattern, 0.0, 1.0));
    fragColor = vec4(result, 1.0);
}
