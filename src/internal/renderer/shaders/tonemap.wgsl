// Tonemap shader - applies tonemapping to a fullscreen quad
// TonemapParams is 16 bytes (4 x u32)

struct TonemapParams {
    mode: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: TonemapParams;

// Fullscreen triangle vertex shader (no vertex buffers needed)
@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var out: VertexOutput;

    let x = f32((vertex_index & 1u) << 2u);
    let y = f32((vertex_index & 2u) << 1u);

    out.position = vec4<f32>(x - 1.0, 1.0 - y, 0.0, 1.0);
    out.uv = vec2<f32>(x * 0.5, y * 0.5);

    return out;
}

// ACES filmic tonemapping curve
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3(0.0), vec3(1.0));
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(source_texture, texture_sampler, uv);

    var rgb: vec3<f32>;
    switch (params.mode) {
        case 1u: {
            // ACES filmic tonemapping
            rgb = aces_tonemap(color.rgb);
        }
        default: {
            // Mode 0 (bypass): clamp to [0,1]
            rgb = clamp(color.rgb, vec3(0.0), vec3(1.0));
        }
    }

    return vec4<f32>(rgb, color.a);
}
