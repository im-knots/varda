/*{
    "DESCRIPTION": "Graph Network — physics-driven floating nodes that connect by proximity",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative"],
    "INPUTS": [
        {"NAME": "speed",          "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0,  "MAX": 3.0,  "LABEL": "Speed"},
        {"NAME": "rotation",       "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0,  "MAX": 1.0,  "LABEL": "Rotation"},
        {"NAME": "zoom",           "TYPE": "float", "DEFAULT": 1.0,  "MIN": 0.3,  "MAX": 3.0,  "LABEL": "Zoom"},
        {"NAME": "tracking",       "TYPE": "float", "DEFAULT": 0.0,  "MIN": 0.0,  "MAX": 1.0,  "LABEL": "Cluster Tracking"},
        {"NAME": "node_count",     "TYPE": "float", "DEFAULT": 32.0, "MIN": 3.0,  "MAX": 48.0, "LABEL": "Node Count"},
        {"NAME": "connect_dist",   "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.1,  "MAX": 1.5,  "LABEL": "Connect Distance"},
        {"NAME": "max_connections","TYPE": "float", "DEFAULT": 6.0,  "MIN": 1.0,  "MAX": 20.0, "LABEL": "Max Connections"},
        {"NAME": "stickiness",     "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0,  "MAX": 1.0,  "LABEL": "Stickiness"},
        {"NAME": "attraction",     "TYPE": "float", "DEFAULT": 0.5,  "MIN": 0.0,  "MAX": 2.0,  "LABEL": "Attraction"},
        {"NAME": "node_size",      "TYPE": "float", "DEFAULT": 0.015,"MIN": 0.004,"MAX": 0.05, "LABEL": "Node Size"},
        {"NAME": "edge_width",     "TYPE": "float", "DEFAULT": 0.003,"MIN": 0.001,"MAX": 0.01, "LABEL": "Edge Width"},
        {"NAME": "hue_shift",      "TYPE": "float", "DEFAULT": 0.05, "MIN": 0.0,  "MAX": 0.3,  "LABEL": "Edge Hue Shift"},
        {"NAME": "color_base",     "TYPE": "color", "DEFAULT": [0.0, 1.0, 0.8, 1.0], "LABEL": "Base Color"},
        {"NAME": "bg_color",       "TYPE": "color", "DEFAULT": [0.02, 0.03, 0.06, 1.0], "LABEL": "Background"}
    ],
    "PASSES": [
        {"TARGET": "stateBuffer", "PERSISTENT": true, "FLOAT": true}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "speed", "INDEX": 0},
        {"PARAM": "rotation", "INDEX": 1}
    ]
}*/

#version 450

layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;

