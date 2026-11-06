/*{
    "DESCRIPTION": "Aurora: thin emissive curtains over a dark sky, with field-aligned rays, a sharp lower edge and color that follows height",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Speed"},
        {"NAME": "activity", "TYPE": "float", "DEFAULT": 0.44, "MIN": 0.0, "MAX": 1.0, "LABEL": "Activity"},
        {"NAME": "exposure", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Exposure"},
        {"NAME": "ray_speed", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Ray Speed"},
        {"NAME": "position_rate", "TYPE": "float", "GROUP": "Motion", "DEFAULT": 0.0, "MIN": -8.0, "MAX": 8.0, "LABEL": "Position Rate"},
        {"NAME": "fold", "TYPE": "float", "GROUP": "Form", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Fold"},
        {"NAME": "rays", "TYPE": "float", "GROUP": "Form", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Rays"},
        {"NAME": "palette", "TYPE": "long", "GROUP": "Palette", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6], "LABELS": ["Natural", "Polar", "Great Storm", "Solar", "Ultraviolet", "Glacier", "Neon"], "LABEL": "Palette"},
        {"NAME": "top_amount", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 4.0, "LABEL": "Top Amount"},
        {"NAME": "hem_amount", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 4.0, "LABEL": "Hem Amount"},
        {"NAME": "tint_amount", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.22, "MIN": 0.0, "MAX": 1.0, "LABEL": "Tint Amount"},
        {"NAME": "glow", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Glow"},
        {"NAME": "sky", "TYPE": "float", "GROUP": "Sky", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Sky"},
        {"NAME": "ground", "TYPE": "float", "GROUP": "Sky", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Lake and Trees"},
        {"NAME": "star_amount", "TYPE": "float", "GROUP": "Sky", "DEFAULT": 0.07, "MIN": 0.0, "MAX": 1.0, "LABEL": "Stars"},
        {"NAME": "camera_tilt", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 38.0, "MIN": -5.0, "MAX": 90.0, "LABEL": "Tilt"},
        {"NAME": "camera_pan", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 62.0, "MIN": -180.0, "MAX": 180.0, "LABEL": "Pan"},
        {"NAME": "fov", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 84.0, "MIN": 20.0, "MAX": 120.0, "LABEL": "Field of View"},
        {"NAME": "steps", "TYPE": "long", "GROUP": "Detail", "DEFAULT": 16, "VALUES": [8, 12, 16], "LABELS": ["8", "12", "16"], "SPECIALIZE": true, "LABEL": "Steps"},
        {"NAME": "march_scale", "TYPE": "float", "GROUP": "Detail", "DEFAULT": 0.5, "MIN": 0.25, "MAX": 1.0, "LABEL": "March Scale"},
        {"NAME": "grain", "TYPE": "float", "GROUP": "Detail", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0, "LABEL": "Step Jitter"},
        {"NAME": "sensor_noise", "TYPE": "float", "GROUP": "Detail", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 2.0, "LABEL": "Sensor Noise"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "INDEX": 0, "SCALE": 1.0},
        {"PARAM": "ray_speed", "INDEX": 1, "SCALE": 1.0},
        {"PARAM": "position_rate", "INDEX": 2, "SCALE": 1.0}
    ],
    "SPECIALIZE_PASSES": true,
    "PASSES": [
        {"TARGET": "field", "WIDTH": "1024", "HEIGHT": "1024"},
        {"TARGET": "soft", "WIDTH": "192", "HEIGHT": "192"},
        {"TARGET": "lines", "WIDTH": "$WIDTH*$march_scale", "HEIGHT": "$HEIGHT*$march_scale"},
        {"TARGET": "glow_a", "WIDTH": "$WIDTH/8", "HEIGHT": "$HEIGHT/8"},
        {"TARGET": "glow_b", "WIDTH": "$WIDTH/16", "HEIGHT": "$HEIGHT/16"},
        {"TARGET": "glow_c", "WIDTH": "$WIDTH/8", "HEIGHT": "$HEIGHT/8"},
        {}
    ]
}*/

#version 450

layout(location = 0) in vec2 uv;
layout(location = 0) out vec4 fragColor;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint FRAMEINDEX;
    int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
    vec2 JITTER;
    int JITTERINDEX;
    int HISTORYVALID;
};

layout(set = 0, binding = 1) uniform sampler texSampler;
// Sheet map: phase pair, sheet brightness, fine phase offset.
layout(set = 0, binding = 2) uniform texture2D field;
// Slow map: lagged red envelope, diffuse glow, lit share, lower edge lift.
layout(set = 0, binding = 3) uniform texture2D soft;
// Line accumulators: 557.7, prompt N2, 630, lower hem.
layout(set = 0, binding = 4) uniform texture2D lines;
layout(set = 0, binding = 5) uniform texture2D glow_a;
layout(set = 0, binding = 6) uniform texture2D glow_b;
layout(set = 0, binding = 7) uniform texture2D glow_c;

layout(set = 0, binding = 8) uniform UserParams {
    float speed;
    float activity;
    float exposure;
    float ray_speed;
    float position_rate;
    float fold;
    float rays;
    int palette;
    float top_amount;
    float hem_amount;
    float tint_amount;
    float glow;
    float sky;
    float ground;
    float star_amount;
    float camera_tilt;
    float camera_pan;
    float fov;
    int steps;
    float march_scale;
    float grain;
    float sensor_noise;
};

