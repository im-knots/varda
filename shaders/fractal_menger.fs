/*{
    "DESCRIPTION": "Menger Sponge - Raymarched 3D fractal explorer with flythrough camera and orbit trap coloring",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "iterations", "TYPE": "float", "DEFAULT": 6.0, "MIN": 3.0, "MAX": 12.0, "LABEL": "Iterations"},
        {"NAME": "menger_scale", "TYPE": "float", "DEFAULT": 3.0, "MIN": 2.0, "MAX": 5.0, "LABEL": "Scale"},
        {"NAME": "offset", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 2.0, "LABEL": "Offset"},
        {"NAME": "cam_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -20.0, "MAX": 20.0, "LABEL": "Camera X"},
        {"NAME": "cam_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -20.0, "MAX": 20.0, "LABEL": "Camera Y"},
        {"NAME": "cam_z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -20.0, "MAX": 20.0, "LABEL": "Camera Z"},
        {"NAME": "rot_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate X"},
        {"NAME": "rot_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate Y"},
        {"NAME": "rot_z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate Z"},
        {"NAME": "fov", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 2.5, "LABEL": "FOV"},
        {"NAME": "fly_speed", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Fly Speed"},
        {"NAME": "max_steps", "TYPE": "float", "DEFAULT": 128.0, "MIN": 40.0, "MAX": 256.0, "LABEL": "Max Steps"},
        {"NAME": "ao_strength", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.0, "LABEL": "AO Strength"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 0.8, 1.0, 1.0], "LABEL": "Color Warm"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [1.0, 0.2, 0.0, 1.0], "LABEL": "Color Cool"},
        {"NAME": "color_mode", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Color Mode"},
        {"NAME": "sun_elev", "TYPE": "float", "DEFAULT": 0.6, "MIN": -1.0, "MAX": 1.0, "LABEL": "Sun Elevation"},
        {"NAME": "sun_azim", "TYPE": "float", "DEFAULT": 0.8, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Sun Azimuth"},
        {"NAME": "shadow_strength", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Shadow Strength"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [{"PARAM": "fly_speed", "INDEX": 0, "SCALE": 0.3}]
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
    float iterations;
    float menger_scale;
    float offset;
    float cam_x;
    float cam_y;
    float cam_z;
    float rot_x;
    float rot_y;
    float rot_z;
    float fov;
    float fly_speed;
    float max_steps;

    float ao_strength;
    vec4 color1;
    vec4 color2;
    float color_mode;
    float sun_elev;
    float sun_azim;
    float shadow_strength;
    vec4 bg_color;
};

mat3 rotX(float a){float c=cos(a),s=sin(a);return mat3(1,0,0,0,c,-s,0,s,c);}
mat3 rotY(float a){float c=cos(a),s=sin(a);return mat3(c,0,s,0,1,0,-s,0,c);}

// Orbit trap output from last mengerDE call
vec4 g_trap = vec4(0.0);
float g_pixFootprint = 0.0; // pixel footprint for LOD — set before marching

// Menger sponge DE with vec4 orbit trap tracking
vec2 mengerDE(vec3 pos) {
    vec3 z = pos;
    float sc = menger_scale;
    float off = offset;
    vec4 trap = vec4(1e10);
    int iters = int(iterations);
    float pf = g_pixFootprint;

    vec3 a = abs(z);
    float d = max(a.x - 1.0, max(a.y - 1.0, a.z - 1.0));

    float pw = 1.0;
    for (int i = 0; i < 12; i++) {
        if (i >= iters) break;
        // LOD: bail when detail scale is sub-pixel
        if (pf > 0.0 && 1.0/pw < pf) break;
        z = abs(z);
        if (z.x < z.y) z.xy = z.yx;
        if (z.x < z.z) z.xz = z.zx;
        if (z.y < z.z) z.yz = z.zy;
        trap = min(trap, vec4(abs(z), dot(z, z)));
        z = z * sc - off * (sc - 1.0);
        if (z.z < -0.5 * off * (sc - 1.0)) z.z += off * (sc - 1.0);
        pw *= sc;
        float cx = max(abs(z.y), abs(z.z)) - off;
        float cy = max(abs(z.x), abs(z.z)) - off;
        float cz = max(abs(z.x), abs(z.y)) - off;
        d = max(d, min(cx, min(cy, cz)) / pw);
    }
    g_trap = trap;
    // Fudge factor to avoid overstepping thin features
    d *= 0.7;
    // Clamp DE to pixel footprint — sub-pixel detail is aliasing noise
    if (pf > 0.0) d = max(d, pf * 0.5);
    return vec2(d, trap.w);
}

float de(vec3 p) { return mengerDE(p).x; }

// Box intersection for bounding volume optimization
vec2 boxIntersect(vec3 ro, vec3 rd, vec3 boxSize) {
    vec3 m = 1.0 / rd;
    vec3 n = m * ro;
    vec3 k = abs(m) * boxSize;
    vec3 t1 = -n - k;
    vec3 t2 = -n + k;
    float tN = max(max(t1.x, t1.y), t1.z);
    float tF = min(min(t2.x, t2.y), t2.z);
    if (tN > tF || tF < 0.0) return vec2(-1.0);
    return vec2(tN, tF);
}

// Tetrahedron normal with pixel-footprint epsilon
vec3 calcNormal(vec3 p, float pixSize) {
    float e = max(pixSize * 0.5, 2e-5);
    vec2 h = vec2(e, -e);
    vec3 n = h.xyy * de(p + h.xyy) +
             h.yyx * de(p + h.yyx) +
             h.yxy * de(p + h.yxy) +
             h.xxx * de(p + h.xxx);
    float len = length(n);
    return (len > 1e-20) ? n / len : vec3(0.0, 1.0, 0.0);
}

// Improved soft shadows (IQ's improved technique)
float softShadow(vec3 ro, vec3 rd, float tmin, float tmax, float k) {
    float res = 1.0;
    float t = tmin;
    float ph = 1e10;
    for (int i = 0; i < 24; i++) {
        float h = de(ro + rd * t);
        float y = h * h / (2.0 * ph);
        float d2 = sqrt(max(h * h - y * y, 0.0));
        res = min(res, k * d2 / max(0.0, t - y));
        ph = h;
        t += clamp(h, 0.005, 0.2);
        if (res < 0.001 || t > tmax) break;
    }
    return clamp(res, 0.0, 1.0);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float pixelSize = 1.0 / RENDERSIZE.y;

    // Camera
    mat3 camRot = rotY(rot_y) * rotX(rot_x);
    vec3 fw = camRot * vec3(0, 0, 1);
    vec3 ri = normalize(cross(fw, vec3(0, 1, 0)));
    vec3 up = cross(ri, fw);
    float cr = cos(rot_z), sr = sin(rot_z);
    vec3 ri2 = ri * cr + up * sr;
    vec3 up2 = up * cr - ri * sr;
    float ft = PHASE_TIME_0;
    vec3 ro = vec3(cam_x, cam_y, cam_z) + vec3(sin(ft*0.7)*0.5, sin(ft*0.4)*0.3, cos(ft*0.3)*0.5);
    vec3 rd = normalize(p.x * ri2 + p.y * up2 + fw / tan(fov * 0.5 + 0.3));

    vec3 sunDir = normalize(vec3(cos(sun_azim)*cos(sun_elev), sin(sun_elev), sin(sun_azim)*cos(sun_elev)));
    vec3 light2 = normalize(vec3(-0.707, 0.0, 0.707));

    // Bounding box optimization
    vec2 bb = boxIntersect(ro, rd, vec3(1.2));
    float tStart = max(bb.x, 0.0);
    float tEnd = bb.y;

    // Raymarch
    float dist = tStart;
    int steps = 0;
    int maxS = int(max_steps);
    bool hit = false;
    float minD = 1e10;

    float lastD = 0.0;
    float minDdist = dist;
    float minStep = pixelSize * 2.0; // prevent stalling when camera is inside fractal
    if (bb.y > 0.0) { // bb.y > 0 means the box is ahead (works both outside and inside)
        for (int i = 0; i < 256; i++) {
            if (i >= maxS) break;
            g_pixFootprint = pixelSize * max(dist, 0.5); // LOD: scale iterations to pixel size
            vec3 pos = ro + rd * dist;
            vec2 hitInfo = mengerDE(pos);
            float d = hitInfo.x;
            float ad = abs(d);
            if (ad < minD) { minD = ad; minDdist = dist; }
            float thresh = max(pixelSize * dist * 0.25, 5e-7);
            if (ad < thresh && dist > minStep * 4.0) { hit = true; steps = i; lastD = d; break; }
            dist += max(d, minStep);
            steps = i;
            if (dist > tEnd + 0.1 || dist > 50.0) break;
        }
        g_pixFootprint = 0.0; // full precision for normals/shadows
    }

    // Binary search refinement for crisp edges
    if (hit) {
        float lo = dist - abs(lastD) * 2.0;
        float hi = dist;
        lo = max(lo, 0.0);
        for (int j = 0; j < 8; j++) {
            float mid = (lo + hi) * 0.5;
            float d = mengerDE(ro + rd * mid).x;
            float thresh = max(pixelSize * mid * 0.25, 5e-7);
            if (abs(d) < thresh) { hi = mid; } else { lo = mid; }
        }
        dist = hi;
        mengerDE(ro + rd * dist); // re-evaluate for g_trap
    }

    // Soft hit: step-exhausted ray that got very close to the surface
    float softThresh = max(pixelSize * minDdist * 8.0, 0.001);
    if (!hit && minD < softThresh && minDdist > minStep * 4.0) {
        hit = true;
        dist = minDdist;
        mengerDE(ro + rd * dist);
    }

    vec3 col = bg_color.rgb;

    if (hit) {
        vec3 hp = ro + rd * dist;
        float pxSz = pixelSize * dist;
        vec3 n = calcNormal(hp, pxSz);
        if (dot(n, rd) > 0.0) n = -n;

        // Orbit-trap based AO
        float occ = clamp(0.05 * log(g_trap.w + 1.0), 0.0, 1.0);
        occ = 1.0 - ao_strength * (1.0 - occ);

        // Color mode selection
        float cm = color_mode;
        vec3 albedo;
        if (cm < 0.5) {
            // Orbit trap coloring
            albedo = vec3(0.01);
            albedo = mix(albedo, color1.rgb, clamp(g_trap.x * 0.5, 0.0, 1.0));
            albedo = mix(albedo, color2.rgb, clamp(g_trap.y * 0.5, 0.0, 1.0));
            albedo = mix(albedo, mix(color1.rgb, color2.rgb, 0.5), clamp(pow(g_trap.z * 0.3, 4.0), 0.0, 1.0));
        } else if (cm < 1.5) {
            float t = clamp(dist * 0.05, 0.0, 1.0);
            albedo = mix(color1.rgb, color2.rgb, t);
        } else {
            float t = float(steps) / float(maxS);
            albedo = mix(color1.rgb, color2.rgb, t);
        }

        // Shadows
        float shad = 1.0;
        if (shadow_strength > 0.01) {
            shad = mix(1.0, softShadow(hp + n * 0.01, sunDir, 0.01, 5.0, 8.0), shadow_strength);
        }

        // Multi-light illumination (IQ style)
        float dif1 = clamp(dot(sunDir, n), 0.0, 1.0) * shad;
        vec3 hal = normalize(sunDir - rd);
        float spe = pow(clamp(dot(n, hal), 0.0, 1.0), 32.0) * dif1
                   * (0.04 + 0.96 * pow(clamp(1.0 - dot(hal, sunDir), 0.0, 1.0), 5.0));
        float dif2 = clamp(0.5 + 0.5 * dot(light2, n), 0.0, 1.0) * occ;
        float dif3 = (0.7 + 0.3 * n.y) * (0.2 + 0.8 * occ);
        float fac = clamp(1.0 + dot(rd, n), 0.0, 1.0);

        vec3 lin = vec3(0.0);
        lin += 7.0 * vec3(1.50, 1.10, 0.70) * dif1;
        lin += 4.0 * vec3(0.25, 0.20, 0.15) * dif2;
        lin += 1.5 * vec3(0.10, 0.20, 0.30) * dif3;
        lin += 2.5 * vec3(0.35, 0.30, 0.25) * (0.05 + 0.95 * occ);
        lin += 4.0 * fac * occ;

        col = albedo * lin;
        col = pow(col, vec3(0.7, 0.9, 1.0));
        col += spe * 15.0;


    } else {
        // Edge glow for near-miss rays
        float glow = exp(-minD * 200.0) * 0.15;
        col = mix(col, mix(color1.rgb, color2.rgb, 0.5), glow);
    }

    // Gamma correction
    col = sqrt(clamp(col, 0.0, 1.0));
    fragColor = vec4(col, 1.0);
}
