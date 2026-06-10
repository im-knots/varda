/*{
    "DESCRIPTION": "Crystal Cave - fly through a 3D cave filled with growing crystal formations",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D"],
    "INPUTS": [
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 3.0, "LABEL": "Zoom"},
        {"NAME": "fly_speed", "TYPE": "float", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 2.0, "LABEL": "Fly Speed"},
        {"NAME": "rot_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate X"},
        {"NAME": "rot_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate Y"},
        {"NAME": "rot_z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate Z"},
        {"NAME": "rot_speed_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin X"},
        {"NAME": "rot_speed_y", "TYPE": "float", "DEFAULT": 0.02, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin Y"},
        {"NAME": "rot_speed_z", "TYPE": "float", "DEFAULT": 0.0, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin Z"},
        {"NAME": "crystal_size", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.05, "MAX": 0.8, "LABEL": "Crystal Size"},
        {"NAME": "crystal_growth", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Crystal Growth"},
        {"NAME": "crystal_density", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 2.0, "LABEL": "Crystal Density"},
        {"NAME": "cave_width", "TYPE": "float", "DEFAULT": 1.8, "MIN": 0.8, "MAX": 3.5, "LABEL": "Cave Width"},
        {"NAME": "color_crystal", "TYPE": "color", "DEFAULT": [0.3, 0.6, 1.0, 1.0], "LABEL": "Crystal Color"},
        {"NAME": "color_glow", "TYPE": "color", "DEFAULT": [0.7, 0.4, 1.0, 1.0], "LABEL": "Crystal Glow"},
        {"NAME": "color_cave", "TYPE": "color", "DEFAULT": [0.08, 0.05, 0.12, 1.0], "LABEL": "Cave Wall"},
        {"NAME": "glow_strength", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 2.0, "LABEL": "Glow Strength"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "fly_speed", "INDEX": 0},
        {"PARAM": "rot_speed_x", "INDEX": 1},
        {"PARAM": "rot_speed_y", "INDEX": 2},
        {"PARAM": "rot_speed_z", "INDEX": 3}
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
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform UserParams {
    float zoom;
    float fly_speed;
    float rot_x;
    float rot_y;
    float rot_z;
    float rot_speed_x;
    float rot_speed_y;
    float rot_speed_z;
    float crystal_size;
    float crystal_growth;
    float crystal_density;
    float cave_width;
    vec4 color_crystal;
    vec4 color_glow;
    vec4 color_cave;
    float glow_strength;
};

// Hash
vec3 hash3(vec3 p) {
    p = vec3(dot(p,vec3(127.1,311.7,74.7)), dot(p,vec3(269.5,183.3,246.1)), dot(p,vec3(113.5,271.9,124.6)));
    return fract(sin(p)*43758.5453);
}
float hash1(vec3 p) { return fract(sin(dot(p,vec3(127.1,311.7,74.7)))*43758.5453); }

// Rotation
mat3 rotX(float a){float c=cos(a),s=sin(a);return mat3(1,0,0,0,c,-s,0,s,c);}
mat3 rotY(float a){float c=cos(a),s=sin(a);return mat3(c,0,s,0,1,0,-s,0,c);}
mat3 rotZ(float a){float c=cos(a),s=sin(a);return mat3(c,-s,0,s,c,0,0,0,1);}

// Noise for cave walls
float noise3(vec3 p) {
    vec3 i=floor(p); vec3 f=fract(p);
    f=f*f*(3.0-2.0*f);
    float a=hash1(i), b=hash1(i+vec3(1,0,0)), c=hash1(i+vec3(0,1,0)), d=hash1(i+vec3(1,1,0));
    float e=hash1(i+vec3(0,0,1)), g=hash1(i+vec3(1,0,1)), h=hash1(i+vec3(0,1,1)), k=hash1(i+vec3(1,1,1));
    return mix(mix(mix(a,b,f.x),mix(c,d,f.x),f.y), mix(mix(e,g,f.x),mix(h,k,f.x),f.y), f.z);
}


// Crystal SDF: elongated hexagonal prism tapering to a point
float sdCrystal(vec3 p, float h, float r) {
    vec3 ap = abs(p);
    float hex = max(ap.x, ap.x * 0.5 + ap.z * 0.866) - r;
    float taper = max(0.0, ap.y - h * 0.5) * 0.8;
    return max(hex + taper, ap.y - h);
}

// Cave tunnel: inverted noisy cylinder along Z
float caveSDF(vec3 p) {
    float tunnel = length(p.xy) - cave_width;
    tunnel += noise3(p * 0.5) * 1.0 + noise3(p * 1.1) * 0.4;
    return tunnel;
}

// Crystal field via Voronoi placement
// Returns (distance, crystal_id)
vec2 crystalField(vec3 p) {
    vec3 cs = vec3(1.0 / crystal_density);
    vec3 cp = p / cs;
    vec3 i = floor(cp);
    vec3 f = fract(cp);
    float minD = 100.0;
    float id = 0.0;

    for (int x = -1; x <= 1; x++)
    for (int y = -1; y <= 1; y++)
    for (int z = -1; z <= 1; z++) {
        vec3 nb = vec3(float(x), float(y), float(z));
        vec3 cell = i + nb;
        vec3 rnd = hash3(cell);

        vec3 cpos = (nb + rnd * 0.7 + 0.15 - f) * cs;

        // Growth animation
        float gp = fract(rnd.x * 7.0 + TIME * 0.05 * crystal_growth);
        float gf = smoothstep(0.0, 0.4, gp);
        gf *= mix(1.0, 1.0 - smoothstep(0.8, 1.0, gp), 1.0 - crystal_growth);
        float sz = crystal_size * (0.3 + rnd.y * 0.7) * gf;
        if (sz < 0.01) continue;

        // Random orientation
        float tx = (rnd.z - 0.5) * 1.8;
        float tz = (rnd.x - 0.5) * 1.8;
        vec3 lp = cpos;
        float cx=cos(tx),sx=sin(tx),cz=cos(tz),sz2=sin(tz);
        lp = vec3(lp.x, cx*lp.y-sx*lp.z, sx*lp.y+cx*lp.z);
        lp = vec3(cz*lp.x-sz2*lp.y, sz2*lp.x+cz*lp.y, lp.z);

        float d = sdCrystal(lp, sz * 1.8, sz * 0.22);
        if (d < minD) { minD = d; id = hash1(cell); }
    }
    return vec2(minD, id);
}

// Scene: we are INSIDE the cave. Returns (dist, material)
// material: 0 = cave wall, >0.5 = crystal (encoded id)
vec2 sceneMap(vec3 p) {
    float cave = caveSDF(p);
    vec2 crys = crystalField(p);
    // Inside cave when cave < 0
    if (cave > 0.0) return vec2(cave, 0.0); // wall
    if (crys.x < 0.003) return vec2(crys.x, 1.0 + crys.y);
    return vec2(min(-cave, crys.x), 0.0);
}

vec3 calcNormal(vec3 p) {
    vec2 e = vec2(0.004, 0.0);
    return normalize(vec3(
        sceneMap(p+e.xyy).x - sceneMap(p-e.xyy).x,
        sceneMap(p+e.yxy).x - sceneMap(p-e.yxy).x,
        sceneMap(p+e.yyx).x - sceneMap(p-e.yyx).x));
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = PHASE_TIME_0;

    // Rotation
    mat3 rot = rotZ(rot_z + PHASE_TIME_3) * rotY(rot_y + PHASE_TIME_2) * rotX(rot_x + PHASE_TIME_1);

    // Camera: fly through tunnel with gentle sway
    vec3 ro = vec3(sin(t*0.3)*0.4, cos(t*0.45)*0.3, t*2.0);
    vec3 fw = rot * vec3(0,0,1);
    vec3 up = rot * vec3(0,1,0);
    vec3 ri = normalize(cross(fw, up));
    up = cross(ri, fw);
    float fov = 0.8 / max(zoom, 0.3);
    vec3 rd = normalize(fw + p.x*ri*fov + p.y*up*fov);

    // Raymarch — 64 steps max
    float dist = 0.0;
    vec2 hit = vec2(20.0, 0.0);
    for (int i = 0; i < 64; i++) {
        hit = sceneMap(ro + rd * dist);
        if (abs(hit.x) < 0.002) break;
        dist += max(abs(hit.x) * 0.6, 0.015);
        if (dist > 20.0) break;
    }

    vec3 col = color_cave.rgb * 0.01; // deep darkness default

    if (dist < 20.0) {
        vec3 hp = ro + rd * dist;
        vec3 n = calcNormal(hp);
        float mat = hit.y;

        // Lighting: headlamp from camera
        float diff = max(dot(n, -rd), 0.0) * 0.7 + 0.1;
        float spec = pow(max(dot(reflect(rd, n), -rd), 0.0), 48.0);

        if (mat > 0.5) {
            // Crystal surface
            float cid = mat - 1.0;
            vec3 cCol = mix(color_crystal.rgb, color_glow.rgb, cid);
            float fresnel = pow(1.0 - abs(dot(n, -rd)), 3.0);
            col = cCol * diff;
            col += cCol * fresnel * glow_strength * 0.8;
            col += vec3(1.0) * spec * 0.4;
            col += color_glow.rgb * glow_strength * 0.1;
        } else {
            // Cave rock
            float rock = noise3(hp * 3.0) * 0.3 + 0.7;
            col = color_cave.rgb * diff * rock;
        }

        // Distance fog
        col *= exp(-dist * 0.1);

        // Crystal proximity glow on nearby surfaces
        vec2 crys = crystalField(hp);
        float prox = 1.0 / (1.0 + crys.x * crys.x * 15.0);
        vec3 glowCol = mix(color_crystal.rgb, color_glow.rgb, crys.y);
        col += glowCol * prox * glow_strength * 0.25;
    }

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}