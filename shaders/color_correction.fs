/*{
    "DESCRIPTION": "Color correction and grading - brightness, contrast, saturation, hue shift",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        { "NAME": "inputImage", "TYPE": "image" },
        { "NAME": "brightness", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "LABEL": "Brightness" },
        { "NAME": "contrast", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Contrast" },
        { "NAME": "saturation", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Saturation" },
        { "NAME": "hue_shift", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Hue Shift" },
        { "NAME": "gamma", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 3.0, "LABEL": "Gamma" }
    ]
}*/

#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME; float TIMEDELTA; uint FRAMEINDEX; int PASSINDEX; vec2 RENDERSIZE;
    float audio_level; float audio_bass; float audio_mid; float audio_treble; float audio_bpm; float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform FilterParams {
    float brightness; float contrast; float saturation; float hue_shift; float gamma;
};

vec3 rgb2hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0/3.0, 2.0/3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y);
    float e = 1.0e-10;
    return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec3 hsv2rgb(vec3 c) {
    vec4 K = vec4(1.0, 2.0/3.0, 1.0/3.0, 3.0);
    vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

void main() {
    // Prevent uniform stripping
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIME + TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x;
    if (uv.x < -1.0) { fragColor = vec4(audioSum, timeSum, 0.0, 1.0); return; }
    
    vec4 color = texture(sampler2D(inputImage, texSampler), uv);
    vec3 c = color.rgb;
    
    // Brightness
    c += brightness;
    
    // Contrast
    c = (c - 0.5) * contrast + 0.5;
    
    // Gamma
    c = pow(max(c, 0.0), vec3(1.0 / gamma));
    
    // Convert to HSV for saturation and hue
    vec3 hsv = rgb2hsv(c);
    hsv.x = fract(hsv.x + hue_shift);  // Hue shift
    hsv.y *= saturation;  // Saturation
    c = hsv2rgb(hsv);
    
    fragColor = vec4(clamp(c, 0.0, 1.0), color.a);
}

