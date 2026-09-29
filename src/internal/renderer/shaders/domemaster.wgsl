// Domemaster projection: maps 5 cubemap faces (front, right, back, left, top)
// to an equidistant azimuthal fisheye. Each output pixel becomes a hemisphere
// direction, which samples the matching face.

struct DomemasterParams {
    // Field of view in radians (pi = 180 degrees full dome)
    fov: f32,
    // Content tilt angle in radians (0 = zenith centered)
    tilt: f32,
    // Content rotation: azimuth (Y axis), elevation (X axis)
    content_az: f32,
    content_el: f32,
    // Content rotation: roll (Z axis) + padding
    content_roll: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

@group(0) @binding(0)
var texture_sampler: sampler;

@group(0) @binding(1)
var face_front: texture_2d<f32>;

@group(0) @binding(2)
var face_right: texture_2d<f32>;

@group(0) @binding(3)
var face_back: texture_2d<f32>;

@group(0) @binding(4)
var face_left: texture_2d<f32>;

@group(0) @binding(5)
var face_top: texture_2d<f32>;

@group(0) @binding(6)
var<uniform> params: DomemasterParams;

// Sample a cubemap direction from 5 individual face textures.
// Direction is in dome space: +Y = up (zenith), +Z = forward, +X = right.
fn sample_cubemap(dir: vec3<f32>) -> vec4<f32> {
    let abs_dir = abs(dir);

    // Pick the face by dominant axis.
    if abs_dir.y >= abs_dir.x && abs_dir.y >= abs_dir.z && dir.y > 0.0 {
        // Top face (+Y): project onto XZ plane
        let u = dir.x / abs_dir.y * 0.5 + 0.5;
        let v = -dir.z / abs_dir.y * 0.5 + 0.5;
        return textureSample(face_top, texture_sampler, vec2<f32>(u, v));
    }
    if abs_dir.z >= abs_dir.x && abs_dir.z >= abs_dir.y {
        if dir.z > 0.0 {
            // Front face (+Z)
            let u = dir.x / abs_dir.z * 0.5 + 0.5;
            let v = -dir.y / abs_dir.z * 0.5 + 0.5;
            return textureSample(face_front, texture_sampler, vec2<f32>(u, v));
        } else {
            // Back face (-Z)
            let u = -dir.x / abs_dir.z * 0.5 + 0.5;
            let v = -dir.y / abs_dir.z * 0.5 + 0.5;
            return textureSample(face_back, texture_sampler, vec2<f32>(u, v));
        }
    }
    if dir.x > 0.0 {
        // Right face (+X)
        let u = -dir.z / abs_dir.x * 0.5 + 0.5;
        let v = -dir.y / abs_dir.x * 0.5 + 0.5;
        return textureSample(face_right, texture_sampler, vec2<f32>(u, v));
    }
    // Left face (-X)
    let u = dir.z / abs_dir.x * 0.5 + 0.5;
    let v = -dir.y / abs_dir.x * 0.5 + 0.5;
    return textureSample(face_left, texture_sampler, vec2<f32>(u, v));
}

@fragment
fn fs_main(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    // UV [0,1] to centered [-1,1].
    let centered = uv * 2.0 - vec2<f32>(1.0, 1.0);

    let r = length(centered);

    // Outside the dome circle is black.
    if r > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // Equidistant: angle from zenith = r * (fov/2).
    let half_fov = params.fov * 0.5;
    let angle_from_zenith = r * half_fov;

    let azimuth = atan2(centered.x, -centered.y);

    // Dome space: +Y = up (zenith).
    let sin_angle = sin(angle_from_zenith);
    let cos_angle = cos(angle_from_zenith);

    var dir = vec3<f32>(
        sin_angle * sin(azimuth),   // X = right
        cos_angle,                   // Y = up (zenith)
        sin_angle * cos(azimuth)    // Z = forward
    );

    // Dome tilt (X axis).
    let cos_tilt = cos(params.tilt);
    let sin_tilt = sin(params.tilt);
    let tilted_y = dir.y * cos_tilt - dir.z * sin_tilt;
    let tilted_z = dir.y * sin_tilt + dir.z * cos_tilt;
    dir = vec3<f32>(dir.x, tilted_y, tilted_z);

    // Rotate the sampling direction opposite to the content rotation.
    // Order: roll (Z), elevation (X), azimuth (Y).
    let cr = cos(-params.content_roll);
    let sr = sin(-params.content_roll);
    dir = vec3<f32>(dir.x * cr - dir.y * sr, dir.x * sr + dir.y * cr, dir.z);
    let ce = cos(-params.content_el);
    let se = sin(-params.content_el);
    dir = vec3<f32>(dir.x, dir.y * ce - dir.z * se, dir.y * se + dir.z * ce);
    let ca = cos(-params.content_az);
    let sa = sin(-params.content_az);
    dir = vec3<f32>(dir.x * ca + dir.z * sa, dir.y, -dir.x * sa + dir.z * ca);

    return sample_cubemap(dir);
}
