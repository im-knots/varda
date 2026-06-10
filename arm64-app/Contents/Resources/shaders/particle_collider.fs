/*{
    "DESCRIPTION": "Particle Collider - ATLAS/CERN-style collision with cascading fission tracks",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D"],
    "INPUTS": [
        {"NAME": "zoom", "TYPE": "float", "DEFAULT": 2.5, "MIN": 0.3, "MAX": 5.0, "LABEL": "Zoom"},
        {"NAME": "collision_rate", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 3.0, "LABEL": "Collision Rate"},
        {"NAME": "rot_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate X"},
        {"NAME": "rot_y", "TYPE": "float", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate Y"},
        {"NAME": "rot_speed_x", "TYPE": "float", "DEFAULT": 0.02, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin X"},
        {"NAME": "rot_speed_y", "TYPE": "float", "DEFAULT": 0.03, "MIN": -0.3, "MAX": 0.3, "LABEL": "Spin Y"},
        {"NAME": "track_length", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.1, "MAX": 2.0, "LABEL": "Track Length"},
        {"NAME": "curvature", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.5, "LABEL": "Magnetic Curvature"},
        {"NAME": "source_pairs", "TYPE": "float", "DEFAULT": 2.0, "MIN": 2.0, "MAX": 16.0, "LABEL": "Source Pairs (2/4/8/16)"},
        {"NAME": "cascade_depth", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 8.0, "LABEL": "Cascade Depth"},
        {"NAME": "glow_intensity", "TYPE": "float", "DEFAULT": 1.5, "MIN": 0.0, "MAX": 4.0, "LABEL": "Glow"},
        {"NAME": "color_beam", "TYPE": "color", "DEFAULT": [0.3, 0.7, 1.0, 1.0], "LABEL": "Beam Color"},
        {"NAME": "color_hot", "TYPE": "color", "DEFAULT": [1.0, 0.9, 0.3, 1.0], "LABEL": "Collision Color"},
        {"NAME": "color_track", "TYPE": "color", "DEFAULT": [0.1, 0.9, 0.5, 1.0], "LABEL": "Track Color"},
        {"NAME": "color_secondary", "TYPE": "color", "DEFAULT": [0.9, 0.2, 0.6, 1.0], "LABEL": "Secondary Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.0, 0.0, 0.02, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "collision_rate", "INDEX": 0},
        {"PARAM": "rot_speed_x", "INDEX": 1},
        {"PARAM": "rot_speed_y", "INDEX": 2}
    ]
}*/

#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME; float TIMEDELTA; uint FRAMEINDEX; int PASSINDEX;
    vec2 RENDERSIZE;
    float audio_level; float audio_bass; float audio_mid; float audio_treble;
    float audio_bpm; float audio_beat_phase;
    vec4 DATE;
    float PHASE_TIME_0; float PHASE_TIME_1; float PHASE_TIME_2; float PHASE_TIME_3;
};

layout(set = 0, binding = 1) uniform UserParams {
    float zoom; float collision_rate;
    float rot_x; float rot_y; float rot_speed_x; float rot_speed_y;
    float track_length; float curvature; float source_pairs; float cascade_depth; float glow_intensity;
    vec4 color_beam; vec4 color_hot; vec4 color_track; vec4 color_secondary; vec4 bg_color;
};

#define PI 3.14159265359
#define TAU 6.28318530718
float hash(float n) { return fract(sin(n)*43758.5453); }
vec3 hash3v(float n) { return vec3(hash(n), hash(n+71.0), hash(n+137.0)); }

// 3D→2D projection: rotate then perspective project
vec2 proj(vec3 p, mat3 rot) {
    vec3 r = rot * p;
    float persp = zoom / (2.0 + r.z);
    return r.xy * persp;
}

// Distance from screen point to projected 3D segment
float dProjSeg(vec2 sc, vec3 a3, vec3 b3, mat3 rot) {
    vec2 a = proj(a3, rot), b = proj(b3, rot);
    vec2 ba = b-a, pa = sc-a;
    float h = clamp(dot(pa,ba)/(dot(ba,ba)+1e-8), 0.0, 1.0);
    return length(pa - ba*h);
}

// Compute a point on a curved particle track (helical in magnetic field)
vec3 trackPt(float t, float seed, vec3 ori, vec3 dir, float charge) {
    float mom = 1.0 - t*t*0.5;
    float ca = t * curvature * charge * TAU;
    vec3 up = abs(dir.y) < 0.9 ? vec3(0,1,0) : vec3(1,0,0);
    vec3 p1 = normalize(cross(dir, up));
    vec3 p2 = cross(dir, p1);
    float r = 0.12 * mom / (abs(charge)+0.3);
    return ori + dir*t*track_length*mom + p1*sin(ca)*r + p2*(1.0-cos(ca))*r;
}

