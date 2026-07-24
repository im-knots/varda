/*{
    "DESCRIPTION": "Clouds - flythrough of a raymarched volumetric cloud layer with sun glow and rim-lit cloud shadowing",
    "CREDIT": "Varda VJ (ported from a Shadertoy cloud flythrough sketch, https://www.shadertoy.com/view/4sXGRM)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "sway", "TYPE": "float", "DEFAULT": 5000.0, "MIN": 0.0, "MAX": 12000.0, "LABEL": "Flight Sway"},
        {"NAME": "altitude", "TYPE": "float", "DEFAULT": 5000.0, "MIN": 0.0, "MAX": 10000.0, "LABEL": "Altitude"},
        {"NAME": "altitude_wobble", "TYPE": "float", "DEFAULT": 1500.0, "MIN": 0.0, "MAX": 4000.0, "LABEL": "Altitude Wobble"},
        {"NAME": "forward_speed", "TYPE": "float", "DEFAULT": 6000.0, "MIN": 500.0, "MAX": 15000.0, "LABEL": "Flight Speed"},
        {"NAME": "steps", "TYPE": "float", "DEFAULT": 100.0, "MIN": 32.0, "MAX": 256.0, "LABEL": "Raymarch Steps"},
        {"NAME": "scale", "TYPE": "float", "DEFAULT": 0.00025, "MIN": 0.00008, "MAX": 0.0008, "LABEL": "Cloud Scale"},
        {"NAME": "coverage", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 0.85, "LABEL": "Coverage"},
        {"NAME": "cloud_base", "TYPE": "float", "DEFAULT": 0.0, "MIN": -4000.0, "MAX": 6000.0, "LABEL": "Cloud Floor"},
        {"NAME": "cloud_top", "TYPE": "float", "DEFAULT": 10000.0, "MIN": 2000.0, "MAX": 16000.0, "LABEL": "Cloud Ceiling"},
        {"NAME": "sun_elevation", "TYPE": "float", "DEFAULT": 0.25, "MIN": -0.3, "MAX": 1.0, "LABEL": "Sun Elevation"},
        {"NAME": "sun_azimuth", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Sun Azimuth"},
        {"NAME": "sun_intensity", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Sun Intensity"},
        {"NAME": "sun_sharpness", "TYPE": "float", "DEFAULT": 350.0, "MIN": 50.0, "MAX": 800.0, "LABEL": "Sun Sharpness"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 2.5, "LABEL": "Brightness"},
        {"NAME": "sky_color", "TYPE": "color", "DEFAULT": [0.05, 0.2, 0.5, 1.0], "LABEL": "Sky Color"},
        {"NAME": "cloud_light", "TYPE": "color", "DEFAULT": [1.0, 0.98, 0.95, 1.0], "LABEL": "Cloud Light"},
        {"NAME": "cloud_dark", "TYPE": "color", "DEFAULT": [0.3, 0.3, 0.2, 1.0], "LABEL": "Cloud Shadow"}
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
    float sway;
    float altitude;
    float altitude_wobble;
    float forward_speed;
    float steps;
    float scale;
    float coverage;
    float cloud_base;
    float cloud_top;
    float sun_elevation;
    float sun_azimuth;
    float sun_intensity;
    float sun_sharpness;
    float brightness;
    vec4 sky_color;
    vec4 cloud_light;
    vec4 cloud_dark;
};

const mat3 FBM_ROT = mat3(0.00, 1.60, 1.20, -1.60, 0.72, -0.96, -1.20, -0.96, 1.28);

float hash(float n) {
    return fract(cos(n) * 114514.1919);
}

// 3D value noise on an integer lattice keyed by a single scalar hash of
// the cell id — cheap enough to call 4x per fbm() and dozens of times
// per raymarch step.
float noise(in vec3 x) {
    vec3 p = floor(x);
    vec3 f = smoothstep(0.0, 1.0, fract(x));

    float n = p.x + p.y * 10.0 + p.z * 100.0;

    return mix(
        mix(mix(hash(n + 0.0), hash(n + 1.0), f.x),
            mix(hash(n + 10.0), hash(n + 11.0), f.x), f.y),
        mix(mix(hash(n + 100.0), hash(n + 101.0), f.x),
            mix(hash(n + 110.0), hash(n + 111.0), f.x), f.y), f.z);
}

float fbm(vec3 p) {
    float f = 0.5000 * noise(p);
    p = FBM_ROT * p;
    f += 0.2500 * noise(p);
    p = FBM_ROT * p;
    f += 0.1666 * noise(p);
    p = FBM_ROT * p;
    f += 0.0834 * noise(p);
    return f;
}

// Weaving flythrough path: sways side to side and bobs in altitude while
// advancing through the cloud field at forward_speed.
vec3 cameraPath(float time) {
    return vec3(
        sway * sin(time),
        altitude + altitude_wobble * sin(0.5 * time),
        forward_speed * time
    );
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); this shader's
    // raymarch was ported from Shadertoy's bottom-left/y-up convention,
    // so flip y here to keep "up" on screen mapped to positive p.y.
    vec2 p = 2.0 * vec2(uv.x, 1.0 - uv.y) - 1.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    float time = PHASE_TIME_0 + 57.5;
    vec3 campos = cameraPath(time);
    vec3 camtar = cameraPath(time + 0.4);

    vec3 front = normalize(camtar - campos);
    vec3 right = normalize(cross(front, vec3(0.0, 1.0, 0.0)));
    vec3 up = normalize(cross(right, front));
    vec3 fragAt = normalize(p.x * right + p.y * up + front);

    vec3 lightDir = normalize(vec3(
        cos(sun_azimuth * 6.28318) * cos(sun_elevation * 1.5708),
        sin(sun_elevation * 1.5708),
        sin(sun_azimuth * 6.28318) * cos(sun_elevation * 1.5708)
    ));

    // Fixed draw distance (paired with `scale`'s cloud-puff frequency);
    // `steps` trades raymarch quality for performance without changing
    // how far the flythrough reaches.
    float maxDepth = 100000.0;
    int stepCount = int(clamp(steps, 8.0, 96.0));
    float stepSize = maxDepth / float(stepCount);

    vec4 sum = vec4(0.0);
    for (int i = 0; i < 96; i++) {
        if (i >= stepCount) break;
        float depth = float(i) * stepSize;
        vec3 ray = campos + fragAt * depth;
        if (cloud_base < ray.y && ray.y < cloud_top) {
            float density = smoothstep(coverage, 1.0, fbm(ray * scale));
            vec3 localColor = mix(cloud_light.rgb, cloud_dark.rgb, density);
            float a = (1.0 - sum.a) * density;
            sum += vec4(localColor * a, a);
        }
    }

    float shadeMask = smoothstep(0.7, 1.0, sum.a);
    sum.rgb /= sum.a + 0.0001;

    float sundot = clamp(dot(fragAt, lightDir), 0.0, 1.0);
    vec3 col = 0.8 * sky_color.rgb;
    col += 0.47 * sun_intensity * vec3(1.6, 1.4, 1.0) * pow(sundot, sun_sharpness);
    col += 0.4 * sun_intensity * vec3(0.8, 0.9, 1.0) * pow(sundot, 2.0);

    sum.rgb -= 0.6 * vec3(0.8, 0.75, 0.7) * pow(sundot, 13.0) * shadeMask;
    sum.rgb += 0.2 * vec3(1.3, 1.2, 1.0) * pow(sundot, 5.0) * (1.0 - shadeMask);

    col = mix(col, sum.rgb, sum.a);
    col *= brightness;

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