layout(std140, set = 0, binding = 0) uniform ISFUniforms {
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

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D stateBuffer;

layout(std140, set = 0, binding = 3) uniform UserParams {
    float speed;
    float rotation;
    float zoom;
    float tracking;
    float node_count;
    float connect_dist;
    float max_connections;
    float stickiness;
    float attraction;
    float node_size;
    float edge_width;
    float hue_shift;
    vec4 color_base;
    vec4 bg_color;
};

// --- Helpers ---

float hash(vec2 p) {
    return fract(sin(dot(p, vec2(127.1, 311.7))) * 43758.5453);
}

vec2 hash2(vec2 p) {
    return vec2(hash(p), hash(p + vec2(37.0, 91.0)));
}

float dSegment(vec2 p, vec2 a, vec2 b) {
    vec2 pa = p - a, ba = b - a;
    float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

vec3 hsv2rgb(vec3 c) {
    vec3 p = abs(fract(c.xxx + vec3(1.0, 2.0/3.0, 1.0/3.0)) * 6.0 - 3.0);
    return c.z * mix(vec3(1.0), clamp(p - 1.0, 0.0, 1.0), c.y);
}

vec3 rgb2hsv(vec3 c) {
    vec4 K = vec4(0.0, -1.0/3.0, 2.0/3.0, -1.0);
    vec4 p = mix(vec4(c.bg, K.wz), vec4(c.gb, K.xy), step(c.b, c.g));
    vec4 q = mix(vec4(p.xyw, c.r), vec4(c.r, p.yzx), step(p.x, c.r));
    float d = q.x - min(q.w, q.y);
    float e = 1.0e-10;
    return vec3(abs(q.z + (q.w - q.y) / (6.0 * d + e)), d / (q.x + e), q.x);
}

vec2 rot2(vec2 v, float a) {
    float c = cos(a), s = sin(a);
    return mat2(c, s, -s, c) * v;
}

// Read node state from buffer row 0: xy = position, zw = velocity
vec4 readState(int i) {
    vec2 st = vec2((float(i) + 0.5) / RENDERSIZE.x, 0.5 / RENDERSIZE.y);
    return texture(sampler2D(stateBuffer, texSampler), st);
}

// Per-node base speed — each node drifts at its own rate
float nodeBaseSpeed(int i) {
    return 0.3 + hash(vec2(float(i), 5.0)) * 0.7;
}

// Per-pair rest length: how short this edge wants to be
// stickiness=0 → rest near connect_dist (loose). stickiness=1 → rest near random min (tight).
float pairRestLen(int i, int j) {
    float h = hash(vec2(float(min(i,j)) * 13.7, float(max(i,j)) * 7.3));
    return connect_dist * mix(0.7, 0.1 + 0.3 * h, stickiness);
}

const int MAX_NODES = 48;

void main() {
    // Uniform guard — keep all ISF uniforms alive
    float _keep = (audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase
                 + TIMEDELTA + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w
                 + PHASE_TIME_0 + PHASE_TIME_2 + PHASE_TIME_3) * 1e-7;

    int numNodes = clamp(int(node_count), 3, MAX_NODES);

    if (PASSINDEX == 0) {
        // === PHYSICS PASS — buffer row 0 stores node state (pos.xy, vel.zw) ===
        vec2 fc = gl_FragCoord.xy;
        int id = int(fc.x);

        if (fc.y < 1.5 && id < numNodes) {
            float fi = float(id);
            vec4 prev = readState(id);
            vec2 pos = prev.xy;
            vec2 vel = prev.zw;

            // Seed positions on first frame or uninitialized nodes
            if (FRAMEINDEX == 0u || (pos == vec2(0.0) && vel == vec2(0.0))) {
                pos = (hash2(vec2(fi, 100.0)) - 0.5) * 4.0;
                vel = (hash2(vec2(fi, 200.0)) - 0.5) * 0.1;
            }

            float dt = min(TIMEDELTA, 0.05);
            float mySpeed = nodeBaseSpeed(id);

            // Drift: smooth wandering unique to this node
            float angle = hash(vec2(fi, 10.0)) * 6.28
                        + TIME * (0.1 + hash(vec2(fi, 11.0)) * 0.15);
            vec2 force = vec2(cos(angle), sin(angle)) * mySpeed * 0.4;

            // Spring forces from connected neighbors (limited by max_connections)
            int maxConn = clamp(int(max_connections), 1, 20);
            int connCount = 0;
            for (int j = 0; j < MAX_NODES; j++) {
                if (j >= numNodes || j == id) continue;
                vec2 oPos = readState(j).xy;
                vec2 diff = oPos - pos;
                float dist = length(diff);

                if (dist < connect_dist && dist > 0.001) {
                    if (connCount < maxConn) {
                        vec2 dir = diff / dist;
                        float restLen = pairRestLen(id, j);
                        float displacement = dist - restLen;
                        float weight = nodeBaseSpeed(j) / max(dist, 0.05);
                        force += dir * displacement * attraction * weight * 0.5;
                        connCount++;
                    }
                }

                // Soft repulsion prevents overlap
                if (dist > 0.001 && dist < 0.06) {
                    force -= (diff / dist) * 0.03 / (dist * dist);
                }
            }

            // Soft walls keep nodes on screen
            float bound = 3.0;
            force.x -= smoothstep(bound * 0.8, bound, pos.x) * 3.0;
            force.x += smoothstep(-bound * 0.8, -bound, -pos.x) * 3.0;
            force.y -= smoothstep(bound * 0.8, bound, pos.y) * 3.0;
            force.y += smoothstep(-bound * 0.8, -bound, -pos.y) * 3.0;

            // Integrate
            vel += force * dt;
            vel *= exp(-2.5 * dt); // damping
            pos += vel * dt;

            fragColor = vec4(pos, vel);
        } else if (fc.y < 1.5 && id == MAX_NODES) {
            // === CAMERA PIXEL — find largest cluster, smooth track it ===
            vec2 np[MAX_NODES];
            for (int i = 0; i < MAX_NODES; i++) {
                np[i] = (i < numNodes) ? readState(i).xy : vec2(99.0);
            }

            // Union-Find
            int uf_p[MAX_NODES];
            int uf_s[MAX_NODES];
            for (int i = 0; i < MAX_NODES; i++) { uf_p[i] = i; uf_s[i] = 1; }

            for (int i = 0; i < MAX_NODES; i++) {
                if (i >= numNodes) break;
                for (int j = i + 1; j < MAX_NODES; j++) {
                    if (j >= numNodes) break;
                    if (length(np[i] - np[j]) < connect_dist) {
                        int ri = i;
                        for (int s = 0; s < 5; s++) { if (uf_p[ri] != ri) ri = uf_p[uf_p[ri]]; }
                        int rj = j;
                        for (int s = 0; s < 5; s++) { if (uf_p[rj] != rj) rj = uf_p[uf_p[rj]]; }
                        if (ri != rj) {
                            if (uf_s[ri] < uf_s[rj]) { uf_p[ri] = rj; uf_s[rj] += uf_s[ri]; }
                            else                      { uf_p[rj] = ri; uf_s[ri] += uf_s[rj]; }
                        }
                    }
                }
            }

            // Largest cluster root
            int bestRoot = 0; int bestSz = 0;
            for (int i = 0; i < MAX_NODES; i++) {
                if (i >= numNodes) break;
                int r = i;
                for (int s = 0; s < 5; s++) { if (uf_p[r] != r) r = uf_p[uf_p[r]]; }
                if (uf_s[r] > bestSz) { bestSz = uf_s[r]; bestRoot = r; }
            }

            // Cluster centroid + radius
            vec2 cc = vec2(0.0); int cn = 0; float cr = 0.0;
            for (int i = 0; i < MAX_NODES; i++) {
                if (i >= numNodes) break;
                int r = i;
                for (int s = 0; s < 5; s++) { if (uf_p[r] != r) r = uf_p[uf_p[r]]; }
                if (r == bestRoot) { cc += np[i]; cn++; }
            }
            cc /= max(float(cn), 1.0);
            for (int i = 0; i < MAX_NODES; i++) {
                if (i >= numNodes) break;
                int r = i;
                for (int s = 0; s < 5; s++) { if (uf_p[r] != r) r = uf_p[uf_p[r]]; }
                if (r == bestRoot) cr = max(cr, length(np[i] - cc));
            }
            cr = max(cr, 0.15) + 0.08;

            // Smooth toward target (exponential moving average)
            vec4 prevCam = readState(MAX_NODES);
            float dt = min(TIMEDELTA, 0.05);
            float rate = 1.0 - exp(-2.0 * dt);
            if (FRAMEINDEX == 0u || prevCam == vec4(0.0)) {
                fragColor = vec4(cc, cr, 0.0);
            } else {
                fragColor = vec4(mix(prevCam.xy, cc, rate), mix(prevCam.z, cr, rate), 0.0);
            }

        } else {
            fragColor = vec4(0.0);
        }

    } else {
        // === RENDER PASS ===
        // Read smoothed camera state from pixel 24
        vec4 cam = readState(MAX_NODES);
        vec2 camCenter = cam.xy;
        float camRadius = cam.z;

        // Zoom: plain zoom around origin. Tracking: blend toward largest cluster.
        vec2 center = camCenter * tracking;
        float scale = mix(0.5 / max(zoom, 0.1), camRadius, tracking);

        vec2 p = (uv - 0.5) * 2.0;
        p.x *= RENDERSIZE.x / RENDERSIZE.y;
        p *= scale;
        p = rot2(p, PHASE_TIME_1);
        p += center;

        // Load node positions
        vec2 positions[MAX_NODES];
        for (int i = 0; i < MAX_NODES; i++) {
            if (i >= numNodes) break;
            positions[i] = readState(i).xy;
        }

        vec3 col = bg_color.rgb;
        vec3 baseHSV = rgb2hsv(color_base.rgb);
        float baseHue = baseHSV.x;

        // --- Edges (respect max_connections per node) ---
        int maxConn = clamp(int(max_connections), 1, 20);
        int connCounts[MAX_NODES];
        for (int i = 0; i < MAX_NODES; i++) connCounts[i] = 0;

        // Visible radius: edge line vanishes beyond edge_width*1.5,
        // glow exp(-de*200)*0.06 < 0.001 when de > 0.03
        float edgeCutoff = max(edge_width * 1.5, 0.03);

        for (int i = 0; i < MAX_NODES; i++) {
            if (i >= numNodes) break;
            for (int j = i + 1; j < MAX_NODES; j++) {
                if (j >= numNodes) break;

                // AABB early-out: skip if pixel is far from both endpoints
                vec2 mn = min(positions[i], positions[j]) - edgeCutoff;
                vec2 mx = max(positions[i], positions[j]) + edgeCutoff;
                if (p.x < mn.x || p.x > mx.x || p.y < mn.y || p.y > mx.y) continue;

                float dist = length(positions[i] - positions[j]);
                if (dist > connect_dist) continue;
                if (connCounts[i] >= maxConn || connCounts[j] >= maxConn) continue;

                connCounts[i]++;
                connCounts[j]++;

                // Distance to line segment — skip color math if too far
                float de = dSegment(p, positions[i], positions[j]);
                if (de > edgeCutoff) continue;

                float edgeFade = 1.0 - dist / connect_dist;
                edgeFade *= edgeFade;

                float eh = hash(vec2(float(i), float(j)));
                float h = fract(baseHue + eh * hue_shift * 5.0);
                vec3 edgeCol = hsv2rgb(vec3(h, baseHSV.y, baseHSV.z));

                float edgeLine = smoothstep(edge_width * 1.5, edge_width * 0.3, de) * edgeFade * 0.7;
                float edgeGlow = exp(-de * 200.0) * edgeFade * 0.06;
                col += edgeCol * (edgeLine + edgeGlow);
            }
        }

        // --- Nodes ---
        // Glow becomes negligible beyond ~3x node_size
        float nodeCutoff = node_size * 4.0;
        float nodeCutoffSq = nodeCutoff * nodeCutoff;
        for (int i = 0; i < MAX_NODES; i++) {
            if (i >= numNodes) break;
            vec2 diff = p - positions[i];
            float ndSq = dot(diff, diff);
            if (ndSq > nodeCutoffSq) continue;
            float nd = sqrt(ndSq);
            float h = fract(baseHue + float(i) * hue_shift);
            vec3 nodeCol = hsv2rgb(vec3(h, baseHSV.y, baseHSV.z));
            float core = smoothstep(node_size * 0.5, node_size * 0.1, nd);
            float glow = exp(-ndSq / (node_size * node_size * 3.0)) * 0.3;
            col += nodeCol * (core + glow);
        }

        col = clamp(col + _keep, 0.0, 1.0);
        fragColor = vec4(col, 1.0);
    }
}