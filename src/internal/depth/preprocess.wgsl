// Depth-sensor shader preprocessor: turns a raw R16Uint depth stream into
// fields for ISF shaders.
//
// Three fullscreen passes at sensor resolution:
//   fs_normalize: clip, hole-fill, normalize, temporally smooth; emit depth + motion
//   fs_mask:      silhouette occupancy with a feathered edge
//   fs_color:     mirrored passthrough of the sensor's color stream
//
// Normalized depth `0.0` means invalid: out of range or unresolved.

struct Params {
    // near_mm, far_mm, 1/(far-near) mm, smoothing (EMA factor 0..1)
    range: vec4<f32>,
    // width, height, hole_fill radius (texels), mask_feather radius (texels)
    dims: vec4<f32>,
    // motion_gain, dt seconds, mirror (0 or 1), unused
    misc: vec4<f32>,
};

@group(0) @binding(0) var<uniform> P: Params;

const INVALID: f32 = 0.0;
const MAX_RADIUS: i32 = 8;

// Deadband on the frame-to-frame depth delta, in normalized units. History is
// R16Float (ULP ~1e-3 near 1.0), so a still scene leaves a rounding residual
// that the divide by `dt` amplifies into visible motion. Set above that floor
// and below real movement.
const MOTION_DEADBAND: f32 = 4.0e-3;

// Fullscreen triangle. No vertex buffer.
@vertex
fn vs_fullscreen(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    let x = f32(i32(vi) / 2) * 4.0 - 1.0;
    let y = f32(i32(vi) & 1) * 4.0 - 1.0;
    return vec4<f32>(x, y, 0.0, 1.0);
}

fn dims_i() -> vec2<i32> {
    return vec2<i32>(i32(P.dims.x), i32(P.dims.y));
}

// Map a fragment coordinate to a source texel, applying the mirror flip.
fn src_coord(frag: vec2<f32>) -> vec2<i32> {
    let d = dims_i();
    var x = i32(frag.x);
    if (P.misc.z > 0.5) {
        x = d.x - 1 - x;
    }
    return vec2<i32>(clamp(x, 0, d.x - 1), clamp(i32(frag.y), 0, d.y - 1));
}

// ── Pass A: normalize + motion ───────────────────────────────────────────────

@group(0) @binding(1) var depth_src: texture_2d<u32>;
@group(0) @binding(2) var prev_depth: texture_2d<f32>;

// Normalizes a raw millimeter reading to 0..1 across [near, far].
// Returns INVALID for zero (unresolved) and out-of-range samples.
fn normalize_mm(mm: f32) -> f32 {
    if (mm <= 0.0 || mm < P.range.x || mm > P.range.y) {
        return INVALID;
    }
    // Clamp keeps the output finite when near == far.
    return clamp((mm - P.range.x) * P.range.z, 1.0e-4, 1.0);
}

fn load_norm(c: vec2<i32>) -> f32 {
    let d = dims_i();
    let cc = vec2<i32>(clamp(c.x, 0, d.x - 1), clamp(c.y, 0, d.y - 1));
    return normalize_mm(f32(textureLoad(depth_src, cc, 0).r));
}

// Fills an unresolved texel from the nearest valid sample within `radius`.
// Kinect v1 holes are IR shadows beside limbs, so the adjacent surface is the
// right value; a mean would blur the silhouette edge into the background.
fn hole_fill(c: vec2<i32>, radius: i32) -> f32 {
    var best = INVALID;
    var best_d2 = 1.0e9;
    for (var dy = -MAX_RADIUS; dy <= MAX_RADIUS; dy = dy + 1) {
        if (abs(dy) > radius) { continue; }
        for (var dx = -MAX_RADIUS; dx <= MAX_RADIUS; dx = dx + 1) {
            if (abs(dx) > radius) { continue; }
            let d2 = f32(dx * dx + dy * dy);
            if (d2 >= best_d2) { continue; }
            let v = load_norm(c + vec2<i32>(dx, dy));
            if (v > INVALID) {
                best = v;
                best_d2 = d2;
            }
        }
    }
    return best;
}

struct NormalizeOut {
    @location(0) depth: f32,
    // Same value, kept as history for next frame's motion.
    @location(1) history: f32,
    @location(2) motion: vec2<f32>,
};

