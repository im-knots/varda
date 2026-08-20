struct ConvertParams {
    width: u32,
    height: u32,
    dither: u32,
    _padding: u32,
}

@group(0) @binding(0)
var source: texture_2d<f32>;

@group(0) @binding(1)
var<uniform> params: ConvertParams;

// Two 16-bit values are packed into each u32. On NDI's supported
// little-endian platforms this produces the required native u16 planes.
@group(0) @binding(2)
var<storage, read_write> output_words: array<u32>;

fn rec709_oetf(linear: vec3<f32>) -> vec3<f32> {
    let value = clamp(linear, vec3<f32>(0.0), vec3<f32>(1.0));
    let lower = value * 4.5;
    let upper = 1.099 * pow(value, vec3<f32>(0.45)) - 0.099;
    return select(upper, lower, value < vec3<f32>(0.018));
}

fn hash_noise(pixel: vec2<u32>, salt: u32) -> f32 {
    var value = pixel.x * 0x9e3779b9u;
    value = value ^ (pixel.y * 0x85ebca6bu);
    value = value ^ salt;
    value = value ^ (value >> 16u);
    value = value * 0x7feb352du;
    value = value ^ (value >> 15u);
    value = value * 0x846ca68bu;
    value = value ^ (value >> 16u);
    return f32(value & 0x00ffffffu) / 16777215.0 - 0.5;
}

fn quantize_code(code: f32, pixel: vec2<u32>, salt: u32, low: f32, high: f32) -> u32 {
    var adjusted = code;
    if (params.dither == 1u) {
        adjusted += hash_noise(pixel, salt);
    }
    return u32(round(clamp(adjusted, low, high))) << 6u;
}

@compute @workgroup_size(16, 8, 1)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let pair = id.x;
    let y = id.y;
    let x0 = pair * 2u;
    if (x0 >= params.width || y >= params.height) {
        return;
    }
    let x1 = x0 + 1u;
    let rgb0 = rec709_oetf(textureLoad(source, vec2<i32>(i32(x0), i32(y)), 0).rgb);
    let rgb1 = rec709_oetf(textureLoad(source, vec2<i32>(i32(x1), i32(y)), 0).rgb);

    let luma0 = dot(rgb0, vec3<f32>(0.2126, 0.7152, 0.0722));
    let luma1 = dot(rgb1, vec3<f32>(0.2126, 0.7152, 0.0722));
    let y0 = quantize_code(
        64.0 + luma0 * 876.0,
        vec2<u32>(x0, y),
        0xa511e9b3u,
        64.0,
        940.0,
    );
    let y1 = quantize_code(
        64.0 + luma1 * 876.0,
        vec2<u32>(x1, y),
        0x63d83595u,
        64.0,
        940.0,
    );

    let chroma_rgb = (rgb0 + rgb1) * 0.5;
    let chroma_luma = dot(chroma_rgb, vec3<f32>(0.2126, 0.7152, 0.0722));
    let cb = (chroma_rgb.b - chroma_luma) / 1.8556;
    let cr = (chroma_rgb.r - chroma_luma) / 1.5748;
    let u = quantize_code(
        512.0 + cb * 896.0,
        vec2<u32>(pair, y),
        0xc2b2ae35u,
        64.0,
        960.0,
    );
    let v = quantize_code(
        512.0 + cr * 896.0,
        vec2<u32>(pair, y),
        0x27d4eb2fu,
        64.0,
        960.0,
    );

    let pairs_per_row = params.width / 2u;
    let word_index = y * pairs_per_row + pair;
    let y_plane_words = pairs_per_row * params.height;
    output_words[word_index] = y0 | (y1 << 16u);
    output_words[y_plane_words + word_index] = u | (v << 16u);
}
