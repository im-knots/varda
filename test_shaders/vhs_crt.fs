/*{
    "DESCRIPTION": "VHS/CRT - retro video distortion with tracking errors",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize", "Glitch"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "distortion", "LABEL": "Distortion", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "noise_amount", "LABEL": "Noise", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "color_bleed", "LABEL": "Color Bleed", "TYPE": "float", "DEFAULT": 0.005, "MIN": 0.0, "MAX": 0.02},
        {"NAME": "scanline_strength", "LABEL": "Scanlines", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 0.8},
        {"NAME": "tracking_error", "LABEL": "Tracking Error", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "vignette_amount", "LABEL": "Vignette", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0}
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
    float distortion;
    float noise_amount;
    float color_bleed;
    float scanline_strength;
    float tracking_error;
    float vignette_amount;
};

float hash(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv;
    float t = TIME;

    // Tracking error: horizontal offset that varies with time
    float trackingLine = smoothstep(0.0, 0.02, abs(fract(t * 0.3) - p.y)) *
                         smoothstep(0.0, 0.02, abs(fract(t * 0.3 + 0.05) - p.y));
    float trackShift = (1.0 - trackingLine) * tracking_error * 0.05;
    p.x += trackShift;

    // Wavy horizontal distortion
    p.x += sin(p.y * 50.0 + t * 3.0) * distortion * 0.003;
    p.x += sin(p.y * 130.0 + t * 7.0) * distortion * 0.001;

    // Color bleed / chromatic separation (VHS chroma shift)
    float r = texture(sampler2D(inputImage, texSampler), p + vec2(color_bleed, 0.0)).r;
    float g = texture(sampler2D(inputImage, texSampler), p).g;
    float b = texture(sampler2D(inputImage, texSampler), p - vec2(color_bleed, 0.0)).b;
    vec3 col = vec3(r, g, b);

    // Static noise
    float noise = hash(uv * RENDERSIZE + float(FRAMEINDEX) * 1.7) * 2.0 - 1.0;
    col += noise * noise_amount;

    // Scanlines
    float scanline = sin(uv.y * RENDERSIZE.y * 3.14159) * 0.5 + 0.5;
    col *= 1.0 - scanline_strength * (1.0 - scanline);

    // CRT vignette (rounded rectangle falloff)
    vec2 vig = uv * (1.0 - uv);
    float vigAmount = vig.x * vig.y * 15.0;
    vigAmount = clamp(pow(vigAmount, vignette_amount * 0.5), 0.0, 1.0);
    col *= vigAmount;

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
