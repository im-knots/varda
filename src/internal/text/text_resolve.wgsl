// Composites the premultiplied text layer over the background into the
// deck's straight-alpha target, for backgrounds that are not opaque. See
// /spec/text-source.md § Rendering.

@group(0) @binding(0) var layer: texture_2d<f32>;
@group(0) @binding(1) var<uniform> background: vec4<f32>;

@vertex
fn vs_full(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(uv * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn fs_resolve(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let text = textureLoad(layer, vec2<i32>(position.xy), 0);
    let under = background.a * (1.0 - text.a);
    let alpha = text.a + under;
    if (alpha <= 0.0) {
        return vec4<f32>(0.0);
    }
    return vec4<f32>((text.rgb + background.rgb * under) / alpha, alpha);
}
