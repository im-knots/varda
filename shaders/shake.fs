/*{
    "DESCRIPTION": "Shake - handheld camera shake with smooth noise, rotation and motion blur",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "rate", "LABEL": "Rate", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.0, "MAX": 20.0},
        {"NAME": "amplitude", "LABEL": "Amplitude", "TYPE": "float", "DEFAULT": 0.02, "MIN": 0.0, "MAX": 0.1},
        {"NAME": "rotation", "LABEL": "Rotation", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 10.0},
        {"NAME": "zoom_cover", "LABEL": "Zoom to Cover", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "edge", "LABEL": "Edges", "TYPE": "long", "DEFAULT": 1, "VALUES": [0, 1, 2], "LABELS": ["Clamp", "Mirror", "Black"]},
        {"NAME": "motion_blur", "LABEL": "Motion Blur", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "rate", "INDEX": 0, "SCALE": 1.0}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;

layout(set = 0, binding = 3) uniform UserParams {
    float rate;
    float amplitude;
    float rotation;
    float zoom_cover;
    int edge;
    float motion_blur;
    float amount;
};

// Smooth noise in [-1, 1]: three sines at unrelated frequencies.
float wobble(float t, float seed) {
    return 0.5 * sin(t * 1.0 + seed) + 0.3 * sin(t * 2.31 + seed * 1.7) + 0.2 * sin(t * 4.57 + seed * 2.9);
}

// Shake at phase t: x, y offset in frame heights, and angle in radians.
vec3 shakeAt(float t) {
    float k = amount;
    return vec3(wobble(t * 6.0, 0.0) * amplitude * k,
                wobble(t * 6.0, 11.3) * amplitude * k,
                wobble(t * 6.0, 23.7) * radians(rotation) * k);
}

vec4 fetch(vec2 s) {
    if (edge == 2 && (any(lessThan(s, vec2(0.0))) || any(greaterThan(s, vec2(1.0))))) return vec4(0.0);
    if (edge == 1) s = 1.0 - abs(mod(s, 2.0) - 1.0);
    return texture(sampler2D(inputImage, texSampler), clamp(s, 0.0, 1.0));
}

void main() {
    float aspect = RENDERSIZE.x / RENDERSIZE.y;
    // Zoom that hides the edges at full amplitude and rotation:
    // a rotated frame needs cos + sin * aspect, a shifted one 2 * offset more.
    float a = radians(rotation) * amount;
    float cover = cos(a) + sin(a) * max(aspect, 1.0 / aspect) + 2.0 * amplitude * amount;
    float zoom = mix(1.0, cover, zoom_cover);

    // Motion blur spans a fixed stretch of the shake path behind the current
    // position, so it reads the same at any rate.
    int taps = motion_blur > 0.0 ? 6 : 1;
    float span = motion_blur * 0.08;
    vec4 sum = vec4(0.0);
    for (int i = 0; i < 6; i++) {
        if (i >= taps) break;
        float t = PHASE_TIME_0 - span * float(i) / 5.0;
        vec3 s = shakeAt(t);
        vec2 p = (uv - 0.5) * vec2(aspect, 1.0);
        float c = cos(-s.z), sn = sin(-s.z);
        p = mat2(c, sn, -sn, c) * p / zoom - s.xy;
        sum += fetch(p / vec2(aspect, 1.0) + 0.5);
    }
    vec4 col = sum / float(taps);
    fragColor = vec4(max(col.rgb, 0.0), clamp(col.a, 0.0, 1.0));
}
