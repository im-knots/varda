// Point-cloud reprojection pass for depth sensors.
//
// One point per depth texel: the vertex shader reads depth[u,v] from an
// R16Uint texture, deprojects (u,v,depth) into camera-space XYZ using the
// device intrinsics, orbits/zooms a virtual camera around the cloud, projects
// to clip space, and emits a small screen-space quad (point splat). The
// fragment shader colours each point by RGB sample or a depth ramp.
//
// See spec/depth-sensors.md.

struct Params {
    // fx, fy, cx, cy
    intrinsics: vec4<f32>,
    // depth width, depth height, depth_min_mm, depth_max_mm
    dims_range: vec4<f32>,
    // orbit_yaw, orbit_pitch, zoom, point_size_px
    view: vec4<f32>,
    // color_mode (0 rgb, 1 depth ramp, 2 solid), depth_scale_m, target_w, target_h
    misc: vec4<f32>,
    // solid color rgb + unused
    solid: vec4<f32>,
};

@group(0) @binding(0) var depth_tex: texture_2d<u32>;
@group(0) @binding(1) var rgb_tex: texture_2d<f32>;
@group(0) @binding(2) var<uniform> params: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec3<f32>,
    @location(1) valid: f32,
};

fn hsv_ramp(t: f32) -> vec3<f32> {
    // Simple blue→red depth ramp (near = warm, far = cool inverted here).
    let c = clamp(t, 0.0, 1.0);
    return vec3<f32>(c, 0.4 + 0.4 * sin(c * 3.14159), 1.0 - c);
}

@vertex
fn vs_main(@builtin(vertex_index) vid: u32) -> VsOut {
    var out: VsOut;

    let w = u32(params.dims_range.x);
    let h = u32(params.dims_range.y);
    // 6 vertices per point (two triangles forming a splat quad).
    let point_idx = vid / 6u;
    let corner = vid % 6u;

    let px = point_idx % w;
    let py = point_idx / w;

    let raw = textureLoad(depth_tex, vec2<i32>(i32(px), i32(py)), 0).r;
    let depth_min = params.dims_range.z;
    let depth_max = params.dims_range.w;

    // Cull invalid / out-of-range depth.
    if (raw == 0u || f32(raw) < depth_min || f32(raw) > depth_max) {
        out.pos = vec4<f32>(0.0, 0.0, -10.0, 1.0);
        out.valid = 0.0;
        return out;
    }

    let z_m = f32(raw) * params.misc.y; // metres
    let fx = params.intrinsics.x;
    let fy = params.intrinsics.y;
    let cx = params.intrinsics.z;
    let cy = params.intrinsics.w;

    // Deproject to camera-space (metres). Y is flipped to point up.
    let x = (f32(px) - cx) * z_m / fx;
    let y = -(f32(py) - cy) * z_m / fy;
    let z = z_m;

    // Centre the cloud roughly and orbit.
    let yaw = params.view.x;
    let pitch = params.view.y;
    let zoom = params.view.z;
    var p = vec3<f32>(x, y, z - 1.5);

    let cy_ = cos(yaw); let sy = sin(yaw);
    let cp = cos(pitch); let sp = sin(pitch);
    // Yaw around Y.
    p = vec3<f32>(cy_ * p.x + sy * p.z, p.y, -sy * p.x + cy_ * p.z);
    // Pitch around X.
    p = vec3<f32>(p.x, cp * p.y - sp * p.z, sp * p.y + cp * p.z);

    // Simple perspective projection.
    let dist = 2.5 / zoom;
    let pz = p.z + dist;
    let aspect = params.misc.z / max(params.misc.w, 1.0);
    var clip = vec3<f32>(p.x / (pz * aspect), p.y / pz, pz * 0.1);

    // Screen-space splat expansion.
    let size = params.view.w;
    var offset = vec2<f32>(0.0, 0.0);
    switch corner {
        case 0u: { offset = vec2<f32>(-1.0, -1.0); }
        case 1u: { offset = vec2<f32>( 1.0, -1.0); }
        case 2u: { offset = vec2<f32>( 1.0,  1.0); }
        case 3u: { offset = vec2<f32>(-1.0, -1.0); }
        case 4u: { offset = vec2<f32>( 1.0,  1.0); }
        default: { offset = vec2<f32>(-1.0,  1.0); }
    }
    let px_size = size / max(params.misc.z, 1.0) * 2.0;

    out.pos = vec4<f32>(clip.x + offset.x * px_size, clip.y + offset.y * px_size, clip.z, 1.0);
    out.valid = 1.0;

    let mode = params.misc.x;
    if (mode < 0.5) {
        let rgb = textureLoad(rgb_tex, vec2<i32>(i32(px), i32(py)), 0).rgb;
        out.color = rgb;
    } else if (mode < 1.5) {
        let t = (f32(raw) - depth_min) / max(depth_max - depth_min, 1.0);
        out.color = hsv_ramp(t);
    } else {
        out.color = params.solid.rgb;
    }
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (in.valid < 0.5) {
        discard;
    }
    return vec4<f32>(in.color, 1.0);
}
