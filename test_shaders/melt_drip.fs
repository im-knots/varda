/*{
    "DESCRIPTION": "Melting/dripping distortion effect - makes the image look like it's melting and dripping down",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Filter", "Distortion"],
    "INPUTS": [
        {
            "NAME": "inputImage",
            "TYPE": "image"
        },
        {
            "NAME": "melt_amount",
            "TYPE": "float",
            "DEFAULT": 0.15,
            "MIN": 0.0,
            "MAX": 0.5
        },
        {
            "NAME": "drip_speed",
            "TYPE": "float",
            "DEFAULT": 1.0,
            "MIN": 0.0,
            "MAX": 5.0
        },
        {
            "NAME": "wave_frequency",
            "TYPE": "float",
            "DEFAULT": 8.0,
            "MIN": 1.0,
            "MAX": 20.0
        },
        {
            "NAME": "horizontal_wobble",
            "TYPE": "float",
            "DEFAULT": 0.03,
            "MIN": 0.0,
            "MAX": 0.2
        },
        {
            "NAME": "drip_intensity",
            "TYPE": "float",
            "DEFAULT": 0.5,
            "MIN": 0.0,
            "MAX": 1.0
        }
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
    // Audio
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

// Uniform inputs
layout(set = 0, binding = 3) uniform FilterParams {
    float melt_amount;
    float drip_speed;
    float wave_frequency;
    float horizontal_wobble;
    float drip_intensity;
};

// Noise function for organic dripping
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);

    float a = hash(i);
    float b = hash(i + vec2(1.0, 0.0));
    float c = hash(i + vec2(0.0, 1.0));
    float d = hash(i + vec2(1.0, 1.0));

    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

void main() {
    // Use all ISF uniforms to prevent SPIRV from stripping them
    // Conditional based on UV - compiler cannot prove it's always false
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0 && uv.y < -1.0) {
        fragColor = vec4(audioSum, timeSum, 0.0, 1.0);
        return;
    }

    vec2 distorted_uv = uv;

    // Create dripping waves that move downward
    float time = TIME * drip_speed;

    // Multiple layers of sine waves for organic dripping
    float wave1 = sin(uv.x * wave_frequency + time) * melt_amount;
    float wave2 = sin(uv.x * wave_frequency * 1.7 - time * 0.7) * melt_amount * 0.5;
    float wave3 = sin(uv.x * wave_frequency * 2.3 + time * 1.3) * melt_amount * 0.25;

    // Drips get stronger toward bottom (gravity effect)
    float gravity = pow(1.0 - uv.y, 2.0) * drip_intensity;

    // Add noise for organic feel
    float drip_noise = noise(vec2(uv.x * 10.0, time * 0.5)) * 0.5 + 0.5;

    // Vertical displacement (melting down)
    distorted_uv.y -= (wave1 + wave2 + wave3) * gravity * drip_noise;

    // Horizontal wobble
    float wobble = sin(uv.y * wave_frequency * 2.0 + time * 2.0) * horizontal_wobble;
    wobble += sin(uv.y * wave_frequency * 3.5 - time * 1.5) * horizontal_wobble * 0.5;
    distorted_uv.x += wobble * (1.0 - uv.y);  // More wobble at top

    // Add some vertical stretching for melty feel
    float stretch = 1.0 + melt_amount * 0.5 * sin(uv.x * wave_frequency + time);
    distorted_uv.y = 0.5 + (distorted_uv.y - 0.5) * stretch;

    // Clamp UVs
    distorted_uv = clamp(distorted_uv, 0.0, 1.0);

    // Sample the distorted image
    vec4 color = texture(sampler2D(inputImage, texSampler), distorted_uv);

    // Add slight color bleeding for extra trippy effect
    float bleed = melt_amount * 0.3;
    color.r = texture(sampler2D(inputImage, texSampler), distorted_uv + vec2(bleed * 0.01, 0.0)).r;
    color.b = texture(sampler2D(inputImage, texSampler), distorted_uv - vec2(bleed * 0.01, 0.0)).b;

    fragColor = color;
    fragColor.a = 1.0;
}

