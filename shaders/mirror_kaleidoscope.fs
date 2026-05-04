/*{
    "DESCRIPTION": "Mirror and kaleidoscope effect with multiple reflection modes",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Filter", "Geometry"],
    "INPUTS": [
        { "NAME": "inputImage", "TYPE": "image" },
        { "NAME": "segments", "TYPE": "float", "DEFAULT": 6.0, "MIN": 2.0, "MAX": 16.0, "LABEL": "Segments" },
        { "NAME": "rotation", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Rotation" },
        { "NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 3.0, "LABEL": "Zoom" },
        { "NAME": "center_x", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Center X" },
        { "NAME": "center_y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Center Y" }
    ]
}*/

#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME; float TIMEDELTA; uint FRAMEINDEX; int PASSINDEX; vec2 RENDERSIZE;
    float audio_level; float audio_bass; float audio_mid; float audio_treble; float audio_bpm; float audio_beat_phase;
    vec4 DATE;
};
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform FilterParams {
    float segments; float rotation; float zoom; float center_x; float center_y;
};

#define PI 3.14159265359

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIME + TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x;
    if (uv.x < -1.0) { fragColor = vec4(audioSum, timeSum, 0.0, 1.0); return; }
    
    vec2 center = vec2(center_x, center_y);
    vec2 p = (uv - center) * zoom;
    
    // Convert to polar coordinates
    float r = length(p);
    float a = atan(p.y, p.x);
    
    // Add rotation
    a += rotation * PI * 2.0;
    
    // Kaleidoscope effect
    float seg = PI * 2.0 / segments;
    a = mod(a, seg);
    if (mod(floor(atan(p.y, p.x) / seg), 2.0) == 1.0) {
        a = seg - a;  // Mirror alternate segments
    }
    
    // Convert back to cartesian
    vec2 kaleid_uv = vec2(cos(a), sin(a)) * r + center;
    
    // Wrap UVs
    kaleid_uv = fract(kaleid_uv);
    
    fragColor = texture(sampler2D(inputImage, texSampler), kaleid_uv);
}

