/*{
    "DESCRIPTION": "Water Ripples - a simulated water surface that refracts the image",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Filter", "Distort"],
    "INPUTS": [
        {"NAME": "inputImage", "TYPE": "image"},
        {"NAME": "refraction", "LABEL": "Refraction", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "damping", "LABEL": "Damping", "TYPE": "float", "DEFAULT": 0.985, "MIN": 0.9, "MAX": 0.999},
        {"NAME": "wave_speed", "LABEL": "Wave Speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 1.0},
        {"NAME": "drop_rate", "LABEL": "Drop Rate", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.0, "MAX": 20.0},
        {"NAME": "drop_size", "LABEL": "Drop Size", "TYPE": "float", "DEFAULT": 0.015, "MIN": 0.003, "MAX": 0.1},
        {"NAME": "drop", "LABEL": "Drop", "TYPE": "event"},
        {"NAME": "drop_x", "LABEL": "Drop X", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "drop_y", "LABEL": "Drop Y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0},
        {"NAME": "motion_drive", "LABEL": "Motion Drive", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 4.0},
        {"NAME": "highlight", "LABEL": "Highlight", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 2.0},
        {"NAME": "amount", "LABEL": "Amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "drop_rate", "INDEX": 0, "SCALE": 1.0}
    ],
    "PASSES": [
        {"TARGET": "sim", "PERSISTENT": true, "FORMAT": "rgba32float", "WIDTH": "$WIDTH/2", "HEIGHT": "$HEIGHT/2"},
        {"TARGET": "slope", "WIDTH": "$WIDTH/2", "HEIGHT": "$HEIGHT/2"}
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
layout(set = 0, binding = 3) uniform texture2D sim;
layout(set = 0, binding = 4) uniform texture2D slope;

layout(set = 0, binding = 5) uniform UserParams {
    float refraction;
    float damping;
    float wave_speed;
    float drop_rate;
    float drop_size;
    uint drop;
    float drop_x;
    float drop_y;
    float motion_drive;
    float highlight;
    float amount;
};

// sim texel: r = height, g = height last frame, b = input luma last frame
// plus 1 (0 before the first write), a = whole drops released so far (the
// same in every texel).

ivec2 simSize;

vec4 simAt(ivec2 p) {
    return texelFetch(sampler2D(sim, texSampler), clamp(p, ivec2(0), simSize - 1), 0);
}

vec2 hash2(float n) {
    return fract(sin(vec2(n * 12.9898, n * 78.233)) * 43758.5453);
}

float bump(vec2 at, vec2 center, float aspect) {
    vec2 d = (at - center) * vec2(aspect, 1.0);
    float r = max(drop_size, 1e-3);
    return 4.0 * exp(-dot(d, d) / (r * r));
}

void main() {
    simSize = textureSize(sampler2D(sim, texSampler), 0);
    float aspect = float(simSize.x) / float(simSize.y);

    if (PASSINDEX == 0) {
        ivec2 p = ivec2(gl_FragCoord.xy);
        vec4 s = simAt(p);
        float n = simAt(p + ivec2(1, 0)).r + simAt(p - ivec2(1, 0)).r
                + simAt(p + ivec2(0, 1)).r + simAt(p - ivec2(0, 1)).r;
        // Explicit wave equation. Scaling k by the frame time squared keeps the
        // wave's screen speed the same at any frame rate; k stays at or below
        // the 0.5 stability limit, so waves slow down below about 60 fps.
        float frames = TIMEDELTA * 60.0;
        float k = min(0.5 * wave_speed * wave_speed * frames * frames, 0.5);
        float h = (2.0 * s.r - s.g + k * (n - 4.0 * s.r)) * pow(damping, frames);

        float drops = floor(PHASE_TIME_0);
        float fresh = clamp(drops - s.a, 0.0, 3.0);
        for (int i = 0; i < 3; i++) {
            if (float(i) >= fresh) break;
            h += bump(uv, hash2(drops - float(i)), aspect);
        }
        if (drop == 1u) h += bump(uv, vec2(drop_x, drop_y), aspect);

        vec3 c = max(texture(sampler2D(inputImage, texSampler), uv).rgb, 0.0);
        float luma = dot(c, vec3(0.2126, 0.7152, 0.0722));
        if (s.b > 0.5) h += (luma - (s.b - 1.0)) * motion_drive;

        fragColor = vec4(clamp(h, -8.0, 8.0), s.r, luma + 1.0, drops);
        return;
    }

    if (PASSINDEX == 1) {
        ivec2 p = ivec2(gl_FragCoord.xy);
        float dx = simAt(p + ivec2(1, 0)).r - simAt(p - ivec2(1, 0)).r;
        float dy = simAt(p + ivec2(0, 1)).r - simAt(p - ivec2(0, 1)).r;
        fragColor = vec4(0.5 * dx, 0.5 * dy, 0.0, 1.0);
        return;
    }

    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    vec2 g = texture(sampler2D(slope, texSampler), uv).xy;
    vec4 bent = texture(sampler2D(inputImage, texSampler), uv - g * refraction * 0.5);
    // Slopes facing a light up and to the left glint; slopes facing away dim.
    float facing = dot(g, vec2(-0.7071)) * 10.0;
    vec3 col = bent.rgb * (1.0 - 0.3 * highlight * clamp(-facing, 0.0, 1.0))
             + highlight * facing * max(facing, 0.0);
    fragColor = vec4(max(mix(src.rgb, col, amount), 0.0), src.a);
}
