// 3D Dome Preview Shader
// Renders a hemisphere mesh with domemaster texture mapped via equidistant azimuthal projection.
// Uses MVP (Model-View-Projection) matrix for orbit camera.

struct Uniforms {
    mvp: mat4x4<f32>,
    // [azimuth_rad, elevation_rad, roll_rad, 0]
    content_rotation: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var dome_sampler: sampler;
@group(0) @binding(2) var dome_texture: texture_2d<f32>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coord: vec2<f32>,
    @location(1) world_normal: vec3<f32>,
}

// Rotate around X axis
fn rotate_x(v: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(v.x, c * v.y - s * v.z, s * v.y + c * v.z);
}

// Rotate around Y axis
fn rotate_y(v: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * v.x + s * v.z, v.y, -s * v.x + c * v.z);
}

// Rotate around Z axis
fn rotate_z(v: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec3<f32>(c * v.x - s * v.y, s * v.x + c * v.y, v.z);
}

// Compute domemaster UV from a dome surface direction, with content rotation applied.
fn domemaster_uv(dir_in: vec3<f32>, content_az: f32, content_el: f32, content_roll: f32) -> vec2<f32> {
    // Apply inverse content rotation to the sampling direction
    // Order: roll (Z) → elevation (X) → azimuth (Y)
    var d = rotate_z(dir_in, -content_roll);
    d = rotate_x(d, -content_el);
    d = rotate_y(d, -content_az);

    // Equidistant azimuthal projection: polar angle from zenith (+Y)
    let polar = acos(clamp(d.y, -1.0, 1.0));
    let azimuth = atan2(d.z, d.x);

    // 90° truncation: full hemisphere
    let max_angle = 1.5707963; // pi/2
    let r = min(polar / max_angle, 1.0);

    let uv_x = 0.5 + r * 0.5 * cos(azimuth);
    let uv_y = 0.5 + r * 0.5 * sin(azimuth);
    return clamp(vec2<f32>(uv_x, uv_y), vec2<f32>(0.0), vec2<f32>(1.0));
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = uniforms.mvp * vec4<f32>(in.position, 1.0);
    out.tex_coord = in.uv;
    out.world_normal = normalize(in.position);
    return out;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Use world normal to compute rotated domemaster UV
    let uv = domemaster_uv(
        in.world_normal,
        uniforms.content_rotation.x,
        uniforms.content_rotation.y,
        uniforms.content_rotation.z,
    );
    let color = textureSample(dome_texture, dome_sampler, uv);

    // Simple hemisphere lighting
    let light_dir = normalize(vec3<f32>(0.3, 1.0, 0.5));
    let ndotl = max(dot(in.world_normal, light_dir), 0.0);
    let ambient = 0.4;
    let lighting = ambient + (1.0 - ambient) * ndotl;

    return vec4<f32>(color.rgb * lighting, color.a);
}
