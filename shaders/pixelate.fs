/*{
    "DESCRIPTION": "Pixelation / mosaic effect",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {
            "NAME": "inputImage",
            "TYPE": "image"
        },
        {
            "NAME": "pixel_size",
            "LABEL": "Pixel Size",
            "TYPE": "float",
            "DEFAULT": 8.0,
            "MIN": 1.0,
            "MAX": 64.0
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
    float pixel_size;
};

void main() {
    float size = pixel_size;
    
    vec2 pixelCount = RENDERSIZE / size;
    vec2 pixelUV = floor(uv * pixelCount) / pixelCount;
    pixelUV += 0.5 / pixelCount; // Center of pixel
    
    fragColor = texture(sampler2D(inputImage, texSampler), pixelUV);
}

