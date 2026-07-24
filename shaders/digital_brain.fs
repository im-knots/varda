/*{
    "DESCRIPTION": "Digital Brain - glowing voronoi-noise plasma that reads as a drifting field of electric circuits/neurons, with pulsing 'moving electrons' on higher octaves",
    "CREDIT": "Varda VJ (ported from 'Digital Brain' by srtuss, 2013)",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 3.0, "LABEL": "Zoom"},
        {"NAME": "electron_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Electron Intensity"},
        {"NAME": "vignette_strength", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 2.0, "LABEL": "Vignette Strength"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 2.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Brightness"},
        {"NAME": "color_bias", "TYPE": "float", "DEFAULT": 0.0, "MIN": -2.0, "MAX": 2.0, "LABEL": "Color Bias"},
        {"NAME": "tint", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0, 1.0], "LABEL": "Tint"}
    ],
    "PHASE_INPUTS": [{"PARAM": "speed", "INDEX": 0, "SCALE": 1.0}]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

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
};

layout(set = 0, binding = 1) uniform UserParams {
    float speed;
    float zoom;
    float electron_intensity;
    float vignette_strength;
    float brightness;
    float color_bias;
    vec4 tint;
};

// rotate position around axis
vec2 rotate(vec2 p, float a)
{
    return vec2(p.x * cos(a) - p.y * sin(a), p.x * sin(a) + p.y * cos(a));
}

// 1D random numbers
float rand(float n)
{
    return fract(sin(n) * 43758.5453123);
}

// 2D random numbers
vec2 rand2(in vec2 p)
{
    return fract(vec2(sin(p.x * 591.32 + p.y * 154.077), cos(p.x * 391.32 + p.y * 49.077)));
}

// 1D noise
float noise1(float p)
{
    float fl = floor(p);
    float fc = fract(p);
    return mix(rand(fl), rand(fl + 1.0), fc);
}

// voronoi distance noise, based on iq's articles
float voronoi(in vec2 x)
{
    vec2 p = floor(x);
    vec2 f = fract(x);

    vec2 res = vec2(8.0);
    for (int j = -1; j <= 1; j++) {
        for (int i = -1; i <= 1; i++) {
            vec2 b = vec2(i, j);
            vec2 r = vec2(b) - f + rand2(p + b);

            // chebyshev distance, one of many ways to do this
            float d = max(abs(r.x), abs(r.y));

            if (d < res.x) {
                res.y = res.x;
                res.x = d;
            } else if (d < res.y) {
                res.y = d;
            }
        }
    }
    return res.y - res.x;
}

// 2D random 3-vector, built from the same sin/cos hash idiom as rand2
// above (a third channel added with different coefficients/phase offset
// so it decorrelates from the first two).
vec3 rand3(in vec2 p)
{
    return fract(vec3(
        sin(p.x * 591.32 + p.y * 154.077),
        cos(p.x * 391.32 + p.y * 49.077),
        sin(p.x * 127.1 + p.y * 311.7 + 7.0)
    ));
}

// Smooth 2D value noise (3-channel), used below in place of the original's
// iChannel0 texture lookup: Varda generators have no bound input image, so
// the low-frequency texture sample used purely as a slowly-drifting color
// source is replaced with a procedural value noise built from rand3.
vec3 colorNoise(in vec2 p)
{
    vec2 i = floor(p);
    vec2 f = fract(p);
    vec2 u = f * f * (3.0 - 2.0 * f);

    vec3 a = rand3(i);
    vec3 b = rand3(i + vec2(1.0, 0.0));
    vec3 c = rand3(i + vec2(0.0, 1.0));
    vec3 d = rand3(i + vec2(1.0, 1.0));

    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    float flicker = noise1(PHASE_TIME_0 * 2.0) * 0.8 + 0.4;

    // Varda's uv is top-left origin (y grows downward), the opposite of
    // Shadertoy's bottom-left/y-up fragCoord convention, so flip y here.
    // This particular pattern is a camera-less, fully rotation/reflection
    // symmetric voronoi noise field with no "up" reference, gravity, or
    // horizon, so the flip has no visible effect on this shader -- it's
    // applied anyway for consistency with every other ported shader in
    // this library.
    vec2 fuv = vec2(uv.x, 1.0 - uv.y);
    vec2 p = (fuv - 0.5) * 2.0;
    vec2 suv = p;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float v = 0.0;

    // a bit of camera movement
    vec2 q = p * ((0.6 + sin(PHASE_TIME_0 * 0.1) * 0.4) * zoom);
    q = rotate(q, sin(PHASE_TIME_0 * 0.3));
    q += PHASE_TIME_0 * 0.4;

    // add some noise octaves
    float a = 0.6;
    float f = 1.0;

    for (int i = 0; i < 3; i++) {
        float v1 = voronoi(q * f + 5.0);
        float v2 = 0.0;

        // make the moving electrons-effect for higher octaves
        if (i > 0) {
            v2 = voronoi(q * f * 0.5 + 50.0 + PHASE_TIME_0);

            float va = 1.0 - smoothstep(0.0, 0.1, v1);
            float vb = 1.0 - smoothstep(0.0, 0.08, v2);
            v += a * pow(va * (0.5 + vb), 2.0) * electron_intensity;
        }

        // make sharp edges
        v1 = 1.0 - smoothstep(0.0, 0.3, v1);

        // noise is used as intensity map
        v2 = a * noise1(v1 * 5.5 + 0.1);

        // octave 0's intensity changes a bit
        if (i == 0) {
            v += v2 * flicker;
        } else {
            v += v2;
        }

        f *= 3.0;
        a *= 0.7;
    }

    // slight vignetting
    v *= exp(-vignette_strength * length(suv)) * 1.2;

    // The original samples iChannel0 (a generic noise/image texture) at two
    // very low frequencies purely to get a slowly-varying per-channel tint;
    // replaced here with the procedural colorNoise() above, sampled at the
    // same two scales so the color still drifts extremely slowly as the
    // camera moves rather than looking static. The original's hardcoded
    // fallbacks (vec3(1.0, 2.0, 4.0), or the commented-out "old blueish"
    // vec3(6.0, 4.0, 2.0)) guided the target range for these exponents.
    vec3 cexp = colorNoise(q * 0.001) * 3.0 + colorNoise(q * 0.01);
    cexp *= 1.4;
    cexp = clamp(cexp + color_bias, 0.3, 8.0);

    vec3 col = vec3(pow(v, cexp.x), pow(v, cexp.y), pow(v, cexp.z)) * brightness;
    col *= tint.rgb;

    fragColor = vec4(col, 1.0);
}
