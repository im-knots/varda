/*{
    "DESCRIPTION": "Concentric animated rings generator",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        {"NAME": "ring_count", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 32.0, "LABEL": "Ring Count"},
        {"NAME": "ring_width", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.05, "MAX": 1.0, "LABEL": "Ring Width"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 0.8, 1.0, 1.0], "LABEL": "Color 1"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [1.0, 0.2, 0.5, 1.0], "LABEL": "Color 2"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"},
        {"NAME": "center_x", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Center X"},
        {"NAME": "center_y", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Center Y"}
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

layout(set = 0, binding = 1) uniform UserParams {
    float ring_count;
    float ring_width;
    float anim_speed;
    vec4 color1;
    vec4 color2;
    vec4 bg_color;
    float center_x;
    float center_y;
};

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = uv - vec2(center_x, center_y);
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float dist = length(p);
    float t = TIME * anim_speed;

    // Create rings using sine wave on distance
    float rings = sin((dist * ring_count * 6.283) - t) * 0.5 + 0.5;

    // Sharpen rings based on width
    float edge = smoothstep(0.5 - ring_width * 0.5, 0.5, rings) - smoothstep(0.5, 0.5 + ring_width * 0.5, rings);
    float ring = smoothstep(1.0 - ring_width, 1.0, rings);

    // Audio reactivity - pulse ring width with bass
    ring = clamp(ring * (1.0 + audio_bass * 0.5), 0.0, 1.0);

    // Color based on distance
    vec3 col = mix(color1.rgb, color2.rgb, fract(dist * ring_count * 0.5 - t * 0.1));

    vec3 finalColor = mix(bg_color.rgb, col, ring);
    float alpha = mix(bg_color.a, max(color1.a, color2.a), ring);

    fragColor = vec4(finalColor * alpha, alpha);
}
