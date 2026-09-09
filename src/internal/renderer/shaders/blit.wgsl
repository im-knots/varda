// Blit shader - copies a texture with opacity, UV transform, and output rotation
// BlitParams is 48 bytes and follows WGSL's 16-byte struct alignment.

struct BlitParams {
    opacity: f32,
    rotation: u32,
    uv_scale: vec2<f32>,
    uv_offset: vec2<f32>,
    // 1 = source is premultiplied-alpha (scale rgb+a by opacity); 0 = straight (scale alpha only).
    premultiplied: u32,
    // 1 = apply the sRGB transfer function on output. Used when blitting
    // linear-light content into a NON-sRGB target that will be sampled by a
    // consumer expecting gamma-encoded data (egui previews). Leave 0 for sRGB
    // targets, where the hardware does the encode on write.
    srgb_encode: u32,
    // Number of destination code intervals (255 or 1023).
    quantization_levels: f32,
    // Static destination-aware dither toggle.
    dither_enabled: u32,
    // Transfer at this boundary: 0 = SDR (sRGB), 1 = HDR10 (ST 2084 PQ),
    // 2 = HLG, 3 = EDR (linear, no encode).
    transfer: u32,
    // Peak luminance in cd/m² for the PQ path. Unused when transfer == 0.
    peak_nits: f32,
    _padding: vec2<u32>,
}

// ── HDR encode (see /spec/hdr-color-management.md, mirrored from hdr.rs) ──

// ITU-R BT.2408 HDR Reference White. Linear 1.0 maps here, so existing content
// keeps its brightness and the range above 1.0 becomes the HDR gain.
const HDR_REFERENCE_WHITE_NITS: f32 = 203.0;
const PQ_MAX_NITS: f32 = 10000.0;
const PQ_M1: f32 = 0.1593017578125;
const PQ_M2: f32 = 78.84375;
const PQ_C1: f32 = 0.8359375;
const PQ_C2: f32 = 18.8515625;
const PQ_C3: f32 = 18.6875;

// SMPTE ST 2084 inverse EOTF: absolute cd/m² to a [0,1] PQ code.
fn pq_from_nits(nits: vec3<f32>) -> vec3<f32> {
    let y = clamp(nits / PQ_MAX_NITS, vec3<f32>(0.0), vec3<f32>(1.0));
    let ym = pow(y, vec3<f32>(PQ_M1));
    return pow((PQ_C1 + PQ_C2 * ym) / (1.0 + PQ_C3 * ym), vec3<f32>(PQ_M2));
}

// Linear BT.709 to linear BT.2020 primaries. Applied to linear values before the
// transfer encode, never after it.
fn bt2020_from_rec709(rgb: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        dot(rgb, vec3<f32>(0.6274039, 0.32928304, 0.04331306)),
        dot(rgb, vec3<f32>(0.06909729, 0.9195401, 0.01136263)),
        dot(rgb, vec3<f32>(0.01639144, 0.0880133, 0.8955953)),
    );
}

// Scene-linear Rec.709 to a PQ-encoded BT.2020 code, anchored at reference white
// and clamped to the output's configured peak so declared mastering metadata
// stays true by construction.
fn hdr10_from_linear(rgb: vec3<f32>, peak_nits: f32) -> vec3<f32> {
    let wide = bt2020_from_rec709(max(rgb, vec3<f32>(0.0)));
    let nits = min(wide * HDR_REFERENCE_WHITE_NITS, vec3<f32>(peak_nits));
    return pq_from_nits(nits);
}

// ── HLG (ARIB STD-B67 / BT.2100), mirrored from hdr.rs ──

const HLG_A: f32 = 0.17883277;
const HLG_B: f32 = 0.28466892;
const HLG_C: f32 = 0.55991073;
// Scene-linear value that encodes to 75% signal, which BT.2408 defines as HDR
// Reference White. Varda's linear 1.0 lands here, the same anchor PQ uses.
const HLG_REFERENCE_WHITE_SIGNAL: f32 = 0.2649631;

fn hlg_oetf(e_in: vec3<f32>) -> vec3<f32> {
    let e = clamp(e_in, vec3<f32>(0.0), vec3<f32>(1.0));
    let lo = sqrt(3.0 * e);
    let hi = HLG_A * log(max(12.0 * e - HLG_B, vec3<f32>(1e-6))) + HLG_C;
    return select(hi, lo, e <= vec3<f32>(1.0 / 12.0));
}

// Scene-linear Rec.709 to an HLG-encoded BT.2020 signal. Relative rather than
// absolute, so there is no peak: 1.0 is the display's nominal peak, and values
// past the top of the range clamp.
fn hlg_from_linear(rgb: vec3<f32>) -> vec3<f32> {
    let wide = bt2020_from_rec709(max(rgb, vec3<f32>(0.0)));
    return hlg_oetf(wide * HLG_REFERENCE_WHITE_SIGNAL);
}

