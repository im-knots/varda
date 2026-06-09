/*{
    "DESCRIPTION": "Mandelbulb - Raymarched 3D fractal explorer with flythrough camera and orbit trap coloring",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "iterations", "TYPE": "float", "DEFAULT": 12.0, "MIN": 3.0, "MAX": 20.0, "LABEL": "Iterations"},
        {"NAME": "power", "TYPE": "float", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 16.0, "LABEL": "Power"},
        {"NAME": "bailout", "TYPE": "float", "DEFAULT": 8.0, "MIN": 1.5, "MAX": 100.0, "LABEL": "Bailout"},
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
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.4, 1.0], "LABEL": "Color Warm"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.8, 0.0, 1.0, 1.0], "LABEL": "Color Cool"},
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
    float power;
    float bailout;
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

vec2 mandelbulbDE(vec3 pos) {
    vec3 w = pos;
    float m = dot(w, w);
    vec4 trap = vec4(abs(w), m);
    float dz = 1.0;
    int iters = int(iterations);
    float pf = g_pixFootprint;

    if (abs(power - 8.0) < 0.01) {
        // IQ polynomial power-8 DE (no trig)
        for (int i = 0; i < 20; i++) {
            if (i >= iters) break;
            // LOD: bail when detail scale is sub-pixel
            if (pf > 0.0 && dz > 0.0 && 1.0/dz < pf) break;
            float m2 = m*m, m4 = m2*m2;
            dz = 8.0*sqrt(m4*m2*m)*dz + 1.0;

            float x = w.x, x2 = x*x, x4 = x2*x2;
            float y = w.y, y2 = y*y, y4 = y2*y2;
            float z = w.z, z2 = z*z, z4 = z2*z2;

            float k3 = x2 + z2;
            float k2 = inversesqrt(k3*k3*k3*k3*k3*k3*k3);
            float k1 = x4 + y4 + z4 - 6.0*y2*z2 - 6.0*x2*y2 + 2.0*z2*x2;
            float k4 = x2 - y2 + z2;

            w.x = pos.x +  64.0*x*y*z*(x2-z2)*k4*(x4-6.0*x2*z2+z4)*k1*k2;
            w.y = pos.y + -16.0*y2*k3*k4*k4 + k1*k1;
            w.z = pos.z +  -8.0*y*k4*(x4*x4 - 28.0*x4*x2*z2 + 70.0*x4*z4 - 28.0*x2*z2*z4 + z4*z4)*k1*k2;

            trap = min(trap, vec4(abs(w), m));
            m = dot(w, w);
            if (m > 256.0) break;
        }
    } else {
        // Trig fallback for arbitrary power
        float pw = power;
        for (int i = 0; i < 20; i++) {
            if (i >= iters) break;
            if (pf > 0.0 && dz > 0.0 && 1.0/dz < pf) break;
            float r = sqrt(m);
            if (r > bailout) break;
            float theta = acos(clamp(w.z / r, -1.0, 1.0));
            float phi = atan(w.y, w.x);
            dz = pow(r, pw - 1.0) * pw * dz + 1.0;
            float zr = pow(r, pw);
            theta *= pw; phi *= pw;
            w = zr * vec3(sin(theta)*cos(phi), sin(theta)*sin(phi), cos(theta)) + pos;
            trap = min(trap, vec4(abs(w), dot(w, w)));
            m = dot(w, w);
            if (m > 256.0) break;
        }
    }

    g_trap = vec4(m, trap.yzw);
    float r = sqrt(m);
    float dist = 0.5 * log(r) * r / dz;
    // Clamp DE to prevent overstepping at far distances (fractalforums fix)
    dist = min(abs(dist), 1.0);
    // Fudge factor: DE is an approximation, multiply by < 1 to avoid overstepping thin features
    dist *= 0.65;
    // Clamp DE to pixel footprint — sub-pixel detail is aliasing noise
    if (pf > 0.0) dist = max(dist, pf * 0.25);
    return vec2(dist, trap.x);
}

float de(vec3 p) { return mandelbulbDE(p).x; }

float boundingSphere(vec3 ro, vec3 rd, float r) {
    float b = dot(ro, rd);
    float c = dot(ro, ro) - r*r;
    float h = b*b - c;
    if (h < 0.0) return -1.0;
    return -b - sqrt(h);
}

float g_hitDE = 0.0; // cached DE at hit point — reused for cheaper normals

vec3 calcNormal(vec3 p, float pixSize) {
    // Use cached hit DE for central value — saves 1 of 4 DE evaluations
    // For mandelbulb where each DE has expensive pow(), this matters
    float e = max(pixSize * 0.25, 2e-5);
    float d0 = g_hitDE; // cached from march
    vec3 n = vec3(
        de(p + vec3(e, 0, 0)) - d0,
        de(p + vec3(0, e, 0)) - d0,
        de(p + vec3(0, 0, e)) - d0
    );
    float len = length(n);
    return (len > 1e-20) ? n / len : vec3(0.0, 1.0, 0.0);
}

float softShadow(vec3 ro, vec3 rd, float tmin, float tmax, float k) {
    float res = 1.0;
    float t = tmin;
    float ph = 1e10;
    for (int i = 0; i < 24; i++) {
        float h = de(ro + rd * t);
        float y = h*h / (2.0*ph);
        float d2 = sqrt(max(h*h - y*y, 0.0));
        res = min(res, k*d2 / max(0.0, t - y));
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
    vec3 ro = vec3(cam_x, cam_y, cam_z) + vec3(sin(ft*0.7)*0.4, sin(ft*0.4)*0.25, cos(ft*0.3)*0.4);
    vec3 rd = normalize(p.x * ri2 + p.y * up2 + fw / tan(fov * 0.5 + 0.3));

    vec3 sunDir = normalize(vec3(cos(sun_azim)*cos(sun_elev), sin(sun_elev), sin(sun_azim)*cos(sun_elev)));
    vec3 light2 = normalize(vec3(-sunDir.x, 0.0, -sunDir.z)); // bounce light opposite sun

    // Bounding sphere optimization
    float bsT = boundingSphere(ro, rd, 1.2);
    float pixelSize = 1.0 / RENDERSIZE.y;
    vec3 col = bg_color.rgb;
    float minD = 1e10;
    int steps = 0;
    int maxS = int(max_steps);

    if (bsT >= 0.0 || dot(ro, ro) < 1.44) {
        float dist = max(bsT, 0.0);
        vec2 hitInfo = vec2(0.0);
        bool hit = false;
        float minDdist = dist;
        float minStep = pixelSize * 2.0; // prevent stalling when camera is inside fractal

        for (int i = 0; i < 256; i++) {
            if (i >= maxS) break;
            g_pixFootprint = pixelSize * max(dist, 0.5); // LOD: scale iterations to pixel size
            hitInfo = mandelbulbDE(ro + rd * dist);
            float d = hitInfo.x;
            float ad = abs(d);
            if (ad < minD) { minD = ad; minDdist = dist; }
            float thresh = max(pixelSize * dist * 0.15, 5e-7);
            if (d < thresh && dist > minStep * 4.0) { hit = true; steps = i; break; }
            dist += max(d, minStep);
            steps = i;
            if (dist > 50.0) break;
        }
        g_pixFootprint = 0.0; // full precision for normals/shadows

        // Binary search refinement: bisect to find precise surface (8 iterations)
        if (hit) {
            float lo = dist - hitInfo.x * 2.0;
            float hi = dist;
            lo = max(lo, 0.0);
            for (int j = 0; j < 8; j++) {
                float mid = (lo + hi) * 0.5;
                float d = mandelbulbDE(ro + rd * mid).x;
                float thresh = max(pixelSize * mid * 0.15, 5e-7);
                if (d < thresh) { hi = mid; } else { lo = mid; }
            }
            dist = hi;
            mandelbulbDE(ro + rd * dist); // re-evaluate to set g_trap
        }

        // Soft hit: step-exhausted ray that got very close to the surface
        float softThresh = max(pixelSize * minDdist * 8.0, 0.001);
        if (!hit && minD < softThresh && minDdist > minStep * 4.0) {
            hit = true;
            dist = minDdist;
            mandelbulbDE(ro + rd * dist);
        }

        if (hit) {
            vec3 hp = ro + rd * dist;
            float pxAtHit = pixelSize * dist;
            // Save orbit trap before normal/shadow DE calls corrupt it
            vec4 savedTrap = g_trap;
            g_hitDE = de(hp); // cache for normal calc
            vec3 n = calcNormal(hp, pxAtHit);
            if (dot(n, rd) > 0.0) n = -n;
            g_trap = savedTrap; // restore clean orbit trap

            // Orbit-trap AO
            float occ = clamp(0.05 * log(g_trap.x), 0.0, 1.0);
            occ = mix(1.0, occ, ao_strength);

            // Coloring
            float cm = color_mode;
            vec3 albedo;
            if (cm < 0.5) {
                // Orbit-trap coloring (IQ style)
                albedo = vec3(0.01);
                albedo = mix(albedo, color1.rgb, clamp(g_trap.y, 0.0, 1.0));
                albedo = mix(albedo, color2.rgb, clamp(g_trap.z * g_trap.z, 0.0, 1.0));
                albedo = mix(albedo, mix(color1.rgb, color2.rgb, 0.5), clamp(pow(g_trap.w, 6.0), 0.0, 1.0));
            } else if (cm < 1.5) {
                albedo = mix(color1.rgb, color2.rgb, clamp(dist * 0.1, 0.0, 1.0));
            } else {
                albedo = mix(color1.rgb, color2.rgb, float(steps) / float(maxS));
            }

            // Multi-light illumination (IQ style)
            float dif1 = clamp(dot(sunDir, n), 0.0, 1.0);
            vec3 hal = normalize(sunDir - rd);
            float spe = pow(clamp(dot(n, hal), 0.0, 1.0), 32.0) * dif1
                      * (0.04 + 0.96 * pow(clamp(1.0 - dot(hal, sunDir), 0.0, 1.0), 5.0));

            // Adaptive shadow: skip shadow march for distant surfaces where shadows
            // are sub-pixel detail. Saves 24 DE evaluations for far fragments.
            float shad = 1.0;
            if (shadow_strength > 0.01 && pxAtHit < 0.02) {
                shad = mix(1.0, softShadow(hp + n * 0.01, sunDir, 0.01, 5.0, 8.0), shadow_strength);
            }
            dif1 *= shad;

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
            float glow = exp(-minD * 200.0) * 0.15;
            col = mix(col, mix(color1.rgb, color2.rgb, 0.5), glow);
        }
    }

    return col;
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // 2-sample rotated-grid AA: two samples at diagonal subpixel offsets
    // Eliminates jagged edges on fractal silhouettes with only 2x cost
    vec2 px = 1.0 / RENDERSIZE;
    vec3 col = renderSample(uv + vec2( 0.33, -0.33) * px)
             + renderSample(uv + vec2(-0.33,  0.33) * px);
    col *= 0.5;

    col = sqrt(clamp(col, 0.0, 1.0)); // gamma correction
    fragColor = vec4(col, 1.0);
}
