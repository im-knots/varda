// Polygon vertex + fragment shader with per-surface homography warp.
// Used for rendering textured polygon surfaces with perspective-correct warping.

struct PolygonParams {
    opacity: f32,
    _pad: f32,
    uv_scale: vec2<f32>,
    uv_offset: vec2<f32>,
    _pad2: vec2<f32>,
    // 3x3 homography matrix, stored as 3 x vec4 (xyz used, w padding)
    h_row0: vec4<f32>,
    h_row1: vec4<f32>,
    h_row2: vec4<f32>,
};

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: PolygonParams;

struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply homography: H * [x, y, 1]
    let p = vec3<f32>(in.position, 1.0);
    let hx = dot(params.h_row0.xyz, p);
    let hy = dot(params.h_row1.xyz, p);
    let hw = dot(params.h_row2.xyz, p);

    // Set clip coords so that after perspective divide we get the correct screen position.
    // NDC.x = hx/hw * 2 - 1,  NDC.y = 1 - hy/hw * 2
    // By setting w_clip = hw, the GPU does the divide for us AND
    // interpolates varyings (UV) perspective-correctly.
    out.position = vec4<f32>(hx * 2.0 - hw, hw - hy * 2.0, 0.0, hw);
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // Apply UV transform for content mapping modes
    let source_uv = uv * params.uv_scale + params.uv_offset;

    // Clamp — pixels outside the source are black
    if (source_uv.x < 0.0 || source_uv.x > 1.0 || source_uv.y < 0.0 || source_uv.y > 1.0) {
        return vec4<f32>(0.0, 0.0, 0.0, params.opacity);
    }

    var color = textureSample(source_texture, texture_sampler, source_uv);
    color.a *= params.opacity;
    return color;
}