@fragment
fn fs_normalize(@builtin(position) pos: vec4<f32>) -> NormalizeOut {
    let sc = src_coord(pos.xy);
    let radius = i32(round(P.dims.z));

    var d = normalize_mm(f32(textureLoad(depth_src, sc, 0).r));
    if (d <= INVALID && radius > 0) {
        d = hole_fill(sc, radius);
    }

    // History is in output space, so it is read unmirrored.
    let hc = vec2<i32>(i32(pos.x), i32(pos.y));
    let prev = textureLoad(prev_depth, hc, 0).r;

    var smoothed = d;
    // Blend only when both samples are valid, so a subject entering the frame
    // appears at once instead of fading in from the background.
    if (d > INVALID && prev > INVALID) {
        smoothed = mix(d, prev, clamp(P.range.w, 0.0, 0.99));
    }

    var motion = vec2<f32>(0.0, 0.0);
    if (smoothed > INVALID && prev > INVALID && abs(smoothed - prev) > MOTION_DEADBAND) {
        // Central-difference gradient in UV units.
        let px = 1.0 / max(P.dims.x, 1.0);
        let py = 1.0 / max(P.dims.y, 1.0);
        let gx = (load_norm(sc + vec2<i32>(1, 0)) - load_norm(sc - vec2<i32>(1, 0))) / (2.0 * px);
        let gy = (load_norm(sc + vec2<i32>(0, 1)) - load_norm(sc - vec2<i32>(0, 1))) / (2.0 * py);
        let g = vec2<f32>(gx, gy);
        let gl = length(g);
        if (gl > 1.0e-5) {
            let rate = (smoothed - prev) / max(P.misc.y, 1.0e-4);
            motion = (g / gl) * rate * P.misc.x;
        }
    }

    var out: NormalizeOut;
    out.depth = smoothed;
    out.history = smoothed;
    out.motion = motion;
    return out;
}

// ── Pass B: silhouette mask ──────────────────────────────────────────────────

@group(0) @binding(3) var depth_norm: texture_2d<f32>;
@group(0) @binding(5) var prev_mask: texture_2d<f32>;

struct MaskOut {
    @location(0) mask: f32,
    // Kept as history so the next frame can decay against it.
    @location(1) history: f32,
};

@fragment
fn fs_mask(@builtin(position) pos: vec4<f32>) -> MaskOut {
    // `depth` is already in output space and mirrored by pass A.
    let c = vec2<i32>(i32(pos.x), i32(pos.y));
    let d = dims_i();
    let radius = i32(round(P.dims.w));

    var occupancy = 0.0;
    if (radius <= 0) {
        occupancy = select(0.0, 1.0, textureLoad(depth_norm, c, 0).r > INVALID);
        return finish_mask(c, occupancy);
    }

    var sum = 0.0;
    var count = 0.0;
    for (var dy = -MAX_RADIUS; dy <= MAX_RADIUS; dy = dy + 1) {
        if (abs(dy) > radius) { continue; }
        for (var dx = -MAX_RADIUS; dx <= MAX_RADIUS; dx = dx + 1) {
            if (abs(dx) > radius) { continue; }
            let cc = vec2<i32>(
                clamp(c.x + dx, 0, d.x - 1),
                clamp(c.y + dy, 0, d.y - 1),
            );
            sum = sum + select(0.0, 1.0, textureLoad(depth_norm, cc, 0).r > INVALID);
            count = count + 1.0;
        }
    }
    return finish_mask(c, sum / max(count, 1.0));
}

// Temporal hysteresis on the silhouette. Kinect depth validity flickers along
// body edges, which spatial feathering can't hide. Rising edges are instant;
// falling edges decay, so a texel that blinks out stays filled.
fn finish_mask(c: vec2<i32>, occupancy: f32) -> MaskOut {
    let prev = textureLoad(prev_mask, c, 0).r;
    // `smoothing` also sets how long a dropped texel is held.
    let decay = mix(0.35, 0.02, clamp(P.range.w, 0.0, 0.99));
    let held = max(occupancy, prev - decay);

    var out: MaskOut;
    out.mask = held;
    out.history = held;
    return out;
}

// ── Pass C: color passthrough ────────────────────────────────────────────────

@group(0) @binding(4) var rgb_src: texture_2d<f32>;

@fragment
fn fs_color(@builtin(position) pos: vec4<f32>) -> @location(0) vec4<f32> {
    return textureLoad(rgb_src, src_coord(pos.xy), 0);
}
