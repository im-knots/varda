// Layer fold: blends up to FOLD_WIDTH layers in one pass, bottom to top, in
// registers. FoldParams is 160 bytes and matches the struct in layers.rs.

const FOLD_WIDTH: u32 = 8u;

struct FoldLayer {
    opacity: f32,
    blend_mode: u32,
    // 1 = premultiplied alpha; 0 = straight.
    premultiplied: u32,
    _pad: u32,
}

struct FoldParams {
    // What the first layer is drawn over when nothing is below.
    base: vec4<f32>,
    layer_count: u32,
    // 1 = start from below_texture; 0 = start from base.
    has_below: u32,
    _pad: vec2<u32>,
    layers: array<FoldLayer, FOLD_WIDTH>,
}

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var layer_0: texture_2d<f32>;
@group(0) @binding(2)
var layer_1: texture_2d<f32>;
@group(0) @binding(3)
var layer_2: texture_2d<f32>;
@group(0) @binding(4)
var layer_3: texture_2d<f32>;
@group(0) @binding(5)
var layer_4: texture_2d<f32>;
@group(0) @binding(6)
var layer_5: texture_2d<f32>;
@group(0) @binding(7)
var layer_6: texture_2d<f32>;
@group(0) @binding(8)
var layer_7: texture_2d<f32>;

// The composite so far, when an earlier pass drew it.
@group(0) @binding(9)
var below_texture: texture_2d<f32>;

@group(0) @binding(10)
var<uniform> params: FoldParams;

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

fn sample_layer(index: u32, uv: vec2<f32>) -> vec4<f32> {
    switch (index) {
        case 0u: { return textureSampleLevel(layer_0, texture_sampler, uv, 0.0); }
        case 1u: { return textureSampleLevel(layer_1, texture_sampler, uv, 0.0); }
        case 2u: { return textureSampleLevel(layer_2, texture_sampler, uv, 0.0); }
        case 3u: { return textureSampleLevel(layer_3, texture_sampler, uv, 0.0); }
        case 4u: { return textureSampleLevel(layer_4, texture_sampler, uv, 0.0); }
        case 5u: { return textureSampleLevel(layer_5, texture_sampler, uv, 0.0); }
        case 6u: { return textureSampleLevel(layer_6, texture_sampler, uv, 0.0); }
        default: { return textureSampleLevel(layer_7, texture_sampler, uv, 0.0); }
    }
}

// The first layer over the base. It ignores its blend mode. A premultiplied
// layer keeps its color at every alpha, so it is not un-premultiplied here.
fn over_base(layer: FoldLayer, src: vec4<f32>) -> vec4<f32> {
    let a = src.a * layer.opacity;
    var rgb = src.rgb * a;
    if (layer.premultiplied == 1u) {
        rgb = src.rgb * layer.opacity;
    }
    return vec4<f32>(rgb + params.base.rgb * (1.0 - a), a + params.base.a * (1.0 - a));
}

fn blend_layer(layer: FoldLayer, sampled: vec4<f32>, dst: vec4<f32>) -> vec4<f32> {
    // The blend math and OVER below need straight alpha, so un-premultiply
    // premultiplied sources (channel composites). a ≈ 0 early-outs below.
    var src = sampled;
    if (layer.premultiplied == 1u && src.a > EPSILON) {
        src = vec4<f32>(src.rgb / src.a, src.a);
    }

    let src_a = src.a * layer.opacity;

    // A fully transparent source contributes nothing.
    if (src_a <= 0.0) {
        return dst;
    }

    var blended: vec3<f32>;
    let mode = layer.blend_mode;

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

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    var composite: vec4<f32>;
    var next = 0u;
    if (params.has_below == 1u) {
        composite = textureSampleLevel(below_texture, texture_sampler, uv, 0.0);
    } else {
        composite = over_base(params.layers[0], sample_layer(0u, uv));
        next = 1u;
    }
    for (var index = next; index < params.layer_count; index++) {
        composite = blend_layer(params.layers[index], sample_layer(index, uv), composite);
    }
    return composite;
}
