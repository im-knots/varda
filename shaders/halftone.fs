/*{
    "DESCRIPTION": "Halftone - print-style dot pattern",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "dot_size", "LABEL": "Dot Size", "TYPE": "float", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 30.0},
        {"NAME": "dot_angle", "LABEL": "Angle", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.571},
        {"NAME": "hardness", "LABEL": "Hardness", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "color_mode", "LABEL": "Mode (0=BW 1=Color 2=CMYK)", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Background"}
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

layout(set = 0, binding = 3) uniform UserParams {
    float dot_size;
    float dot_angle;
    float hardness;
    float color_mode;
    vec4 bg_color;
};

float halftoneLayer(vec2 coord, float intensity) {
    vec2 cell = floor(coord) + 0.5;
    vec2 f = fract(coord) - 0.5;
    float dist = length(f);
    float radius = sqrt(intensity) * 0.5;
    float edge = mix(0.05, 0.002, hardness);
    return smoothstep(radius + edge, radius - edge, dist);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec2 pixCoord = uv * RENDERSIZE / dot_size;

    // Rotate grid
    float ca = cos(dot_angle), sa = sin(dot_angle);
    vec2 rotCoord = vec2(pixCoord.x * ca - pixCoord.y * sa, pixCoord.x * sa + pixCoord.y * ca);

    int mode = int(floor(color_mode + 0.5));

    if (mode == 0) {
        // Black and white
        float lum = dot(src.rgb, vec3(0.2126, 0.7152, 0.0722));
        float dot = halftoneLayer(rotCoord, lum);
        vec3 col = mix(bg_color.rgb, vec3(0.0), dot);
        fragColor = vec4(col, src.a);
    } else if (mode == 1) {
        // Color: use luminance for size, keep color
        float lum = dot(src.rgb, vec3(0.2126, 0.7152, 0.0722));
        float dot = halftoneLayer(rotCoord, lum);
        vec3 col = mix(bg_color.rgb, src.rgb, dot);
        fragColor = vec4(col, src.a);
    } else {
        // CMYK simulation with angle offsets
        float c = 1.0 - src.r;
        float m = 1.0 - src.g;
        float y = 1.0 - src.b;
        float k = min(min(c, m), y);

        float a1 = dot_angle + 0.262; // 15 deg offset
        float a2 = dot_angle + 1.309; // 75 deg
        float a3 = dot_angle;
        float a4 = dot_angle + 0.785; // 45 deg

        vec2 rc = vec2(pixCoord.x * cos(a1) - pixCoord.y * sin(a1), pixCoord.x * sin(a1) + pixCoord.y * cos(a1));
        vec2 rm = vec2(pixCoord.x * cos(a2) - pixCoord.y * sin(a2), pixCoord.x * sin(a2) + pixCoord.y * cos(a2));
        vec2 ry = vec2(pixCoord.x * cos(a3) - pixCoord.y * sin(a3), pixCoord.x * sin(a3) + pixCoord.y * cos(a3));
        vec2 rk = vec2(pixCoord.x * cos(a4) - pixCoord.y * sin(a4), pixCoord.x * sin(a4) + pixCoord.y * cos(a4));

        float dC = halftoneLayer(rc, c);
        float dM = halftoneLayer(rm, m);
        float dY = halftoneLayer(ry, y);
        float dK = halftoneLayer(rk, k);

        vec3 col = vec3(1.0);
        col -= vec3(dC, 0.0, 0.0); // Cyan removes red
        col -= vec3(0.0, dM, 0.0); // Magenta removes green
        col -= vec3(0.0, 0.0, dY); // Yellow removes blue
        col -= vec3(dK);           // Key (black)
        col = clamp(col, 0.0, 1.0);

        fragColor = vec4(col, src.a);
    }
}
