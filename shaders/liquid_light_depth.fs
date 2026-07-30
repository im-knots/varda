/*{
    "DESCRIPTION": "Kinect liquid light - 1960s oil/water/dye overhead projector look driven by a live depth sensor. Three immiscible dyes advect through a fluid field that bodies push around, with thin-film iridescence and an orbiting 3D camera that parallaxes the depth relief. Requires an attached depth sensor (Kinect v1).",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "Interactive"],
    "INPUTS": [
        {"NAME": "flow_speed", "TYPE": "float", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0, "LABEL": "Flow Speed"},
        {"NAME": "blob_scale", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.5, "MAX": 4.0, "LABEL": "Blob Scale"},
        {"NAME": "color_intensity", "TYPE": "float", "DEFAULT": 1.2, "MIN": 0.3, "MAX": 2.0, "LABEL": "Color Intensity"},
        {"NAME": "edge_glow", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.5, "LABEL": "Edge Glow"},
        {"NAME": "dye_spread", "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.0, "MAX": 1.0, "LABEL": "Dye Spread"},
        {"NAME": "warmth", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0, "LABEL": "Projector Warmth"},
        {"NAME": "vignette_amt", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Vignette"},
        {"NAME": "palette", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Palette (0=Classic 1=Acid 2=Sunset 3=Deep)"},
        {"NAME": "agitation", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0, "LABEL": "Agitation"},
        {"NAME": "focus_soft", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0, "LABEL": "Soft Focus"},
        {"NAME": "silhouette_force", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Silhouette Force"},
        {"NAME": "motion_force", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Motion Force"},
        {"NAME": "outline_width", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 1.0, "LABEL": "Outline Width"},
        {"NAME": "persistence", "TYPE": "float", "DEFAULT": 0.96, "MIN": 0.5, "MAX": 0.995, "LABEL": "Fluid Persistence"},
        {"NAME": "depth_tint", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Depth Tint"},
        {"NAME": "iridescence", "TYPE": "float", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 1.0, "LABEL": "Iridescence"},
        {"NAME": "orbit_yaw", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Orbit Yaw"},
        {"NAME": "orbit_pitch", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Orbit Pitch"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.4, "MAX": 2.5, "LABEL": "Zoom"},
        {"NAME": "relief", "TYPE": "float", "DEFAULT": 0.45, "MIN": 0.0, "MAX": 1.0, "LABEL": "3D Relief"}
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
    float iridescence;
    float orbit_yaw;
    float orbit_pitch;
    float zoom;
    float relief;
};

// Kinect v1 is 4:3. Sampling it with raw UVs across a 16:9 deck stretches people
// horizontally, so the sensor field is fitted (letterboxed) into the deck.
const float SENSOR_ASPECT = 4.0 / 3.0;
// Fixed UV offset for the mask gradient — resolution-independent, unlike a
// per-render-texel difference, so thresholds behave the same at 720p and 4K.
const float GRAD_STEP = 1.0 / 200.0;
const int POM_STEPS = 16;

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

// Divergence-free ambient flow: the curl of a scalar FBM potential. The slow
// convective churn of oil on a hot projector plate, present with or without
// anyone in frame.
vec2 curl_flow(vec2 p, float t) {
    float e = 0.02;
    float n0 = fbm(p + vec2(0.0, e) + t, 3);
    float n1 = fbm(p - vec2(0.0, e) + t, 3);
    float n2 = fbm(p + vec2(e, 0.0) - t, 3);
    float n3 = fbm(p - vec2(e, 0.0) - t, 3);
    return vec2(n0 - n1, n3 - n2) / (2.0 * e);
}

// Map deck UV to sensor UV, preserving the sensor's 4:3 aspect.
vec2 sensor_uv(vec2 p) {
    float target = RENDERSIZE.x / max(RENDERSIZE.y, 1.0);
    vec2 s = target > SENSOR_ASPECT
        ? vec2(target / SENSOR_ASPECT, 1.0)
        : vec2(1.0, SENSOR_ASPECT / target);
    return (p - 0.5) * s + 0.5;
}

bool in_frame(vec2 p) {
    return p.x >= 0.0 && p.x <= 1.0 && p.y >= 0.0 && p.y <= 1.0;
}

float mask_at(vec2 p) {
    vec2 s = sensor_uv(p);
    if (!in_frame(s)) return 0.0;
    return texture(sampler2D(mask, texSampler), s).r;
}

float depth_at(vec2 p) {
    vec2 s = sensor_uv(p);
    if (!in_frame(s)) return 0.0;
    return texture(sampler2D(depth, texSampler), s).r;
}

vec2 motion_at(vec2 p) {
    vec2 s = sensor_uv(p);
    if (!in_frame(s)) return vec2(0.0);
    return texture(sampler2D(motion, texSampler), s).xy;
}

// Relief height for the parallax camera: nearer subjects stand taller, empty
// space and unresolved texels lie flat at the back plane.
//
// One texture fetch, deliberately. This runs once per parallax march step, so
// folding in a second `mask` fetch here would cost 32 fetches per pixel before
// anything is shaded. `depth` is already 0 outside the range, which is the same
// information the mask carries.
float height_at(vec2 p) {
    float d = depth_at(p);
    return d > 0.0 ? 1.0 - d : 0.0;
}

vec2 vel_at(vec2 p) {
    return texture(sampler2D(velocity, texSampler), clamp(p, 0.0, 1.0)).xy;
}

vec3 dye_at(vec2 p) {
    return texture(sampler2D(dye, texSampler), clamp(p, 0.0, 1.0)).rgb;
}

// ---- Palettes: three immiscible dye hues per look ----
//
// Three distinct pigments rather than one ramp is what gives a real liquid light
// show its colour complexity — the dyes advect independently and mix where they
// meet, so the field never collapses to a single gradient.

void palette_dyes(float which, out vec3 a, out vec3 b, out vec3 c) {
    float w = clamp(which, 0.0, 3.0);
    // Classic: cobalt / vermilion / amber
    vec3 a0 = vec3(0.12, 0.30, 0.95), b0 = vec3(0.95, 0.22, 0.10), c0 = vec3(1.00, 0.78, 0.20);
    // Acid: emerald / chartreuse / magenta
    vec3 a1 = vec3(0.00, 0.85, 0.45), b1 = vec3(0.80, 0.95, 0.10), c1 = vec3(1.00, 0.10, 0.80);
    // Sunset: violet / coral / gold
    vec3 a2 = vec3(0.45, 0.10, 0.70), b2 = vec3(1.00, 0.38, 0.28), c2 = vec3(1.00, 0.85, 0.45);
    // Deep: ink / teal / ice
    vec3 a3 = vec3(0.05, 0.10, 0.45), b3 = vec3(0.05, 0.70, 0.80), c3 = vec3(0.80, 0.97, 1.00);

    a = mix(a0, a1, clamp(w, 0.0, 1.0));
    a = mix(a, a2, clamp(w - 1.0, 0.0, 1.0));
    a = mix(a, a3, clamp(w - 2.0, 0.0, 1.0));
    b = mix(b0, b1, clamp(w, 0.0, 1.0));
    b = mix(b, b2, clamp(w - 1.0, 0.0, 1.0));
    b = mix(b, b3, clamp(w - 2.0, 0.0, 1.0));
    c = mix(c0, c1, clamp(w, 0.0, 1.0));
    c = mix(c, c2, clamp(w - 1.0, 0.0, 1.0));
    c = mix(c, c3, clamp(w - 2.0, 0.0, 1.0));
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
    float decay = clamp(persistence, 0.5, 0.995);

    // ── Pass 0: velocity — self-advect, ambient churn, body force ────────────
    if (PASSINDEX == 0) {
        float mx = mask_at(uv + vec2(GRAD_STEP, 0.0)) - mask_at(uv - vec2(GRAD_STEP, 0.0));
        float my = mask_at(uv + vec2(0.0, GRAD_STEP)) - mask_at(uv - vec2(0.0, GRAD_STEP));
        vec2 mgrad = vec2(mx, my);

        vec2 v = vel_at(uv);
        // Semi-Lagrangian self-advection: trace back along the field.
        vec2 back = uv - v * texel * 60.0 * (0.2 + flow_speed);
        v = mix(v, vel_at(back), 0.85);

        // Ambient convection — the plate is always moving, even in an empty room.
        v += curl_flow(uv * blob_scale * 3.0, t * 0.35) * agitation * 0.08;
        // Bodies displace fluid outward along the silhouette normal ...
        v += mgrad * silhouette_force * 4.0;
        // ... and drag it along when they actually move, which is what makes the
        // dye swirl on a gesture rather than merely pooling around a shape.
        v += motion_at(uv) * motion_force * 0.05;

        v *= decay;
        fragColor = vec4(v + keep, 0.0, 1.0);
        return;
    }

    // ── Pass 1: dye — three pigments advected, emitted from different cues ───
    if (PASSINDEX == 1) {
        float mx = mask_at(uv + vec2(GRAD_STEP, 0.0)) - mask_at(uv - vec2(GRAD_STEP, 0.0));
        float my = mask_at(uv + vec2(0.0, GRAD_STEP)) - mask_at(uv - vec2(0.0, GRAD_STEP));
        float edge = clamp(length(vec2(mx, my)), 0.0, 1.0);
        float m = mask_at(uv);
        float d = depth_at(uv);

        vec2 v = vel_at(uv);
        vec2 back = uv - v * texel * 60.0 * (0.2 + flow_speed);
        vec3 col = dye_at(back);

        // Diffuse so the pigments read as fluid rather than as streaks.
        float sp = 1.0 + dye_spread * 5.0;
        vec3 blur = dye_at(back + vec2(texel.x * sp, 0.0))
                  + dye_at(back - vec2(texel.x * sp, 0.0))
                  + dye_at(back + vec2(0.0, texel.y * sp))
                  + dye_at(back - vec2(0.0, texel.y * sp));
        col = mix(col, blur * 0.25, dye_spread * 0.4);

        // Break every emission up with a drifting FBM so no pigment ever lays
        // down as a flat slab — this is most of what stops the field reading as
        // two colours.
        float grain = fbm(uv * blob_scale * 6.0 + vec2(t * 0.25, -t * 0.18), 3);
        float band = fbm(uv * blob_scale * 2.0 - t * 0.12, 3);

        // Pigment A: the silhouette contour.
        float emit_a = edge * (0.55 + 0.45 * grain);
        // Pigment B: the body interior, banded by distance so a subject is not a
        // solid fill — nearer parts lay down more.
        float near = 1.0 - clamp(d, 0.0, 1.0);
        float emit_b = m * (0.25 + 0.75 * band) * mix(0.35, 1.0, near * depth_tint);
        // Pigment C: movement only, so gestures leave coloured wakes.
        float emit_c = clamp(length(motion_at(uv)) * 0.35, 0.0, 1.0) * (0.4 + 0.6 * grain);

        // Accumulate rather than clamp to a ceiling: a `max()` against the emit
        // pins the field at 1.0 wherever a body is and flattens every gradient.
        col.r = col.r * decay + emit_a * 0.28;
        col.g = col.g * decay + emit_b * 0.16;
        col.b = col.b * decay + emit_c * 0.30;

        fragColor = vec4(clamp(col, 0.0, 2.0) + keep, 1.0);
        return;
    }

    // ── Final pass: 3D parallax camera, then the liquid-light look ───────────

    // Orbit the depth relief. Parallax occlusion marching against the height
    // field gives real occlusion — nearer bodies hide what is behind them as the
    // camera swings — which a flat UV shear cannot.
    vec2 uv0 = (uv - 0.5) / max(zoom, 0.05) + 0.5;
    vec3 view = normalize(vec3((orbit_yaw - 0.5) * 2.0, (orbit_pitch - 0.5) * 2.0, 1.0));
    vec2 ray = view.xy / max(abs(view.z), 0.3) * relief * 0.35;

    vec2 p = uv0;
    if (relief > 0.001) {
        vec2 step_uv = ray / float(POM_STEPS);
        float layer = 1.0;
        float dl = 1.0 / float(POM_STEPS);
        float h = height_at(p);
        for (int i = 0; i < POM_STEPS; i++) {
            if (layer <= h) break;
            p += step_uv;
            layer -= dl;
            h = height_at(p);
        }
    }

    vec3 col3 = dye_at(p);

    // Soft focus: an out-of-focus overhead projector never resolves an edge.
    float blurAmt = focus_soft * 3.0;
    vec3 soft = dye_at(p + vec2(texel.x, texel.y) * blurAmt)
              + dye_at(p - vec2(texel.x, texel.y) * blurAmt)
              + dye_at(p + vec2(texel.x, -texel.y) * blurAmt)
              + dye_at(p - vec2(texel.x, -texel.y) * blurAmt);
    col3 = mix(col3, soft * 0.25, focus_soft * 0.6);

    vec3 pa, pb, pc;
    palette_dyes(palette, pa, pb, pc);

    float thickness = col3.r + col3.g + col3.b;
    vec3 col = col3.r * pa + col3.g * pb + col3.b * pc;

    // Thin-film interference: the oily rainbow sheen where the dye film varies
    // in thickness. This is the other half of the colour complexity — it shifts
    // hue continuously with thickness instead of stepping through a ramp.
    vec3 irid = 0.5 + 0.5 * cos(6.28318 * (thickness * 1.8 + vec3(0.0, 0.33, 0.67))
                                + fbm(p * blob_scale * 4.0 + t * 0.2, 3) * 3.0);
    col = mix(col, col * irid * 1.6, iridescence * smoothstep(0.02, 0.6, thickness));

    // Wet rim where pigments meet.
    float tx = dye_at(p + vec2(texel.x, 0.0)).r - dye_at(p - vec2(texel.x, 0.0)).r;
    float ty = dye_at(p + vec2(0.0, texel.y)).r - dye_at(p - vec2(0.0, texel.y)).r;
    col += pc * length(vec2(tx, ty)) * edge_glow * 8.0;

    // The literal fluid outline of the person, riding the live mask contour at
    // the parallax-resolved position.
    float rmx = mask_at(p + vec2(GRAD_STEP, 0.0)) - mask_at(p - vec2(GRAD_STEP, 0.0));
    float rmy = mask_at(p + vec2(0.0, GRAD_STEP)) - mask_at(p - vec2(0.0, GRAD_STEP));
    float redge = clamp(length(vec2(rmx, rmy)), 0.0, 1.0);
    col += mix(pb, pc, 0.5) * smoothstep(0.3 - outline_width * 0.28, 0.36, redge)
         * (0.3 + edge_glow * 0.5);

    col *= color_intensity;
    // Warm tungsten cast of an overhead projector lamp.
    col *= mix(vec3(1.0), vec3(1.12, 0.98, 0.82), warmth);
    // Round lamp falloff.
    vec2 cc = uv - 0.5;
    col *= mix(1.0, 1.0 - dot(cc, cc) * 2.2, vignette_amt);
    // No dye means no light — an empty field must read as black, not as a flat
    // colour that looks like a working shader with nothing in front of it.
    col *= smoothstep(0.0, 0.05, thickness);

    fragColor = vec4(max(col + keep, 0.0), 1.0);
}