layout(constant_id = 0) const int SPEC_STEPS = 16;
layout(constant_id = 1) const int SPEC_PASSINDEX = 0;

int specialized(int value) {
    return value;
}
#define STEPS specialized(SPEC_STEPS)
#define PASS specialized(SPEC_PASSINDEX)

const float TAU = 6.28318530717959;

// Earth radius and map geometry, km.
const float EARTH = 6371.0;
const float MAP_R0 = 300.0;
const float MAP_REACH = 2200.0;
// Stored phase wraps every 8 sheets, so a long step still unwraps.
const float PHASE_PERIOD = 8.0;
// March shell, km.
const float SHELL_LOW = 88.0;
const float SHELL_HIGH = 400.0;
// Mean sheet spacing, km.
const float SPACING = 170.0;
// Sheet half width in phase units, about 1.7 km.
const float SHEET_W = 0.016;
// The comb's width in phase, a little over the sheet's own. Light stays that of
// SHEET_W, so this only softens sheets seen edge on.
const float COMB_W = 0.03;
const float COMB_WIDE = 0.22;
// Below this path in km a sheet adds nothing visible, so it is not read.
const float MIN_PATH = 0.06;
// Where time zero falls in the display, a moment with one clear ribbon.
const float START = 5000.0;
// Magnetic zenith sits 12 degrees south of the true zenith.
const float FIELD_TILT = 0.2126;

// ── Noise ───────────────────────────────────────────────────────────────

float hash(ivec3 c) {
    uvec3 v = uvec3(c) * 1664525u + 1013904223u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> 16u;
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    return float(v.y >> 8u) / 16777216.0;
}

// Value noise over (p, tau), with the gradient in p.
vec3 noiseGrad(vec2 p, float tau, int seed) {
    vec2 ip = floor(p);
    vec2 f = p - ip;
    vec2 u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    vec2 du = 30.0 * f * f * (f * (f - 2.0) + 1.0);
    ivec2 i = ivec2(ip);
    float kf = floor(tau);
    float w = tau - kf;
    w = w * w * (3.0 - 2.0 * w);
    int k = int(kf) + seed * 7919;
    float a0 = hash(ivec3(i.x, i.y, k));
    float b0 = hash(ivec3(i.x + 1, i.y, k));
    float c0 = hash(ivec3(i.x, i.y + 1, k));
    float d0 = hash(ivec3(i.x + 1, i.y + 1, k));
    float a1 = hash(ivec3(i.x, i.y, k + 1));
    float b1 = hash(ivec3(i.x + 1, i.y, k + 1));
    float c1 = hash(ivec3(i.x, i.y + 1, k + 1));
    float d1 = hash(ivec3(i.x + 1, i.y + 1, k + 1));
    float a = mix(a0, a1, w);
    float b = mix(b0, b1, w);
    float c = mix(c0, c1, w);
    float d = mix(d0, d1, w);
    float k1 = b - a;
    float k2 = c - a;
    float k3 = a - b - c + d;
    return vec3(a + k1 * u.x + k2 * u.y + k3 * u.x * u.y,
                du.x * (k1 + k3 * u.y),
                du.y * (k2 + k3 * u.x));
}

// One-dimensional value noise over (x, tau).
float noise1(float x, float tau, int seed) {
    float ix = floor(x);
    float f = x - ix;
    float u = f * f * (3.0 - 2.0 * f);
    float kf = floor(tau);
    float w = tau - kf;
    w = w * w * (3.0 - 2.0 * w);
    int i = int(ix);
    int k = int(kf);
    float a = mix(hash(ivec3(i, seed, k)), hash(ivec3(i, seed, k + 1)), w);
    float b = mix(hash(ivec3(i + 1, seed, k)), hash(ivec3(i + 1, seed, k + 1)), w);
    return mix(a, b, u);
}

// Divergence-free displacement from a value-noise potential.
vec2 curl(vec2 p, vec2 scale, float tau, float amount, int seed) {
    vec3 n = noiseGrad(p / scale, tau, seed);
    return amount * vec2(n.z, -n.y * scale.y / scale.x);
}

// ── Sheet field ─────────────────────────────────────────────────────────

// Ground position in km (x east, y north) to and from the fisheye map.
vec2 mapFromGround(vec2 q) {
    float r = length(q);
    float limit = MAP_REACH / (MAP_REACH + MAP_R0);
    return 0.5 + 0.5 * q / max(r + MAP_R0, 1e-3) / limit;
}

vec2 groundFromMap(vec2 m) {
    float limit = MAP_REACH / (MAP_REACH + MAP_R0);
    vec2 d = (m - 0.5) * 2.0 * limit;
    float u = min(length(d), 0.999);
    float r = MAP_R0 * u / (1.0 - u);
    return d / max(u, 1e-5) * r;
}

// Warped coordinates: x runs along the sheets, y across them. The coarse
// warp stays near linear over a march step; the fine warp only moves where a
// ray meets a sheet.
vec2 warpCoarse(vec2 q, float t) {
    vec2 p = q + vec2(0.0, t * 0.12);
    p += curl(p, vec2(1300.0, 640.0), t / 240.0, 760.0, 1);
    p += curl(p, vec2(420.0, 190.0), t / 80.0, 300.0 * fold, 2);
    return p;
}

vec2 warpFine(vec2 p, float t) {
    p += curl(p, vec2(92.0, 50.0), t / 24.0, 44.0 * fold, 3);
    p += curl(p, vec2(26.0, 14.0), t / 7.0, 9.0 * fold, 4);
    return p;
}

