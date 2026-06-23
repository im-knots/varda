// LUT application shader — applies optional 1D shaper + 3D LUT to a fullscreen quad.

struct LutParams {
    // Domain min/max for the 3D LUT (used to scale input to [0,1] for texture lookup)
    domain_min: vec3<f32>,
    has_shaper: u32,  // 0 = no shaper, 1 = has shaper
    domain_max: vec3<f32>,
    _pad: u32,
    // Shaper domain (only used when has_shaper == 1)
    shaper_domain_min: vec3<f32>,
    _pad2: u32,
    shaper_domain_max: vec3<f32>,
    _pad3: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0)
var source_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: LutParams;

@group(0) @binding(3)
var lut_sampler: sampler;

@group(0) @binding(4)
var lut_3d: texture_3d<f32>;

@group(0) @binding(5)
var shaper_sampler: sampler;

@group(0) @binding(6)
var shaper_1d: texture_2d<f32>;

// Fullscreen triangle vertex shader (same as tonemap)
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;
    let x = f32((vertex_index & 1u) << 2u);
    let y = f32((vertex_index & 2u) << 1u);
    out.position = vec4<f32>(x - 1.0, 1.0 - y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5, y * 0.5);
    return out;
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(source_texture, source_sampler, uv);
    var rgb = color.rgb;

    // Apply 1D shaper if present (redistributes precision in input range)
    if (params.has_shaper == 1u) {
        let shaper_range = params.shaper_domain_max - params.shaper_domain_min;
        let shaper_uv = clamp((rgb - params.shaper_domain_min) / shaper_range, vec3(0.0), vec3(1.0));
        // Sample shaper as a 1D lookup — stored as a Nx1 2D texture, one sample per channel
        rgb = vec3(
            textureSample(shaper_1d, shaper_sampler, vec2(shaper_uv.r, 0.5)).r,
            textureSample(shaper_1d, shaper_sampler, vec2(shaper_uv.g, 0.5)).g,
            textureSample(shaper_1d, shaper_sampler, vec2(shaper_uv.b, 0.5)).b,
        );
    }

    // Scale input from [domain_min, domain_max] to [0, 1] for 3D texture lookup
    let lut_range = params.domain_max - params.domain_min;
    let lut_uv = clamp((rgb - params.domain_min) / lut_range, vec3(0.0), vec3(1.0));

    // Sample 3D LUT with trilinear interpolation
    rgb = textureSample(lut_3d, lut_sampler, lut_uv).rgb;

    return vec4<f32>(rgb, color.a);
}
