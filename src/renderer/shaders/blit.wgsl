// Blit shader - copies a texture with opacity and UV transform for scaling modes
// BlitParams is 32 bytes (8 x f32)

struct BlitParams {
    opacity: f32,
    _pad: f32,
    uv_scale: vec2<f32>,
    uv_offset: vec2<f32>,
    _pad2: vec2<f32>,
}

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: BlitParams;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // Apply UV transform for scaling modes
    let source_uv = uv * params.uv_scale + params.uv_offset;

    // Clamp to [0,1] — pixels outside the source are black (for Fit/Center modes)
    if (source_uv.x < 0.0 || source_uv.x > 1.0 || source_uv.y < 0.0 || source_uv.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, params.opacity);
    }

    var color = textureSample(source_texture, texture_sampler, source_uv);
    color.a *= params.opacity;
    return color;
}

