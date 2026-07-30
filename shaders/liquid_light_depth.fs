/*{
    "DESCRIPTION": "Kinect liquid light - 1960s oil/water/dye projector look driven by a live depth sensor. Bodies in the sensor's view push a real advected fluid field and read as flowing dye outlines. Requires an attached depth sensor (Kinect v1).",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "Interactive"],
    "INPUTS": [
        {"NAME": "flow_speed", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0, "LABEL": "Flow Speed"},
        {"NAME": "blob_scale", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0, "LABEL": "Blob Scale"},
        {"NAME": "color_intensity", "TYPE": "float", "DEFAULT": 1.2, "MIN": 0.3, "MAX": 2.0, "LABEL": "Color Intensity"},
        {"NAME": "edge_glow", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.5, "LABEL": "Edge Glow"},
        {"NAME": "dye_spread", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Dye Spread"},
        {"NAME": "warmth", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0, "LABEL": "Projector Warmth"},
        {"NAME": "vignette_amt", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Vignette"},
        {"NAME": "palette", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Palette (0=Classic 1=Acid 2=Sunset 3=Deep)"},
        {"NAME": "agitation", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Agitation"},
        {"NAME": "focus_soft", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Soft Focus"},
        {"NAME": "silhouette_force", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 2.0, "LABEL": "Silhouette Force"},
        {"NAME": "motion_force", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Motion Force"},
        {"NAME": "outline_width", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 1.0, "LABEL": "Outline Width"},
        {"NAME": "persistence", "TYPE": "float", "DEFAULT": 0.94, "MIN": 0.5, "MAX": 0.995, "LABEL": "Fluid Persistence"},
        {"NAME": "depth_tint", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Depth Tint"}
    ],
    "PASSES": [
        {"TARGET": "velocity", "PERSISTENT": true, "FLOAT": true},
        {"TARGET": "dye", "PERSISTENT": true, "FLOAT": true}
    ],
    "PREPROCESSORS": [
        {"NAME": "depth", "TYPE": "depth_sensor"},
        {"NAME": "mask", "TYPE": "depth_sensor"},
        {"NAME": "motion", "TYPE": "depth_sensor"}
    ],
    "PHASE_INPUTS": [{"PARAM": "flow_speed", "INDEX": 0}]
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

// Pass buffers (declaration order): velocity then dye.
layout(set = 0, binding = 2) uniform texture2D velocity;
layout(set = 0, binding = 3) uniform texture2D dye;

// PREPROCESSOR textures from the depth_sensor preprocessor (declaration order).
layout(set = 0, binding = 4) uniform texture2D depth;
layout(set = 0, binding = 5) uniform texture2D mask;
layout(set = 0, binding = 6) uniform texture2D motion;

layout(set = 0, binding = 7) uniform UserParams {
    float flow_speed;
    float blob_scale;
    float color_intensity;
    float edge_glow;
    float dye_spread;
    float warmth;
    float vignette_amt;
    float palette;
    float agitation;
    float focus_soft;
    float silhouette_force;
    float motion_force;
    float outline_width;
    float persistence;
    float depth_tint;
};

// ---- Noise primitives (quintic-interpolated, no grid artifacts) ----

float hash21(vec2 p) {
    vec3 p3 = fract(vec3(p.xyx) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

float noise2(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    float a = hash21(i);
    float b = hash21(i + vec2(1, 0));
    float c = hash21(i + vec2(0, 1));
    float d = hash21(i + vec2(1, 1));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

float fbm(vec2 p, int octaves) {
    float val = 0.0;
    float amp = 0.5;
    mat2 rot = mat2(0.8, 0.6, -0.6, 0.8);
    for (int i = 0; i < 5; i++) {
        if (i >= octaves) break;
        val += amp * noise2(p);
        p = rot * p * 2.03;
        amp *= 0.5;
    }
    return val;
}

// Divergence-free ambient flow: the curl of a scalar FBM potential. This is the
// slow convective churn of oil on a hot projector plate, present with or without
// anyone in frame.
vec2 curl_flow(vec2 p, float t) {
    float e = 0.02;
    float n0 = fbm(p + vec2(0.0, e) + t, 3);
    float n1 = fbm(p - vec2(0.0, e) + t, 3);
    float n2 = fbm(p + vec2(e, 0.0) - t, 3);
    float n3 = fbm(p - vec2(e, 0.0) - t, 3);
    return vec2(n0 - n1, n3 - n2) / (2.0 * e);
}

float mask_at(vec2 p) {
    return texture(sampler2D(mask, texSampler), clamp(p, 0.0, 1.0)).r;
}

vec2 vel_at(vec2 p) {
    return texture(sampler2D(velocity, texSampler), clamp(p, 0.0, 1.0)).xy;
}

vec4 dye_at(vec2 p) {
    return texture(sampler2D(dye, texSampler), clamp(p, 0.0, 1.0));
}

// ---- Palettes: the four looks liquid_light.fs ships, keyed off dye density ----

vec3 palette_color(float t, float which) {
    t = clamp(t, 0.0, 1.0);
    vec3 classic = mix(vec3(0.05, 0.10, 0.55), vec3(0.95, 0.25, 0.10), t);
    classic = mix(classic, vec3(1.0, 0.85, 0.25), smoothstep(0.55, 1.0, t));

    vec3 acid = mix(vec3(0.0, 0.45, 0.20), vec3(0.85, 0.95, 0.10), t);
    acid = mix(acid, vec3(1.0, 0.15, 0.75), smoothstep(0.6, 1.0, t));

    vec3 sunset = mix(vec3(0.35, 0.05, 0.30), vec3(1.0, 0.45, 0.15), t);
    sunset = mix(sunset, vec3(1.0, 0.90, 0.60), smoothstep(0.65, 1.0, t));

    vec3 deep = mix(vec3(0.02, 0.05, 0.20), vec3(0.10, 0.60, 0.85), t);
    deep = mix(deep, vec3(0.75, 0.95, 1.0), smoothstep(0.7, 1.0, t));

    float w = clamp(which, 0.0, 3.0);
    vec3 col = mix(classic, acid, clamp(w, 0.0, 1.0));
    col = mix(col, sunset, clamp(w - 1.0, 0.0, 1.0));
    col = mix(col, deep, clamp(w - 2.0, 0.0, 1.0));
    return col;
}

void main() {
    vec2 texel = 1.0 / RENDERSIZE;

    // Uniform guard — keep the ISF/audio uniforms live for binding consistency.
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIME + TIMEDELTA + float(FRAMEINDEX)
        + DATE.x + DATE.y + DATE.z + DATE.w
        + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    float keep = (audioSum + timeSum) * 1e-8;

    float t = PHASE_TIME_0;
    float m = mask_at(uv);

    // Silhouette gradient: points out of the body, so bodies push fluid away.
    float mx = mask_at(uv + vec2(texel.x, 0.0)) - mask_at(uv - vec2(texel.x, 0.0));
    float my = mask_at(uv + vec2(0.0, texel.y)) - mask_at(uv - vec2(0.0, texel.y));
    vec2 mgrad = vec2(mx, my) / (2.0 * max(texel.x, texel.y));
    float edge = length(mgrad);

    // ── Pass 0: velocity — self-advect, add ambient churn, inject body force ──
    if (PASSINDEX == 0) {
        vec2 v = vel_at(uv);

        // Semi-Lagrangian self-advection: trace back along the field.
        vec2 back = uv - v * texel * 60.0 * (0.2 + flow_speed);
        v = mix(v, vel_at(back), 0.85);

        // Ambient convection, scaled by agitation and the blob scale knob.
        v += curl_flow(uv * blob_scale * 3.0, t * 0.35) * agitation * 0.06;

        // Bodies displace fluid outward along the silhouette normal ...
        v += mgrad * silhouette_force * 0.02;
        // ... and drag it along when they actually move. This is what makes the
        // dye swirl on a gesture rather than merely sitting around a shape.
        v += texture(sampler2D(motion, texSampler), uv).xy * motion_force * 0.05;

        v *= clamp(persistence, 0.5, 0.995);
        fragColor = vec4(v + keep, 0.0, 1.0);
        return;
    }

    // ── Pass 1: dye — advect by velocity, emit at the silhouette contour ─────
    if (PASSINDEX == 1) {
        vec2 v = vel_at(uv);
        vec2 back = uv - v * texel * 60.0 * (0.2 + flow_speed);
        vec4 d = dye_at(back);

        // Diffuse a little so the dye reads as fluid rather than as streaks.
        float sp = 1.0 + dye_spread * 5.0;
        vec4 blur = dye_at(back + vec2(texel.x * sp, 0.0))
                  + dye_at(back - vec2(texel.x * sp, 0.0))
                  + dye_at(back + vec2(0.0, texel.y * sp))
                  + dye_at(back - vec2(0.0, texel.y * sp));
        d = mix(d, blur * 0.25, dye_spread * 0.35);

        // The contour is the dye source: emit where the mask gradient is strong.
        float emit = smoothstep(0.05, 0.6, edge * 0.02);
        // Nearer subjects emit hotter dye, so depth reads as colour temperature.
        float near = 1.0 - clamp(texture(sampler2D(depth, texSampler), uv).r, 0.0, 1.0);
        float hue = clamp(0.35 + near * depth_tint * 0.65, 0.0, 1.0);

        d.r = max(d.r * clamp(persistence, 0.5, 0.995), emit);
        d.g = mix(d.g * clamp(persistence, 0.5, 0.995), hue, emit);
        // Interior fill so bodies are solid dye, not just outlines.
        d.b = max(d.b * clamp(persistence, 0.5, 0.995), m * 0.7);

        fragColor = vec4(max(d.rgb + keep, 0.0), 1.0);
        return;
    }

    // ── Final pass: the liquid-light look, applied to the dye field ──────────
    vec4 d = dye_at(uv);
    float density = clamp(d.r + d.b * 0.8, 0.0, 1.5);

    // Soft focus: an out-of-focus overhead projector never resolves an edge.
    float blurAmt = focus_soft * 3.0;
    vec4 soft = dye_at(uv + vec2(texel.x, texel.y) * blurAmt)
              + dye_at(uv - vec2(texel.x, texel.y) * blurAmt)
              + dye_at(uv + vec2(texel.x, -texel.y) * blurAmt)
              + dye_at(uv - vec2(texel.x, -texel.y) * blurAmt);
    density = mix(density, clamp(soft.r * 0.25 + soft.b * 0.2, 0.0, 1.5), focus_soft * 0.6);

    vec3 col = palette_color(density * 0.9 + d.g * 0.4, palette) * color_intensity;

    // Dye-boundary glow — the wet rim where two colours meet on the plate.
    float dx = dye_at(uv + vec2(texel.x, 0.0)).r - dye_at(uv - vec2(texel.x, 0.0)).r;
    float dy = dye_at(uv + vec2(0.0, texel.y)).r - dye_at(uv - vec2(0.0, texel.y)).r;
    col += palette_color(1.0, palette) * length(vec2(dx, dy)) * edge_glow * 6.0;

    // The literal fluid outline of the person, riding the live mask contour.
    float rim = smoothstep(0.02, 0.02 + outline_width * 0.5, edge * 0.02);
    col += palette_color(0.85, palette) * rim * (0.4 + edge_glow * 0.6);

    // Warm tungsten cast of an overhead projector lamp.
    col *= mix(vec3(1.0), vec3(1.12, 0.98, 0.82), warmth);

    // Round lamp falloff.
    vec2 c = uv - 0.5;
    col *= mix(1.0, 1.0 - dot(c, c) * 2.2, vignette_amt);

    col *= density * 0.6 + 0.35;

    fragColor = vec4(max(col + keep, 0.0), 1.0);
}
