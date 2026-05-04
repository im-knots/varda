/*{
    "DESCRIPTION": "Tile - repeat/tile the image in a grid",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "tile_x", "LABEL": "Tiles X", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 16.0},
        {"NAME": "tile_y", "LABEL": "Tiles Y", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 16.0},
        {"NAME": "mirror", "LABEL": "Mirror", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "offset_x", "LABEL": "Offset X", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0},
        {"NAME": "offset_y", "LABEL": "Offset Y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0}
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
    float tile_x;
    float tile_y;
    float mirror;
    float offset_x;
    float offset_y;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 tiled = uv * vec2(tile_x, tile_y) + vec2(offset_x, offset_y);

    if (mirror > 0.5) {
        // Mirror on even tiles
        vec2 cell = floor(tiled);
        vec2 f = fract(tiled);
        if (mod(cell.x, 2.0) >= 1.0) f.x = 1.0 - f.x;
        if (mod(cell.y, 2.0) >= 1.0) f.y = 1.0 - f.y;
        tiled = f;
    } else {
        tiled = fract(tiled);
    }

    vec4 result = texture(sampler2D(inputImage, texSampler), tiled);
    fragColor = result;
}
