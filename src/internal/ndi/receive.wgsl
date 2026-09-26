// Expands received 8-bit UYVY into RGB, the inverse of the send path's
// `uyvy` entry point: BT.709 matrix, limited range, Rec.709 OETF.

// One `U Y0 V Y1` quad per texel, so the texture is half the frame's width.
@group(0) @binding(0)
var packed: texture_2d<f32>;

@vertex
fn fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

fn rec709_inverse_oetf(encoded: vec3<f32>) -> vec3<f32> {
    let lower = encoded / 4.5;
    let upper = pow((encoded + 0.099) / 1.099, vec3<f32>(1.0 / 0.45));
    return select(upper, lower, encoded < vec3<f32>(0.081));
}

@fragment
fn uyvy(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let x = u32(position.x);
    let quad = textureLoad(packed, vec2<i32>(i32(x / 2u), i32(position.y)), 0) * 255.0;
    let luma_code = select(quad.y, quad.w, (x & 1u) == 1u);

    let luma = (luma_code - 16.0) / 219.0;
    let cb = (quad.x - 128.0) / 224.0;
    let cr = (quad.z - 128.0) / 224.0;
    let r = luma + 1.5748 * cr;
    let b = luma + 1.8556 * cb;
    let g = (luma - 0.2126 * r - 0.0722 * b) / 0.7152;

    // The target is sRGB-encoded: write linear light and let the store encode.
    let encoded = clamp(vec3<f32>(r, g, b), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rec709_inverse_oetf(encoded), 1.0);
}
