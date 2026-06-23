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

// ── Mode 1: ACES filmic tonemapping curve ──
fn aces_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = 2.51;
    let b = 0.03;
    let c = 2.43;
    let d = 0.59;
    let e = 0.14;
    return clamp((x * (a * x + b)) / (x * (c * x + d) + e), vec3(0.0), vec3(1.0));
}

// ── Mode 2: Reinhard ──
fn reinhard_tonemap(x: vec3<f32>) -> vec3<f32> {
    return x / (x + vec3(1.0));
}

// ── Mode 3: Reinhard Extended ──
fn reinhard_extended_tonemap(x: vec3<f32>) -> vec3<f32> {
    let max_white = 4.0;
    return x * (vec3(1.0) + x / vec3(max_white * max_white)) / (vec3(1.0) + x);
}

// ── Mode 4: Hable Filmic (Uncharted 2) ──
fn hable_partial(x: vec3<f32>) -> vec3<f32> {
    let A = 0.15;
    let B = 0.50;
    let C = 0.10;
    let D = 0.20;
    let E = 0.02;
    let F = 0.30;
    return ((x * (A * x + C * B) + D * E) / (x * (A * x + B) + D * F)) - E / F;
}

fn hable_filmic_tonemap(color: vec3<f32>) -> vec3<f32> {
    let W = 11.2;
    return hable_partial(color) / hable_partial(vec3(W));
}

// ── Mode 5: Uchimura (Gran Turismo) ──
fn uchimura_curve(x: f32, P: f32, a: f32, m: f32, l: f32, c: f32, b: f32) -> f32 {
    let l0 = ((P - m) * l) / a;
    let L0 = m - m / a;
    let L1 = m + (1.0 - m) / a;
    let S0 = m + l0;
    let S1 = m + a * l0;
    let C2 = (a * P) / (P - S1);
    let CP = -C2 / P;
    var w0 = 1.0 - smoothstep(0.0, m, x);
    var w2 = step(m + l0, x);
    var w1 = 1.0 - w0 - w2;
    let T = m * pow(x / m, c) + b;
    let S = P - (P - S1) * exp(CP * (x - S0));
    let L = m + a * (x - m);
    return T * w0 + L * w1 + S * w2;
}

fn uchimura_tonemap(color: vec3<f32>) -> vec3<f32> {
    return vec3(
        uchimura_curve(color.r, 1.0, 1.0, 0.22, 0.4, 1.33, 0.0),
        uchimura_curve(color.g, 1.0, 1.0, 0.22, 0.4, 1.33, 0.0),
        uchimura_curve(color.b, 1.0, 1.0, 0.22, 0.4, 1.33, 0.0),
    );
}

// ── Mode 6: Lottes (AMD) ──
fn lottes_tonemap(x: vec3<f32>) -> vec3<f32> {
    let a = vec3(1.6);
    let d = vec3(0.977);
    let hdr_max = vec3(8.0);
    let mid_in = vec3(0.18);
    let mid_out = vec3(0.267);
    let b = (-pow(mid_in, a) + pow(hdr_max, a) * mid_out) /
            ((pow(hdr_max, a * d) - pow(mid_in, a * d)) * mid_out);
    let c = (pow(hdr_max, a * d) * pow(mid_in, a) - pow(hdr_max, a) * pow(mid_in, a * d) * mid_out) /
            ((pow(hdr_max, a * d) - pow(mid_in, a * d)) * mid_out);
    return pow(x, a) / (pow(x, a * d) * b + c);
}

// ── Mode 7: AgX ──
fn agx_default_contrast_approx(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx_tonemap(color: vec3<f32>) -> vec3<f32> {
    let agx_mat = mat3x3<f32>(
        vec3(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3(0.0792237451477643, 0.0791661274605434, 0.879142973793104)
    );
    var val = agx_mat * max(color, vec3(1e-10));
    val = clamp(log2(val), vec3(-12.47393), vec3(4.026069));
    val = (val - vec3(-12.47393)) / (vec3(4.026069) - vec3(-12.47393));
    val = agx_default_contrast_approx(val);
    let agx_mat_inv = mat3x3<f32>(
        vec3(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3(-0.0990297440797205, -0.0989611768448433, 1.15107367264116)
    );
    val = agx_mat_inv * val;
    return clamp(val, vec3(0.0), vec3(1.0));
}

// ── Mode 8: Khronos PBR Neutral ──
fn pbr_neutral_tonemap(color_in: vec3<f32>) -> vec3<f32> {
    let start_compression = 0.8 - 0.04;
    let desaturation = 0.15;
    var color = color_in;
    let x = min(color.r, min(color.g, color.b));
    var offset: f32;
    if (x < 0.08) {
        offset = x - 6.25 * x * x;
    } else {
        offset = 0.04;
    }
    color -= vec3(offset);
    let peak = max(color.r, max(color.g, color.b));
    if (peak < start_compression) {
        return color;
    }
    let d = 1.0 - start_compression;
    let new_peak = 1.0 - d * d / (peak + d - start_compression);
    color *= new_peak / peak;
    let g = 1.0 - 1.0 / (desaturation * (peak - new_peak) + 1.0);
    return mix(color, vec3(new_peak), g);
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let color = textureSample(source_texture, texture_sampler, uv);

    var rgb: vec3<f32>;
    switch (params.mode) {
        case 1u: { rgb = aces_tonemap(color.rgb); }
        case 2u: { rgb = reinhard_tonemap(color.rgb); }
        case 3u: { rgb = reinhard_extended_tonemap(color.rgb); }
        case 4u: { rgb = hable_filmic_tonemap(color.rgb); }
        case 5u: { rgb = uchimura_tonemap(color.rgb); }
        case 6u: { rgb = lottes_tonemap(color.rgb); }
        case 7u: { rgb = agx_tonemap(color.rgb); }
        case 8u: { rgb = pbr_neutral_tonemap(color.rgb); }
        default: { rgb = clamp(color.rgb, vec3(0.0), vec3(1.0)); }
    }

    return vec4<f32>(rgb, color.a);
}
