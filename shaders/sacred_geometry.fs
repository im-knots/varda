/*{
    "DESCRIPTION": "Sacred Geometry - Flower of Life, Metatron's Cube, Sri Yantra, Seed of Life, Vesica Piscis, Pentagram, Fibonacci Spiral",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "pattern", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 6.0, "LABEL": "Pattern (0=Flower 1=Metatron 2=Sri Yantra 3=Seed 4=Vesica 5=Pentagram 6=Fibonacci)"},
        {"NAME": "layers", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 6.0, "LABEL": "Layers"},
        {"NAME": "scale", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 3.0, "LABEL": "Scale"},
        {"NAME": "rotation", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 6.283, "LABEL": "Rotation"},
        {"NAME": "anim_speed", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 3.0, "LABEL": "Speed"},
        {"NAME": "line_width", "TYPE": "float", "DEFAULT": 0.012, "MIN": 0.002, "MAX": 0.04, "LABEL": "Line Width"},
        {"NAME": "glow_amount", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 3.0, "LABEL": "Glow"},
        {"NAME": "color1", "TYPE": "color", "DEFAULT": [0.9, 0.75, 0.3, 1.0], "LABEL": "Line Color"},
        {"NAME": "color2", "TYPE": "color", "DEFAULT": [0.3, 0.5, 0.9, 1.0], "LABEL": "Inner Color"},
        {"NAME": "bg_color", "TYPE": "color", "DEFAULT": [0.02, 0.01, 0.05, 1.0], "LABEL": "Background"}
    ],
    "PHASE_INPUTS": [{"PARAM": "anim_speed", "INDEX": 0}]
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
    float pattern;
    float layers;
    float scale;
    float rotation;
    float anim_speed;
    float line_width;
    float glow_amount;
    vec4 color1;
    vec4 color2;
    vec4 bg_color;
};

#define PI 3.14159265359
#define TAU 6.28318530718

// Distance to a circle ring
float dCircle(vec2 p, vec2 center, float radius) {
    return abs(length(p - center) - radius);
}

// Distance to a line segment
float dSegment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Distance to an equilateral triangle (outline)
float dTriangle(vec2 p, vec2 center, float radius, float rot) {
    float d = 1e6;
    for (int i = 0; i < 3; i++) {
        float a1 = rot + float(i) * TAU / 3.0;
        float a2 = rot + float(i + 1) * TAU / 3.0;
        vec2 v1 = center + radius * vec2(cos(a1), sin(a1));
        vec2 v2 = center + radius * vec2(cos(a2), sin(a2));
        d = min(d, dSegment(p, v1, v2));
    }
    return d;
}

// Flower of Life: overlapping circles in hexagonal arrangement
float flowerOfLife(vec2 p, float r, int ringCount) {
    float d = 1e6;
    // Early-out: skip if pixel is far from pattern
    float pDist = length(p);
    if (pDist > r * float(ringCount + 2)) return pDist - r;

    // Center circle
    d = min(d, dCircle(p, vec2(0.0), r));
    // Rings of circles (capped at 3 for performance)
    int maxRing = min(ringCount, 3);
    for (int ring = 1; ring <= 3; ring++) {
        if (ring > maxRing) break;
        int count = ring * 6;
        for (int i = 0; i < 18; i++) {
            if (i >= count) break;
            float angle = float(i) * TAU / float(count) + PI / 6.0;
            vec2 center = vec2(cos(angle), sin(angle)) * r * float(ring);
            d = min(d, dCircle(p, center, r));
        }
    }
    return d;
}

// Metatron's Cube: 13 circles + connecting lines
float metatronsCube(vec2 p, float r, int layerCount) {
    float d = 1e6;
    // Early-out: skip if pixel is far from pattern
    float pDist = length(p);
    if (pDist > r * 2.5) return pDist - r;

    // 13 node positions: center + 6 inner + 6 outer
    vec2 nodes[13];
    nodes[0] = vec2(0.0);
    for (int i = 0; i < 6; i++) {
        float a = float(i) * TAU / 6.0;
        nodes[i + 1] = vec2(cos(a), sin(a)) * r;
        nodes[i + 7] = vec2(cos(a + TAU / 12.0), sin(a + TAU / 12.0)) * r * 1.732;
    }

    // Circles at each node
    int nodeCount = layerCount >= 3 ? 13 : (layerCount >= 2 ? 7 : 1);
    for (int i = 0; i < 13; i++) {
        if (i >= nodeCount) break;
        d = min(d, dCircle(p, nodes[i], r * 0.15));
    }

    // Connecting lines — only between nearby nodes to reduce O(n²) cost
    if (layerCount >= 2) {
        for (int i = 0; i < 13; i++) {
            if (i >= nodeCount) break;
            // Skip node if pixel is far from it
            if (length(p - nodes[i]) > r * 2.0) continue;
            for (int j = i + 1; j < 13; j++) {
                if (j >= nodeCount) break;
                d = min(d, dSegment(p, nodes[i], nodes[j]));
            }
        }
    }

    // Outer circle and hexagon
    if (layerCount >= 2) {
        d = min(d, dCircle(p, vec2(0.0), r * 1.732));
        // Hexagon
        for (int i = 0; i < 6; i++) {
            float a1 = float(i) * TAU / 6.0;
            float a2 = float(i + 1) * TAU / 6.0;
            vec2 v1 = vec2(cos(a1), sin(a1)) * r;
            vec2 v2 = vec2(cos(a2), sin(a2)) * r;
            d = min(d, dSegment(p, v1, v2));
        }
    }

    return d;
}

// Sri Yantra: concentric triangles pointing up and down
float sriYantra(vec2 p, float r, int layerCount) {
    float d = 1e6;
    // Outer circle
    d = min(d, dCircle(p, vec2(0.0), r * 1.2));

    // Concentric triangles: upward and downward pointing
    for (int i = 0; i < 6; i++) {
        if (i >= layerCount * 2) break;
        float s = r * (1.0 - float(i) * 0.14);
        if (i % 2 == 0) {
            // Upward triangle
            d = min(d, dTriangle(p, vec2(0.0, -s * 0.08), s, -PI / 2.0));
        } else {
            // Downward triangle
            d = min(d, dTriangle(p, vec2(0.0, s * 0.08), s, PI / 2.0));
        }
    }

    // Inner petals from triangle intersections (lotus-like)
    if (layerCount >= 3) {
        d = min(d, dCircle(p, vec2(0.0), r * 0.15));
    }

    // Outer square (bhupura gate)
    if (layerCount >= 2) {
        float sq = r * 1.35;
        d = min(d, dSegment(p, vec2(-sq, -sq), vec2(sq, -sq)));
        d = min(d, dSegment(p, vec2(sq, -sq), vec2(sq, sq)));
        d = min(d, dSegment(p, vec2(sq, sq), vec2(-sq, sq)));
        d = min(d, dSegment(p, vec2(-sq, sq), vec2(-sq, -sq)));
        // Gate openings (T-shaped extensions at midpoints)
        float gate = r * 0.15;
        // Top gate
        d = min(d, dSegment(p, vec2(-gate, sq), vec2(-gate, sq + gate)));
        d = min(d, dSegment(p, vec2(gate, sq), vec2(gate, sq + gate)));
        d = min(d, dSegment(p, vec2(-gate, sq + gate), vec2(gate, sq + gate)));
        // Bottom gate
        d = min(d, dSegment(p, vec2(-gate, -sq), vec2(-gate, -sq - gate)));
        d = min(d, dSegment(p, vec2(gate, -sq), vec2(gate, -sq - gate)));
        d = min(d, dSegment(p, vec2(-gate, -sq - gate), vec2(gate, -sq - gate)));
        // Left gate
        d = min(d, dSegment(p, vec2(-sq, -gate), vec2(-sq - gate, -gate)));
        d = min(d, dSegment(p, vec2(-sq, gate), vec2(-sq - gate, gate)));
        d = min(d, dSegment(p, vec2(-sq - gate, -gate), vec2(-sq - gate, gate)));
        // Right gate
        d = min(d, dSegment(p, vec2(sq, -gate), vec2(sq + gate, -gate)));
        d = min(d, dSegment(p, vec2(sq, gate), vec2(sq + gate, gate)));
        d = min(d, dSegment(p, vec2(sq + gate, -gate), vec2(sq + gate, gate)));
    }

    return d;
}

// Seed of Life: 7 equal circles — one center + 6 around it
float seedOfLife(vec2 p, float r, int layerCount) {
    float d = 1e6;
    // Center circle
    d = min(d, dCircle(p, vec2(0.0), r));
    // 6 surrounding circles
    for (int i = 0; i < 6; i++) {
        float a = float(i) * TAU / 6.0;
        vec2 c = vec2(cos(a), sin(a)) * r;
        d = min(d, dCircle(p, c, r));
    }
    // Layer 2: outer ring of 6 circles on the petal tips
    if (layerCount >= 2) {
        for (int i = 0; i < 6; i++) {
            float a = float(i) * TAU / 6.0 + TAU / 12.0;
            vec2 c = vec2(cos(a), sin(a)) * r * 1.732;
            d = min(d, dCircle(p, c, r));
        }
    }
    // Layer 3+: encompassing circle
    if (layerCount >= 3) {
        d = min(d, dCircle(p, vec2(0.0), r * 2.0));
    }
    if (layerCount >= 4) {
        d = min(d, dCircle(p, vec2(0.0), r * 2.5));
    }
    return d;
}

// Vesica Piscis: two overlapping circles forming the almond (mandorla)
float vesicaPiscis(vec2 p, float r, int layerCount) {
    float d = 1e6;
    float offset = r * 0.5;
    // Two primary circles
    d = min(d, dCircle(p, vec2(-offset, 0.0), r));
    d = min(d, dCircle(p, vec2(offset, 0.0), r));
    // Layer 2: vertical axis line through intersections + horizontal axis
    if (layerCount >= 2) {
        float h = sqrt(r * r - offset * offset);
        d = min(d, dSegment(p, vec2(0.0, -h), vec2(0.0, h)));
        d = min(d, dSegment(p, vec2(-offset - r, 0.0), vec2(offset + r, 0.0)));
    }
    // Layer 3: torus-like nested vesicas at 60° intervals
    if (layerCount >= 3) {
        for (int i = 1; i < 3; i++) {
            float a = float(i) * TAU / 6.0;
            float ca = cos(a), sa = sin(a);
            vec2 c1 = vec2(-offset * ca, -offset * sa);
            vec2 c2 = vec2(offset * ca, offset * sa);
            d = min(d, dCircle(p, c1, r));
            d = min(d, dCircle(p, c2, r));
        }
    }
    // Layer 4+: outer containing circle
    if (layerCount >= 4) {
        d = min(d, dCircle(p, vec2(0.0), r + offset));
    }
    if (layerCount >= 5) {
        // Inner trefoil arcs
        for (int i = 0; i < 6; i++) {
            float a = float(i) * TAU / 6.0;
            vec2 c = vec2(cos(a), sin(a)) * offset;
            d = min(d, dCircle(p, c, r * 0.5));
        }
    }
    return d;
}

// Pentagram: five-pointed star inscribed in a circle with golden ratio proportions
float pentagram(vec2 p, float r, int layerCount) {
    float d = 1e6;
    // Outer circle
    d = min(d, dCircle(p, vec2(0.0), r));
    // 5 vertices of the pentagon
    vec2 verts[5];
    for (int i = 0; i < 5; i++) {
        float a = float(i) * TAU / 5.0 - PI / 2.0;
        verts[i] = vec2(cos(a), sin(a)) * r;
    }
    // Pentagon outline
    for (int i = 0; i < 5; i++) {
        d = min(d, dSegment(p, verts[i], verts[(i + 1) % 5]));
    }
    // Star lines: connect every other vertex
    if (layerCount >= 2) {
        for (int i = 0; i < 5; i++) {
            d = min(d, dSegment(p, verts[i], verts[(i + 2) % 5]));
        }
    }
    // Inner pentagon (formed by star intersections, golden ratio scaled)
    if (layerCount >= 3) {
        float phi = (sqrt(5.0) - 1.0) / 2.0; // golden ratio conjugate
        float innerR = r * phi * phi;
        vec2 inner[5];
        for (int i = 0; i < 5; i++) {
            float a = float(i) * TAU / 5.0 - PI / 2.0 + TAU / 10.0;
            inner[i] = vec2(cos(a), sin(a)) * innerR;
        }
        for (int i = 0; i < 5; i++) {
            d = min(d, dSegment(p, inner[i], inner[(i + 1) % 5]));
        }
        // Nested star in inner pentagon
        if (layerCount >= 4) {
            for (int i = 0; i < 5; i++) {
                d = min(d, dSegment(p, inner[i], inner[(i + 2) % 5]));
            }
        }
    }
    // Layer 5+: golden spiral arcs approximated as circles
    if (layerCount >= 5) {
        float phi = (1.0 + sqrt(5.0)) / 2.0;
        for (int i = 0; i < 5; i++) {
            float arcR = r * pow(1.0 / phi, float(i));
            d = min(d, dCircle(p, vec2(0.0), arcR));
        }
    }
    return d;
}

// Fibonacci / Golden Spiral: spiral with golden-ratio-spaced radial lines
float fibonacciSpiral(vec2 p, float r, int layerCount) {
    float d = 1e6;
    float phi = (1.0 + sqrt(5.0)) / 2.0;
    float goldenAngle = TAU / (phi * phi);

    // Spiral: reduced from 128 to 48 samples per arm, max 4 arms
    int spiralArms = min(max(1, layerCount), 4);
    for (int arm = 0; arm < 4; arm++) {
        if (arm >= spiralArms) break;
        float armOffset = float(arm) * TAU / float(spiralArms);
        for (int i = 0; i < 48; i++) {
            float t = float(i) / 48.0;
            float angle = t * TAU * 3.0 + armOffset;
            float radius = r * 0.05 + r * t * t * 1.5;
            vec2 sp = vec2(cos(angle), sin(angle)) * radius;
            d = min(d, length(p - sp));
        }
    }

    // Golden-angle dots (reduced from 120 to 40 max)
    if (layerCount >= 2) {
        int seeds = min(layerCount * 20, 40);
        for (int i = 0; i < 40; i++) {
            if (i >= seeds) break;
            float a = float(i) * goldenAngle;
            float rad = r * sqrt(float(i) / float(seeds)) * 1.2;
            vec2 seedPos = vec2(cos(a), sin(a)) * rad;
            float seedDist = length(p - seedPos) - r * 0.015;
            d = min(d, max(seedDist, 0.0));
        }
    }

    // Outer circle
    d = min(d, dCircle(p, vec2(0.0), r * 1.5));

    return d;
}

void main() {
    // Uniform guard — prevent SPIR-V from optimizing out unused uniforms
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // Coordinate setup: centered, aspect-corrected
    vec2 p = (uv - 0.5) * 2.0;
    p.x *= RENDERSIZE.x / RENDERSIZE.y;

    // Apply rotation (animated + manual)
    float t = PHASE_TIME_0;
    float rot = rotation + t * 0.1;
    float cs = cos(rot), sn = sin(rot);
    p = mat2(cs, -sn, sn, cs) * p;

    // Scale
    p /= scale;

    float r = 0.35; // Base radius for patterns
    int layerCount = int(layers);

    // Compute distance based on selected pattern
    float d;
    int pat = int(floor(pattern + 0.5));
    if (pat <= 0) {
        d = flowerOfLife(p, r, layerCount);
    } else if (pat == 1) {
        d = metatronsCube(p, r * 1.5, layerCount);
    } else if (pat == 2) {
        d = sriYantra(p, r * 1.2, layerCount);
    } else if (pat == 3) {
        d = seedOfLife(p, r, layerCount);
    } else if (pat == 4) {
        d = vesicaPiscis(p, r, layerCount);
    } else if (pat == 5) {
        d = pentagram(p, r * 0.8, layerCount);
    } else {
        d = fibonacciSpiral(p, r * 0.6, layerCount);
    }

    // Render: crisp line + glow
    float lw = line_width / scale;
    float line = smoothstep(lw, lw * 0.2, d);

    // Soft glow around geometry
    float glowFalloff = lw * glow_amount * 3.0 + 0.001;
    float glow = exp(-d * d / (glowFalloff * glowFalloff)) * glow_amount * 0.4;

    // Breathing animation on inner color
    float breath = sin(t * 0.5) * 0.5 + 0.5;
    float centerGlow = exp(-dot(p, p) / (r * r * 0.5)) * breath * 0.3;

    // Compose
    vec3 col = bg_color.rgb;
    col += color2.rgb * centerGlow;
    col += color1.rgb * (line + glow);
    col = clamp(col, 0.0, 1.0);

    fragColor = vec4(col, 1.0);
}
