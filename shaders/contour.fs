/*{
    "DESCRIPTION": "Contour - topographic lines that trace levels of brightness",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "count", "LABEL": "Lines", "TYPE": "float", "DEFAULT": 12.0, "MIN": 2.0, "MAX": 64.0},
        {"NAME": "line_width", "LABEL": "Line Width", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0},
        {"NAME": "flow_speed", "LABEL": "Flow Speed", "TYPE": "float", "DEFAULT": 0.2, "MIN": -2.0, "MAX": 2.0},
        {"NAME": "smoothing", "LABEL": "Smoothing", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "line_color", "LABEL": "Line Color", "TYPE": "color", "DEFAULT": [0.9, 0.95, 1.0, 1.0]},
        {"NAME": "use_source_color", "LABEL": "Source Color", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "fill", "LABEL": "Fill", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "flow_speed", "MULTIPLY_BY": "count", "INDEX": 0, "SCALE": 1.0}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float count;
    float line_width;
    float flow_speed;
    float smoothing;
    vec4 line_color;
    float use_source_color;
    float fill;
    float amount;
};

// Brightness on the sRGB curve, so lines are spaced evenly to the eye. Above
// 1.0 it continues linearly.
float level(vec2 p) {
    vec3 c = max(texture(sampler2D(inputImage, texSampler), p).rgb, 0.0);
    float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    return l <= 1.0 ? (l <= 0.0031308 ? l * 12.92 : 1.055 * pow(l, 1.0 / 2.4) - 0.055)
                    : 1.0 + (l - 1.0) * (1.055 / 2.4);
}

void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec2 r = smoothing * 4.0 / RENDERSIZE;
    float l = level(uv);
    l = mix(l, (l + level(uv + vec2(r.x, 0.0)) + level(uv - vec2(r.x, 0.0))
                  + level(uv + vec2(0.0, r.y)) + level(uv - vec2(0.0, r.y))) * 0.2, step(1e-4, smoothing));

    float v = l * count + PHASE_TIME_0;
    float fw = max(fwidth(v), 1e-4);
    float d = abs(fract(v + 0.5) - 0.5) / fw;
    float line = 1.0 - smoothstep(0.5 * line_width - 0.5, 0.5 * line_width + 0.5, d);

    vec3 ink = mix(line_color.rgb, src.rgb, use_source_color);
    vec3 col = mix(src.rgb * fill, ink, line);
    fragColor = vec4(max(mix(src.rgb, col, amount), 0.0), src.a);
}
