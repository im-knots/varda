/*{
    "DESCRIPTION": "Sharpen - unsharp mask sharpening",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "sharpen_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0},
        {"NAME": "sharpen_radius", "LABEL": "Radius", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 5.0}
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
    float sharpen_amount;
    float sharpen_radius;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 texel = sharpen_radius / RENDERSIZE;
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Sample neighbors (cross pattern)
    vec4 top = texture(sampler2D(inputImage, texSampler), uv + vec2(0.0, texel.y));
    vec4 bot = texture(sampler2D(inputImage, texSampler), uv - vec2(0.0, texel.y));
    vec4 lft = texture(sampler2D(inputImage, texSampler), uv - vec2(texel.x, 0.0));
    vec4 rgt = texture(sampler2D(inputImage, texSampler), uv + vec2(texel.x, 0.0));

    // Unsharp mask: original + (original - blur) * amount
    vec4 blur = (top + bot + lft + rgt) * 0.25;
    vec4 sharp = src + (src - blur) * sharpen_amount;

    fragColor = vec4(clamp(sharp.rgb, 0.0, 1.0), src.a);
}
