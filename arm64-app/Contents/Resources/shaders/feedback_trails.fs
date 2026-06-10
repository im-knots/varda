/*{
    "DESCRIPTION": "Psychedelic trails effect — like waving your hand in front of your face on psychedelics. Moving things leave ghostly color-shifted trails that linger and fade. Static parts stay clean.",
    "CREDIT": "Varda VJ",
    "CATEGORIES": ["Filter", "Feedback"],
    "INPUTS": [
        { "NAME": "inputImage", "TYPE": "image" },
        { "NAME": "trail_length",    "TYPE": "float", "DEFAULT": 0.92, "MIN": 0.0, "MAX": 0.99, "LABEL": "Trail Length" },
        { "NAME": "motion_sens",     "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.01, "MAX": 0.3,  "LABEL": "Motion Sensitivity" },
        { "NAME": "color_spread",    "TYPE": "float", "DEFAULT": 0.004,"MIN": 0.0,  "MAX": 0.02, "LABEL": "Rainbow Spread" },
        { "NAME": "trail_zoom",      "TYPE": "float", "DEFAULT": 0.002,"MIN": -0.01,"MAX": 0.01, "LABEL": "Trail Zoom" },
        { "NAME": "trail_rotate",    "TYPE": "float", "DEFAULT": 0.003,"MIN": -0.03,"MAX": 0.03, "LABEL": "Trail Rotate" }
    ],
    "PASSES": [
        { "TARGET": "trailBuffer", "PERSISTENT": true },
        { "TARGET": "prevFrame",   "PERSISTENT": true }
    ]
}*/

#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME; float TIMEDELTA; uint FRAMEINDEX; int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level; float audio_bass; float audio_mid; float audio_treble;
    float audio_bpm; float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};
layout(set = 0, binding = 1) uniform sampler samp;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform texture2D trailBuffer;
layout(set = 0, binding = 4) uniform texture2D prevFrame;
layout(set = 0, binding = 5) uniform FilterParams {
    float trail_length; float motion_sens; float color_spread;
    float trail_zoom; float trail_rotate;
};

vec2 rot(vec2 p, float a) {
    return vec2(p.x*cos(a) - p.y*sin(a), p.x*sin(a) + p.y*cos(a));
}

void main() {
    // Keep all uniforms alive
    float _k = (audio_level+audio_bass+audio_mid+audio_treble+audio_bpm+audio_beat_phase
               +TIMEDELTA+float(FRAMEINDEX)+DATE.x+DATE.y+DATE.z+DATE.w) * 1e-7;

    vec4 cur = texture(sampler2D(inputImage, samp), uv);

    // ── Pass 0: write trailBuffer ───────────────────────────────────
    if (PASSINDEX == 0) {
        // Compare current input to the CLEAN previous frame (prevFrame)
        // to find pixels that actually moved
        vec4 prev = texture(sampler2D(prevFrame, samp), uv);
        float motion = smoothstep(motion_sens * 0.4, motion_sens, length(cur.rgb - prev.rgb));

        // Read old trails with slight warp for that drifting psychedelic feel
        vec2 tc = uv - 0.5;
        tc *= 1.0 + trail_zoom;
        tc = rot(tc, trail_rotate);
        tc = clamp(tc + 0.5, 0.001, 0.999);

        // Chromatic-split sample of old trail for rainbow fringing
        vec3 oldTrail;
        oldTrail.r = texture(sampler2D(trailBuffer, samp), tc + vec2(color_spread, 0.0)).r;
        oldTrail.g = texture(sampler2D(trailBuffer, samp), tc).g;
        oldTrail.b = texture(sampler2D(trailBuffer, samp), tc - vec2(color_spread, 0.0)).b;

        // Fade old trails
        oldTrail *= trail_length;

        // Where motion happened: stamp current color into trail.
        // Where static: just let old trails keep fading.
        vec3 result = mix(oldTrail, cur.rgb, motion);

        fragColor = vec4(result + _k, 1.0);
    }

    // ── Pass 1: write prevFrame (store clean current for next frame) ─
    else if (PASSINDEX == 1) {
        fragColor = vec4(cur.rgb + _k, 1.0);
    }

    // ── Final pass: composite to screen ─────────────────────────────
    else {
        vec3 trail = texture(sampler2D(trailBuffer, samp), uv).rgb;

        // Show clean current image, with trail ghosts layered underneath.
        // Where there are no trails (black), you just see the input.
        // Where there are trails, they bleed through as ghostly echoes.
        // Screen blend: bright trails glow, dark areas stay clean.
        vec3 out_color = 1.0 - (1.0 - cur.rgb) * (1.0 - trail * 0.6);

        fragColor = vec4(out_color + _k, 1.0);
    }
}
