// Draws a linear texture into the viewport inside egui's render pass. The alpha
// channel acts as it does on an egui image, which is premultiplied.

@group(0) @binding(0)
var source_sampler: sampler;

@group(0) @binding(1)
var source: texture_2d<f32>;

// In pixels. 0 = square corners.
@group(0) @binding(2)
var<uniform> corner_radius: f32;

// A thumbnail covers many source texels per pixel, so one bilinear read would
// alias. These reads are spread over the pixel at the 8x MSAA sample positions.
fn sample_footprint(uv: vec2<f32>) -> vec4<f32> {
    let x = dpdx(uv) / 16.0;
    let y = dpdy(uv) / 16.0;
    var sum = vec4<f32>(0.0);
    sum += textureSampleLevel(source, source_sampler, uv + 1.0 * x - 3.0 * y, 0.0);
    sum += textureSampleLevel(source, source_sampler, uv - 1.0 * x + 3.0 * y, 0.0);
    sum += textureSampleLevel(source, source_sampler, uv + 5.0 * x + 1.0 * y, 0.0);
    sum += textureSampleLevel(source, source_sampler, uv - 3.0 * x - 5.0 * y, 0.0);
    sum += textureSampleLevel(source, source_sampler, uv - 5.0 * x + 5.0 * y, 0.0);
    sum += textureSampleLevel(source, source_sampler, uv - 7.0 * x - 1.0 * y, 0.0);
    sum += textureSampleLevel(source, source_sampler, uv + 3.0 * x + 7.0 * y, 0.0);
    sum += textureSampleLevel(source, source_sampler, uv + 7.0 * x - 7.0 * y, 0.0);
    return clamp(sum / 8.0, vec4<f32>(0.0), vec4<f32>(1.0));
}

// How much of the pixel lies inside the rounded rect, antialiased over one pixel.
fn corner_coverage(uv: vec2<f32>) -> f32 {
    let size = 1.0 / vec2<f32>(abs(dpdx(uv.x)), abs(dpdy(uv.y)));
    let from_corner = (abs(uv - 0.5) - 0.5) * size + corner_radius;
    let inside = min(max(from_corner.x, from_corner.y), 0.0);
    let distance = length(max(from_corner, vec2<f32>(0.0))) + inside - corner_radius;
    return clamp(0.5 - distance, 0.0, 1.0);
}

fn gamma_from_linear(rgb: vec3<f32>) -> vec3<f32> {
    let lower = rgb * 12.92;
    let higher = 1.055 * pow(rgb, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(higher, lower, rgb < vec3<f32>(0.0031308));
}

// An sRGB target encodes on write.
@fragment
fn fs_srgb_target(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return sample_footprint(uv) * corner_coverage(uv);
}

@fragment
fn fs_gamma_target(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = sample_footprint(uv);
    return vec4<f32>(gamma_from_linear(color.rgb), color.a) * corner_coverage(uv);
}