// Across-sheet phase with uneven spacing. Monotonic in y.
float acrossPhase(float y) {
    float v = y / SPACING;
    return v + 0.42 * sin(v * 0.83 + 1.3) + 0.2 * sin(v * 1.91 + 4.1);
}

// Lit share of the sky at a place, smooth across several sheets.
float litField(float along, float F, float t) {
    float wide = noiseGrad(vec2(along / 1500.0, F / 1.25), t / 90.0, 30).x;
    float near = noiseGrad(vec2(along / 320.0, F / 1.1), t / 33.0, 31).x;
    float v = 0.72 * wide + 0.28 * near;
    float gate = 0.8 - 0.5 * activity;
    return smoothstep(gate - 0.07, gate + 0.1, v) * smoothstep(0.0, 0.06, activity);
}

// Coarse phase pair, sheet brightness, fine phase offset.
vec4 sheetField(vec2 q, float t, float tRay, float texelKm) {
    vec2 c = warpCoarse(q, t);
    vec2 p = warpFine(c, t);
    float Fc = acrossPhase(c.y);
    float F = acrossPhase(p.y);
    // Brightness and rays key on the sheet's number, not on the phase, so a
    // march hit that lands a little off the sheet reads the same values.
    float sheet = floor(F + 0.5);
    float envelope = litField(p.x, sheet, t) * (0.55 + 0.9 * noise1(sheet * 0.83, 0.0, 12));

    // Rays drift along the sheet at their own rates. Broad bundles carry bright
    // and dim stretches, and thin filaments at three finer scales sit inside
    // them, each a ridge of noise so it stays a line and not a soft band. How
    // strong the filaments are varies along the sheet, so some stretches stay
    // smooth and the pattern never reads as even stripes.
    float bundle = noise1((p.x + tRay * 1.4) / 90.0 + sheet * 3.1, tRay / 11.0, 80);
    float ray = noise1((p.x - tRay * 3.5) / 27.0 + sheet * 5.3, tRay / 3.4, 81);
    float line1 = 1.0 - abs(2.0 * noise1((p.x + tRay * 2.6) / 13.0 + sheet * 7.7, tRay / 2.2, 82) - 1.0);
    float line2 = 1.0 - abs(2.0 * noise1((p.x - tRay * 4.8) / 6.0 + sheet * 9.1, tRay / 1.4, 84) - 1.0);
    float line3 = 1.0 - abs(2.0 * noise1((p.x + tRay * 7.5) / 2.8 + sheet * 11.3, tRay / 0.9, 85) - 1.0);
    float fineWeight = 1.0 - smoothstep(0.9, 2.4, texelKm);
    float strands = 0.55 * line1 * line1 * line1 + 0.5 * line2 * line2 * line2 * line2
                  + fineWeight * 0.45 * line3 * line3 * line3 * line3;
    float patchy = smoothstep(0.25, 0.7, noise1(p.x / 190.0 + sheet * 2.3, t / 45.0, 86));
    float shaped = 0.5 * bundle + 0.3 * ray;
    shaped = pow(max(shaped, 0.0), 2.2) * 3.6 + mix(0.12, 1.1, patchy) * strands;
    // Brightness pools along the ribbon over hundreds of km.
    float pool = noise1(p.x / 420.0 + sheet * 1.7, t / 70.0, 83);

    float angle = Fc * TAU / PHASE_PERIOD;
    return vec4(sin(angle), cos(angle), envelope * (0.35 + 1.5 * pool * pool) * max(mix(0.75, shaped, rays), 0.0), F - Fc);
}

// ── View ────────────────────────────────────────────────────────────────

vec3 viewRay(vec2 pixel, vec2 size) {
    vec2 p = (2.0 * pixel - size) / size.y;
    p.y = -p.y;
    float f = 1.0 / tan(radians(fov) * 0.5);
    vec3 rd = normalize(vec3(p, f));
    float ct = cos(radians(camera_tilt));
    float st = sin(radians(camera_tilt));
    rd = vec3(rd.x, ct * rd.y + st * rd.z, ct * rd.z - st * rd.y);
    float cp = cos(radians(camera_pan));
    float sp = sin(radians(camera_pan));
    return vec3(cp * rd.x + sp * rd.z, rd.y, cp * rd.z - sp * rd.x);
}

// Unit screen direction of the field line through a pixel. Field lines are
// parallel, so on screen they all run through one vanishing point. Below the
// horizon the lake mirrors them.
vec2 fieldDirection(vec2 pixel, vec2 size) {
    vec2 p = (2.0 * pixel - size) / size.y;
    p.y = -p.y;
    float f = 1.0 / tan(radians(fov) * 0.5);
    vec3 rd = viewRay(pixel, size);
    vec3 field = vec3(0.0, rd.y < 0.0 ? -1.0 : 1.0, -FIELD_TILT);
    float ct = cos(radians(camera_tilt));
    float st = sin(radians(camera_tilt));
    float cp = cos(radians(camera_pan));
    float sp = sin(radians(camera_pan));
    float x = cp * field.x - sp * field.z;
    float z = sp * field.x + cp * field.z;
    float y = ct * field.y - st * z;
    z = st * field.y + ct * z;
    vec2 along = f * vec2(x, y) - p * z;
    along.y = -along.y;
    return along / max(length(along), 1e-4);
}