// Particle type → color: 6 subatomic particle species
// 0=photon(beam), 1=muon(track), 2=pion(secondary), 3=kaon(hot), 4=electron(blend), 5=neutrino(blend)
vec3 particleColor(float seed) {
    int ptype = int(floor(hash(seed + 99.0) * 6.0));
    if (ptype == 0) return color_beam.rgb;
    if (ptype == 1) return color_track.rgb;
    if (ptype == 2) return color_secondary.rgb;
    if (ptype == 3) return color_hot.rgb;
    if (ptype == 4) return mix(color_beam.rgb, color_track.rgb, 0.5);
    return mix(color_hot.rgb, color_secondary.rgb, 0.5);
}

// Distance from screen point to a projected particle track
// Segment count adapts to curvature to keep curves smooth
vec2 evalTrack(vec2 sc, float seed, vec3 ori, vec3 dir, float charge, float energy, mat3 rot) {
    float minD = 1e6;
    float bestE = 0.0;
    vec3 prev = ori;
    int nSeg = 8 + int(curvature * 8.0);
    for (int i = 1; i <= 20; i++) {
        if (i > nSeg) break;
        float t = float(i) / float(nSeg);
        vec3 cur = trackPt(t, seed, ori, dir, charge);
        float d = dProjSeg(sc, prev, cur, rot);
        if (d < minD) { minD = d; bestE = energy * (1.0 - t*0.8); }
        prev = cur;
    }
    return vec2(minD, bestE);
}

// Add a single track's glow contribution with its particle-type color
vec3 trackGlow(vec2 r, float seed) {
    float g = r.y * exp(-r.x*r.x / (0.0003*glow_intensity+1e-5));
    return particleColor(seed) * g;
}

