/*{
    "DESCRIPTION": "Color Replace - match a source color and replace it with a target color",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Color"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "source_color", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.0, 1.0], "LABEL": "Source Color"},
        {"NAME": "target_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 1.0, 1.0], "LABEL": "Target Color"},
        {"NAME": "tolerance", "LABEL": "Tolerance", "TYPE": "float", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "softness", "LABEL": "Edge Softness", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 0.5},
        {"NAME": "preserve_luma", "LABEL": "Preserve Luminance", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "hue_only", "LABEL": "Hue Only Match", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
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
    vec4 source_color;
    vec4 target_color;
    float tolerance;
    float softness;
    float preserve_luma;
    float hue_only;
    float amount;
};

vec3 rgb2hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0 / 3.0, 2.0 / 3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y);
    float e = 1.0e-10;
    return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec3 hsv2rgb(vec3 c) {
    vec4 K = vec4(1.0, 2.0 / 3.0, 1.0 / 3.0, 3.0);
    vec3 p = abs(fract(c.xxx + K.xyz) * 6.0 - K.www);
    return c.z * mix(K.xxx, clamp(p - K.xxx, 0.0, 1.0), c.y);
}

float hueDistance(vec3 a, vec3 b) {
    float d = abs(a.x - b.x);
    return min(d, 1.0 - d);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);

    // Compute match strength
    float dist;
    if (hue_only > 0.5) {
        // Match by hue only (ignores brightness/saturation differences)
        vec3 srcHSV = rgb2hsv(src.rgb);
        vec3 keyHSV = rgb2hsv(source_color.rgb);
        // Weight hue distance heavily, but gate on minimum saturation to avoid matching grays
        float hueDist = hueDistance(srcHSV, keyHSV);
        float satGate = smoothstep(0.05, 0.15, srcHSV.y);
        dist = mix(1.0, hueDist, satGate);
    } else {
        // Match by RGB Euclidean distance
        dist = distance(src.rgb, source_color.rgb);
    }

    float match_amt = 1.0 - smoothstep(tolerance, tolerance + softness, dist);

    // Build replaced color
    vec3 replaced = target_color.rgb;

    if (preserve_luma > 0.0) {
        // Keep the original pixel's luminance, apply only the hue/sat from target
        float srcLuma = dot(src.rgb, vec3(0.299, 0.587, 0.114));
        float tgtLuma = dot(target_color.rgb, vec3(0.299, 0.587, 0.114));
        float lumaScale = (tgtLuma > 0.001) ? srcLuma / tgtLuma : 1.0;
        vec3 lumaPreserved = target_color.rgb * lumaScale;
        replaced = mix(replaced, clamp(lumaPreserved, 0.0, 1.0), preserve_luma);
    }

    // Blend: original → replaced, weighted by match and amount
    vec3 result = mix(src.rgb, replaced, match_amt * amount);

    fragColor = vec4(result, src.a);
}