// Path length from the observer to altitude h along a ray.
float pathAt(vec3 rd, float h) {
    float b = EARTH * rd.y;
    return sqrt(b * b + h * (2.0 * EARTH + h)) - b;
}

float heightAt(vec3 rd, float s) {
    return sqrt(EARTH * EARTH + s * (2.0 * EARTH * rd.y + s)) - EARTH;
}

// Footprint of a ray point, traced down the field to the base height.
vec2 footAt(vec3 rd, float s, float h) {
    vec2 ground = rd.xz * s * EARTH / (EARTH + h);
    ground.y += (h - SHELL_LOW) * FIELD_TILT;
    return ground;
}

// ── March ───────────────────────────────────────────────────────────────

float ign(vec2 pixel) {
    return fract(52.9829189 * fract(dot(pixel, vec2(0.06711056, 0.00583715))));
}

// Integral of a logistic comb with sheets at the integers. Each period adds 1.
float comb(float x, float w) {
    float n = floor(x + 0.5);
    return n + 0.5 * tanh(clamp((x - n) / w, -9.0, 9.0));
}

float sech2(float x) {
    float c = cosh(clamp(x, -9.0, 9.0));
    return 1.0 / (c * c);
}

// Prompt emission against height over the local lower edge, km.
float promptProfile(float x, float top) {
    return smoothstep(-6.0, 8.0, x) * exp(-max(x - 7.0, 0.0) / top);
}

// Line emission per km of sheet at height h: 557.7, prompt N2, 630, lower hem.
// Color comes from height. Green and N2 share one profile. Red is a second, wider
// profile above them, with yellow where the two overlap. Above that sunlit N2
// turns the tops magenta. The hem is N2 light where green is suppressed.
vec4 emission(float h, float bright, float lift) {
    float edge = 103.0 - 2.0 * min(bright, 1.0) + lift;
    float top = (12.0 + 44.0 * min(bright, 1.5)) * (1.0 + clamp(lift, -9.0, 9.0) / 12.0);
    float above = h - edge;
    float prompt = promptProfile(above, top) * (1.0 - smoothstep(200.0, 340.0, h));
    float green = prompt * smoothstep(-1.0, 3.0, above);
    float hem = prompt * (1.0 - smoothstep(0.0, 7.0, above)) * smoothstep(0.2, 1.0, bright);
    float high = (h - 235.0 - 1.5 * lift) / 95.0;
    float red = smoothstep(115.0, 215.0, h) * exp(-high * high) * (1.0 - smoothstep(330.0, SHELL_HIGH, h));
    float sunlit = smoothstep(190.0, 300.0, h) * (1.0 - smoothstep(320.0, SHELL_HIGH, h));
    return 0.2 * vec4(bright * green, bright * prompt + 0.2 * sqrt(bright) * sunlit, sqrt(bright) * red, bright * hem);
}

// Light of one sheet at a place on the ray, per km of path. The coarse phase
// places the meeting and the fine offset moves it, so a step never samples a
// fold it cannot resolve.
vec4 sheetLight(vec3 rd, float s, float slope, float reach, float sheet) {
    float h = heightAt(rd, s);
    vec4 first = textureLod(sampler2D(field, texSampler), mapFromGround(footAt(rd, s, h)), 0.0);
    float offset = first.w;
    // One Newton step onto the sheet: the phase left over at this estimate plus
    // the fine offset. Rays read off the sheet would wander with the step.
    float left = atan(first.x, first.y) * PHASE_PERIOD / TAU - sheet;
    left -= PHASE_PERIOD * floor(left / PHASE_PERIOD + 0.5);
    // Damped so the shift fades out, with no jump, as the ray turns along the sheet.
    float shift = -left * slope / (slope * slope + 1e-7) - offset * slope / (slope * slope + 1e-6);
    shift = clamp(shift, -0.75 * reach, 0.75 * reach);
    float s1 = max(s + shift, 1.0);
    float h1 = heightAt(rd, s1);
    vec2 at = mapFromGround(footAt(rd, s1, h1));
    vec4 c = textureLod(sampler2D(field, texSampler), at, 0.0);
    float full = slope + (abs(shift) > 0.1 ? (c.w - offset) / shift : 0.0);
    float gather = (abs(slope) + 0.005) / (abs(full) + 0.005);
    vec2 slow = textureLod(sampler2D(soft, texSampler), at, 0.0).zw;
    // A ray running along a sheet passes many rays in one reading, so it sees
    // their mean.
    float clean = smoothstep(8e-4, 3e-3, abs(slope) / max(length(rd.xz), 0.15));
    float sharp = c.z;
    // A ray crosses a sheet at a shallow angle over a long path and passes many
    // strands, so one reading lands on a different strand in each pixel. Averaging
    // along that path gives the strands the ray really sees.
    float inside = min(SHEET_W / max(abs(slope), 1e-5), 0.5 * reach);
    if (inside > 2.0) {
        float a0 = max(s1 - inside, 1.0);
        float a1 = s1 + inside;
        sharp = (sharp + textureLod(sampler2D(field, texSampler), mapFromGround(footAt(rd, a0, heightAt(rd, a0))), 0.0).z
                       + textureLod(sampler2D(field, texSampler), mapFromGround(footAt(rd, a1, heightAt(rd, a1))), 0.0).z) / 3.0;
    }
    float bright = mix(0.8 * slow.x, sharp, clean);
    return emission(h1, bright, slow.y) * min(gather, 1.5);
}

