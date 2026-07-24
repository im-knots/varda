/*{
    "DESCRIPTION": "Varda VJ port of an untitled Shadertoy superquadric Truchet-tube raymarcher — first-person flythrough of an infinite tunnel built from randomly-oriented superquadric 'truchet arc' cells",
    "CREDIT": "Varda VJ (ported from an untitled/uncredited Shadertoy superquadric Truchet-tube raymarcher)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "thickness", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.02, "MAX": 0.3, "LABEL": "Tube Thickness"},
        {"NAME": "superquad_power", "TYPE": "float", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 16.0, "LABEL": "Superquadric Power"},
        {"NAME": "raymarch_steps", "TYPE": "float", "DEFAULT": 64.0, "MIN": 16.0, "MAX": 128.0, "LABEL": "Raymarch Steps"},
        {"NAME": "light_intensity", "TYPE": "float", "DEFAULT": 1.4, "MIN": 0.0, "MAX": 4.0, "LABEL": "Light Intensity"},
        {"NAME": "env_intensity", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0, "LABEL": "Environment Glow"},
        {"NAME": "vignette_power", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.05, "MAX": 2.0, "LABEL": "Vignette Power"},
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
    float thickness;
    float superquad_power;
    float raymarch_steps;
    float light_intensity;
    float env_intensity;
    float vignette_power;
    vec4 tint;
};

float rand(vec3 r) {
    return fract(sin(dot(r.xy, vec2(1.38984 * sin(r.z), 1.13233 * cos(r.z)))) * 653758.5453);
}

// One superquadric "arc" segment: a rounded square-tube bent through 90
// degrees, expressed as a distance field via the p-norm blend of the
// radial and axial superquadric terms (SuperQuadPower controls how
// square vs. round the tube's cross-section reads).
float truchetarc(vec3 pos) {
    float sqp = clamp(superquad_power, 2.0, 16.0);
    float r = length(pos.xy);
    return pow(pow(abs(r - 0.5), sqp) + pow(abs(pos.z - 0.5), sqp), 1.0 / sqp) - thickness;
}

// A unit cell contains three arcs (one per axis pair); taking the min
// of all three is what makes each cell a continuous truchet tile that
// connects seamlessly to its neighbors regardless of orientation.
float truchetcell(vec3 pos) {
    return min(min(
        truchetarc(pos),
        truchetarc(vec3(pos.z, 1.0 - pos.x, pos.y))),
        truchetarc(vec3(1.0 - pos.y, 1.0 - pos.z, pos.x)));
}

// Per-grid-cell hash picks one of 8 axis-swap/flip orientations for the
// cell's arcs, which is what gives the tunnel its non-repeating, tiled
// truchet look rather than an obviously periodic lattice.
float distfunc(vec3 pos) {
    vec3 cellpos = fract(pos);
    vec3 gridpos = floor(pos);

    float rnd = rand(gridpos);

    if (rnd < 1.0 / 8.0) return truchetcell(vec3(cellpos.x, cellpos.y, cellpos.z));
    else if (rnd < 2.0 / 8.0) return truchetcell(vec3(cellpos.x, 1.0 - cellpos.y, cellpos.z));
    else if (rnd < 3.0 / 8.0) return truchetcell(vec3(1.0 - cellpos.x, cellpos.y, cellpos.z));
    else if (rnd < 4.0 / 8.0) return truchetcell(vec3(1.0 - cellpos.x, 1.0 - cellpos.y, cellpos.z));
    else if (rnd < 5.0 / 8.0) return truchetcell(vec3(cellpos.y, cellpos.x, 1.0 - cellpos.z));
    else if (rnd < 6.0 / 8.0) return truchetcell(vec3(cellpos.y, 1.0 - cellpos.x, 1.0 - cellpos.z));
    else if (rnd < 7.0 / 8.0) return truchetcell(vec3(1.0 - cellpos.y, cellpos.x, 1.0 - cellpos.z));
    else return truchetcell(vec3(1.0 - cellpos.y, 1.0 - cellpos.x, 1.0 - cellpos.z));
}

vec3 gradient(vec3 pos) {
    const float eps = 0.0001;
    float mid = distfunc(pos);
    return vec3(
        distfunc(pos + vec3(eps, 0.0, 0.0)) - mid,
        distfunc(pos + vec3(0.0, eps, 0.0)) - mid,
        distfunc(pos + vec3(0.0, 0.0, eps)) - mid
    );
}

// Raymarch + shade (inlined from the original's mainVR helper — it was
// only a Shadertoy VR-mode convenience wrapper, no VR-specific
// semantics survive here).
vec3 raymarch(vec3 ray_pos, vec3 ray_dir, int max_steps) {
    float i = float(max_steps);
    for (int j = 0; j < 128; j++) {
        if (j >= max_steps) break;
        float dist = distfunc(ray_pos);
        ray_pos += dist * ray_dir;
        if (abs(dist) < 0.001) { i = float(j); break; }
    }

    vec3 normal = normalize(gradient(ray_pos));

    float ao = 1.0 - i / float(max_steps);
    float what = pow(max(0.0, dot(normal, -ray_dir)), 2.0);
    float light = ao * what * light_intensity;

    vec3 col = (cos(ray_pos / 2.0) + 2.0) / 3.0;

    // The original samples an environment-reflection texture (iChannel0)
    // here; Varda generators are self-contained with no bound input
    // image, so this replaces that texture lookup with a cheap
    // procedural "fake environment" glow — a 3-band cosine palette keyed
    // off the reflected direction, reading as a soft varied sky/env tint
    // rather than a flat color.
    vec3 reflected = reflect(ray_dir, normal);
    vec3 env = 0.5 + 0.5 * cos(6.2831 * (vec3(0.1, 0.3, 0.5) + dot(reflected, vec3(0.577)) * vec3(1.0, 1.7, 2.3)));

    return col * light + env_intensity * env;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); this shader's
    // fisheye-ish camera projection was ported from Shadertoy's
    // bottom-left/y-up fragCoord convention, so flip y here before
    // building `coords` — otherwise "up" in the tunnel would render at
    // the bottom of frame and the tube would appear vertically mirrored.
    vec2 res = RENDERSIZE;
    vec2 fragXY = vec2(uv.x, 1.0 - uv.y) * res;
    vec2 coords = (2.0 * fragXY - res) / length(res);

    const float pi = 3.141592;
    float t = PHASE_TIME_0 / 3.0;

    // Camera-facing rotation: squaring the base matrix twice (m*=m twice)
    // compounds the transform fourfold, carried over faithfully from the
    // original.
    float a = t;
    mat3 m = mat3(
        0.0, 1.0, 0.0,
        -sin(a), 0.0, cos(a),
        cos(a), 0.0, sin(a)
    );
    m *= m;
    m *= m;

    vec3 ray_dir = m * normalize(vec3(2.0 * coords, -1.0 + dot(coords, coords)));

    // Handwritten Lissajous-style camera path threading through the tube.
    vec3 ray_pos = vec3(
        2.0 * (sin(t + sin(2.0 * t) / 2.0) / 2.0 + 0.5),
        2.0 * (sin(t - sin(2.0 * t) / 2.0 - pi / 2.0) / 2.0 + 0.5),
        2.0 * ((-2.0 * (t - sin(4.0 * t) / 4.0) / pi) + 0.5 + 0.5)
    );

    int max_steps = int(clamp(raymarch_steps, 4.0, 128.0));
    vec3 col = raymarch(ray_pos, ray_dir, max_steps);

    float vigPow = clamp(vignette_power, 0.05, 4.0);
    float vignette = pow(max(0.0, 1.0 - length(coords)), vigPow);
    col *= vignette;
    col *= tint.rgb;

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
