// Composite blend shader - reads source layer and destination (composite-so-far),
// blends per-pixel based on blend_mode uniform.
// CompositeParams is 32 bytes (8 x f32)

struct CompositeParams {
    opacity: f32,
    blend_mode: u32,
    uv_scale: vec2<f32>,
    uv_offset: vec2<f32>,
    _pad: vec2<f32>,
}

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var dest_texture: texture_2d<f32>;

@group(0) @binding(3)
var<uniform> params: CompositeParams;

const EPSILON: f32 = 0.001;

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // Sample source with UV transform (scaling modes)
    let source_uv = uv * params.uv_scale + params.uv_offset;

    var src: vec4<f32>;
    if (source_uv.x < 0.0 || source_uv.x > 1.0 || source_uv.y < 0.0 || source_uv.y > 1.0) {
        src = vec4<f32>(0.0, 0.0, 0.0, 0.0);
    } else {
        src = textureSample(source_texture, texture_sampler, source_uv);
    }

    // Sample destination at raw UV (full composite, no transform)
    let dst = textureSample(dest_texture, texture_sampler, uv);

    // Apply opacity to source alpha
    let src_a = src.a * params.opacity;

    // Early out: fully transparent source contributes nothing
    if (src_a <= 0.0) {
        return dst;
    }

    // Compute blended RGB based on blend mode
    var blended: vec3<f32>;
    let mode = params.blend_mode;

    if (mode == 0u) {
        // Normal (alpha-over): just use source color
        blended = src.rgb;
    } else if (mode == 1u) {
        // Add
        blended = clamp(src.rgb + dst.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (mode == 2u) {
        // Subtract
        blended = clamp(dst.rgb - src.rgb, vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (mode == 3u) {
        // Multiply
        blended = src.rgb * dst.rgb;
    } else if (mode == 4u) {
        // Screen
        blended = vec3<f32>(1.0) - (vec3<f32>(1.0) - src.rgb) * (vec3<f32>(1.0) - dst.rgb);
    } else if (mode == 5u) {
        // Overlay (conditional per channel)
        blended = vec3<f32>(
            select(1.0 - 2.0 * (1.0 - src.r) * (1.0 - dst.r), 2.0 * src.r * dst.r, dst.r < 0.5),
            select(1.0 - 2.0 * (1.0 - src.g) * (1.0 - dst.g), 2.0 * src.g * dst.g, dst.g < 0.5),
            select(1.0 - 2.0 * (1.0 - src.b) * (1.0 - dst.b), 2.0 * src.b * dst.b, dst.b < 0.5),
        );
    } else if (mode == 6u) {
        // Soft Light (Pegtop)
        blended = (vec3<f32>(1.0) - 2.0 * src.rgb) * dst.rgb * dst.rgb + 2.0 * src.rgb * dst.rgb;
    } else if (mode == 7u) {
        // Hard Light (conditional per channel)
        blended = vec3<f32>(
            select(1.0 - 2.0 * (1.0 - src.r) * (1.0 - dst.r), 2.0 * src.r * dst.r, src.r < 0.5),
            select(1.0 - 2.0 * (1.0 - src.g) * (1.0 - dst.g), 2.0 * src.g * dst.g, src.g < 0.5),
            select(1.0 - 2.0 * (1.0 - src.b) * (1.0 - dst.b), 2.0 * src.b * dst.b, src.b < 0.5),
        );
    } else if (mode == 8u) {
        // Color Dodge: dst / (1 - src), clamped
        blended = clamp(vec3<f32>(
            dst.r / max(1.0 - src.r, EPSILON),
            dst.g / max(1.0 - src.g, EPSILON),
            dst.b / max(1.0 - src.b, EPSILON),
        ), vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (mode == 9u) {
        // Color Burn: 1 - (1-dst)/src, clamped
        blended = clamp(vec3<f32>(
            1.0 - (1.0 - dst.r) / max(src.r, EPSILON),
            1.0 - (1.0 - dst.g) / max(src.g, EPSILON),
            1.0 - (1.0 - dst.b) / max(src.b, EPSILON),
        ), vec3<f32>(0.0), vec3<f32>(1.0));
    } else if (mode == 10u) {
        // Difference
        blended = abs(src.rgb - dst.rgb);
    } else if (mode == 11u) {
        // Exclusion
        blended = src.rgb + dst.rgb - 2.0 * src.rgb * dst.rgb;
    } else if (mode == 12u) {
        // Darken
        blended = min(src.rgb, dst.rgb);
    } else if (mode == 13u) {
        // Lighten
        blended = max(src.rgb, dst.rgb);
    } else if (mode == 14u) {
        // Linear Burn: src + dst - 1, clamped to 0
        blended = max(src.rgb + dst.rgb - vec3<f32>(1.0), vec3<f32>(0.0));
    } else {
        // Fallback: Normal
        blended = src.rgb;
    }

    // Mix based on source alpha and compute final alpha (standard OVER)
    let result_rgb = mix(dst.rgb, blended, src_a);
    let result_a = src_a + dst.a * (1.0 - src_a);

    return vec4<f32>(result_rgb, result_a);
}