// Path through sheets, in km, on a chord where phase runs from Fa to Fb, sheets
// at integers. Where a ray nearly follows a sheet, the smallest phase error moves
// their meeting by tens of km, so that path spreads over a wide comb.
float chordPath(float Fa, float Fb, float ds, float level) {
    float span = abs(Fb - Fa);
    float Flo = min(Fa, Fb);
    float Fhi = max(Fa, Fb);
    float wide = smoothstep(4.5e-3, 1.5e-3, span / ds / level);
    if (span < 1e-3) {
        float mid = 0.5 * (Fa + Fb);
        mid -= floor(mid + 0.5);
        return ds * SHEET_W * mix(sech2(mid / COMB_W) / COMB_W, sech2(mid / COMB_WIDE) / COMB_WIDE, wide);
    }
    float across = comb(Fhi, COMB_W) - comb(Flo, COMB_W);
    if (wide > 0.0) {
        across = mix(across, comb(Fhi, COMB_WIDE) - comb(Flo, COMB_WIDE), wide);
    }
    return 2.0 * SHEET_W * ds * across / span;
}

// Slope of the cubic across a step, u in 0 to 1.
float cubicSlope(float u, float d, float m0, float m1) {
    return 6.0 * u * (1.0 - u) * d + (1.0 - u) * (1.0 - 3.0 * u) * m0 + u * (3.0 * u - 2.0) * m1;
}

vec4 march(vec3 rd, float jitter) {
    if (rd.y < 0.002) {
        return vec4(0.0);
    }
    float n = float(STEPS);
    float range = SHELL_HIGH - SHELL_LOW;

    // Unwrapped coarse phase at every step boundary, one extra at each end.
    float hs[19];
    float ss[19];
    float Fs[19];
    float previous = 0.0;
    float running = 0.0;
    for (int i = 0; i <= STEPS + 2; i++) {
        float x = (float(i) - 1.0 + jitter) / n;
        float h = SHELL_LOW + range * x * abs(x) * sqrt(abs(x));
        float s = pathAt(rd, h);
        vec2 p = textureLod(sampler2D(field, texSampler), mapFromGround(footAt(rd, s, h)), 0.0).xy;
        float a = atan(p.x, p.y);
        float d = a - previous;
        d -= TAU * floor(d / TAU + 0.5);
        running = i == 0 ? a * PHASE_PERIOD / TAU : running + d * PHASE_PERIOD / TAU;
        previous = a;
        hs[i] = h;
        ss[i] = s;
        Fs[i] = running;
    }

    // Slopes of the cubic spline through the phase nodes. Warps bend phase within
    // a step, and where a ray nearly follows a sheet the error of a local slope
    // estimate moves their meeting by tens of km, a ladder at the step spacing.
    float ms[19];
    float sub[19];
    ms[0] = (Fs[1] - Fs[0]) / (ss[1] - ss[0]);
    sub[0] = 0.0;
    int lastNode = STEPS + 2;
    for (int i = 1; i < lastNode; i++) {
        float h0 = ss[i] - ss[i - 1];
        float h1 = ss[i + 1] - ss[i];
        float d0 = (Fs[i] - Fs[i - 1]) / h0;
        float d1 = (Fs[i + 1] - Fs[i]) / h1;
        float pivot = 2.0 * (h0 + h1) - h1 * sub[i - 1];
        sub[i] = h0 / pivot;
        ms[i] = (3.0 * (h1 * d0 + h0 * d1) - h1 * ms[i - 1]) / pivot;
    }
    ms[lastNode] = (Fs[lastNode] - Fs[lastNode - 1]) / (ss[lastNode] - ss[lastNode - 1]);
    for (int i = lastNode - 1; i >= 0; i--) {
        ms[i] -= sub[i] * ms[i + 1];
    }

    // Low rays run so far per step that sheets turn to lines at the step
    // spacing, so they take the mean of the slow map.
    float lowRay = smoothstep(0.1, 0.04, rd.y);
    vec4 sum = vec4(0.0);
    for (int i = 1; i <= STEPS; i++) {
        float s0 = ss[i];
        float ds = ss[i + 1] - s0;
        float F0 = Fs[i];
        float F1 = Fs[i + 1];
        float d = (F1 - F0) / ds;
        float m0 = ms[i];
        float m1 = ms[i + 1];

        float coarse = max(smoothstep(1.1, 2.4, abs(F1 - F0)), lowRay);
        // Most steps pass between sheets and nothing near them needs the cubic.
        float reach = 0.1 + 0.5 * abs(F1 - F0) + 0.25 * smoothstep(5e-3, 1.5e-3, abs(d));
        float centre = 0.5 * (F0 + F1);
        bool near = abs(centre - floor(centre + 0.5)) < reach;
        vec4 light = vec4(0.0);
        if (coarse < 1.0 && near) {
            // Chords of the cubic through the nodes find how much of the step
            // lies in each sheet, with no texture reads. A sheet is then read once
            // for the step, at the middle of that path.
            float level = max(length(rd.xz), 0.15);
            float Fa = F0;
            float held = 0.0;
            float heldAt = 0.0;
            float heldSheet = floor(F0 + 0.5);
            for (int q = 1; q <= 4; q++) {
                float u = float(q) * 0.25;
                float u2 = u * u;
                float Fb = mix(F0, F1, u2 * (3.0 - 2.0 * u)) + ds * u * (1.0 - u) * ((1.0 - u) * m0 - u * m1);
                float sheet = floor(0.5 * (Fa + Fb) + 0.5);
                if (sheet != heldSheet) {
                    if (held > MIN_PATH) {
                        float v = heldAt / held;
                        light += held * sheetLight(rd, s0 + v * ds, cubicSlope(v, d, m0, m1), ds, heldSheet);
                    }
                    held = 0.0;
                    heldAt = 0.0;
                    heldSheet = sheet;
                }
                float path = chordPath(Fa, Fb, 0.25 * ds, level);
                held += path;
                heldAt += path * (u - 0.125);
                Fa = Fb;
            }
            if (held > MIN_PATH) {
                float v = heldAt / held;
                light += held * sheetLight(rd, s0 + v * ds, cubicSlope(v, d, m0, m1), ds, heldSheet);
            }
        }
        if (coarse > 0.0) {
            light *= 1.0 - coarse;
        }
        // Red and diffuse light from the slow map, over a capped path so low
        // rays do not fill with red.
        float h = 0.5 * (hs[i] + hs[i + 1]);
        if (h > 130.0 || coarse > 0.0) {
            float s = pathAt(rd, h);
            vec4 mean = textureLod(sampler2D(soft, texSampler), mapFromGround(footAt(rd, s, h)), 0.0);
            float path = min(ds, 70.0);
            float dr = (h - 225.0) / 75.0;
            light.z += 2.2 * mean.x * exp(-dr * dr) * path;
            light.y += 0.7 * mean.x * exp(-dr * dr) * path;
            light.xy += mean.y * smoothstep(105.0, 135.0, h) * (1.0 - smoothstep(170.0, 260.0, h)) * path;
            light += coarse * 2.0 * SHEET_W * ds * 0.06
                   * (emission(hs[i], mean.z, mean.w) + 2.0 * emission(h, mean.z, mean.w)
                      + emission(hs[i + 1], mean.z, mean.w));
        }
        sum += light;
    }

    return sum;
}

