/*{
    "DESCRIPTION": "Edge detection with glow effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {
            "NAME": "inputImage",
            "TYPE": "image"
        },
        {
            "NAME": "edge_strength",
            "LABEL": "Edge Strength",
            "TYPE": "float",
            "DEFAULT": 1.0,
            "MIN": 0.0,
            "MAX": 3.0
        },
        {
            "NAME": "glow_amount",
            "LABEL": "Glow Amount",
            "TYPE": "float",
            "DEFAULT": 0.5,
            "MIN": 0.0,
            "MAX": 1.0
        },
        {
            "NAME": "glow_color",
            "LABEL": "Glow Color",
            "TYPE": "color",
            "DEFAULT": [1.0, 0.5, 0.0, 1.0]
        },
        {
            "NAME": "show_original",
            "LABEL": "Show Original",
            "TYPE": "bool",
            "DEFAULT": true
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
    float edge_strength;
    float glow_amount;
    vec4 glow_color;
    float show_original;
};

float luminance(vec3 c) {
    return dot(c, vec3(0.299, 0.587, 0.114));
}

void main() {
    vec2 texel = 1.0 / RENDERSIZE;
    
    // Sobel edge detection
    float tl = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(-texel.x, texel.y)).rgb);
    float t  = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(0.0, texel.y)).rgb);
    float tr = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, texel.y)).rgb);
    float l  = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(-texel.x, 0.0)).rgb);
    float r  = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, 0.0)).rgb);
    float bl = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(-texel.x, -texel.y)).rgb);
    float b  = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(0.0, -texel.y)).rgb);
    float br = luminance(texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, -texel.y)).rgb);
    
    float gx = -tl - 2.0*l - bl + tr + 2.0*r + br;
    float gy = -tl - 2.0*t - tr + bl + 2.0*b + br;
    float edge = sqrt(gx*gx + gy*gy) * edge_strength;
    
    vec4 original = texture(sampler2D(inputImage, texSampler), uv);
    vec3 glow = glow_color.rgb * edge * glow_amount * (1.0 + audio_bass);
    
    if (show_original > 0.5) {
        fragColor = vec4(original.rgb + glow, 1.0);
    } else {
        fragColor = vec4(glow, 1.0);
    }
}

