/*{
    "DESCRIPTION": "Psychedelic feedback trails - creates visual tracers like seeing trails on psychedelics",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Filter", "Feedback"],
    "INPUTS": [
        {
            "NAME": "inputImage",
            "TYPE": "image"
        },
        {
            "NAME": "feedback_amount",
            "TYPE": "float",
            "DEFAULT": 0.85,
            "MIN": 0.0,
            "MAX": 0.99
        },
        {
            "NAME": "color_shift",
            "TYPE": "float",
            "DEFAULT": 0.02,
            "MIN": 0.0,
            "MAX": 0.1
        },
        {
            "NAME": "zoom_amount",
            "TYPE": "float",
            "DEFAULT": 0.005,
            "MIN": -0.02,
            "MAX": 0.02
        },
        {
            "NAME": "rotation_speed",
            "TYPE": "float",
            "DEFAULT": 0.01,
            "MIN": -0.05,
            "MAX": 0.05
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
layout(set = 0, binding = 3) uniform texture2D feedbackBuffer;

// Uniform inputs
layout(set = 0, binding = 4) uniform FilterParams {
    float feedback_amount;
    float color_shift;
    float zoom_amount;
    float rotation_speed;
};

vec2 rotate2D(vec2 p, float angle) {
    float c = cos(angle);
    float s = sin(angle);
    return vec2(p.x * c - p.y * s, p.x * s + p.y * c);
}

void main() {
    // Use all ISF uniforms in actual output to prevent stripping
    float uniformKeeper = (audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase) * 0.000001
                        + (TIMEDELTA + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w) * 0.000001;

    vec2 center = vec2(0.5);
    vec2 centered_uv = uv - center;

    if (PASSINDEX == 0) {
        // First pass: blend input with feedback buffer

        // Apply zoom and rotation to feedback UV
        vec2 feedback_uv = centered_uv;
        feedback_uv *= (1.0 + zoom_amount);  // Subtle zoom
        feedback_uv = rotate2D(feedback_uv, rotation_speed);  // Subtle rotation
        feedback_uv += center;

        // Clamp to valid range
        feedback_uv = clamp(feedback_uv, 0.001, 0.999);

        // Sample current input
        vec4 current = texture(sampler2D(inputImage, texSampler), uv);

        // Sample feedback with slight color shift for trippy rainbow trails
        vec4 feedback;
        feedback.r = texture(sampler2D(feedbackBuffer, texSampler), feedback_uv + vec2(color_shift, 0.0)).r;
        feedback.g = texture(sampler2D(feedbackBuffer, texSampler), feedback_uv).g;
        feedback.b = texture(sampler2D(feedbackBuffer, texSampler), feedback_uv - vec2(color_shift, 0.0)).b;
        feedback.a = 1.0;

        // Blend: new content shows through, old content fades
        fragColor = mix(current, feedback, feedback_amount);
        fragColor.rgb += uniformKeeper;
        fragColor.a = 1.0;
    } else {
        // Final pass: output the result
        fragColor = texture(sampler2D(feedbackBuffer, texSampler), uv);
        fragColor.rgb += uniformKeeper;
    }
}