// ── Glow ────────────────────────────────────────────────────────────────

// Four diagonal taps, half a destination texel out.
#define TENT(source, at, texel) (0.25 * ( \
    textureLod(sampler2D(source, texSampler), (at) + (texel) * vec2(-0.5, -0.5), 0.0).xyz + \
    textureLod(sampler2D(source, texSampler), (at) + (texel) * vec2(0.5, -0.5), 0.0).xyz + \
    textureLod(sampler2D(source, texSampler), (at) + (texel) * vec2(-0.5, 0.5), 0.0).xyz + \
    textureLod(sampler2D(source, texSampler), (at) + (texel) * vec2(0.5, 0.5), 0.0).xyz))

// ── Output ──────────────────────────────────────────────────────────────

vec3 stars(vec3 rd, vec2 size) {
    vec3 m = abs(rd);
    int face = m.x > m.y ? (m.x > m.z ? 0 : 2) : (m.y > m.z ? 1 : 2);
    float major = face == 0 ? rd.x : (face == 1 ? rd.y : rd.z);
    vec2 side = (face == 0 ? rd.yz : (face == 1 ? rd.xz : rd.xy)) / abs(major);
    const float CELLS = 150.0;
    vec2 g = side * CELLS;
    vec2 cell = floor(g);
    ivec3 c = ivec3(int(cell.x), int(cell.y), face + (major < 0.0 ? 3 : 0));
    if (hash(c + ivec3(0, 0, 40)) > star_amount * 0.55) {
        return vec3(0.0);
    }
    vec2 at = vec2(hash(c + ivec3(0, 0, 50)), hash(c + ivec3(0, 0, 60))) * 0.7 + 0.15;
    vec2 place = (cell + at) / CELLS;
    float axis = major < 0.0 ? -1.0 : 1.0;
    vec3 dir = face == 0 ? vec3(axis, place) : (face == 1 ? vec3(place.x, axis, place.y) : vec3(place, axis));
    vec3 away = rd - normalize(dir);
    float pixel = 2.0 * tan(radians(fov) * 0.5) / size.y;
    float r2 = dot(away, away) / (pixel * pixel * 0.45);
    float mag = hash(c + ivec3(0, 0, 70));
    float level = 0.008 + 0.35 * mag * mag * mag * mag * mag * mag * mag * mag * mag * mag;
    float warmth = hash(c + ivec3(0, 0, 80));
    return exp(-r2) * level * mix(vec3(1.0, 0.86, 0.72), vec3(0.8, 0.9, 1.0), warmth);
}

// Each palette is a gradient by height: hem, body, tint through the body, top.
// In linear light. No channel is exactly 0, since a channel at 0 never rises
// through a per-channel tonemap. Natural follows the emission lines.
const int STOPS = 4;
const vec3 PALETTE[28] = vec3[28](
    vec3(1.00, 0.30, 0.50), vec3(0.30, 1.00, 0.28), vec3(0.32, 0.20, 1.00), vec3(1.00, 0.28, 0.22),
    vec3(1.00, 0.10, 0.75), vec3(0.04, 1.00, 0.62), vec3(0.20, 0.25, 1.00), vec3(0.60, 0.06, 1.00),
    vec3(1.00, 0.20, 0.60), vec3(0.34, 1.00, 0.10), vec3(0.75, 0.10, 1.00), vec3(2.60, 0.14, 0.75),
    vec3(1.00, 0.70, 0.10), vec3(1.00, 0.42, 0.05), vec3(1.00, 0.08, 0.30), vec3(0.70, 0.04, 0.80),
    vec3(0.10, 0.90, 1.00), vec3(0.50, 0.14, 1.00), vec3(1.00, 0.12, 0.70), vec3(0.10, 0.30, 1.00),
    vec3(0.80, 0.45, 1.00), vec3(0.08, 0.60, 1.00), vec3(0.50, 0.30, 1.00), vec3(0.85, 0.10, 0.90),
    vec3(1.00, 0.80, 0.10), vec3(1.00, 0.10, 0.62), vec3(0.20, 0.40, 1.00), vec3(0.05, 0.80, 1.00)
);

