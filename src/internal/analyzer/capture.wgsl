// Reduces a deck frame to an analyzer's size. Each output pixel averages its
// footprint in the source with a grid of bilinear taps, each covering about
// two texels a side. The sRGB target encodes the linear result on write.

struct Params {
    // Source texels per output pixel, per axis.
    footprint: vec2<f32>,
    // Bilinear taps per axis.
    taps: vec2<u32>,
}

@group(0) @binding(0)
var source_texture: texture_2d<f32>;

@group(0) @binding(1)
var source_sampler: sampler;

@group(0) @binding(2)
var<uniform> params: Params;

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let source_size = vec2<f32>(textureDimensions(source_texture));
    let origin = floor(position.xy) * params.footprint;
    let spacing = params.footprint / vec2<f32>(params.taps);
    var sum = vec4<f32>(0.0);
    for (var y = 0u; y < params.taps.y; y++) {
        for (var x = 0u; x < params.taps.x; x++) {
            let texel = origin + (vec2<f32>(f32(x), f32(y)) + 0.5) * spacing;
            sum += textureSampleLevel(source_texture, source_sampler, texel / source_size, 0.0);
        }
    }
    return clamp(sum / f32(params.taps.x * params.taps.y), vec4<f32>(0.0), vec4<f32>(1.0));
}
