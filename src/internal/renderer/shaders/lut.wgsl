// LUT pass: optional 1D shaper, then a 3D LUT.

struct LutParams {
    // 3D LUT input domain, scaled to [0,1] for lookup.
    domain_min: vec3<f32>,
    has_shaper: u32,  // 0 = no shaper, 1 = has shaper
    domain_max: vec3<f32>,
    _pad: u32,
    // Shaper domain (only used when has_shaper == 1)
    shaper_domain_min: vec3<f32>,
    _pad2: u32,
    shaper_domain_max: vec3<f32>,
    // 0 = display-referred (calibration slot, input used as-is).
    // 1 = scene-referred: ACEScct encode before the lookup, decode after.
    scene_referred: u32,
}

// ── ACEScct, mirrored from acescct.rs ──
const ACESCCT_A: f32 = 10.5402377;
const ACESCCT_B: f32 = 0.072905534;
const ACESCCT_LINEAR_BREAK: f32 = 0.0078125;
const ACESCCT_ENCODED_BREAK: f32 = 0.15525114;
const ACESCCT_LOG_OFFSET: f32 = 9.72;
const ACESCCT_LOG_SCALE: f32 = 17.52;

fn acescct_from_linear(rgb: vec3<f32>) -> vec3<f32> {
    let safe = max(rgb, vec3<f32>(0.0));
    let toe = ACESCCT_A * safe + ACESCCT_B;
    // Guard the log2 input: it is undefined for non-positive values, which the
    // toe handles anyway.
    let logged = (log2(max(safe, vec3<f32>(1e-10))) + ACESCCT_LOG_OFFSET) / ACESCCT_LOG_SCALE;
    return select(logged, toe, safe <= vec3<f32>(ACESCCT_LINEAR_BREAK));
}

fn linear_from_acescct(rgb: vec3<f32>) -> vec3<f32> {
    let toe = (rgb - ACESCCT_B) / ACESCCT_A;
    let logged = exp2(rgb * ACESCCT_LOG_SCALE - ACESCCT_LOG_OFFSET);
    return select(logged, toe, rgb <= vec3<f32>(ACESCCT_ENCODED_BREAK));
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

    // Scene-referred look LUTs are authored against ACEScct.
    if (params.scene_referred == 1u) {
        rgb = acescct_from_linear(rgb);
    }

    // 1D shaper, if present.
    if (params.has_shaper == 1u) {
        let shaper_range = params.shaper_domain_max - params.shaper_domain_min;
        let shaper_uv = clamp((rgb - params.shaper_domain_min) / shaper_range, vec3(0.0), vec3(1.0));
        // Shaper is an Nx1 2D texture, sampled once per channel.
        rgb = vec3(
            textureSample(shaper_1d, shaper_sampler, vec2(shaper_uv.r, 0.5)).r,
            textureSample(shaper_1d, shaper_sampler, vec2(shaper_uv.g, 0.5)).g,
            textureSample(shaper_1d, shaper_sampler, vec2(shaper_uv.b, 0.5)).b,
        );
    }

    // Scale input from [domain_min, domain_max] to [0, 1].
    let lut_range = params.domain_max - params.domain_min;
    let lut_uv = clamp((rgb - params.domain_min) / lut_range, vec3(0.0), vec3(1.0));

    // Trilinear 3D lookup.
    rgb = textureSample(lut_3d, lut_sampler, lut_uv).rgb;

    // Back to scene-linear for the output transform.
    if (params.scene_referred == 1u) {
        rgb = linear_from_acescct(rgb);
    }

    return vec4<f32>(rgb, color.a);
}