// One stop of the selected palette. Red and blue carry far less luminance than
// green, so the body gains part of the way to green's luminance and palettes hold
// the same level.
vec3 paletteColor(int stop) {
    vec3 color = PALETTE[palette * STOPS + stop];
    if (stop == 1) {
        color *= sqrt(0.76 / dot(color, vec3(0.2126, 0.7152, 0.0722)));
    }
    return color;
}

vec3 skyLight(vec3 rd, vec3 body) {
    float e = max(rd.y, 0.0);
    float band = exp(-pow((e - 0.22) / 0.14, 2.0));
    float floorGlow = 0.007 + 0.009 * band;
    return floorGlow * mix(vec3(0.55, 0.9, 0.78), body / max(body.r, max(body.g, body.b)), 0.6);
}

vec3 outputColor(vec4 line, vec3 rd, vec3 hem, vec3 body, vec3 tint, vec3 top) {
    // A ray looking up a sheet gathers its whole height. The limit is soft and
    // keeps the line ratios, so the corona stays a color and not a white hole.
    line /= 1.0 + max(line.x, line.z) / 2.2;
    float e = max(rd.y, 0.002);
    float air = 1.0 / (e + 0.025 * exp(-11.0 * e));
    vec3 pass_through = exp(-air * vec3(0.094, 0.2, 0.05));
    vec3 g = line.x * pass_through.x * body;
    vec3 n2 = line.y * (pass_through.y * 1.3 * tint_amount * tint
                      + pass_through.z * 0.03 * top);
    vec3 r = line.z * pass_through.z * top_amount * 1.4 * top;
    vec3 fringe = line.w * pass_through.z * hem_amount * hem;
    vec3 col = exposure * (g + n2 + r + fringe);

    // Photographed aurora is far less saturated than its lines. Light from
    // several lines mixes on the sensor, and the sky veil washes it further.
    col = mix(vec3(dot(col, vec3(0.2126, 0.7152, 0.0722))), col, 0.82);

    // Pale highlights above a knee, then all channels scale to a limit, so HDR
    // outputs with no tonemap shoulder keep the same path to white.
    float y = dot(col, vec3(0.2126, 0.7152, 0.0722));
    const float KNEE = 2.2;
    if (y > KNEE) {
        col = mix(col, vec3(y), 1.0 - sqrt(KNEE / y));
    }
    float peak = max(col.r, max(col.g, col.b));
    const float LIMIT = 4.0;
    col *= LIMIT / max(peak, LIMIT);
    return col;
}

// Height of the land above the horizon by azimuth: low hills under two rows of
// conifers. In sine of elevation, so it holds its size across resolutions.
float landHeight(float azimuth) {
    float hills = 0.012 + 0.03 * noise1(azimuth * 2.3, 0.0, 90) + 0.012 * noise1(azimuth * 9.0, 0.0, 91);
    float tall = 0.0;
    for (int row = 0; row < 2; row++) {
        float x = azimuth * (row == 0 ? 95.0 : 160.0) + float(row) * 0.37;
        float cell = floor(x);
        float f = x - cell;
        int i = int(cell);
        float size = hash(ivec3(i, row, 92));
        float tree = max(1.0 - abs(2.0 * f - 1.0) * (1.4 + size), 0.0);
        tall = max(tall, tree * (0.008 + 0.02 * size * size) * (row == 0 ? 1.0 : 0.7));
    }
    return hills + tall;
}

