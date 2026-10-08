/*{
    "DESCRIPTION": "Lens Streaks - anamorphic streaks and ghost reflections from bright areas",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Stylize"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "threshold", "LABEL": "Threshold", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "streak_intensity", "LABEL": "Streak Intensity", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "streak_length", "LABEL": "Streak Length", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "angle", "LABEL": "Angle", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 180.0},
        {"NAME": "ghost_intensity", "LABEL": "Ghost Intensity", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "ghost_count", "LABEL": "Ghosts", "TYPE": "long", "DEFAULT": 3, "VALUES": [0, 1, 2, 3, 4, 5, 6], "LABELS": ["0", "1", "2", "3", "4", "5", "6"]},
        {"NAME": "chroma", "LABEL": "Chroma", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "tint", "LABEL": "Tint", "TYPE": "color", "DEFAULT": [0.75, 0.85, 1.0, 1.0]},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PASSES": [
        {"TARGET": "bright", "WIDTH": "$WIDTH/4", "HEIGHT": "$HEIGHT/4"},
        {"TARGET": "streakA", "WIDTH": "$WIDTH/4", "HEIGHT": "$HEIGHT/4"},
        {"TARGET": "streakB", "WIDTH": "$WIDTH/4", "HEIGHT": "$HEIGHT/4"},
        {"TARGET": "streakC", "WIDTH": "$WIDTH/4", "HEIGHT": "$HEIGHT/4"}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform texture2D bright;
layout(set = 0, binding = 4) uniform texture2D streakA;
layout(set = 0, binding = 5) uniform texture2D streakB;
layout(set = 0, binding = 6) uniform texture2D streakC;

layout(set = 0, binding = 7) uniform UserParams {
    float threshold;
    float streak_intensity;
    float streak_length;
    float angle;
    float ghost_intensity;
    int ghost_count;
    float chroma;
    vec4 tint;
    float amount;
};

// One streak pass: 7 taps along the streak axis, `spacing` texels apart, each
// weighted by the per-texel falloff raised to its distance. Three passes with
// spacing 1, 4 and 16 reach about 64 texels each way.
vec3 streak(texture2D src, float spacing) {
    vec2 texel = 1.0 / vec2(textureSize(sampler2D(bright, texSampler), 0));
    float a = radians(angle);
    vec2 dir = vec2(cos(a), -sin(a)) * texel * spacing;
    float falloff = mix(0.6, 0.97, streak_length);
    vec3 sum = vec3(0.0);
    float total = 0.0;
    for (int i = -3; i <= 3; i++) {
        float w = pow(falloff, spacing * float(abs(i)));
        sum += texture(sampler2D(src, texSampler), uv + dir * float(i)).rgb * w;
        total += w;
    }
    return sum / total;
}

vec3 ghosts() {
    vec3 sum = vec3(0.0);
    vec2 fromCenter = uv - 0.5;
    for (int k = 1; k <= 6; k++) {
        if (k > ghost_count) break;
        // Each ghost mirrors the bright pass through the center at its own scale.
        float s = mix(-1.3, 0.7, fract(float(k) * 0.618034));
        vec3 spread = 1.0 + chroma * 0.08 * vec3(-1.0, 0.0, 1.0);
        vec3 g;
        g.r = texture(sampler2D(bright, texSampler), 0.5 + fromCenter * s * spread.r).r;
        g.g = texture(sampler2D(bright, texSampler), 0.5 + fromCenter * s * spread.g).g;
        g.b = texture(sampler2D(bright, texSampler), 0.5 + fromCenter * s * spread.b).b;
        vec2 at = fromCenter * s;
        float edge = clamp(1.0 - length(at) * 1.6, 0.0, 1.0);
        sum += g * edge * edge / float(k);
    }
    return sum * 3.0;
}

void main() {
    if (PASSINDEX == 0) {
        // Average a 2x2 block of quarter-size texels' worth of input, then keep
        // what sits above the threshold with a soft knee.
        vec2 texel = 1.0 / vec2(textureSize(sampler2D(bright, texSampler), 0));
        vec3 c = vec3(0.0);
        c += texture(sampler2D(inputImage, texSampler), uv + texel * vec2(-0.25, -0.25)).rgb;
        c += texture(sampler2D(inputImage, texSampler), uv + texel * vec2(0.25, -0.25)).rgb;
        c += texture(sampler2D(inputImage, texSampler), uv + texel * vec2(-0.25, 0.25)).rgb;
        c += texture(sampler2D(inputImage, texSampler), uv + texel * vec2(0.25, 0.25)).rgb;
        c = max(c * 0.25, 0.0);
        float l = max(c.r, max(c.g, c.b));
        float knee = 0.25 * threshold + 1e-4;
        float soft = clamp(l - threshold + knee, 0.0, 2.0 * knee);
        soft = soft * soft / (4.0 * knee);
        float keep = max(soft, l - threshold) / max(l, 1e-4);
        fragColor = vec4(c * keep, 1.0);
        return;
    }
    if (PASSINDEX == 1) {
        fragColor = vec4(streak(bright, 1.0), 1.0);
        return;
    }
    if (PASSINDEX == 2) {
        fragColor = vec4(streak(streakA, 4.0), 1.0);
        return;
    }
    if (PASSINDEX == 3) {
        fragColor = vec4(streak(streakB, 16.0), 1.0);
        return;
    }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    // A blur spreads the energy over about 2 * reach texels; scale it back up
    // so the streak core stays bright.
    vec3 streaks = texture(sampler2D(streakC, texSampler), uv).rgb * 8.0;
    vec3 flare = streaks * streak_intensity + ghosts() * ghost_intensity;
    fragColor = vec4(max(src.rgb + flare * tint.rgb * amount, 0.0), src.a);
}
