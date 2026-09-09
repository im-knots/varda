// Content light level reduction for HDR10 mastering metadata.
//
// CTA-861.3 defines MaxCLL as the largest light level of any pixel across the
// programme, and MaxFALL as the largest frame-average light level. Both are
// properties of the encoded content, so this measures the frame that is about to
// be written rather than anything upstream of it.
//
// Reduces 4x4 at a time into (max, sum, count). Carrying the count rather than a
// mean keeps edge tiles exact: a frame whose dimensions are not a multiple of the
// reduction factor has partial tiles, and averaging those as if they were full
// would bias MaxFALL.
//
// See /spec/hdr-recording-output.md and measure.rs, which holds this to known
// values rather than to inspection.

struct MeasureParams {
    // 1 on the first pass, where the source is PQ-encoded video and each texel
    // must be decoded to absolute luminance. 0 on later passes, where the source
    // is already (max, sum, count).
    decode_pq: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0)
var source_texture: texture_2d<f32>;

@group(0) @binding(1)
var<uniform> params: MeasureParams;

// SMPTE ST 2084 constants.
const PQ_M1: f32 = 0.1593017578125;
const PQ_M2: f32 = 78.84375;
const PQ_C1: f32 = 0.8359375;
const PQ_C2: f32 = 18.8515625;
const PQ_C3: f32 = 18.6875;
const PQ_MAX_NITS: f32 = 10000.0;

// PQ EOTF: a [0,1] code to absolute luminance in cd/m².
fn nits_from_pq(code: vec3<f32>) -> vec3<f32> {
    let c = pow(clamp(code, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / PQ_M2));
    let num = max(c - PQ_C1, vec3<f32>(0.0));
    let den = max(PQ_C2 - PQ_C3 * c, vec3<f32>(1e-6));
    return pow(num / den, vec3<f32>(1.0 / PQ_M1)) * PQ_MAX_NITS;
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> @builtin(position) vec4<f32> {
    let x = f32((vertex_index & 1u) << 2u);
    let y = f32((vertex_index & 2u) << 1u);
    return vec4<f32>(x - 1.0, 1.0 - y, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let dst = vec2<i32>(floor(position.xy));
    let dims = vec2<i32>(textureDimensions(source_texture));

    var largest = 0.0;
    var total = 0.0;
    var samples = 0.0;

    for (var dy = 0; dy < 4; dy = dy + 1) {
        for (var dx = 0; dx < 4; dx = dx + 1) {
            let p = dst * 4 + vec2<i32>(dx, dy);
            // Outside the source: a partial tile, not a pixel to invent.
            if (p.x >= dims.x || p.y >= dims.y) {
                continue;
            }
            let texel = textureLoad(source_texture, p, 0);
            if (params.decode_pq == 1u) {
                let nits = nits_from_pq(texel.rgb);
                // CTA-861.3 light level is the largest component, not luminance.
                let level = max(nits.r, max(nits.g, nits.b));
                largest = max(largest, level);
                total = total + level;
                samples = samples + 1.0;
            } else {
                largest = max(largest, texel.r);
                total = total + texel.g;
                samples = samples + texel.b;
            }
        }
    }

    return vec4<f32>(largest, total, samples, 1.0);
}