void main() {
    // Position runs both clocks together, forward or back. It is a rate, so a
    // change never skips the display to another moment.
    float t = PHASE_TIME_0 + PHASE_TIME_2 + START;
    float tRay = PHASE_TIME_1 + PHASE_TIME_2 + START;

    if (PASS == 0) {
        vec2 m = gl_FragCoord.xy / RENDERSIZE;
        vec2 q = groundFromMap(m);
        float r = length(q);
        float texelKm = (r + MAP_R0) * 2.0 * MAP_REACH / (MAP_REACH + MAP_R0) / RENDERSIZE.x;
        fragColor = sheetField(q, t, tRay, texelKm);
        return;
    }

    if (PASS == 1) {
        vec2 q = groundFromMap(gl_FragCoord.xy / RENDERSIZE);
        vec2 p = warpCoarse(q, t);
        float F = acrossPhase(p.y);
        float lit = litField(p.x, F, t);
        float dn = (F - floor(F + 0.5)) / 0.6;
        float veil = noiseGrad(vec2(p.x / 1100.0, p.y / 700.0), t / 120.0, 41).x;
        float diffuse = max(lit * exp(-dn * dn), 0.55 * smoothstep(0.25, 0.7, veil) * smoothstep(0.0, 0.1, activity));

        // Red is slow and blurred: the lit field some seconds ago, with only a
        // faint trace of single sheets.
        float lag = t - 18.0;
        vec2 pLag = warpCoarse(q, lag);
        float Fl = acrossPhase(pLag.y);
        float dl = (Fl - floor(Fl + 0.5)) / 0.5;
        float redEnv = litField(pLag.x, Fl, lag) * (0.3 + 0.7 * exp(-dl * dl));

        // Lower edges wander in height along and across sheets, so they do not
        // stack as parallel terraces.
        float lift = (noiseGrad(vec2(p.x / 300.0, F / 1.7), t / 60.0, 40).x - 0.42) * 26.0;
        fragColor = vec4(redEnv * 0.0002, diffuse * 0.00028, lit, lift);
        return;
    }

    if (PASS == 2) {
        vec2 pixel = gl_FragCoord.xy;
        float jitter = 0.5 + grain * (ign(pixel) - 0.5);
        // The lake mirrors the sky, so a ray below the horizon marches upward.
        vec3 rd = viewRay(pixel, RENDERSIZE);
        if (rd.y < 0.0 && ground <= 0.0) {
            fragColor = vec4(0.0);
            return;
        }
        fragColor = march(vec3(rd.x, abs(rd.y), rd.z), jitter);
        return;
    }

    if (PASS == 3) {
        vec2 texel = 1.0 / RENDERSIZE;
        fragColor = vec4(TENT(lines, gl_FragCoord.xy * texel, texel), 1.0);
        return;
    }

    if (PASS == 4) {
        vec2 texel = 1.0 / RENDERSIZE;
        fragColor = vec4(TENT(glow_a, gl_FragCoord.xy * texel, texel), 1.0);
        return;
    }

    if (PASS == 5) {
        vec2 texel = 1.0 / RENDERSIZE;
        fragColor = vec4(TENT(glow_b, gl_FragCoord.xy * texel, texel), 1.0);
        return;
    }

    vec2 at = gl_FragCoord.xy / RENDERSIZE;
    // Smooth the reduced march along the field line through this pixel. Strands
    // lie along it, so they keep their width while the step jitter averages out.
    vec2 stepAlong = fieldDirection(gl_FragCoord.xy, RENDERSIZE) / vec2(textureSize(sampler2D(lines, texSampler), 0));
    float kept = 0.0;
    vec4 farSum = vec4(0.0);
    vec4 line = 0.28 * textureLod(sampler2D(lines, texSampler), at, 0.0)
              + 0.24 * (textureLod(sampler2D(lines, texSampler), at + 1.2 * stepAlong, 0.0)
                      + textureLod(sampler2D(lines, texSampler), at - 1.2 * stepAlong, 0.0))
              + 0.12 * (textureLod(sampler2D(lines, texSampler), at + 2.6 * stepAlong, 0.0)
                      + textureLod(sampler2D(lines, texSampler), at - 2.6 * stepAlong, 0.0));
    // Farther reaches take step-spaced ladders out of far sheets. They only count
    // where they match this pixel, so a sharp lower edge does not smear.
    for (int i = 0; i < 4; i++) {
        float reach = i < 2 ? 5.5 : 9.0;
        vec4 far = textureLod(sampler2D(lines, texSampler), at + (i % 2 == 0 ? reach : -reach) * stepAlong, 0.0);
        float gap = (far.x + far.z - line.x - line.z) / (0.3 * (line.x + line.z) + 0.02);
        kept += exp(-gap * gap);
        farSum += exp(-gap * gap) * far;
    }
    line = (line + 0.5 * farSum) / (1.0 + 0.5 * kept);
    vec3 halo = textureLod(sampler2D(glow_c, texSampler), at, 0.0).xyz;
    vec3 rd = viewRay(gl_FragCoord.xy, RENDERSIZE);
    bool below = rd.y < 0.0;
    vec3 up = vec3(rd.x, abs(rd.y), rd.z);
    // The floor keeps comb tails from lifting black when decks stack in Add.
    vec4 lit = max(line + vec4(0.3 * glow * halo, 0.0) - 2e-4, 0.0);
    vec3 body = paletteColor(1);
    vec3 col = outputColor(lit, up, paletteColor(0), body, paletteColor(2), paletteColor(3));
    col += sky * (skyLight(up, body) + stars(up, RENDERSIZE));
    if (below) {
        // Still water: dim at a steep angle, close to a mirror at a shallow one.
        float grazing = 1.0 - abs(rd.y);
        grazing *= grazing;
        col *= ground * mix(0.3, 0.85, grazing * grazing);
    }
    if (ground > 0.0 && up.y < 0.12) {
        float edge = 1.5 * tan(radians(fov) * 0.5) / RENDERSIZE.y;
        float land = landHeight(atan(rd.x, rd.z) + 3.2);
        float cover = 1.0 - smoothstep(land - edge, land + edge, up.y);
        col = mix(col, sky * vec3(0.0012, 0.0016, 0.0016), cover * ground);
    }

    // Low-light sensor noise, changing each frame and stronger in the mids. It
    // scales with the picture, so exact black stays exact black.
    float noise = hash(ivec3(int(gl_FragCoord.x), int(gl_FragCoord.y), int(FRAMEINDEX))) - 0.5;
    col *= 1.0 + sensor_noise * 0.16 * noise;
    fragColor = vec4(max(col, 0.0), 1.0);
}
