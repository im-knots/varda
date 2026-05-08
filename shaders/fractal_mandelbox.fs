/*{
    "DESCRIPTION": "Mandelbox - Raymarched 3D fractal explorer with flythrough camera, fog, and orbit trap coloring",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "iterations", "TYPE": "float", "DEFAULT": 10.0, "MIN": 3.0, "MAX": 16.0, "LABEL": "Iterations"},
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
        {"NAME": "max_steps", "TYPE": "float", "DEFAULT": 96.0, "MIN": 40.0, "MAX": 200.0, "LABEL": "Max Steps"},
        {"NAME": "fog_density", "TYPE": "float", "DEFAULT": 0.1, "MIN": 0.0, "MAX": 1.0, "LABEL": "Fog Density"},
        {"NAME": "ao_strength", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.0, "LABEL": "AO Strength"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.9, 0.7, 0.2, 1.0], "LABEL": "Color Warm"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.2, 0.4, 0.9, 1.0], "LABEL": "Color Cool"},
        {"NAME": "color_mode", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Color Mode"},
        {"NAME": "sun_elev", "TYPE": "float", "DEFAULT": 0.6, "MIN": -1.0, "MAX": 1.0, "LABEL": "Sun Elevation"},
        {"NAME": "sun_azim", "TYPE": "float", "DEFAULT": 0.8, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Sun Azimuth"},
        {"NAME": "shadow_strength", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Shadow Strength"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.0, 1.0], "LABEL": "Background"}
    ]
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
    float fog_density;
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

vec2 mandelboxDE(vec3 pos) {
    float minR2 = min_radius * min_radius;
    float fixR2 = fixed_radius * fixed_radius;
    float sc = scale;
    vec3 z = pos;
    float dr = 1.0;
    float trap = 1e10;
    int iters = int(iterations);
    for (int i = 0; i < 16; i++) {
        if (i >= iters) break;
        z = clamp(z, -fold_limit, fold_limit) * 2.0 - z;
        float r2 = dot(z, z);
        trap = min(trap, r2);
        if (r2 < minR2) {
            float f = fixR2 / minR2;
            z *= f; dr *= f;
        } else if (r2 < fixR2) {
            float f = fixR2 / r2;
            z *= f; dr *= f;
        }
        z = z * sc + pos;
        dr = dr * abs(sc) + 1.0;
    }
    return vec2(length(z) / abs(dr), trap);
}

float de(vec3 p) { return mandelboxDE(p).x; }

// Tetrahedron normal — 4 DE calls instead of 6
vec3 calcNormal(vec3 p, float d) {
    float e = clamp(d * 0.002, 0.0001, 0.005);
    vec2 h = vec2(e, -e);
    return normalize(
        h.xyy * de(p + h.xyy) +
        h.yyx * de(p + h.yyx) +
        h.yxy * de(p + h.yxy) +
        h.xxx * de(p + h.xxx));
}

float softShadow(vec3 ro, vec3 rd, float tmin, float tmax, float k) {
    float res = 1.0;
    float t = tmin;
    for (int i = 0; i < 12; i++) {
        float h = de(ro + rd * t);
        res = min(res, k * h / t);
        t += clamp(h, 0.02, 0.5);
        if (h < 0.002 || t > tmax) break;
    }
    return clamp(res, 0.0, 1.0);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    mat3 camRot = rotY(rot_y) * rotX(rot_x);
    vec3 fw = camRot * vec3(0, 0, 1);
    vec3 ri = normalize(cross(fw, vec3(0, 1, 0)));
    vec3 up = cross(ri, fw);
    float cr = cos(rot_z), sr = sin(rot_z);
    vec3 ri2 = ri * cr + up * sr;
    vec3 up2 = up * cr - ri * sr;
    float ft = fly_speed * TIME * 0.3;
    vec3 ro = vec3(cam_x, cam_y, cam_z) + vec3(sin(ft*0.7)*0.5, sin(ft*0.4)*0.3, cos(ft*0.3)*0.5);
    vec3 rd = normalize(p.x * ri2 + p.y * up2 + fw / tan(fov * 0.5 + 0.3));

    vec3 sunDir = normalize(vec3(cos(sun_azim)*cos(sun_elev), sin(sun_elev), sin(sun_azim)*cos(sun_elev)));

    // Raymarch
    float dist = 0.0;
    int steps = 0;
    int maxS = int(max_steps);
    vec2 hitInfo = vec2(0.0);
    bool hit = false;
    float minD = 1e10; // track closest approach for edge glow
    for (int i = 0; i < 200; i++) {
        if (i >= maxS) break;
        hitInfo = mandelboxDE(ro + rd * dist);
        float d = hitInfo.x;
        float ad = abs(d);
        minD = min(minD, ad);
        float thresh = clamp(dist * 0.0001, 0.0001, 0.002);
        if (ad < thresh && dist > 0.02) { hit = true; steps = i; break; }
        dist += (d > 0.0) ? d * 0.95 : max(ad, 0.02);
        steps = i;
        if (dist > 50.0) break;
    }

    vec3 fogColor = bg_color.rgb;
    vec3 col = fogColor;

    if (hit) {
        vec3 hp = ro + rd * dist;
        vec3 n = calcNormal(hp, dist);

        float cm = color_mode;
        float t;
        if (cm < 0.5) {
            t = clamp(sqrt(hitInfo.y) * 0.5, 0.0, 1.0);
        } else if (cm < 1.5) {
            t = clamp(dist * 0.05, 0.0, 1.0);
        } else {
            t = float(steps) / float(maxS);
        }
        vec3 albedo = mix(color1.rgb, color2.rgb, t);

        float diff = max(dot(n, sunDir), 0.0);
        float amb = 0.3 + 0.15 * (0.5 + 0.5 * n.y);
        float ao = 1.0 - ao_strength * float(steps) / float(maxS);

        float shad = 1.0;
        if (shadow_strength > 0.01) {
            shad = mix(1.0, softShadow(hp + n * 0.01, sunDir, 0.01, 5.0, 8.0), shadow_strength);
        }

        col = albedo * (diff * 0.8 * shad + amb) * ao;
        col += vec3(1.0) * pow(max(dot(reflect(-sunDir, n), -rd), 0.0), 32.0) * 0.3 * shad;
        col = mix(col, fogColor, 1.0 - exp(-dist * fog_density));
    } else {
        // Edge glow: soften silhouettes by blending near-miss rays
        float glow = exp(-minD * 200.0) * 0.15;
        col = mix(col, mix(color1.rgb, color2.rgb, 0.5), glow);
    }

    fragColor = vec4(clamp(col, 0.0, 1.0), 1.0);
}
