// Edge blend post-process shader — smoothstep alpha ramps on edges
// for seamless multi-projector overlap zones.
//
// EdgeBlendParams layout: 32 floats = 128 bytes
// Per-edge: [enabled (as f32), width, gamma, _pad] × 4 edges (left, right, top, bottom)

struct EdgeBlendParams {
    // Left edge: enabled, width, gamma, _pad
    left_enabled: f32,
    left_width: f32,
    left_gamma: f32,
    _pad0: f32,
    // Right edge
    right_enabled: f32,
    right_width: f32,
    right_gamma: f32,
    _pad1: f32,
    // Top edge
    top_enabled: f32,
    top_width: f32,
    top_gamma: f32,
    _pad2: f32,
    // Bottom edge
    bottom_enabled: f32,
    bottom_width: f32,
    bottom_gamma: f32,
    _pad3: f32,
}

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: EdgeBlendParams;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    var color = textureSample(source_texture, texture_sampler, uv);

    var alpha = 1.0;

    // Left edge ramp: UV.x from 0 → left_width
    if (params.left_enabled > 0.5) {
        let t = clamp(uv.x / params.left_width, 0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t); // smoothstep
        alpha *= pow(s, params.left_gamma);
    }

    // Right edge ramp: UV.x from (1 - right_width) → 1
    if (params.right_enabled > 0.5) {
        let t = clamp((1.0 - uv.x) / params.right_width, 0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        alpha *= pow(s, params.right_gamma);
    }

    // Top edge ramp: UV.y from 0 → top_width
    if (params.top_enabled > 0.5) {
        let t = clamp(uv.y / params.top_width, 0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        alpha *= pow(s, params.top_gamma);
    }

    // Bottom edge ramp: UV.y from (1 - bottom_width) → 1
    if (params.bottom_enabled > 0.5) {
        let t = clamp((1.0 - uv.y) / params.bottom_width, 0.0, 1.0);
        let s = t * t * (3.0 - 2.0 * t);
        alpha *= pow(s, params.bottom_gamma);
    }

    // Multiply RGB by alpha for pre-multiplied blending
    return vec4<f32>(color.rgb * alpha, color.a);
}
