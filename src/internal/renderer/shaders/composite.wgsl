// Composite blend shader: blends the source layer over the composite so far
// using the blend_mode uniform. CompositeParams is 32 bytes (8 x f32).

struct CompositeParams {
    opacity: f32,
    blend_mode: u32,
    uv_scale: vec2<f32>,
    uv_offset: vec2<f32>,
    // 1 = source is premultiplied-alpha (un-premultiply before blend math); 0 = straight.
    premultiplied: u32,
    _pad: f32,
}

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var dest_texture: texture_2d<f32>;

@group(0) @binding(3)
var<uniform> params: CompositeParams;

const EPSILON: f32 = 0.001;

// sRGB OETF / EOTF, extended beyond [0,1].
//
// Overlay, Soft Light and Hard Light pivot at 0.5 in gamma-encoded space
// (linear 0.214), so only those modes convert in and out of it.
//
// Above 1.0 the curve continues as a pure power so HDR values survive, and it
// is mirrored through the origin for negatives (Linear Burn goes below zero),
// keeping it monotonic.
fn srgb_encode(v: vec3<f32>) -> vec3<f32> {
    let a = abs(v);
    let hi = 1.055 * pow(a, vec3<f32>(1.0 / 2.4)) - vec3<f32>(0.055);
    let lo = a * 12.92;
    let e = select(hi, lo, a <= vec3<f32>(0.0031308));
    return select(-e, e, v >= vec3<f32>(0.0));
}

fn srgb_decode(v: vec3<f32>) -> vec3<f32> {
    let a = abs(v);
    let hi = pow((a + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4));
    let lo = a / 12.92;
    let e = select(hi, lo, a <= vec3<f32>(0.04045));
    return select(-e, e, v >= vec3<f32>(0.0));
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // Sample source with UV transform (scaling modes).
    let source_uv = uv * params.uv_scale + params.uv_offset;

    var src: vec4<f32>;
    if (source_uv.x < 0.0 || source_uv.x > 1.0 || source_uv.y < 0.0 || source_uv.y > 1.0) {
        src = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    } else {
        src = textureSample(source_texture, texture_sampler, source_uv);
    }

    // The blend math and OVER below need straight alpha, so un-premultiply
    // premultiplied sources (channel composites). a ≈ 0 early-outs below.
    if (params.premultiplied == 1u && src.a > EPSILON) {
        src = vec4<f32>(src.rgb / src.a, src.a);
    }

    // Destination at raw UV (no transform).
    let dst = textureSample(dest_texture, texture_sampler, uv);

    let src_a = src.a * params.opacity;

    // A fully transparent source contributes nothing.
    if (src_a <= 0.0) {
        return dst;
    }

    var blended: vec3<f32>;
    let mode = params.blend_mode;

    // Pivot modes (Overlay=5, Soft Light=6, Hard Light=7) use gamma-encoded
    // operands. Encoding follows the un-premultiply: a non-linear curve must
    // not see coverage-weighted values.
    let pivot_mode = (mode == 5u || mode == 6u || mode == 7u);
    var s = src.rgb;
    var d = dst.rgb;
    if (pivot_mode) {
        s = srgb_encode(s);
        d = srgb_encode(d);
    }

    if (mode == 0u) {
        // Normal (alpha-over)
        blended = s;
    } else if (mode == 1u) {
        // Add (unclamped — values > 1.0 preserved for downstream tonemap)
        blended = s + d;
    } else if (mode == 2u) {
        // Subtract (floor at 0, no ceiling)
        blended = max(d - s, vec3<f32>(0.0));
    } else if (mode == 3u) {
        // Multiply
        blended = s * d;
    } else if (mode == 4u) {
        // Screen
        blended = vec3<f32>(1.0) - (vec3<f32>(1.0) - s) * (vec3<f32>(1.0) - d);
    } else if (mode == 5u) {
        // Overlay (conditional per channel)
        blended = vec3<f32>(
            select(1.0 - 2.0 * (1.0 - s.r) * (1.0 - d.r), 2.0 * s.r * d.r, d.r < 0.5),
            select(1.0 - 2.0 * (1.0 - s.g) * (1.0 - d.g), 2.0 * s.g * d.g, d.g < 0.5),
            select(1.0 - 2.0 * (1.0 - s.b) * (1.0 - d.b), 2.0 * s.b * d.b, d.b < 0.5),
        );
    } else if (mode == 6u) {
        // Soft Light (Pegtop)
        blended = (vec3<f32>(1.0) - 2.0 * s) * d * d + 2.0 * s * d;
    } else if (mode == 7u) {
        // Hard Light (conditional per channel)
        blended = vec3<f32>(
            select(1.0 - 2.0 * (1.0 - s.r) * (1.0 - d.r), 2.0 * s.r * d.r, s.r < 0.5),
            select(1.0 - 2.0 * (1.0 - s.g) * (1.0 - d.g), 2.0 * s.g * d.g, s.g < 0.5),
            select(1.0 - 2.0 * (1.0 - s.b) * (1.0 - d.b), 2.0 * s.b * d.b, s.b < 0.5),
        );
    } else if (mode == 8u) {
        // Color Dodge: dst / (1 - src), unclamped for HDR accumulation
        blended = vec3<f32>(
            d.r / max(1.0 - s.r, EPSILON),
            d.g / max(1.0 - s.g, EPSILON),
            d.b / max(1.0 - s.b, EPSILON),
        );
    } else if (mode == 9u) {
        // Color Burn: 1 - (1-dst)/src, clamped
        blended = clamp(vec3<f32>(
            1.0 - (1.0 - d.r) / max(s.r, EPSILON),
            1.0 - (1.0 - d.g) / max(s.g, EPSILON),
            1.0 - (1.0 - d.b) / max(s.b, EPSILON),
        ), vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (mode == 10u) {
        // Difference
        blended = abs(s - d);
    } else if (mode == 11u) {
        // Exclusion
        blended = s + d - 2.0 * s * d;
    } else if (mode == 12u) {
        // Darken
        blended = min(s, d);
    } else if (mode == 13u) {
        // Lighten
        blended = max(s, d);
    } else if (mode == 14u) {
        // Linear Burn: src + dst - 1 (allow negative, floor at tonemap)
        blended = s + d - vec3<f32>(1.0);
    } else {
        // Fallback: Normal
        blended = s;
    }

    // Back to linear light before the OVER composite below, which mixes
    // against the untouched linear `dst`.
    if (pivot_mode) {
        blended = srgb_decode(blended);
    }

    // Standard OVER.
    let result_rgb = mix(dst.rgb, blended, src_a);
    let result_a = src_a + dst.a * (1.0 - src_a);

    return vec4<f32>(result_rgb, result_a);
}
