/*{
    "DESCRIPTION": "Mandelbox - Raymarched 3D fractal explorer with flythrough camera and orbit trap coloring",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "iterations", "TYPE": "float", "DEFAULT": 15.0, "MIN": 3.0, "MAX": 20.0, "LABEL": "Iterations"},
        {"NAME": "scale", "TYPE": "float", "DEFAULT": -1.5, "MIN": -3.0, "MAX": 3.0, "LABEL": "Scale"},
        {"NAME": "fold_limit", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 2.0, "LABEL": "Fold Limit"},
        {"NAME": "min_radius", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 1.0, "LABEL": "Min Radius"},
        {"NAME": "fixed_radius", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 2.0, "LABEL": "Fixed Radius"},
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
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [1.0, 0.0, 0.4, 1.0], "LABEL": "Color Warm"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.0, 0.6, 1.0, 1.0], "LABEL": "Color Cool"},
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
    float scale;
    float fold_limit;
    float min_radius;
    float fixed_radius;
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

vec4 g_trap = vec4(0.0);
float g_pixFootprint = 0.0; // pixel footprint for LOD — set before marching

vec2 mandelboxDE(vec3 pos) {
    float minR2 = min_radius * min_radius;
    float fixR2 = fixed_radius * fixed_radius;
    float sc = scale;
    vec3 z = pos;
    float dr = 1.0;
    vec4 trap = vec4(abs(z), dot(z, z));
    int iters = int(iterations);
    float pf = g_pixFootprint;
    for (int i = 0; i < 20; i++) {
        if (i >= iters) break;
        // LOD: bail when detail scale is sub-pixel
        if (pf > 0.0 && abs(dr) > 0.0 && 1.0/abs(dr) < pf) break;
        z = clamp(z, -fold_limit, fold_limit) * 2.0 - z;
        float r2 = dot(z, z);
        if (r2 < minR2) {
            float f = fixR2 / minR2;
            z *= f; dr *= f;
        } else if (r2 < fixR2) {
            float f = fixR2 / r2;
            z *= f; dr *= f;
        }
        z = z * sc + pos;
        dr = dr * abs(sc) + 1.0;
        trap = min(trap, vec4(abs(z), dot(z, z)));
    }
    g_trap = trap;
    float absDr = max(abs(dr), 1e-10); // guard against dr=0
    float dist = length(z) / absDr;
    // Clamp DE to prevent overstepping at far distances
    dist = min(abs(dist), 1.0);
    // Fudge factor (Fragmentarium-style) to avoid overstepping thin features
    dist *= 0.7;
    // Clamp DE to pixel footprint — sub-pixel detail is aliasing noise
    if (pf > 0.0) dist = max(dist, pf * 0.25);
    return vec2(dist, trap.w);
}

float de(vec3 p) { return mandelboxDE(p).x; }

float boundingSphere(vec3 ro, vec3 rd, float r) {
    float b = dot(ro, rd);
    float c = dot(ro, ro) - r * r;
    float h = b * b - c;
    if (h < 0.0) return -1.0;
    return -b - sqrt(h);
}

vec3 calcNormal(vec3 p, float pixSize) {
    float e = max(pixSize * 0.25, 2e-5);
    vec2 h = vec2(e, -e);
    vec3 n = h.xyy * de(p + h.xyy) +
             h.yyx * de(p + h.yyx) +
             h.yxy * de(p + h.yxy) +
             h.xxx * de(p + h.xxx);
    float len = length(n);
    // Guard against degenerate normals (NaN/zero)
    return (len > 1e-20) ? n / len : vec3(0.0, 1.0, 0.0);
}

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
    res = clamp(res, 0.0, 1.0);
    return res * res * (3.0 - 2.0 * res); // smoothstep for clean penumbra
}

// Render a single sample at subpixel offset (for AA)
vec3 renderSample(vec2 fragUV) {
    vec2 p = (fragUV - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

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

    float pixelSize = 1.0 / RENDERSIZE.y;

    // Bounding sphere acceleration
    float dist = 0.0;
    float bHit = boundingSphere(ro, rd, 4.0);
    if (dot(ro, ro) > 16.0) {
        if (bHit < 0.0) {
            return bg_color.rgb;
        }
        dist = max(bHit - 0.1, 0.0);
    }

    // Raymarch with minimum step size for interior rendering
    int steps = 0;
    int maxS = int(max_steps);
    vec2 hitInfo = vec2(0.0);
    bool hit = false;
    float minD = 1e10;
    float minDdist = dist;
    float minStep = pixelSize * 2.0; // prevent stalling when camera is inside fractal
    for (int i = 0; i < 256; i++) {
        if (i >= maxS) break;
        g_pixFootprint = pixelSize * max(dist, 0.5); // LOD: scale iterations to pixel size
        hitInfo = mandelboxDE(ro + rd * dist);
        float d = hitInfo.x;
        float ad = abs(d);
        if (ad < minD) { minD = ad; minDdist = dist; }
        float thresh = max(pixelSize * dist * 0.15, 5e-7);
        if (ad < thresh && dist > minStep * 4.0) { hit = true; steps = i; break; }
        dist += max(d, minStep);
        steps = i;
        if (dist > 50.0) break;
    }
    g_pixFootprint = 0.0; // full precision for normals/shadows

    // Binary search refinement for crisp edges
    if (hit) {
        float lo = dist - hitInfo.x * 2.0;
        float hi = dist;
        lo = max(lo, 0.0);
        for (int j = 0; j < 8; j++) {
            float mid = (lo + hi) * 0.5;
            float d = mandelboxDE(ro + rd * mid).x;
            float thresh = max(pixelSize * mid * 0.15, 5e-7);
            if (d < thresh) { hi = mid; } else { lo = mid; }
        }
        dist = hi;
        mandelboxDE(ro + rd * dist); // re-evaluate for g_trap
    }

    // Treat step-exhausted rays that got very close as soft hits
    // This is the key fix: in fractal crevices the DE returns tiny values
    // and the ray crawls without converging. Rather than showing gray
    // background, shade these pixels using their closest approach point.
    float softThresh = max(pixelSize * minDdist * 8.0, 0.001);
    if (!hit && minD < softThresh && minDdist > minStep * 4.0) {
        hit = true;
        dist = minDdist;
        mandelboxDE(ro + rd * dist); // set g_trap
    }

    vec3 col = bg_color.rgb;

    // Shade surface (both hard hits and soft/step-exhausted hits)
    if (hit) {
        vec3 hp = ro + rd * dist;
        float pix = pixelSize * dist;
        // Save orbit trap before normal/shadow DE calls corrupt it
        vec4 savedTrap = g_trap;
        vec3 n = calcNormal(hp, pix);

        // Flip normal to face camera (essential in concavities)
        if (dot(n, rd) > 0.0) n = -n;
        g_trap = savedTrap; // restore clean orbit trap

        // Step-count AO: rays that used many steps are in deep crevices
        float stepAO = 1.0 - float(steps) / float(maxS);
        stepAO = stepAO * stepAO; // quadratic falloff

        // Orbit-trap AO blended with step-count AO
        float occ = clamp(0.05 * log(g_trap.w + 1.0), 0.0, 1.0);
        occ = min(occ, stepAO); // use whichever says more occluded
        occ = mix(1.0, occ, ao_strength);

        // Coloring
        float cm = color_mode;
        vec3 albedo;
        if (cm < 0.5) {
            // Orbit trap coloring
            albedo = vec3(0.01);
            albedo = mix(albedo, color1.rgb, clamp(g_trap.x, 0.0, 1.0));
            albedo = mix(albedo, color2.rgb, clamp(g_trap.y * g_trap.y, 0.0, 1.0));
            albedo = mix(albedo, mix(color1.rgb, color2.rgb, 0.5), clamp(pow(g_trap.z, 4.0), 0.0, 1.0));
        } else if (cm < 1.5) {
            float t = clamp(dist * 0.05, 0.0, 1.0);
            albedo = mix(color1.rgb, color2.rgb, t);
        } else {
            float t = float(steps) / float(maxS);
            albedo = mix(color1.rgb, color2.rgb, t);
        }

        // Sun diffuse
        float dif1 = clamp(dot(sunDir, n), 0.0, 1.0);

        // Adaptive shadow: skip shadow march for distant surfaces
        float shad = 1.0;
        if (shadow_strength > 0.01 && pix < 0.02) {
            shad = mix(1.0, softShadow(hp + n * 0.01, sunDir, 0.01, 5.0, 8.0), shadow_strength);
        }
        dif1 *= shad;

        // Fresnel specular
        vec3 hal = normalize(sunDir - rd);
        float spe = pow(clamp(dot(n, hal), 0.0, 1.0), 32.0) * dif1
                   * (0.04 + 0.96 * pow(clamp(1.0 - dot(hal, sunDir), 0.0, 1.0), 5.0));

        // Bounce light
        float dif2 = clamp(0.5 + 0.5 * dot(light2, n), 0.0, 1.0) * occ;
        // Sky light
        float dif3 = (0.7 + 0.3 * n.y) * (0.2 + 0.8 * occ);
        // Fresnel rim
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
        // True miss: ray escaped to infinity — apply subtle outer glow
        float glow = exp(-minD * 200.0) * 0.15;
        col = mix(col, mix(color1.rgb, color2.rgb, 0.5), glow);
    }

    return col;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // 2-sample rotated-grid AA: two samples at diagonal subpixel offsets
    vec2 px = 1.0 / RENDERSIZE;
    vec3 col = renderSample(uv + vec2( 0.33, -0.33) * px)
             + renderSample(uv + vec2(-0.33,  0.33) * px);
    col *= 0.5;

    // Gamma
    col = sqrt(clamp(col, 0.0, 1.0));
    fragColor = vec4(col, 1.0);
}