// Linear → sRGB (IEC 61966-2-1). Mirrors what an *UnormSrgb render target does
// in hardware, for the cases where we must do it explicitly.
fn gamma_from_linear_rgb(rgb: vec3<f32>) -> vec3<f32> {
    let c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let lower = c * 12.92;
    let higher = 1.055 * pow(c, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, c < vec3<f32>(0.0031308));
}

fn linear_from_gamma_rgb(rgb: vec3<f32>) -> vec3<f32> {
    let c = clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    let lower = c / 12.92;
    let higher = pow((c + 0.055) / 1.055, vec3<f32>(2.4));
    return select(higher, lower, c < vec3<f32>(0.04045));
}

// Deterministic integer hash. It is anchored to destination pixels, so a
// stationary image receives a stationary dither pattern.
fn dither_hash(pixel: vec2<u32>, salt: u32) -> f32 {
    var value = pixel.x * 0x9e3779b9u;
    value = value ^ (pixel.y * 0x85ebca6bu);
    value = value ^ salt;
    value = value ^ (value >> 16u);
    value = value * 0x7feb352du;
    value = value ^ (value >> 15u);
    value = value * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return f32(value & 0x00ffffffu) / 16777215.0 - 0.5;
}

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: BlitParams;

@fragment
fn fs_main(
    @location(0) uv: vec2<f32>,
    @builtin(position) position: vec4<f32>,
) -> @location(0) vec4<f32> {
    // Apply rotation to UVs before sampling
    var rotated_uv = uv;
    switch (params.rotation) {
        case 1u: {
            // 90° CW: (u,v) → (v, 1-u)
            rotated_uv = vec2<f32>(uv.y, 1.0 - uv.x);
        }
        case 2u: {
            // 180°: (u,v) → (1-u, 1-v)
            rotated_uv = vec2<f32>(1.0 - uv.x, 1.0 - uv.y);
        }
        case 3u: {
            // 270° CW: (u,v) → (1-v, u)
            rotated_uv = vec2<f32>(1.0 - uv.y, uv.x);
        }
        default: {
            // 0°: no rotation
        }
    }

    // Apply UV transform for scaling modes
    let source_uv = rotated_uv * params.uv_scale + params.uv_offset;

    // Clamp to [0,1] — pixels outside the source are black (for Fit/Center modes)
    if (source_uv.x < 0.0 || source_uv.x > 1.0 || source_uv.y < 0.0 || source_uv.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, params.opacity);
    }

    var color = textureSample(source_texture, texture_sampler, source_uv);
    if (params.premultiplied == 1u) {
        // Premultiplied source: scale rgb and alpha together so opacity dims the
        // channel uniformly. Paired with PREMULTIPLIED_ALPHA_BLENDING on the target.
        color *= params.opacity;
    } else {
        // Straight source: scale coverage only (rgb is the un-premultiplied colour).
        color.a *= params.opacity;
    }
    // EDR writes linear values to an extended-range surface: no transfer, no
    // gamut matrix, no quantization. `1.0` is the display's SDR white, which is
    // what Varda's linear 1.0 already means, so the output transform's result
    // goes out untouched and the compositor clips at its own headroom.
    // See /spec/hdr-edr-display.md.
    if (params.transfer == 3u) {
        return color;
    }
    if (params.transfer != 0u) {
        // The HDR curves already distribute codes perceptually, so the dither
        // amplitude is one destination LSB of the encoded signal.
        var encoded: vec3<f32>;
        if (params.transfer == 2u) {
            encoded = hlg_from_linear(color.rgb);
        } else {
            encoded = hdr10_from_linear(color.rgb, params.peak_nits);
        }
        if (params.dither_enabled == 1u && params.quantization_levels > 0.0) {
            let pixel = vec2<u32>(position.xy);
            let noise = vec3<f32>(
                dither_hash(pixel, 0xa511e9b3u),
                dither_hash(pixel, 0x63d83595u),
                dither_hash(pixel, 0xc2b2ae35u),
            ) / params.quantization_levels;
            encoded = clamp(encoded + noise, vec3<f32>(0.0), vec3<f32>(1.0));
        }
        return vec4<f32>(encoded, color.a);
    }
    if (params.dither_enabled == 1u && params.quantization_levels > 0.0) {
        var encoded = gamma_from_linear_rgb(color.rgb);
        let pixel = vec2<u32>(position.xy);
        let noise = vec3<f32>(
            dither_hash(pixel, 0xa511e9b3u),
            dither_hash(pixel, 0x63d83595u),
            dither_hash(pixel, 0xc2b2ae35u),
        ) / params.quantization_levels;
        encoded = clamp(encoded + noise, vec3<f32>(0.0), vec3<f32>(1.0));
        if (params.srgb_encode == 1u) {
            color = vec4<f32>(encoded, color.a);
        } else {
            // The sRGB render target applies the forward transfer on write.
            color = vec4<f32>(linear_from_gamma_rgb(encoded), color.a);
        }
    } else if (params.srgb_encode == 1u) {
        color = vec4<f32>(gamma_from_linear_rgb(color.rgb), color.a);
    }
    return color;
}
