// Text deck drawing.
//
// Draws one line or word's coverage mask as a quad, premultiplied, so
// overlapping units blend correctly. `text_resolve.wgsl` turns the result
// into straight alpha when the background is not opaque.

struct Quad {
    // Left, top, right, bottom in clip space.
    rect: vec4<f32>,
    // Straight RGBA.
    color: vec4<f32>,
    // x: clip edge across the mask (0 to 1), y: 1 keeps the part after the
    // edge, 0 the part before, z: transition opacity.
    clip: vec4<f32>,
};

@group(0) @binding(0) var<uniform> quad: Quad;
@group(0) @binding(1) var mask_sampler: sampler;
@group(1) @binding(0) var mask: texture_2d<f32>;

struct QuadOut {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_quad(@builtin(vertex_index) index: u32) -> QuadOut {
    let corner = vec2<f32>(f32(index & 1u), f32((index >> 1u) & 1u));
    var out: QuadOut;
    out.position = vec4<f32>(
        mix(quad.rect.x, quad.rect.z, corner.x),
        mix(quad.rect.y, quad.rect.w, corner.y),
        0.0,
        1.0,
    );
    out.uv = corner;
    return out;
}

@fragment
fn fs_quad(in: QuadOut) -> @location(0) vec4<f32> {
    let coverage = textureSample(mask, mask_sampler, in.uv).r;
    let after = in.uv.x >= quad.clip.x;
    let kept = select(!after, after, quad.clip.y > 0.5);
    let k = coverage * quad.color.a * quad.clip.z * select(0.0, 1.0, kept);
    return vec4<f32>(quad.color.rgb * k, k);
}
