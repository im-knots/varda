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
    _padding: vec2<u32>,
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
