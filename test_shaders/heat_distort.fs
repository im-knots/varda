/*{
    "DESCRIPTION": "Heat Distortion - rising heat wave shimmer",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "heat_amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 0.01, "MIN": 0.0, "MAX": 0.05},
        {"NAME": "heat_speed", "LABEL": "Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0},
        {"NAME": "heat_scale", "LABEL": "Scale", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 30.0},
        {"NAME": "direction", "LABEL": "Direction (0=Up 1=Right 2=Down 3=Left)", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0},
        {"NAME": "turbulence", "LABEL": "Turbulence", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0}
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
    float heat_amount;
    float heat_speed;
    float heat_scale;
    float direction;
    float turbulence;
};

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

float noise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), f.x),
               mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), f.x), f.y);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float t = TIME * heat_speed;
    int dir = int(floor(direction + 0.5));

    // Flow direction
    vec2 flow;
    if (dir == 0) flow = vec2(0.0, 1.0);
    else if (dir == 1) flow = vec2(1.0, 0.0);
    else if (dir == 2) flow = vec2(0.0, -1.0);
    else flow = vec2(-1.0, 0.0);

    vec2 noiseCoord = uv * heat_scale + flow * t;

    float n1 = noise(noiseCoord);
    float n2 = noise(noiseCoord * 2.1 + vec2(5.2, 1.3));
    float distort = (n1 + n2 * turbulence * 0.5) - 0.5 * (1.0 + turbulence * 0.5);

    vec2 offset = vec2(distort) * heat_amount;
    // Perpendicular to flow direction for realistic shimmer
    offset = vec2(-flow.y, flow.x) * distort * heat_amount + flow * distort * heat_amount * 0.3;

    vec4 result = texture(sampler2D(inputImage, texSampler), uv + offset);
    fragColor = result;
}
