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
    // Overlap zone count (as f32) + 3 padding floats
    zone_count: f32,
    _zone_pad0: f32,
    _zone_pad1: f32,
    _zone_pad2: f32,
    // Up to 4 overlap zones: rect = [u_min, v_min, u_max, v_max], cfg = [gamma, _, _, _]
    zone0_rect: vec4<f32>,
    zone0_cfg: vec4<f32>,
    zone1_rect: vec4<f32>,
    zone1_cfg: vec4<f32>,
    zone2_rect: vec4<f32>,
    zone2_cfg: vec4<f32>,
    zone3_rect: vec4<f32>,
    zone3_cfg: vec4<f32>,
};

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var source_texture: texture_2d<f32>;

@group(0) @binding(2)
var<uniform> params: PolygonParams;

// Subtractive hole coverage mask (8i.7). R channel: 1.0 = content, 0.0 = hole.
// Sampled by the surface's bb-normalized uv so it stays warp-agnostic. A 1×1
// white texture is bound for hole-less surfaces (no-op).
@group(0) @binding(3)
var mask_texture: texture_2d<f32>;

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

// Smoothstep blend: t² × (3 − 2t), raised to gamma.
fn blend_alpha(t: f32, gamma: f32) -> f32 {
    let tc = clamp(t, 0.0, 1.0);
    let s = tc * tc * (3.0 - 2.0 * tc);
    return pow(s, gamma);
}

// Compute blend factor for a single overlap zone.
// Returns 1.0 outside the zone, ramps toward 0.0 in the ramp direction inside the zone.
// cfg = [gamma, ramp_x, ramp_y, _pad]
//   ramp_x: +1.0 = fade toward u_max, -1.0 = fade toward u_min, 0.0 = none
//   ramp_y: +1.0 = fade toward v_max, -1.0 = fade toward v_min, 0.0 = none
fn zone_blend(uv: vec2<f32>, rect: vec4<f32>, cfg: vec4<f32>) -> f32 {
    let u_min = rect.x;
    let v_min = rect.y;
    let u_max = rect.z;
    let v_max = rect.w;
    let gamma = cfg.x;
    let ramp_x = cfg.y;
    let ramp_y = cfg.z;

    // Outside the overlap rect: no dimming
    if (uv.x < u_min || uv.x > u_max || uv.y < v_min || uv.y > v_max) {
        return 1.0;
    }

    let zone_w = u_max - u_min;
    let zone_h = v_max - v_min;
    var t = 1.0;

    // Horizontal ramp: fade toward the specified direction
    if (abs(ramp_x) > 0.001 && zone_w > 0.001) {
        if (ramp_x > 0.0) {
            // Fade toward u_max (other surface is to the right)
            t = min(t, (u_max - uv.x) / zone_w);
        } else {
            // Fade toward u_min (other surface is to the left)
            t = min(t, (uv.x - u_min) / zone_w);
        }
    }

    // Vertical ramp: fade toward the specified direction
    if (abs(ramp_y) > 0.001 && zone_h > 0.001) {
        if (ramp_y > 0.0) {
            // Fade toward v_max (other surface is below)
            t = min(t, (v_max - uv.y) / zone_h);
        } else {
            // Fade toward v_min (other surface is above)
            t = min(t, (uv.y - v_min) / zone_h);
        }
    }

    return blend_alpha(t, gamma);
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

    // Per-surface overlap zone blending — dim only within overlap rectangles.
    // Multiplies RGB (pre-multiplied blend).
    let n = i32(params.zone_count);
    var blend = 1.0;
    if (n >= 1) { blend *= zone_blend(uv, params.zone0_rect, params.zone0_cfg); }
    if (n >= 2) { blend *= zone_blend(uv, params.zone1_rect, params.zone1_cfg); }
    if (n >= 3) { blend *= zone_blend(uv, params.zone2_rect, params.zone2_cfg); }
    if (n >= 4) { blend *= zone_blend(uv, params.zone3_rect, params.zone3_cfg); }
    color = vec4<f32>(color.rgb * blend, color.a);

    // Subtractive holes: multiply by coverage so holes (0.0) become black +
    // transparent, feathered edges (8i.8) ramp smoothly.
    let coverage = textureSample(mask_texture, texture_sampler, uv).r;
    color = color * coverage;

    return color;
}
