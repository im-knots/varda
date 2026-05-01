/*{
    "DESCRIPTION": "Kaleidoscope mirror effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distortion"],
    "INPUTS": [
        {
            "NAME": "inputImage",
            "TYPE": "image"
        },
        {
            "NAME": "segments",
            "LABEL": "Segments",
            "TYPE": "float",
            "DEFAULT": 6.0,
            "MIN": 2.0,
            "MAX": 16.0
        },
        {
            "NAME": "rotation",
            "LABEL": "Rotation",
            "TYPE": "float",
            "DEFAULT": 0.0,
            "MIN": 0.0,
            "MAX": 6.283
        },
        {
            "NAME": "center_x",
            "LABEL": "Center X",
            "TYPE": "float",
            "DEFAULT": 0.5,
            "MIN": 0.0,
            "MAX": 1.0
        },
        {
            "NAME": "center_y",
            "LABEL": "Center Y",
            "TYPE": "float",
            "DEFAULT": 0.5,
            "MIN": 0.0,
            "MAX": 1.0
        },
        {
            "NAME": "zoom",
            "LABEL": "Zoom",
            "TYPE": "float",
            "DEFAULT": 1.0,
            "MIN": 0.1,
            "MAX": 3.0
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float segments;
    float rotation;
    float center_x;
    float center_y;
    float zoom;
};

#define PI 3.14159265359

void main() {
    vec2 center = vec2(center_x, center_y);
    vec2 p = uv - center;
    
    // Convert to polar coordinates
    float r = length(p) * zoom;
    float theta = atan(p.y, p.x) + rotation;
    
    // Kaleidoscope effect
    float segmentAngle = 2.0 * PI / segments;
    theta = mod(theta, segmentAngle);
    
    // Mirror every other segment
    if (mod(floor(theta / segmentAngle * 2.0), 2.0) == 1.0) {
        theta = segmentAngle - theta;
    }
    
    // Convert back to cartesian
    vec2 newUV = center + r * vec2(cos(theta), sin(theta));
    
    // Clamp or wrap UV
    newUV = fract(newUV);
    
    fragColor = texture(sampler2D(inputImage, texSampler), newUV);
}

