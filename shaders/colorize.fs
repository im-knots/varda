/*{
    "DESCRIPTION": "Colorize - maps luminance to a color palette",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "palette", "LABEL": "Palette (0=Custom 1=Heat 2=Cool 3=Rainbow 4=Neon)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "intensity", "LABEL": "Intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "color_a", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Dark Color"},
        {"NAME": "color_b", "TYPE": "color", "DEFAULT": [1.0, 0.0, 0.5, 1.0], "LABEL": "Mid Color"},
        {"NAME": "color_c", "TYPE": "color", "DEFAULT": [1.0, 1.0, 0.0, 1.0], "LABEL": "Bright Color"}
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
    float palette;
    float intensity;
    vec4 color_a;
    vec4 color_b;
    vec4 color_c;
};

vec3 paletteColor(float t, int pal) {
    if (pal == 1) {
        // Heat: black → red → yellow → white
        vec3 a = vec3(0.0, 0.0, 0.0);
        vec3 b = vec3(1.0, 0.0, 0.0);
        vec3 c = vec3(1.0, 1.0, 0.0);
        vec3 d = vec3(1.0, 1.0, 1.0);
        if (t < 0.33) return mix(a, b, t * 3.0);
        if (t < 0.66) return mix(b, c, (t - 0.33) * 3.0);
        return mix(c, d, (t - 0.66) * 3.0);
    } else if (pal == 2) {
        // Cool: dark blue → cyan → white
        return mix(mix(vec3(0.0, 0.0, 0.3), vec3(0.0, 0.7, 1.0), t * 2.0),
                   vec3(1.0), smoothstep(0.5, 1.0, t));
    } else if (pal == 3) {
        // Rainbow
        return 0.5 + 0.5 * cos(6.283 * (t + vec3(0.0, 0.33, 0.67)));
    } else if (pal == 4) {
        // Neon: dark → magenta → cyan → white
        vec3 a = vec3(0.05, 0.0, 0.1);
        vec3 b = vec3(1.0, 0.0, 0.8);
        vec3 c = vec3(0.0, 1.0, 1.0);
        if (t < 0.5) return mix(a, b, t * 2.0);
        return mix(b, c, (t - 0.5) * 2.0);
    }
    // Custom: use user colors
    if (t < 0.5) return mix(color_a.rgb, color_b.rgb, t * 2.0);
    return mix(color_b.rgb, color_c.rgb, (t - 0.5) * 2.0);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    float lum = dot(src.rgb, vec3(0.299, 0.587, 0.114));

    int pal = int(floor(palette + 0.5));
    vec3 mapped = paletteColor(lum, pal);

    vec3 result = mix(src.rgb, mapped, intensity);
    fragColor = vec4(result, src.a);
}
