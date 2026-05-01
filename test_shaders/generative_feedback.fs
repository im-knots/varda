/*{
    "DESCRIPTION": "Generative feedback - creates evolving patterns using persistent buffer",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {
            "NAME": "decay",
            "TYPE": "float",
            "DEFAULT": 0.97,
            "MIN": 0.8,
            "MAX": 0.999,
            "LABEL": "Trail Decay"
        },
        {
            "NAME": "speed",
            "TYPE": "float",
            "DEFAULT": 1.0,
            "MIN": 0.0,
            "MAX": 5.0,
            "LABEL": "Animation Speed"
        },
        {
            "NAME": "spawn_rate",
            "TYPE": "float",
            "DEFAULT": 0.05,
            "MIN": 0.0,
            "MAX": 0.5,
            "LABEL": "Spawn Rate"
        },
        {
            "NAME": "color_intensity",
            "TYPE": "float",
            "DEFAULT": 0.3,
            "MIN": 0.0,
            "MAX": 1.0,
            "LABEL": "Color Intensity"
        },
        {
            "NAME": "blur_amount",
            "TYPE": "float",
            "DEFAULT": 0.5,
            "MIN": 0.0,
            "MAX": 1.0,
            "LABEL": "Blur Amount"
        }
    ],
    "PASSES": [
        {
            "TARGET": "feedbackBuffer",
            "PERSISTENT": true
        }
    ]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(std140, set = 0, binding = 0) uniform ISFUniforms {
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

// Pass buffer texture (from PASSES array)
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D feedbackBuffer;

// User parameters - note: multipass shaders have user params at binding 3
layout(std140, set = 0, binding = 3) uniform UserParams {
    float decay;
    float speed;
    float spawn_rate;
    float color_intensity;
    float blur_amount;
};

// Hash function for pseudo-random
float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

// Bilinear blur sampling for smooth feedback
vec4 sampleSmooth(vec2 coord) {
    vec2 texel = 1.0 / RENDERSIZE;
    vec4 center = texture(sampler2D(feedbackBuffer, texSampler), coord);

    // Skip blur if blur_amount is 0
    if (blur_amount < 0.01) {
        return center;
    }

    // Sample neighboring pixels for blur
    vec4 left   = texture(sampler2D(feedbackBuffer, texSampler), coord + vec2(-texel.x, 0.0));
    vec4 right  = texture(sampler2D(feedbackBuffer, texSampler), coord + vec2( texel.x, 0.0));
    vec4 up     = texture(sampler2D(feedbackBuffer, texSampler), coord + vec2(0.0,  texel.y));
    vec4 down   = texture(sampler2D(feedbackBuffer, texSampler), coord + vec2(0.0, -texel.y));

    // Weighted average (center has higher weight)
    vec4 neighbors = (left + right + up + down) * 0.25;
    return mix(center, neighbors, blur_amount * 0.5);
}

void main() {
    // Use all ISF uniforms in the actual output to prevent stripping
    // Add them with epsilon values that don't visibly affect output
    float uniformKeeper = (audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase) * 0.000001
                        + (TIMEDELTA + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w) * 0.000001;

    if (PASSINDEX == 0) {
        // Pass 0: Update the feedback buffer
        // Read previous frame with smooth sampling
        vec4 prev = sampleSmooth(uv);

        // Decay the previous frame (creates trail effect)
        vec3 decayed = prev.rgb * decay;

        // Add new plasma pattern on top
        vec2 p = uv * 2.0 - 1.0;
        p.x *= RENDERSIZE.x / RENDERSIZE.y;

        float t = TIME * speed;
        float v = 0.0;
        v += sin(p.x * 4.0 + t * 1.2);
        v += sin(p.y * 4.0 + t * 0.9);
        v += sin(length(p) * 6.0 - t * 2.0);
        v = v / 3.0;

        // Spawn new color particles based on pattern
        vec3 newColor = vec3(0.0);
        float spawnThreshold = 1.0 - spawn_rate;
        float spawn = step(spawnThreshold, fract(sin(t * 10.0 + hash(uv * 100.0) * 50.0) * 0.5 + 0.5));
        if (spawn > 0.5) {
            newColor.r = sin(v * 3.14159 + t) * 0.5 + 0.5;
            newColor.g = sin(v * 3.14159 + t + 2.094) * 0.5 + 0.5;
            newColor.b = sin(v * 3.14159 + t + 4.188) * 0.5 + 0.5;
            newColor *= color_intensity;
        }

        vec3 outRgb = decayed + newColor + uniformKeeper;
        float outA = clamp(max(max(outRgb.r, outRgb.g), outRgb.b), 0.0, 1.0);
        fragColor = vec4(outRgb, outA);
    } else {
        // Pass 1 (final): Output the feedback buffer with smooth sampling
        vec4 feedback = sampleSmooth(uv);

        // Add some glow effect
        vec3 color = feedback.rgb;
        color = pow(color, vec3(0.9)); // Slight gamma boost

        vec3 outRgb = color + uniformKeeper;
        float outA = clamp(max(max(outRgb.r, outRgb.g), outRgb.b), 0.0, 1.0);
        fragColor = vec4(outRgb, outA);
    }
}

