/*{
    "DESCRIPTION": "Bicycle Day - raymarched 'Amazing Surface' fractal tunnel shaded only by normals and dark edge-detection lines, with a low sun and a procedural rainbow trail",
    "CREDIT": "Varda VJ (ported from Kali's 'Fractal Cartoon', formerly 'DE edge detection'; rainbow-trail math adapted from mu6k's Nyan Cat shader, https://www.shadertoy.com/view/4dXGWH; fractal tree/ground geometry folded in from a hg_sdf-based fractal-forest snippet)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "ray_steps", "TYPE": "float", "DEFAULT": 150.0, "MIN": 60.0, "MAX": 250.0, "LABEL": "Ray Steps"},
        {"NAME": "detail", "TYPE": "float", "DEFAULT": 0.001, "MIN": 0.0003, "MAX": 0.004, "LABEL": "Detail"},
        {"NAME": "brightness", "TYPE": "float", "DEFAULT": 1.2, "MIN": 0.3, "MAX": 3.0, "LABEL": "Brightness"},
        {"NAME": "gamma", "TYPE": "float", "DEFAULT": 1.4, "MIN": 0.5, "MAX": 3.0, "LABEL": "Gamma"},
        {"NAME": "saturation", "TYPE": "float", "DEFAULT": 0.65, "MIN": 0.0, "MAX": 1.5, "LABEL": "Saturation"},
        {"NAME": "show_waves", "TYPE": "bool", "DEFAULT": true, "LABEL": "Waves"},
        {"NAME": "wave_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Wave Amount"},
        {"NAME": "show_border", "TYPE": "bool", "DEFAULT": true, "LABEL": "Vignette Border"},
        {"NAME": "vignette_amount", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 3.0, "LABEL": "Vignette Amount"},
        {"NAME": "show_rainbow", "TYPE": "bool", "DEFAULT": true, "LABEL": "Rainbow Trail"},
        {"NAME": "edge_only", "TYPE": "bool", "DEFAULT": false, "LABEL": "Edges Only"},
        {"NAME": "look_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.5, "MAX": 1.5, "LABEL": "Look X"},
        {"NAME": "look_y", "TYPE": "float", "DEFAULT": -0.05, "MIN": -1.5, "MAX": 1.5, "LABEL": "Look Y"},
        {"NAME": "fov", "TYPE": "float", "DEFAULT": 0.9, "MIN": 0.3, "MAX": 1.5, "LABEL": "FOV"},
        {"NAME": "sun_spin", "TYPE": "float", "DEFAULT": 1.5, "MIN": -3.0, "MAX": 3.0, "LABEL": "Sun Spin"},
        {"NAME": "sun_size", "TYPE": "float", "DEFAULT": 7.0, "MIN": 2.0, "MAX": 12.0, "LABEL": "Sun Size"},
        {"NAME": "sun_audio_react", "TYPE": "float", "DEFAULT": 5.0, "MIN": 0.0, "MAX": 10.0, "LABEL": "Sun Audio React"},
        {"NAME": "camera_amp", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 3.0, "LABEL": "Camera Sway Amount"},
        {"NAME": "camera_speed", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.3, "MAX": 3.0, "LABEL": "Camera Sway Speed"},
        {"NAME": "sky_color", "TYPE": "color", "DEFAULT": [0.5, 0.0, 1.0, 1.0], "LABEL": "Sky Color"},
        {"NAME": "tree_amount", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.0, "LABEL": "Trees Amount"},
        {"NAME": "tree_detail", "TYPE": "float", "DEFAULT": 5.0, "MIN": 2.0, "MAX": 8.0, "LABEL": "Tree Detail"},
        {"NAME": "tree_scale", "TYPE": "float", "DEFAULT": 0.55, "MIN": 0.15, "MAX": 1.5, "LABEL": "Tree Scale"},
        {"NAME": "tree_spacing", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Tree Spacing"},
        {"NAME": "tree_offset", "TYPE": "float", "DEFAULT": 0.55, "MIN": 0.2, "MAX": 1.5, "LABEL": "Roadside Offset"},
        {"NAME": "tree_base_y", "TYPE": "float", "DEFAULT": 0.35, "MIN": -0.5, "MAX": 1.0, "LABEL": "Tree Base Height"},
        {"NAME": "tree_tint", "TYPE": "color", "DEFAULT": [0.6, 0.8, 1.0, 1.0], "LABEL": "Tree Tint"}
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
    float ray_steps;
    float detail;
    float brightness;
    float gamma;
    float saturation;
    uint show_waves; // ISF bool inputs are packed as raw u32 0/1 by the host,
    float wave_amount;
    uint show_border; // not float — reading a u32 bit pattern as float misreads it
    float vignette_amount;
    uint show_rainbow;
    uint edge_only;
    float look_x;
    float look_y;
    float fov;
    float sun_spin;
    float sun_size;
    float sun_audio_react;
    float camera_amp;
    float camera_speed;
    vec4 sky_color;
    float tree_amount;
    float tree_detail;
    float tree_scale;
    float tree_spacing;
    float tree_offset;
    float tree_base_y;
    vec4 tree_tint;
};

const vec3 origin = vec3(-1.0, 0.7, 0.0);
float g_edge = 0.0;
float g_treeMask = 0.0; // 1.0 when the nearest surface at the last de() sample was trees/ground

mat2 rot(float a) {
    return mat2(cos(a), sin(a), -sin(a), cos(a));
}

// "Amazing Surface" fractal
vec4 formula(vec4 p) {
    p.xz = abs(p.xz + 1.0) - abs(p.xz - 1.0) - p.xz;
    p.y -= 0.25;
    p.xy *= rot(radians(35.0));
    p = p * 2.0 / clamp(dot(p.xyz, p.xyz), 0.2, 1.0);
    return p;
}

// -- Fractal trees lining the roadside (technique borrowed from a
// hg_sdf-style fractal-forest snippet: iterated abs-fold + scale +
// rotate, read out through a length6 pseudo-distance so the recursive
// folding reads as branch-like structure instead of a smooth blob).
void pR(inout vec2 p, float a) {
    p = cos(a) * p + sin(a) * vec2(p.y, -p.x);
}

float length6(vec3 p) {
    p = p * p * p;
    p = p * p;
    return pow(p.x + p.y + p.z, 1.0 / 6.0);
}

// The tunnel's roadside curb (the `ro` shape in de()) sits at x ~= -1,
// with its top edge around y ~= 0.35-0.5, and repeats every 6 units in
// z. Trees are placed in that same coordinate frame — folded onto both
// sides of the road and repeated along z at `tree_spacing` — rather
// than reusing the original snippet's unrelated absolute-space offsets,
// so they read as actually growing alongside the path instead of
// floating disconnected from the scene.
float treeField(vec3 pos, float t) {
    int iters = int(clamp(tree_detail, 2.0, 8.0));
    float period = max(tree_spacing, 0.5);
    float scl = max(tree_scale, 0.05);
    // Same "travel distance" driver the reference fractal used (time
    // combined with position along the direction of travel) so the
    // fold warps gently as the camera passes each tree.
    float warp = t * 5.0 - pos.z;

    vec3 tp = pos;
    tp.z = mod(tp.z + period * 0.5, period) - period * 0.5;
    tp.x = abs(pos.x + 1.0) - tree_offset; // fold both roadsides onto one shape
    tp.y -= tree_base_y;
    tp /= scl;

    float l = 0.0;
    for (int i = 0; i < 8; i++) {
        if (i >= iters) break;
        tp.xy = abs(tp.xy);
        tp = tp * 1.25 + vec3(-0.4 + warp * 0.003, -0.85, -0.2);
        pR(tp.xy, 0.35 - warp * 0.01);
        pR(tp.yz, 0.5 + warp * 0.015);
        l = length6(tp);
    }
    float d = (l * pow(1.25, -float(iters)) - 0.1) * scl;
    // Fade to a no-op distance as tree_amount -> 0 so a VJ can dial the
    // geometry fully out without touching anything else.
    return mix(1000.0, d, clamp(tree_amount, 0.0, 1.0));
}

// Distance function
float de(vec3 pos, float t) {
    if (show_waves != 0u) {
        pos.y += sin(pos.z - t * 6.0) * 0.15 * wave_amount; // waves!
    }
    vec3 tpos = pos;
    tpos.z = abs(3.0 - mod(tpos.z, 6.0));
    vec4 p = vec4(tpos, 1.0);
    for (int i = 0; i < 4; i++) { p = formula(p); }
    float fr = (length(max(vec2(0.0), p.yz - 1.5)) - 1.0) / p.w;
    float ro = max(abs(pos.x + 1.0) - 0.3, pos.y - 0.35);
    ro = max(ro, -max(abs(pos.x + 1.0) - 0.1, pos.y - 0.5));
    float trees = treeField(pos, t);
    pos.z = abs(0.25 - mod(pos.z, 0.5));
    ro = max(ro, -max(abs(pos.z) - 0.2, pos.y - 0.3));
    ro = max(ro, -max(abs(pos.z) - 0.01, -pos.y + 0.32));
    float sceneD = min(fr, ro);
    g_treeMask = step(trees, sceneD);
    return min(sceneD, trees);
}

// Camera path
vec3 path(float ti) {
    ti *= camera_speed;
    return vec3(sin(ti), (1.0 - sin(ti * 2.0)) * 0.5, -ti * 5.0) * 0.5 * camera_amp;
}

// Normal + edge detection (edge finder writes into g_edge)
vec3 calcNormalEdge(vec3 p, float t, float det) {
    vec3 e = vec3(0.0, det * 5.0, 0.0);
    float d1 = de(p - e.yxx, t), d2 = de(p + e.yxx, t);
    float d3 = de(p - e.xyx, t), d4 = de(p + e.xyx, t);
    float d5 = de(p - e.xxy, t), d6 = de(p + e.xxy, t);
    float d = de(p, t);
    g_edge = abs(d - 0.5 * (d2 + d1)) + abs(d - 0.5 * (d4 + d3)) + abs(d - 0.5 * (d6 + d5));
    g_edge = min(1.0, pow(g_edge, 0.55) * 15.0);
    return normalize(vec3(d1 - d2, d3 - d4, d5 - d6));
}

// Procedural rainbow-stripe trail (ported from mu6k's Nyan Cat shader; the
// cat sprite itself needed a texture asset we don't ship, so only the
// fully procedural rainbow stripe survives the port).
vec4 rainbow(vec2 p, float t) {
    float s = sin(p.x * 7.0 + t * 70.0) * 0.08;
    p.y += s;
    p.y *= 1.1;

    vec4 c;
    if (p.x > 0.0) c = vec4(0.0);
    else if (0.0 / 6.0 < p.y && p.y < 1.0 / 6.0) c = vec4(255, 43, 14, 255) / 255.0;
    else if (1.0 / 6.0 < p.y && p.y < 2.0 / 6.0) c = vec4(255, 168, 6, 255) / 255.0;
    else if (2.0 / 6.0 < p.y && p.y < 3.0 / 6.0) c = vec4(255, 244, 0, 255) / 255.0;
    else if (3.0 / 6.0 < p.y && p.y < 4.0 / 6.0) c = vec4(51, 234, 5, 255) / 255.0;
    else if (4.0 / 6.0 < p.y && p.y < 5.0 / 6.0) c = vec4(8, 163, 255, 255) / 255.0;
    else if (5.0 / 6.0 < p.y && p.y < 6.0 / 6.0) c = vec4(122, 85, 255, 255) / 255.0;
    else if (abs(p.y) - 0.05 < 0.0001) c = vec4(0.0, 0.0, 0.0, 1.0);
    else if (abs(p.y - 1.0) - 0.05 < 0.0001) c = vec4(0.0, 0.0, 0.0, 1.0);
    else c = vec4(0.0);
    c.a *= 0.8 - min(0.8, abs(p.x * 0.08));
    c.xyz = mix(c.xyz, vec3(length(c.xyz)), 0.15);
    return c;
}

vec3 raymarch(vec3 from, vec3 dir, float t) {
    g_edge = 0.0;
    vec3 p = vec3(0.0);
    float d = 100.0;
    float totdist = 0.0;
    float det = 0.0;
    int steps = int(clamp(ray_steps, 20.0, 300.0));
    for (int i = 0; i < 300; i++) {
        if (i >= steps) break;
        if (d > det && totdist < 25.0) {
            p = from + totdist * dir;
            d = de(p, t);
            det = detail * exp(0.13 * totdist);
            totdist += d;
        }
    }
    vec3 col = vec3(0.0);
    p -= (det - d) * dir;
    vec3 norm = calcNormalEdge(p, t, det);
    if (edge_only != 0u) {
        col = 1.0 - vec3(g_edge); // wireframe view
    } else {
        col = (1.0 - abs(norm)) * max(0.0, 1.0 - g_edge * 0.8); // normal as color, dark edges
        col = mix(col, col * tree_tint.rgb, g_treeMask * 0.35); // subtle tint on trees/ground, secondary to normal/edge shading
    }
    totdist = clamp(totdist, 0.0, 26.0);
    dir.y -= 0.02;

    float sunsize = sun_size - max(0.0, audio_level) * sun_audio_react; // audio-reactive sun size
    float an = atan(dir.x, dir.y) + PHASE_TIME_0 * sun_spin; // angle for drawing/rotating sun
    float s = pow(clamp(1.0 - length(dir.xy) * sunsize - abs(0.2 - mod(an, 0.4)), 0.0, 1.0), 0.1);
    float sb = pow(clamp(1.0 - length(dir.xy) * (sunsize - 0.2) - abs(0.2 - mod(an, 0.4)), 0.0, 1.0), 0.1);
    float sg = pow(clamp(1.0 - length(dir.xy) * (sunsize - 4.5) - 0.5 * abs(0.2 - mod(an, 0.4)), 0.0, 1.0), 3.0);
    float y = mix(0.45, 1.2, pow(smoothstep(0.0, 1.0, 0.75 - dir.y), 2.0)) * (1.0 - sb * 0.5);

    vec3 backg = sky_color.rgb * ((1.0 - s) * (1.0 - sg) * y + (1.0 - sb) * sg * vec3(1.0, 0.8, 0.15) * 3.0);
    backg += vec3(1.0, 0.9, 0.1) * s;
    backg = max(backg, sg * vec3(1.0, 0.9, 0.5));

    col = mix(vec3(1.0, 0.9, 0.3), col, exp(-0.004 * totdist * totdist)); // distant fade to sun color
    if (totdist > 25.0) col = backg; // hit background
    col = pow(col, vec3(gamma)) * brightness;
    col = mix(vec3(length(col)), col, saturation);

    if (edge_only != 0u) {
        col = 1.0 - vec3(length(col));
    } else {
        col *= vec3(1.0, 0.9, 0.85);
        if (show_rainbow != 0u) {
            dir.yx *= rot(dir.x);
            vec2 trailPos = dir.xy + vec2(-3.0 + mod(-t, 6.0), -0.27);
            vec4 rain = rainbow(trailPos * 10.0 + vec2(0.8, 0.5), t);
            if (totdist > 8.0) col = mix(col, max(vec3(0.2), rain.xyz), rain.a * 0.9);
        }
    }
    return col;
}

// Advance the camera along path(), banking/turning to face the path's
// tangent direction.
vec3 move(inout vec3 dir, float t) {
    vec3 go = path(t);
    vec3 adv = path(t + 0.7);
    vec3 advec = normalize(adv - go);
    float an = adv.x - go.x;
    an *= min(1.0, abs(adv.z - go.z)) * sign(adv.z - go.z) * 0.7;
    dir.xy *= mat2(cos(an), sin(an), -sin(an), cos(an));
    an = advec.y * 1.7;
    dir.yz *= mat2(cos(an), sin(an), -sin(an), cos(an));
    an = atan(advec.x, advec.z);
    dir.xz *= mat2(cos(an), sin(an), -sin(an), cos(an));
    return go;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Varda's uv is top-left origin (y grows downward); this shader was
    // ported from Shadertoy's bottom-left/y-up convention, so flip y
    // before building the ray (the sky/sun gradient depends on it).
    vec2 res = RENDERSIZE;
    vec2 fragXY = vec2(uv.x, 1.0 - uv.y) * res;
    vec2 uv2 = fragXY / res * 2.0 - 1.0;
    vec2 oriuv = uv2;
    uv2.y *= res.y / res.x;

    vec2 mouse = vec2(look_x, look_y);
    float t = PHASE_TIME_0 * 0.5;

    vec3 dir = normalize(vec3(uv2 * fov, 1.0));
    dir.yz *= rot(mouse.y);
    dir.xz *= rot(mouse.x);
    vec3 from = origin + move(dir, t);

    vec3 color = raymarch(from, dir, t);

    if (show_border != 0u) {
        float vig = pow(max(0.0, 0.95 - length(oriuv * oriuv * oriuv * vec2(1.05, 1.1))), 0.3);
        vig = clamp(mix(1.0, vig, vignette_amount), 0.0, 1.0);
        color = mix(vec3(0.0), color, vig);
    }

    color = clamp(color, 0.0, 1.0);
    fragColor = vec4(color, 1.0);
}
