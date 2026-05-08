/*{
    "DESCRIPTION": "Mirror / flip effect with various modes",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distortion"],
    "INPUTS": [
        {
            "NAME": "inputImage",
            "TYPE": "image"
        },
        {
            "NAME": "mode",
            "LABEL": "Mirror Mode",
            "TYPE": "long",
            "DEFAULT": 0,
            "VALUES": [0, 1, 2, 3, 4],
            "LABELS": ["Horizontal", "Vertical", "Quad", "Diagonal", "Radial"]
        },
        {
            "NAME": "flip_side",
            "LABEL": "Flip Side",
            "TYPE": "bool",
            "DEFAULT": false
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
    int mode;
    float flip_side;
};

void main() {
    vec2 newUV = uv;
    
    if (mode == 0) {
        // Horizontal mirror
        if (flip_side > 0.5) {
            newUV.x = newUV.x < 0.5 ? 1.0 - newUV.x : newUV.x;
        } else {
            newUV.x = newUV.x > 0.5 ? 1.0 - newUV.x : newUV.x;
        }
    } else if (mode == 1) {
        // Vertical mirror
        if (flip_side > 0.5) {
            newUV.y = newUV.y < 0.5 ? 1.0 - newUV.y : newUV.y;
        } else {
            newUV.y = newUV.y > 0.5 ? 1.0 - newUV.y : newUV.y;
        }
    } else if (mode == 2) {
        // Quad mirror
        newUV = abs(newUV - 0.5) + 0.5;
        if (flip_side > 0.5) newUV = 1.0 - newUV;
    } else if (mode == 3) {
        // Diagonal mirror
        if (newUV.x + newUV.y > 1.0) {
            newUV = 1.0 - newUV.yx;
        }
        if (flip_side > 0.5) newUV = newUV.yx;
    } else if (mode == 4) {
        // Radial mirror
        vec2 centered = newUV - 0.5;
        float angle = atan(centered.y, centered.x);
        float r = length(centered);
        angle = abs(mod(angle + 3.14159, 3.14159 * 0.5) - 3.14159 * 0.25);
        if (flip_side > 0.5) angle = 3.14159 * 0.25 - angle;
        newUV = vec2(cos(angle), sin(angle)) * r + 0.5;
    }
    
    fragColor = texture(sampler2D(inputImage, texSampler), newUV);
}