void main() {
    float audioSum=audio_level+audio_bass+audio_mid+audio_treble+audio_bpm+audio_beat_phase;
    float timeSum=TIMEDELTA+float(FRAMEINDEX)+float(PASSINDEX)+DATE.x+DATE.y+DATE.z+DATE.w+PHASE_TIME_0+PHASE_TIME_1+PHASE_TIME_2+PHASE_TIME_3;
    if (uv.x<-1.0) { fragColor=vec4(audioSum+timeSum,0.0,0.0,1.0); return; }

    vec2 sc = (uv - 0.5) * 2.0;
    sc.x *= RENDERSIZE.x / RENDERSIZE.y;
    float t = PHASE_TIME_0;

    // Build rotation matrix
    float ax = rot_x + PHASE_TIME_1;
    float ay = rot_y + PHASE_TIME_2;
    float cax=cos(ax),sax=sin(ax),cay=cos(ay),say=sin(ay);
    mat3 rot = mat3(cay,0,say, sax*say,cax,-sax*cay, -cax*say,sax,cax*cay);

    vec3 col = bg_color.rgb;

    // Incoming particles flying toward per-pair random collision points
    int nPairs = int(clamp(source_pairs, 2.0, 16.0));
    float period = 3.0;
    for (int ev = 0; ev < 16; ev++) {
        if (ev >= nPairs) break;
        float evOff = float(ev) * period / float(nPairs);
        float age = mod(t + evOff, period);
        if (age < 0.35) {
            float approach = age / 0.35;
            float seed = floor((t + evOff) / period) * 17.3 + float(ev) * 53.1;
            // Each pair gets a random collision point near center
            vec3 collPt = (hash3v(seed + 60.0) - 0.5) * 0.3;
            // Each pair approaches from its own unique random direction
            float theta = hash(seed + 80.0) * PI;
            float phi = hash(seed + 81.0) * TAU;
            vec3 inDir = vec3(sin(theta)*cos(phi), sin(theta)*sin(phi), cos(theta));
            float startDist = 1.2;
            vec3 p1pos = collPt + inDir * startDist * (1.0 - approach);
            vec3 p2pos = collPt - inDir * startDist * (1.0 - approach);
            vec2 sp1 = proj(p1pos, rot), sp2 = proj(p2pos, rot);
            float d1 = length(sc - sp1), d2 = length(sc - sp2);
            float brightness = 0.6 + approach * 0.4;
            col += color_beam.rgb * brightness * exp(-d1*d1*3000.0);
            col += color_beam.rgb * brightness * exp(-d2*d2*3000.0);
            // Faint trail behind each particle
            for (int tr = 1; tr <= 3; tr++) {
                float trA = approach - float(tr) * 0.04;
                if (trA < 0.0) continue;
                vec3 tp1 = collPt + inDir * startDist * (1.0 - trA);
                vec3 tp2 = collPt - inDir * startDist * (1.0 - trA);
                float td1 = length(sc - proj(tp1, rot));
                float td2 = length(sc - proj(tp2, rot));
                float trFade = 0.25 / float(tr);
                col += color_beam.rgb * trFade * exp(-td1*td1*4000.0);
                col += color_beam.rgb * trFade * exp(-td2*td2*4000.0);
            }
        }
    }

    // Collision events with per-track particle-type colors and deep cascades
    for (int ev = 0; ev < 16; ev++) {
        if (ev >= nPairs) break;
        float evOff = float(ev) * period / float(nPairs);
        float rawAge = mod(t + evOff, period);
        float seed = floor((t + evOff) / period) * 17.3 + float(ev) * 53.1;
        float age = rawAge - 0.35;
        if (age < 0.0 || age > 2.5) continue;

        vec3 collPt = (hash3v(seed + 60.0) - 0.5) * 0.3;

        // Collision flash
        vec2 flashP = proj(collPt, rot);
        float flashD = length(sc - flashP);
        col += color_hot.rgb * exp(-age*6.0) * exp(-flashD*flashD*200.0) * 2.0;

        int maxPri = max(3, 10 - nPairs / 2);
        int nPri = max(3, maxPri - 2 + int(hash(seed + 10.0) * 3.0));
        float growLin = clamp(age * 1.2, 0.0, 1.0);
        float grow = growLin * growLin * (3.0 - 2.0 * growLin); // smoothstep ease
        int mxC = int(cascade_depth);

        for (int i = 0; i < 8; i++) {
            if (i >= nPri) break;
            float s = seed + float(i) * 13.7;
            float theta=hash(s)*PI, phi=hash(s+7.0)*TAU;
            vec3 dir = vec3(sin(theta)*cos(phi), sin(theta)*sin(phi), cos(theta));
            float ch = hash(s+3.0) > 0.5 ? 1.0 : -1.0;
            float en = (0.5+hash(s+5.0)*0.5) * grow * exp(-age*0.8);

            // L1: primary track — each is a random particle type
            vec2 r1 = evalTrack(sc, s, collPt, dir, ch, en, rot);
            col += trackGlow(r1, s);

            // L2: secondary decay products
            if (mxC >= 2 && age > 0.2 && hash(s+20.0) > 0.4) {
                float spT = 0.3 + hash(s+25.0)*0.3;
                vec3 spPt = trackPt(spT*grow, s, collPt, dir, ch);
                float sA = age - 0.2, sGL = clamp(sA*1.5,0.0,1.0);
                float sG = sGL*sGL*(3.0-2.0*sGL);
                for (int j = 0; j < 2; j++) {
                    float sj = s+float(j)*29.3+100.0;
                    vec3 d2 = normalize(dir+(hash3v(sj)-0.5)*1.6);
                    float c2 = hash(sj+3.0)>0.5?0.7:-0.7;
                    float e2 = en*0.45*sG*exp(-sA*1.0);
                    vec2 r2 = evalTrack(sc, sj, spPt, d2, c2, e2, rot);
                    col += trackGlow(r2, sj);

                    // L3: tertiary
                    if (mxC >= 3 && sA > 0.2 && hash(sj+40.0) > 0.5) {
                        float t3=0.3+hash(sj+45.0)*0.3;
                        vec3 sp3=trackPt(t3*sG, sj, spPt, d2, c2);
                        float a3=sA-0.2, g3L=clamp(a3*1.8,0.0,1.0);
                        float g3=g3L*g3L*(3.0-2.0*g3L);
                        for (int k = 0; k < 2; k++) {
                            float sk=sj+float(k)*41.7+200.0;
                            vec3 d3=normalize(d2+(hash3v(sk)-0.5)*2.0);
                            float e3=e2*0.4*g3*exp(-a3*1.2);
                            vec2 r3=evalTrack(sc, sk, sp3, d3, hash(sk)-0.5, e3, rot);
                            col += trackGlow(r3, sk);

                            // L4
                            if (mxC >= 4 && a3 > 0.2 && hash(sk+50.0) > 0.55) {
                                float t4=0.3+hash(sk+55.0)*0.3;
                                vec3 sp4=trackPt(t4*g3, sk, sp3, d3, hash(sk)-0.5);
                                float a4=a3-0.2, g4L=clamp(a4*2.0,0.0,1.0);
                                float g4=g4L*g4L*(3.0-2.0*g4L);
                                float sk4=sk+300.0;
                                vec3 d4=normalize(d3+(hash3v(sk4)-0.5)*2.2);
                                float e4=e3*0.35*g4*exp(-a4*1.5);
                                vec2 r4=evalTrack(sc, sk4, sp4, d4, hash(sk4+3.0)-0.5, e4, rot);
                                col += trackGlow(r4, sk4);

                                // L5
                                if (mxC >= 5 && a4 > 0.15 && hash(sk4+60.0) > 0.6) {
                                    float t5=0.3+hash(sk4+65.0)*0.3;
                                    vec3 sp5=trackPt(t5*g4, sk4, sp4, d4, hash(sk4+3.0)-0.5);
                                    float sk5=sk4+400.0;
                                    vec3 d5=normalize(d4+(hash3v(sk5)-0.5)*2.5);
                                    float g5L=clamp((a4-0.15)*2.2,0.0,1.0);
                                    float g5=g5L*g5L*(3.0-2.0*g5L);
                                    float e5=e4*0.3*g5;
                                    vec2 r5=evalTrack(sc, sk5, sp5, d5, hash(sk5)-0.5, e5, rot);
                                    col += trackGlow(r5, sk5);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    col = clamp(col, 0.0, 1.0);
    fragColor = vec4(col, 1.0);
}
