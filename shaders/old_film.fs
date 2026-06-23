/*{
    "DESCRIPTION": "Old Film - vintage film projector look with scratches and flicker",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "scratch_amount", "LABEL": "Scratches", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "dust_amount", "LABEL": "Dust/Spots", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "flicker_amount", "LABEL": "Flicker", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "desaturate", "LABEL": "Desaturate", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "vignette", "LABEL": "Vignette", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "sepia_tone", "LABEL": "Sepia", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0}
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
    float scratch_amount;
    float dust_amount;
    float flicker_amount;
    float desaturate;
    float vignette;
    float sepia_tone;
};

float hash(float n) { return fract(sin(n) * 43758.5453); }
float hash2(vec2 p) { return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453); }

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec3 col = src.rgb;
    float frame = float(FRAMEINDEX);

    // Desaturate
    float lum = dot(col, vec3(0.2126, 0.7152, 0.0722));
    col = mix(col, vec3(lum), desaturate);

    // Sepia tone
    vec3 sepia = vec3(lum * 1.2, lum * 1.0, lum * 0.7);
    col = mix(col, sepia, sepia_tone);

    // Flicker (brightness variation per frame)
    float flicker = 1.0 + (hash(frame * 1.7) - 0.5) * flicker_amount;
    col *= flicker;

    // Vertical scratches
    float scratchX = hash(floor(frame * 0.5) * 7.3);
    float scratch = smoothstep(0.001, 0.0, abs(uv.x - scratchX) - 0.001);
    scratch *= hash(floor(frame * 0.5)) * scratch_amount;
    col += scratch * 0.8;

    // Random second scratch
    float scratchX2 = hash(floor(frame * 0.3) * 13.7);
    float scratch2 = smoothstep(0.001, 0.0, abs(uv.x - scratchX2) - 0.0005);
    scratch2 *= hash(floor(frame * 0.3) + 5.0) * scratch_amount * 0.5;
    col += scratch2 * 0.5;

    // Dust spots
    float dustHash = hash2(floor(uv * 30.0) + frame * 0.1);
    float dust = step(1.0 - dust_amount * 0.02, dustHash);
    col = mix(col, vec3(0.0), dust * 0.7);

    // Vignette
    vec2 vig = uv * (1.0 - uv);
    float vigFactor = clamp(pow(vig.x * vig.y * 15.0, vignette * 0.8), 0.0, 1.0);
    col *= vigFactor;

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, src.a);
}
