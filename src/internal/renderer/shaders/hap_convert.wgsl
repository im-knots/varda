// HAP YCoCg→RGB conversion + dual-plane alpha compositing shader.
// Used for HAP Q (YCoCg DXT5) and HAP Q Alpha (YCoCg DXT5 + BC4 alpha).

struct HapConvertParams {
    opacity: f32,
    /// 0.0 = no YCoCg conversion (passthrough), 1.0 = YCoCg→RGB
    do_ycocg: f32,
    /// 0.0 = single plane (alpha from color texture), 1.0 = dual plane (alpha from separate texture)
    has_alpha_plane: f32,
    _pad: f32,
}

@group(0) @binding(0)
var tex_sampler: sampler;

@group(0) @binding(1)
var color_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: HapConvertParams;

@group(0) @binding(3)
var alpha_texture: texture_2d<f32>;

/// Scaled YCoCg → RGB conversion (matches HAP Q spec).
/// Input: BC3/DXT5 texture where RGB stores YCoCg and A stores scale.
fn ycocg_to_rgb(color: vec4<f32>) -> vec3<f32> {
    let scale = (color.b * (255.0 / 8.0)) + 1.0;
    let co = (color.r - (0.5 * 256.0 / 255.0)) / scale;
    let cg = (color.g - (0.5 * 256.0 / 255.0)) / scale;
    let y = color.a;
    return vec3<f32>(
        y + co - cg,
        y + cg,
        y - co - cg,
    );
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(color_texture, tex_sampler, uv);

    var rgb: vec3<f32>;
    if (params.do_ycocg > 0.5) {
        rgb = ycocg_to_rgb(color);
    } else {
        rgb = color.rgb;
    }

    var alpha: f32;
    if (params.has_alpha_plane > 0.5) {
        // Dual-plane: alpha comes from separate BC4 texture (red channel)
        let alpha_sample = textureSample(alpha_texture, tex_sampler, uv);
        alpha = alpha_sample.r;
    } else {
        alpha = color.a;
    }

    return vec4<f32>(rgb, alpha * params.opacity);
}
