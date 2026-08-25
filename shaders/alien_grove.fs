/*{
    "DESCRIPTION": "Alien Grove - raymarched fractal forest at night: lacy umbel trees rising out of circular wells cut in the terrain, with smaller Menger crystal lattices and recursive fern-corals grown between them; RGB energy pulses run along circuit traces etched into the rock, which meander with it, ring the lip of every well, converge on webs centred under each trunk, climb the trunks and spars, and color their terminal auras beneath a cratered moon and log-periodic fractal halo",
    "CREDIT": "Varda VJ (tree placement after Inigo Quilez's SDF domain-repetition article, https://iquilezles.org/articles/sdfrepetition/; Menger sponge fold and ambient occlusion after Inigo Quilez; integer-free hashes after Dave Hoskins)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "fly_speed", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 3.0, "LABEL": "Fly Speed"},
        {"NAME": "sway_speed", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 2.0, "LABEL": "Camera Sway Speed"},
        {"NAME": "moon_cycle", "TYPE": "float", "DEFAULT": 0.25, "MIN": -2.0, "MAX": 2.0, "LABEL": "Moon Cycle Speed"},
        {"NAME": "canopy_drift", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 2.0, "LABEL": "Canopy / Energy Speed"},
        {"NAME": "cam_height", "TYPE": "float", "DEFAULT": 2.6, "MIN": 0.4, "MAX": 8.0, "LABEL": "Camera Height"},
        {"NAME": "look_x", "TYPE": "float", "DEFAULT": 0.0, "MIN": -1.0, "MAX": 1.0, "LABEL": "Look X"},
        {"NAME": "look_y", "TYPE": "float", "DEFAULT": 0.03, "MIN": -0.5, "MAX": 0.5, "LABEL": "Look Y"},
        {"NAME": "fov", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.4, "MAX": 1.6, "LABEL": "FOV"},
        {"NAME": "sway_amount", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 1.0, "LABEL": "Camera Sway Amount"},
        {"NAME": "forest_density", "TYPE": "float", "DEFAULT": 0.82, "MIN": 0.0, "MAX": 1.0, "LABEL": "Forest Density"},
        {"NAME": "tree_height", "TYPE": "float", "DEFAULT": 5.5, "MIN": 1.0, "MAX": 12.0, "LABEL": "Tree Height"},
        {"NAME": "tree_variation", "TYPE": "float", "DEFAULT": 0.75, "MIN": 0.0, "MAX": 1.0, "LABEL": "Tree Variation"},
        {"NAME": "trunk_warp", "TYPE": "float", "DEFAULT": 0.62, "MIN": 0.0, "MAX": 1.0, "LABEL": "Trunk Warp"},
        {"NAME": "well_radius", "TYPE": "float", "DEFAULT": 1.15, "MIN": 0.0, "MAX": 2.0, "LABEL": "Root Well Radius"},
        {"NAME": "well_depth", "TYPE": "float", "DEFAULT": 2.6, "MIN": 0.0, "MAX": 4.0, "LABEL": "Root Well Depth"},
        {"NAME": "canopy_spread", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.2, "MAX": 1.6, "LABEL": "Canopy Spread"},
        {"NAME": "canopy_arms", "TYPE": "float", "DEFAULT": 11.0, "MIN": 3.0, "MAX": 12.0, "LABEL": "Canopy Arms"},
        {"NAME": "fractal_depth", "TYPE": "float", "DEFAULT": 3.0, "MIN": 1.0, "MAX": 3.0, "LABEL": "Canopy Fractal Depth"},
        {"NAME": "species_density", "TYPE": "float", "DEFAULT": 0.22, "MIN": 0.0, "MAX": 1.0, "LABEL": "Interstitial Species Density"},
        {"NAME": "terrain_relief", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Terrain Relief"},
        {"NAME": "terrain_detail", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 1.0, "LABEL": "Terrain Detail"},
        {"NAME": "moon_size", "TYPE": "float", "DEFAULT": 0.16, "MIN": 0.02, "MAX": 0.5, "LABEL": "Moon Size"},
        {"NAME": "moon_intensity", "TYPE": "float", "DEFAULT": 1.3, "MIN": 0.0, "MAX": 4.0, "LABEL": "Moon Intensity"},
        {"NAME": "halo_rings", "TYPE": "float", "DEFAULT": 3.2, "MIN": 0.0, "MAX": 24.0, "LABEL": "Halo Rings"},
        {"NAME": "halo_rays", "TYPE": "float", "DEFAULT": 7.0, "MIN": 0.0, "MAX": 32.0, "LABEL": "Halo Rays"},
        {"NAME": "halo_depth", "TYPE": "float", "DEFAULT": 5.0, "MIN": 1.0, "MAX": 6.0, "LABEL": "Halo Fractal Depth"},
        {"NAME": "circuit_density", "TYPE": "float", "DEFAULT": 1.35, "MIN": 0.35, "MAX": 4.0, "LABEL": "Circuit Density"},
        {"NAME": "circuit_width", "TYPE": "float", "DEFAULT": 0.015, "MIN": 0.005, "MAX": 0.15, "LABEL": "Circuit Width"},
        {"NAME": "circuit_intensity", "TYPE": "float", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 5.0, "LABEL": "Circuit Intensity"},
        {"NAME": "circuit_etch", "TYPE": "float", "DEFAULT": 0.85, "MIN": 0.0, "MAX": 1.0, "LABEL": "Circuit Etch Depth"},
        {"NAME": "pulse_width", "TYPE": "float", "DEFAULT": 0.09, "MIN": 0.02, "MAX": 0.5, "LABEL": "Energy Pulse Width"},
        {"NAME": "trunk_circuits", "TYPE": "float", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 3.0, "LABEL": "Trunk Circuits"},
        {"NAME": "aura_intensity", "TYPE": "float", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 5.0, "LABEL": "Canopy Aura"},
        {"NAME": "ray_steps", "TYPE": "float", "DEFAULT": 64.0, "MIN": 40.0, "MAX": 200.0, "LABEL": "Ray Steps"},
        {"NAME": "sky_color", "TYPE": "color", "DEFAULT": [0.010, 0.012, 0.075, 1.0], "LABEL": "Sky Zenith"},
        {"NAME": "horizon_color", "TYPE": "color", "DEFAULT": [0.16, 0.06, 0.30, 1.0], "LABEL": "Sky Horizon"},
        {"NAME": "tree_color", "TYPE": "color", "DEFAULT": [0.055, 0.030, 0.11, 1.0], "LABEL": "Tree Color"},
        {"NAME": "terrain_color", "TYPE": "color", "DEFAULT": [0.10, 0.15, 0.34, 1.0], "LABEL": "Terrain Color"},
        {"NAME": "moon_color", "TYPE": "color", "DEFAULT": [0.78, 0.82, 1.0, 1.0], "LABEL": "Moon Color"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "fly_speed", "INDEX": 0, "SCALE": 1.0},
        {"PARAM": "sway_speed", "INDEX": 1, "SCALE": 1.0},
        {"PARAM": "moon_cycle", "INDEX": 2, "SCALE": 1.0},
        {"PARAM": "canopy_drift", "INDEX": 3, "SCALE": 1.0}
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
    float fly_speed;
    float sway_speed;
    float moon_cycle;
    float canopy_drift;
    float cam_height;
    float look_x;
    float look_y;
    float fov;
    float sway_amount;
    float forest_density;
    float tree_height;
    float tree_variation;
    float trunk_warp;
    float well_radius;
    float well_depth;
    float canopy_spread;
    float canopy_arms;
    float fractal_depth;
    float species_density;
    float terrain_relief;
    float terrain_detail;
    float moon_size;
    float moon_intensity;
    float halo_rings;
    float halo_rays;
    float halo_depth;
    float circuit_density;
    float circuit_width;
    float circuit_intensity;
    float circuit_etch;
    float pulse_width;
    float trunk_circuits;
    float aura_intensity;
    float ray_steps;
    vec4 sky_color;
    vec4 horizon_color;
    vec4 tree_color;
    vec4 terrain_color;
    vec4 moon_color;
};

// Cell pitch is a constant, not a parameter: the camera's z is an accumulated
// phase, so scaling world position by a live parameter would slide the whole
// forest past the camera whenever that fader moved. Thinning is done by
// rejecting cells on a hash instead, which leaves every surviving tree where
// it was. See docs/12-isf-authoring.md § Combining two rates.
const float GRID = 4.0;
const float MAX_DIST = 88.0;
const vec3 MOON_DIR = vec3(0.0, 0.09, 1.0);
// World size of the largest cell of the box fold carved into the landscape.
// Detail continues down by thirds from here, so this sets the scale of the
// biggest shelves and the trees are sized against it.
const float FOLD_CELL = 1.15;
// Applied between fold iterations so each level's grid sits at an angle to the
// one above. Without it every level shares one axis-aligned lattice and the
// landscape reads as a tiled floor rather than as eroded rock. A rotation is an
// isometry, so it costs the distance bound nothing.
const mat2 FOLD_ROT = mat2(0.936, 0.352, -0.352, 0.936);
// How much wider than tall a fold cell is. Isotropic cells carve the landscape
// into a field of uniform cubes, which reads as masonry; flattening them lays
// the carve down as sedimentary banding instead.
const float LAYER_ASPECT = 2.2;
// Clearance a trunk's root is continued past the floor of its well, in world
// units, so the trunk always passes through the floor rather than ending above
// it. Held in world units rather than tree-local ones because the wells are cut
// to one depth for the whole grove: a local-space drop would leave the smallest
// trees hanging over their own openings.
const float ROOT_CLEARANCE = 1.2;

// Camera ground position, for the distance-based detail fade.
vec2 g_camXZ = vec2(0.0);

// Integer-free hashes: cheap enough to call from inside the march.
float hash21(vec2 p) {
    p = fract(p * vec2(233.34, 851.73));
    p += dot(p, p + 23.45);
    return fract(p.x * p.y);
}

float vnoise(vec2 p) {
    vec2 i = floor(p);
    vec2 f = fract(p);
    f = f * f * (3.0 - 2.0 * f);
    float a = hash21(i);
    float b = hash21(i + vec2(1.0, 0.0));
    float c = hash21(i + vec2(0.0, 1.0));
    float d = hash21(i + vec2(1.0, 1.0));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

mat2 rot2(float a) {
    float c = cos(a);
    float s = sin(a);
    return mat2(c, s, -s, c);
}

// Rolling base for the landscape. Three cheap octaves only: the fractal
// structure comes from the box fold below, not from stacking noise, which is
// the whole reason this is affordable.
float hillsAt(vec2 xz) {
    return vnoise(xz * 0.042) * 0.68 + vnoise(xz * 0.115) * 0.32;
}

// Quantise the height into strata. The flat shelves are what the box fold then
// carves into, and they are load-bearing for the look: fold a smooth
// heightfield and you get dented dunes, fold a terraced one and you get the
// stacked sedimentary ledges the reference is built from. A little of the
// unquantised height is left in so the strata tilt instead of reading as a
// staircase of perfectly level plates.
// A hard `floor` gives strata with vertical risers, and a heightfield with
// vertical risers has an unbounded gradient, which forces the march to crawl to
// avoid tearing through them. Ramping each riser over a fraction of a step
// keeps the slope finite, which is what lets the step scale below run at 0.7
// instead of 0.5.
float terrace(float h, float steps, float riser) {
    float f = h * steps;
    float i = floor(f);
    return (i + smoothstep(0.5 - riser, 0.5 + riser, f - i)) / steps;
}

float terracedH(vec2 xz) {
    float h = hillsAt(xz);
    return mix(h, terrace(h, 9.0, 0.25), 0.85) * terrain_relief * 4.6;
}

// Ground height for seating a tree. Deliberately not `terracedH`: this runs
// once per candidate cell per march step, four times over, and profiling put
// the full height function at the top of the frame. One octave, quantised onto
// the same strata, gets the tree onto the right shelf; the dropped octaves are
// re-centred so the error is symmetric, and callers sink the trunk by more than
// that error so a tree can never be left hovering.
float footingAt(vec2 xz) {
    float h = vnoise(xz * 0.042) * 0.68 + 0.16;
    return mix(h, floor(h * 9.0) / 9.0, 0.8) * terrain_relief * 4.6;
}

// Reconstruct the nearest live tree from the same cell hash used by sceneMap.
// This is called only after a surface hit, so the circuit shading can attach to
// the generated geometry without carrying IDs through every march step.
bool nearestTreeBase(
    vec2 xz,
    out vec2 base,
    out float seed,
    out float scale
) {
    vec2 gp = xz / GRID;
    vec2 id = round(gp);
    vec2 quad = sign(gp - id);
    float best = 1e9;
    bool found = false;

    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) {
            vec2 cid = id + vec2(float(i), float(j)) * quad;
            float s = hash21(cid);
            if (s > forest_density) continue;

            vec2 jit = vec2(fract(s * 57.311), fract(s * 113.773));
            vec2 b = cid * GRID + (jit - 0.5) * (GRID * 0.35);
            float d2 = dot(xz - b, xz - b);
            if (d2 >= best) continue;

            float pick = fract(s * 331.703);
            best = d2;
            found = true;
            base = b;
            seed = s;
            scale = mix(1.0, mix(0.5, 1.45, pick), clamp(tree_variation, 0.0, 1.0));
        }
    }
    return found;
}

// Full reconstruction is needed only when the visible material is vegetation.
// Ground pixels stop at nearestTreeBase, avoiding a noise lookup per pixel.
bool nearestTreeData(
    vec2 xz,
    out vec2 base,
    out float seed,
    out float scale,
    out float footing,
    out vec2 lean
) {
    bool found = nearestTreeBase(xz, base, seed, scale);
    if (!found) return false;
    float v1 = fract(seed * 71.317);
    float v2 = fract(seed * 191.733);
    footing = footingAt(base) - 0.9 * scale;
    lean = (vec2(fract(v1 * 31.7), fract(v2 * 17.3)) - 0.5)
         * 0.2 * clamp(tree_variation, 0.0, 1.0);
    return true;
}

// Where a trunk actually meets the ground.
//
// The sway shears the whole trunk sideways by `warp * amount`, and the tree is
// seated with its origin sunk below the terrain, so the point where it emerges
// is displaced from the cell anchor it was generated at by up to a metre and a
// half, and that displacement moves as the drift phase advances. Centring the
// circuit web on the anchor therefore leaves it sitting beside the trunk it is
// supposed to feed. This reproduces the shear at the ground line so the web can
// be centred on the trunk instead.
//
// `sceneMap` computes `sp.xz = tp.xz - (warp * amount + tp.y * lean)`, so the
// trunk is wherever that vanishes: `tp.xz = warp * amount + tp.y * lean`, with
// `tp.y` equal to the sink depth at the seated ground height by construction.
vec2 trunkGroundXZ(vec2 base, float seed, float scale, bool primary, float drift) {
    float phase = seed * 6.2831853;
    float sink = primary ? 0.9 : 0.55;
    vec2 freq = primary ? vec2(0.55, 0.42) : vec2(0.62, 0.47);
    float amount = trunk_warp * (primary ? 0.9 : 0.35);
    vec2 warp = vec2(
        sin(sink * freq.x + phase + drift),
        cos(sink * freq.y + phase * 1.7 + drift)
    );
    vec2 lean;
    if (primary) {
        float v1 = fract(seed * 71.317);
        float v2 = fract(seed * 191.733);
        lean = (vec2(fract(v1 * 31.7), fract(v2 * 17.3)) - 0.5)
             * 0.2 * clamp(tree_variation, 0.0, 1.0);
    } else {
        lean = (vec2(fract(seed * 37.7), fract(seed * 91.3)) - 0.5)
             * 0.08 * clamp(tree_variation, 0.0, 1.0);
    }
    return base + scale * (warp * amount + sink * lean);
}

// The nearest trunk across both grids, returned as its ground-level position
// rather than as the cell anchor it grew from. Comparing corrected positions
// matters: the shear is large enough that the nearest anchor and the nearest
// trunk are different trees in a band around every cell boundary.
bool nearestTrunk(
    vec2 xz,
    float drift,
    out vec2 center,
    out float seed,
    out float scale
) {
    float best = 1e9;
    bool found = false;

    vec2 gp = xz / GRID;
    vec2 pid = round(gp);
    vec2 pquad = sign(gp - pid);
    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) {
            vec2 cid = pid + vec2(float(i), float(j)) * pquad;
            float s = hash21(cid);
            if (s > forest_density) continue;
            vec2 jit = vec2(fract(s * 57.311), fract(s * 113.773));
            vec2 b = cid * GRID + (jit - 0.5) * (GRID * 0.35);
            float sc = mix(1.0, mix(0.5, 1.45, fract(s * 331.703)),
                           clamp(tree_variation, 0.0, 1.0));
            vec2 c = trunkGroundXZ(b, s, sc, true, drift);
            float d2 = dot(xz - c, xz - c);
            if (d2 >= best) continue;
            best = d2;
            found = true;
            center = c;
            seed = s;
            scale = sc;
        }
    }

    vec2 gq = xz / GRID - 0.5;
    vec2 iid = round(gq);
    vec2 iquad = sign(gq - iid);
    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) {
            vec2 cid = iid + vec2(float(i), float(j)) * iquad;
            float s = hash21(cid + vec2(41.7, 17.3));
            if (s > species_density) continue;
            vec2 jit = vec2(fract(s * 83.17), fract(s * 149.31));
            vec2 b = (cid + 0.5) * GRID + (jit - 0.5) * (GRID * 0.16);
            float sc = mix(0.52, 0.82, fract(s * 271.91));
            vec2 c = trunkGroundXZ(b, s, sc, false, drift);
            float d2 = dot(xz - c, xz - c);
            if (d2 >= best) continue;
            best = d2;
            found = true;
            center = c;
            seed = s;
            scale = sc;
        }
    }
    return found;
}

float traceMask(float distanceToTrace, float width) {
    return 1.0 - smoothstep(width, width * 2.5, distanceToTrace);
}

float pulseTrain(float phase) {
    float d = abs(fract(phase) - 0.5);
    return 1.0 - smoothstep(pulse_width * 0.25, pulse_width, d);
}

// A branchless HSV rainbow. The usual cosine palette costs three transcendental
// operations every time this is called; this triangular form is nearly free and
// gives the harder electronic color separations the traces need.
vec3 energyRGB(float phase) {
    vec3 wave = abs(mod(phase * 6.0 + vec3(0.0, 4.0, 2.0), 6.0) - 3.0) - 1.0;
    return clamp(wave, 0.0, 1.0);
}

// The circuit lattice at a point: rectilinear board traces plus a radial web
// converging on the nearest trunk. Returns the distance to the nearest trace,
// the packet phase travelling along it, and which conductor it is: 0 for the
// open board, 1 for the radial web, 2 for the ring around a root well's lip.
//
// This hands back the raw distance instead of a coverage mask because the
// shading cuts the trace into the surface rather than painting it on: recovering
// the channel's cross-slope needs the field itself, and a mask has already
// thrown that away. Nearest-trace-wins also means a point is on exactly one
// conductor, so a groove is well defined where board and web overlap.
//
// `warp` displaces the board lattice by the same low-frequency field that bends
// the terrain fold, so traces meander with the rock instead of running along the
// world axes across the top of it. The web is left unwarped: it is anchored to
// its trunk and has to stay concentric with it.
//
// Runs once for a visible ground pixel, not at every raymarch sample.
vec3 circuitField(
    vec2 xz,
    vec2 warp,
    vec2 base,
    float seed,
    float flow,
    bool hasTree
) {
    vec2 q = (xz + warp) * circuit_density;
    vec2 id = floor(q);
    vec2 f = fract(q) - 0.5;
    float h = hash21(id);
    float bend = mix(-0.32, 0.32, fract(h * 91.73));

    // Every tile contains an L trace. Hash-controlled reflections keep the
    // board orthogonal without repeating the same route.
    if (h > 0.5) f = f.yx;
    f *= vec2((fract(h * 37.1) > 0.5) ? -1.0 : 1.0, 1.0);
    // Chebyshev distance is exact on the straight portions and square at the
    // elbow, which suits circuit traces and avoids two square roots per pixel.
    float horizontalD = max(abs(f.y - bend), max(-0.55 - f.x, f.x - bend));
    float verticalD = max(abs(f.x - bend), max(bend - f.y, f.y - 0.55));
    float boardD = min(horizontalD, verticalD) / max(circuit_density, 0.01);
    float boardPhase = flow + dot(id, vec2(0.173, 0.317)) + length(f) * 0.3;

    vec2 rel = xz - base;
    float r = length(rel);
    float webD = 1e9;
    float webKind = 1.0;
    // Increasing flow moves constant-phase packets toward r=0.
    float inwardPhase = flow + r * 0.42 + seed;
    if (hasTree && r < GRID * 0.55) {
        float a = atan(rel.y, rel.x);
        float spokes = 5.0 + floor(seed * 4.0);
        float spokeD = abs(sin(a * spokes + seed * 6.2831853)) * r / spokes;
        float ringD = abs(fract(r * 2.3 + seed) - 0.5) / 2.3;
        // Widen the web out to nothing at its rim so it hands the packet over
        // to the board rather than stopping at a hard circle.
        webD = min(spokeD, ringD)
             + smoothstep(GRID * 0.24, GRID * 0.42, r) * circuit_width * 4.0;
        // A trace around the lip of the root well. The fold chews the rim of the
        // opening into the same plates as the rest of the terrain, so without
        // this the hole reads as a ragged dark patch; the ring is what makes it
        // read as a circular opening, and it ties the web to the thing it feeds.
        // It is tagged apart because a packet's phase is constant all the way
        // round a circle of fixed radius, so the whole ring would blink together
        // and spend most of its time invisible.
        float lipD = abs(r - max(well_radius, 0.0));
        if (lipD < webD) {
            webD = lipD;
            webKind = 2.0;
        }
    }

    if (webD < boardD) return vec3(webD, inwardPhase, webKind);
    return vec3(boardD, boardPhase, 0.0);
}

// How many fold iterations are worth paying for here. Beyond a few tens of
// units the cells are sub-pixel, where they cost the same and only alias.
int foldIters(vec2 xz) {
    float lod = clamp(1.0 - length(xz - g_camXZ) / 52.0, 0.0, 1.0);
    return int(1.5 + clamp(terrain_detail, 0.0, 1.0) * 3.5 * lod);
}

// Menger box fold: intersect the solid with the complement of three
// interlocking square tubes, then repeat at a third of the scale. Each
// iteration multiplies the surface detail by three in every axis, which is
// where the hard-edged self-similar shelves, notches and pits come from.
//
// It is also a true distance bound rather than a heightfield approximation, so
// the march can take full-length steps through it.
float mengerCarve(vec3 p, float d, int iters) {
    float s = 1.0;
    for (int i = 0; i < 5; i++) {
        if (i >= iters) break;
        vec3 a = mod(p * s, 2.0) - 1.0;
        s *= 3.0;
        vec3 r = abs(1.0 - 3.0 * abs(a));
        // 0.82 rather than the canonical 1.0 thins the carving tubes, so each
        // level takes many small bites instead of a few deep ones. Full-width
        // tubes excavate the landscape into scattered pits; thin ones leave a
        // continuous surface that is rough everywhere, which is what reads as
        // eroded rock rather than as rubble.
        float c = min(min(max(r.x, r.y), max(r.y, r.z)), max(r.z, r.x));
        d = max(d, (c - 0.82) / s);
        p.xz = FOLD_ROT * p.xz;
    }
    return d;
}

// The landscape: terraced hills with the box fold cut into them. The height is
// passed in because the caller needs it too, to place the root wells relative to
// the real surface.
float terrainSDF(vec3 p, float h) {
    float k = 2.0 / FOLD_CELL;
    float base = (p.y - h) * 0.7;
    // The carve can only displace the surface by about half a cell, so outside
    // that band the smooth bound is already correct. Carving only ever removes
    // material, so returning the uncarved value early is conservative — and it
    // means open air and deep rock cost one noise pair instead of a fold.
    if (abs(base) > FOLD_CELL) return base;
    // Warp the fold's input so the lattice meanders with the land. Rotating
    // between levels breaks their alignment with each other; this is what
    // breaks the whole structure's alignment with the world axes, so the
    // shelves and pits stop marching in straight rows to the horizon.
    vec3 fp = p;
    fp.xz += vec2(vnoise(p.xz * 0.031), vnoise(p.zx * 0.031 + 5.1)) * 2.6;
    // Scaling *down* in xz keeps the largest scale factor at one, so dividing
    // the result by `k` alone is still a valid bound. Stretching y to the same
    // aspect would inflate the estimate threefold and cost that in step length.
    fp.xz /= LAYER_ASPECT;
    return mengerCarve(fp * k, base * k, foldIters(p.xz)) / k;
}

// Vertical cylinder between two heights, from a radial distance already in hand.
float sdCylinder(float radial, float y, float low, float high) {
    float c = (high + low) * 0.5;
    vec2 q = vec2(radial, abs(y - c) - (high - low) * 0.5);
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0));
}

float sdCapsule(vec3 p, vec3 a, vec3 b, float r) {
    vec3 pa = p - a;
    vec3 ba = b - a;
    float h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-6), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

// Tapered vertical column between y=0 and y=h. Approximate but cheap and
// stable, which matters at four evaluations per march step.
float sdTaper(vec3 p, float h, float r0, float r1) {
    float f = clamp(p.y / max(h, 1e-3), 0.0, 1.0);
    float r = mix(r0, r1, f);
    vec2 q = vec2(length(p.xz) - r, max(p.y - h, -p.y));
    return min(max(q.x, q.y), 0.0) + length(max(q, 0.0));
}

// Fold a plane into one wedge of an n-fold rotation, so a single arm becomes a
// whole ring of them.
vec2 foldRadial(vec2 v, float n) {
    float seg = 6.2831853 / n;
    float a = atan(v.y, v.x);
    a = mod(a + seg * 0.5, seg) - seg * 0.5;
    return vec2(cos(a), sin(a)) * length(v);
}

// Umbrella crown: a squashed hub plus a ring of arms, then the same ring
// re-folded at the arm tips at a smaller scale. Each level divides the
// distance by the accumulated scale, which is what keeps the folded copies a
// valid distance field rather than a self-intersecting mess.
// The twist matrix is the same at every level, so it is built once per tree by
// the caller rather than once per level here — three `sin`/`cos` pairs per cell
// per march step is not a rounding error at this call depth.
float crownSDF(vec3 q, float spread, float arms, float rise, int depth, mat2 twistM) {
    // A flared swelling rather than a ball — a hub wide enough to see is the
    // single thing that most made this read as ball-and-stick. Divided by the
    // squash factor: anisotropic scaling inflates a distance estimate, and an
    // inflated estimate is what the global 0.55 fudge downstream was paying for.
    float d = (length(q * vec3(1.0, 3.4, 1.0)) - 0.05 * spread) / 3.4;

    // A shallow membrane webbing the ribs, perforated by the same box fold as
    // the landscape. Ribs alone read as an antenna however finely they branch,
    // but an unbroken plate reads as a flying saucer; a lacy one reads as
    // foliage. The offset is per-tree so the lace is not the same on all of
    // them. Built before the fold, so it is one continuous bowl.
    float bowl = max(length(q.xz) - spread * 0.74,
                     abs(q.y + length(q.xz) * 0.16 - spread * rise * 0.5) - 0.012);
    const float bk = 7.0;
    d = min(d, mengerCarve(q * bk + vec3(rise * 9.1, spread * 5.3, rise * 3.7), bowl * bk, 2) / bk);

    float sc = 1.0;
    for (int i = 0; i < 3; i++) {
        if (i >= depth) break;
        if (i == 0) {
            // Only the outer ring's count is worth a fader, and only it pays
            // for the `atan`; the sub-branches use a reflection fold instead,
            // which is a handful of arithmetic and reads no differently.
            q.xz = foldRadial(q.xz, arms);
        } else {
            q.xz = abs(q.xz);
            if (q.z > q.x) q.xz = q.zx;
        }
        // The first ring lifts to form the umbrella; the ones grafted onto its
        // tips droop, which is what stops the crown reading as a ball-and-stick
        // model and gives it the weight of foliage.
        float lift = mix(rise, -0.3, min(float(i), 1.0));
        vec3 tip = vec3(spread, spread * lift, 0.0);
        // Thin ribs and small nodes. The reference's crowns are umbels: the
        // structure is legible because the members are fine, not because there
        // are many of them.
        d = min(d, sdCapsule(q, vec3(0.0), tip, 0.021) / sc);
        d = min(d, (length(q - tip) - 0.036) / sc);
        q = (q - tip) * 2.9;
        q.xz = twistM * q.xz;
        sc *= 2.9;
    }
    return d;
}

float sdBox(vec3 p, vec3 b) {
    vec3 q = abs(p) - b;
    return min(max(q.x, max(q.y, q.z)), 0.0) + length(max(q, 0.0));
}

// A faceted central shard with recursively smaller radial spars. Carving the
// union with two Menger iterations turns solid members into a crystalline
// lattice while remaining cheaper than the umbel's three-level crown.
float crystalLatticeSDF(vec3 p, float h, float spread, float seed, int depth) {
    // Three nested shards taper the central silhouette without requiring an
    // oriented-box SDF.
    float d = sdBox(p - vec3(0.0, h * 0.22, 0.0), vec3(0.12, h * 0.22, 0.12));
    d = min(d, sdBox(p - vec3(0.0, h * 0.56, 0.0), vec3(0.085, h * 0.18, 0.085)));
    d = min(d, sdBox(p - vec3(0.0, h * 0.82, 0.0), vec3(0.05, h * 0.14, 0.05)));
    float arms = 4.0 + floor(fract(seed * 73.17) * 3.0);
    vec3 q = p;
    q.xz = foldRadial(q.xz, arms);
    for (int i = 0; i < 3; i++) {
        if (i >= depth) break;
        float fi = float(i);
        float y = h * (0.24 + fi * 0.22);
        float reach = spread * (1.12 - fi * 0.13);
        vec3 a = vec3(0.0, y, 0.0);
        vec3 b = vec3(reach, y + reach * (0.42 - fi * 0.08), 0.0);
        d = min(d, sdCapsule(q, a, b, 0.045 - fi * 0.008));
        d = min(d, sdBox(q - b, vec3(0.065 - fi * 0.01)));
    }
    // A terminal star makes the energy destination legible from a distance.
    vec3 top = p - vec3(0.0, h, 0.0);
    top.xz = foldRadial(top.xz, arms + 1.0);
    d = min(d, sdCapsule(top, vec3(0.0), vec3(spread * 0.42, spread * 0.16, 0.0), 0.035));

    if (d < 0.28) {
        const float ck = 4.3;
        d = mengerCarve(p * ck + seed * 4.0, d * ck, 2) / ck;
    }
    return d;
}

// A stack of recursive frond fans. Each tier uses one radial fold for its main
// fronds and a reflection fold at their tips for a second generation, producing
// a fern/coral silhouette rather than another top-heavy tree.
float fernCoralSDF(vec3 p, float h, float spread, float seed, int depth) {
    float d = sdTaper(p, h * 0.92, 0.10, 0.035);
    float arms = 5.0 + floor(fract(seed * 121.7) * 4.0);
    for (int i = 0; i < 4; i++) {
        if (i > depth) break;
        float fi = float(i);
        float y = h * (0.18 + fi * 0.19);
        float reach = spread * (1.0 - fi * 0.13);
        vec3 q = p - vec3(0.0, y, 0.0);
        q.xz = rot2(seed * 5.0 + fi * 0.72) * q.xz;
        q.xz = foldRadial(q.xz, arms);
        vec3 tip = vec3(reach, reach * (0.12 + fi * 0.035), 0.0);
        d = min(d, sdCapsule(q, vec3(0.0), tip, 0.026));

        vec3 sub = (q - tip) * 3.1;
        sub.xz = abs(sub.xz);
        if (sub.z > sub.x) sub.xz = sub.zx;
        d = min(d, sdCapsule(sub, vec3(0.0), vec3(reach * 0.55, -reach * 0.14, 0.0), 0.045) / 3.1);
        d = min(d, (length(sub - vec3(reach * 0.55, -reach * 0.14, 0.0)) - 0.055) / 3.1);
    }
    return d;
}

// Interstitial cells contain only the new species. Primary cells always remain
// umbels, preserving the original grove's positions and silhouettes.
int interstitialSpecies(float seed) {
    return (fract(seed * 193.71) < 0.48) ? 1 : 2;
}

float interstitialHeight(int species) {
    return tree_height * ((species == 1) ? 0.58 : 0.48);
}

bool interstitialTreeData(
    vec2 xz,
    out vec2 base,
    out float seed,
    out float scale,
    out float footing,
    out vec2 lean
) {
    vec2 cid = round(xz / GRID - 0.5);
    seed = hash21(cid + vec2(41.7, 17.3));
    if (seed > species_density) return false;
    vec2 jit = vec2(fract(seed * 83.17), fract(seed * 149.31));
    base = (cid + 0.5) * GRID + (jit - 0.5) * (GRID * 0.16);
    scale = mix(0.52, 0.82, fract(seed * 271.91));
    footing = footingAt(base) - 0.55 * scale;
    lean = (vec2(fract(seed * 37.7), fract(seed * 91.3)) - 0.5)
         * 0.08 * clamp(tree_variation, 0.0, 1.0);
    return true;
}

vec2 interstitialMap(vec3 p, float drift, float maxDistance) {
    vec2 cid = round(p.xz / GRID - 0.5);
    vec2 center = (cid + 0.5) * GRID;
    vec2 cameraDelta = center - g_camXZ;
    if (dot(cameraDelta, cameraDelta) > 48.0 * 48.0) return vec2(1e5, 0.0);
    float limit = 1.55 + max(maxDistance, 0.0);
    vec2 centerDelta = p.xz - center;
    if (dot(centerDelta, centerDelta) > limit * limit) return vec2(1e5, 0.0);

    float seed = hash21(cid + vec2(41.7, 17.3));
    if (seed > species_density) return vec2(1e5, 0.0);
    vec2 jit = vec2(fract(seed * 83.17), fract(seed * 149.31));
    vec2 base = center + (jit - 0.5) * (GRID * 0.16);
    vec3 q = p - vec3(base.x, 0.0, base.y);
    float scale = mix(0.52, 0.82, fract(seed * 271.91));

    float horizontal = length(q.xz) - 1.35 * scale;
    if (horizontal > maxDistance) return vec2(1e5, 0.0);
    float footing = footingAt(base) - 0.55 * scale;
    int species = interstitialSpecies(seed);
    float h = interstitialHeight(species);
    float vertical = max(footing - q.y, q.y - (footing + h * scale));
    if (max(horizontal, vertical) > maxDistance) return vec2(1e5, 0.0);

    vec3 tp = (q - vec3(0.0, footing, 0.0)) / scale;
    float phase = seed * 6.2831853;
    vec2 warp = vec2(
        sin(tp.y * 0.62 + phase + drift),
        cos(tp.y * 0.47 + phase * 1.7 + drift)
    );
    vec2 lean = (vec2(fract(seed * 37.7), fract(seed * 91.3)) - 0.5)
              * 0.08 * clamp(tree_variation, 0.0, 1.0);
    tp.xz -= warp * (trunk_warp * 0.35) + tp.y * lean;

    float spread = canopy_spread * mix(0.62, 0.88, fract(seed * 59.13));
    int depth = int(clamp(fractal_depth, 1.0, 3.0));
    float d;
    if (species == 1) {
        float bound = max(length(tp.xz) - spread * 1.25, max(-tp.y, tp.y - h));
        d = bound;
        if (bound < 0.28) d = crystalLatticeSDF(tp, h, spread, seed, depth);
        d *= 0.78;
    } else {
        float bound = max(length(tp.xz) - spread * 1.18, max(-tp.y, tp.y - h));
        d = bound;
        if (bound < 0.24) d = fernCoralSDF(tp, h, spread, seed, depth);
        d *= 0.82;
    }
    return vec2(d * scale, float(species + 1));
}

// Scene distance plus a material tag: 0 = ground, 1 = umbel, 2 = crystal,
// 3 = fern/coral.
vec2 sceneMap(vec3 p, float drift) {
    float surfaceH = terracedH(p.xz);
    float ground = terrainSDF(p, surfaceH);
    // Growth is tracked apart from the ground because the root wells reopen it.
    // Carving can only push the ground's surface further away, so the uncarved
    // value is a conservative bound for the cell culls below and every one of
    // them stays valid; only the final comparison needs the carved value.
    vec2 growth = vec2(1e5, 0.0);
    float bound = ground;
    // Distance to the nearest trunk axis, measured only radially. The wells are
    // closed off below by the terrain's own height field rather than by a plane
    // per tree, so the floor is only accumulated as a radius here.
    float shaftR = 1e5;
    float wellR = max(well_radius, 0.0);
    float wellDepth = max(well_depth, 0.0);
    float rootDrop = wellDepth + ROOT_CLEARANCE;

    float var = clamp(tree_variation, 0.0, 1.0);
    int depth = int(clamp(fractal_depth, 1.0, 3.0));
    // Horizontal reach of the widest tree a cell can hold, term by term: the
    // outer ring plus its grafted sub-rings, the trunk shear, and the lean.
    // Measured rather than guessed generously — this bound gates every cell
    // test, so slack in it is paid for on every march step.
    float reach = canopy_spread * 1.95 + trunk_warp * 0.9 + tree_height * 0.1 + 0.15;
    float scaleMax = 1.0 + 0.45 * var;

    // Nothing grows above the canopy or below the deepest root, so one compare
    // skips the whole four-cell test for samples outside that band. The floor
    // has to clear the extended trunks and their wells, not just the footings.
    float bandTop = terrain_relief * 4.6 + (tree_height + reach) * scaleMax;
    if (p.y > bandTop || p.y < -1.2 * scaleMax - rootDrop) {
        return vec2(ground, 0.0);
    }

    vec2 gp = p.xz / GRID;
    vec2 id = round(gp);
    vec2 quad = sign(gp - id);

    for (int j = 0; j < 2; j++) {
        for (int i = 0; i < 2; i++) {
            vec2 cid = id + vec2(float(i), float(j)) * quad;

            // Cheapest reject first: squared distance to the cell's centre
            // against the widest tree it could hold. No hash, no noise, and no
            // square root — this test runs on four cells at every march step,
            // so it is the single hottest line in the shader.
            vec2 dc = p.xz - cid * GRID;
            float lim = GRID * 0.175 + reach * scaleMax + max(bound, 0.0);
            if (dot(dc, dc) > lim * lim) continue;

            float seed = hash21(cid);
            if (seed > forest_density) continue;

            // One hash per cell, and every per-tree variate multiply-and-fracted
            // off it. At four cells per march step, a second and third hash
            // would cost more than all the geometry they feed.
            float pick = fract(seed * 331.703);
            // Jitter stays well inside the cell: the crowns are already wider
            // than the pitch, and only the four nearest cells are tested.
            vec2 jit = vec2(fract(seed * 57.311), fract(seed * 113.773));
            vec2 base = cid * GRID + (jit - 0.5) * (GRID * 0.35);
            vec3 q = p - vec3(base.x, 0.0, base.y);
            float scale = mix(1.0, mix(0.5, 1.45, pick), var);

            if (length(q.xz) - reach * scale > bound) continue;

            // Sunk by more than `footingAt`'s error, so approximating the
            // ground height can bury a trunk but never leave one hovering.
            float footing = footingAt(base) - 0.9 * scale;
            float top = footing + tree_height * scale;

            // Two tight bounds rather than one box around both. A tree is a thin
            // tall column with a small sphere on top, and a box holding both is
            // almost entirely empty, so a box bound admits samples that then pay
            // for the shear, the taper and the fold to no purpose.
            // The trunk now continues below its footing and the well is cut
            // wider than the shear, so this bound has to hold both or a culled
            // cell would drop its opening out of the ground.
            float leanMax = 0.1 * tree_height * var;
            float trunkB = max(length(q.xz)
                                 - (trunk_warp * 1.3 + leanMax) * scale
                                 - max(0.2 * scale, wellR),
                               max(footing - rootDrop - q.y, q.y - top));
            float crownB = length(q - vec3(0.0, top, 0.0))
                         - (canopy_spread * 2.15 + 0.15 + leanMax) * scale;
            if (min(trunkB, crownB) > bound) continue;

            // Everything below is per-tree shape, and only reached by a cell
            // that is actually close enough to matter. Every tree needs its own
            // or the grove reads as an orchard; the variates are decorrelated by
            // multiply-and-fract off the two hashes already in hand.
            float v1 = fract(seed * 71.317);
            float v2 = fract(seed * 191.733);
            float v3 = fract(pick * 113.109);
            float v4 = fract(pick * 47.541);
            float spread = canopy_spread * mix(1.0, mix(0.65, 1.25, v1), var);
            float arms = clamp(floor(canopy_arms + (v2 - 0.5) * 6.0 * var + 0.5), 3.0, 14.0);
            float rise = mix(0.36, mix(0.16, 0.55, v3), var);
            // A few trees branch one level further or one level less than the
            // rest, which breaks up the silhouette more than any dimension does.
            int treeDepth = clamp(depth + int(step(0.84, v4)) - int(step(v4, 0.12 * var)), 1, 3);
            vec2 lean = (vec2(fract(v1 * 31.7), fract(v2 * 17.3)) - 0.5) * 0.2 * var;

            vec3 tp = (q - vec3(0.0, footing, 0.0)) / scale;
            float phase = seed * 6.2831853;
            // Shearing xz by height is what gives the trunks their slow S-bend;
            // the lean term is a second, constant shear, so the crown built in
            // this space ends up correctly over the leaning trunk.
            vec2 warp = vec2(sin(tp.y * 0.55 + phase + drift), cos(tp.y * 0.42 + phase * 1.7 + drift));
            vec3 sp = tp;
            sp.xz -= warp * (trunk_warp * 0.9) + tp.y * lean;

            // Radius to the trunk axis, shared by the well and the root below.
            // Cheap here and nowhere else: `sp` is already the sheared
            // trunk-local position, so both stay centred on the trunk without
            // recomputing the sway.
            //
            // The well's radius is a world measure, not a tree-local one, so
            // every opening is the same size whatever the tree. That is what
            // lets the circuit web outline the lip: the web is laid out in world
            // space and would otherwise only line up for one size of tree.
            float axisR = length(sp.xz);
            if (wellR > 0.0) shaftR = min(shaftR, axisR * scale - wellR);

            // Primary cells remain the original umbel species.
            float girth = mix(0.14, mix(0.095, 0.185, v2), var);
            float trunk = sdTaper(sp, tree_height, girth, girth * 0.3);
            if (trunk < 0.4) {
                const float tk = 3.4;
                trunk = mengerCarve(sp * tk + 0.5, trunk * tk, 2) / tk;
            }
            // A plain root continuing the trunk down past the well's floor, so
            // the tree is seen rising out of the opening rather than resting on
            // its floor. Left uncarved: it is only ever seen in shadow.
            if (wellR > 0.0) {
                trunk = min(trunk, sdCylinder(axisR - girth * 1.05, sp.y,
                                              -rootDrop / scale, 0.5));
            }

            vec3 cq = sp - vec3(0.0, tree_height, 0.0);
            float crownBound = length(cq) - (spread * 1.7 + 0.12);
            float crown = crownBound;
            if (crownBound < 0.3) {
                mat2 twistM = rot2(0.55 + phase * 0.3 + sin(drift + phase) * 0.35);
                crown = crownSDF(cq, spread, arms, rise, treeDepth, twistM);
            }
            float tree = min(trunk, crown) * 0.85 * scale;
            if (tree < growth.x) {
                growth = vec2(tree, 1.0);
                bound = min(bound, tree);
            }
        }
    }
    vec2 secondary = interstitialMap(p, drift, bound);
    if (secondary.x < growth.x) growth = secondary;

    // Close the wells off with the terrain's own height field rather than with a
    // plane under each tree. `footingAt`, which seats the trees, carries only
    // the first noise octave where `terracedH` carries two, so a floor placed
    // relative to a footing lands above the real surface about as often as below
    // it — and a well whose floor is above the ground removes nothing at all.
    // Referencing the same height the terrain is built from makes every opening
    // break the surface, whatever the fold did to it.
    float well = max(shaftR, (surfaceH - wellDepth) - p.y);
    float carved = max(ground, -well);

    // Growth already beat the uncarved ground, and carving only moves the
    // ground away, so a winning tree still wins. When the ground won, the well
    // may have pushed it behind the nearest growth, so compare again.
    if (growth.x < carved) return growth;
    return vec2(carved, 0.0);
}

// Ambient occlusion, sampled along the normal. The fold's value is all in its
// creases, and without occlusion a crease shades identically to a flat, so the
// whole landscape flattens back out into the smooth look this was meant to fix.
// Five taps, once per pixel, against seventy-odd march steps.
float calcAO(vec3 p, vec3 n, float drift) {
    float occ = 0.0;
    float sca = 1.0;
    for (int i = 0; i < 4; i++) {
        float h = 0.02 + 0.16 * float(i);
        occ += (h - sceneMap(p + n * h, drift).x) * sca;
        sca *= 0.7;
    }
    return clamp(1.0 - 2.6 * occ, 0.0, 1.0);
}

vec3 calcNormal(vec3 p, float drift, float t) {
    float e = max(0.0015 * t, 0.004);
    float d0 = sceneMap(p, drift).x;
    vec3 n = vec3(
        sceneMap(p + vec3(e, 0.0, 0.0), drift).x - d0,
        sceneMap(p + vec3(0.0, e, 0.0), drift).x - d0,
        sceneMap(p + vec3(0.0, 0.0, e), drift).x - d0
    );
    float len = length(n);
    return (len > 1e-12) ? n / len : vec3(0.0, 1.0, 0.0);
}

// The moon's disc: radiance in rgb, coverage in a.
//
// Built on a local basis about the moon direction so the disc has stable 2D
// coordinates, which is what lets it carry maria, craters and a terminator
// instead of being the radial blob a sun is.
vec4 moonDisc(vec3 rd, vec3 mdir, float sz, float spin) {
    vec3 mu = normalize(cross(vec3(0.0, 1.0, 0.0), mdir));
    vec3 mv = cross(mdir, mu);
    vec2 md = vec2(dot(rd, mu), dot(rd, mv)) / sz;
    float r2 = dot(md, md);
    if (r2 > 1.2) return vec4(0.0);

    float r = sqrt(r2);
    float cover = smoothstep(1.0, 0.975, r);
    float z = sqrt(max(1.0 - r2, 0.0));
    vec3 n = vec3(md, z);

    // Grey maria over a mottled highland, then finer speckle for craters. Kept
    // low contrast: darken the seas too far and the disc reads as a rock with a
    // bite out of it rather than as a moon.
    float m = vnoise(md * 3.4) * 0.6 + vnoise(md * 8.3) * 0.3;
    float albedo = 0.95 - 0.26 * smoothstep(0.38, 0.78, m);
    albedo *= 0.88 + 0.22 * vnoise(md * 17.0);
    albedo *= 0.93 + 0.14 * vnoise(md * 41.0);

    // The terminator sweeps as the cycle phase accumulates, so the moon waxes
    // and wanes. The light's z stays well positive so it never goes fully new
    // and takes the scene's key light with it.
    vec3 ldir = normalize(vec3(cos(spin) * 0.8, 0.2, 0.75 + 0.2 * sin(spin * 0.7)));
    float lam = max(dot(n, ldir), 0.0);
    // A little fill on the dark side reads as earthshine and keeps the disc a
    // sphere rather than a crescent cut out of the sky.
    lam = max(lam, 0.05);
    // Limb darkening, so the edge is not a hard cut-out.
    float limb = pow(max(z, 0.0), 0.3);

    return vec4(moon_color.rgb * albedo * lam * (0.55 + 0.45 * limb) * 2.6, cover);
}

// Self-similar halo filaments.
//
// The rings are periodic in the *log* of the angular radius, so each ring is a
// scaled copy of its neighbour and they crowd toward the core the way a fractal
// zoom does. A plain sine in the radius spaces them evenly instead, which reads
// as a dartboard. One structure then covers the whole sky: the same octaves that
// give fine rings against the disc give the broad arcs out towards the horizon,
// because in log space those are the same rings at different scales.
//
// The rays double their count each octave, so the spokes subdivide on the same
// schedule as the rings rather than sitting over them as a fixed starburst.
//
// `1 - |cos|` raised to a power peaks narrowly, which is what turns smooth bands
// into filigree.
// Returns the ring filaments in x and the radial spokes in y, so the sky can
// place them differently: the reference's rings run right out to the horizon
// while its spokes stay close in around the core.
vec2 haloFractal(float ang, float a2, float rings, float rays, int depth, float spin, float pxAng) {
    float lr = log(max(ang, 0.004));
    // Turbulence, so the filaments wander instead of being drafted circles.
    lr += (vnoise(vec2(a2 * 1.1, lr * 1.7)) - 0.5) * 0.4;

    float fil = 0.0;
    float spk = 0.0;
    float amp = 1.0;
    float norm = 0.0;
    float f = 1.0;
    for (int i = 0; i < 6; i++) {
        if (i >= depth) break;
        // Once an octave's rings are finer than a pixel they cannot be resolved
        // and only alias, so it fades out. Without this the core boils into
        // moire whenever the camera moves, which is how a log-periodic pattern
        // usually gives itself away.
        float spacing = ang * 6.2831853 / max(rings * f, 1e-3);
        float lod = smoothstep(pxAng, pxAng * 4.0, spacing);
        if (lod > 0.0) {
            // Each octave drifts at its own rate, so the structure churns
            // rather than rotating rigidly.
            float ring = 1.0 - abs(cos(lr * rings * f - spin * (1.0 + 0.35 * float(i))));
            float ray = 1.0 - abs(cos(a2 * rays * f + spin * 0.3));
            // Steep powers: `1 - |cos|` on its own is a broad hump, and stacking
            // broad humps gives a soft glow rather than the filigree the
            // reference is made of. The ray factor multiplies into the rings, so
            // the rays break them into arcs instead of laying a separate grid
            // over them, and it stays sharp so the gaps actually read.
            fil += amp * lod * pow(ring, 9.0) * (0.2 + 0.8 * pow(ray, 5.0));
            spk += amp * lod * pow(ray, 8.0);
        }
        norm += amp;
        amp *= 0.56;
        f *= 2.0;
    }
    return vec2(fil, spk) / max(norm, 1e-4);
}

// Sky, halo and moon. The ring and ray counts multiply position and the spin is
// added afterwards, so changing a count re-spaces the structure without
// rotating it.
vec3 skyColor(vec3 rd, float spin, float pxAng) {
    vec3 mdir = normalize(MOON_DIR);
    float grad = clamp(rd.y * 0.5 + 0.5, 0.0, 1.0);
    vec3 col = mix(horizon_color.rgb, sky_color.rgb, pow(grad, 0.7));

    float ang = acos(clamp(dot(rd, mdir), -1.0, 1.0));
    float halo = exp(-ang * 4.2);
    float bloom = exp(-ang * 1.3);

    vec2 tangent = rd.xy - mdir.xy;
    float a2 = atan(tangent.y, tangent.x);

    vec2 hf = haloFractal(ang, a2, max(halo_rings, 0.05), max(halo_rays, 0.05),
                          int(clamp(halo_depth, 1.0, 6.0)), spin, pxAng);

    // The rings are lit twice: tight around the disc where they are dense and
    // bright, and again through the broad bloom, where the outer octaves are the
    // arcs that sweep across the whole sky. The spokes fall off faster, so they
    // stay a corona around the moon rather than striping the whole dome.
    vec3 glow = moon_color.rgb * halo * (0.18 + 2.6 * hf.x);
    glow += moon_color.rgb * halo * halo * hf.y * 1.3;
    glow += moon_color.rgb * bloom * bloom * hf.x * 1.1;
    glow += horizon_color.rgb * bloom * (0.09 + 0.55 * hf.x);
    col += glow * moon_intensity;

    // The disc sits over its own halo rather than adding into it, so the
    // surface detail is not washed out by the glow it casts.
    vec4 disc = moonDisc(rd, mdir, max(moon_size, 0.02), spin);
    return mix(col, disc.rgb * moon_intensity, disc.a);
}

void main() {
    float audioSum = audio_level + audio_bass + audio_mid + audio_treble + audio_bpm + audio_beat_phase;
    float timeSum = TIMEDELTA + float(FRAMEINDEX) + float(PASSINDEX) + DATE.x + DATE.y + DATE.z + DATE.w + PHASE_TIME_0 + PHASE_TIME_1 + PHASE_TIME_2 + PHASE_TIME_3;
    if (uv.x < -1.0) { fragColor = vec4(audioSum + timeSum, 0.0, 0.0, 1.0); return; }

    // uv is top-left origin; flip so screen-up maps to +y in the ray basis.
    vec2 p = vec2(uv.x, 1.0 - uv.y) * 2.0 - 1.0;
    p.x *= RENDERSIZE.x / max(RENDERSIZE.y, 1.0);

    float travel = PHASE_TIME_0;
    float drift = PHASE_TIME_3;
    float spin = PHASE_TIME_2;
    // Bounded functions of the sway phase, so the amplitude parameter scales a
    // value in [-1,1] instead of an ever-growing one.
    float swayX = sin(PHASE_TIME_1);
    float swayA = cos(PHASE_TIME_1 * 0.7);

    vec3 ro = vec3(swayX * sway_amount * 1.5, 0.0, travel);
    // Set before any terrain lookup: the octave fade is measured from here.
    g_camXZ = ro.xz;
    ro.y = terracedH(ro.xz) + cam_height;

    vec3 fw = normalize(vec3(look_x * 0.6 + swayA * sway_amount * 0.12, look_y, 1.0));
    vec3 ri = normalize(cross(vec3(0.0, 1.0, 0.0), fw));
    vec3 upv = cross(fw, ri);
    vec3 rd = normalize(fw + (p.x * ri + p.y * upv) * fov);
    // Angular size of one pixel, for the halo's level-of-detail fade.
    float pxAng = 2.0 * fov / max(RENDERSIZE.y, 1.0);

    float ceilingY = tree_height * 1.8 + terrain_relief * 4.8 + 3.0;
    int steps = int(clamp(ray_steps, 20.0, 200.0));
    float t = 0.0;
    float mat = 0.0;
    bool hit = false;

    for (int i = 0; i < 200; i++) {
        if (i >= steps) break;
        vec3 pos = ro + rd * t;
        // Nothing above the canopy, so an upward ray that clears it is sky.
        if (pos.y > ceilingY && rd.y > 0.0) break;
        vec2 m = sceneMap(pos, drift);
        if (m.x < 0.0024 * t + 0.0035) { hit = true; mat = m.y; break; }
        t += max(m.x, 0.008);
        if (t > MAX_DIST) break;
    }

    vec3 col;
    if (hit) {
        vec3 pos = ro + rd * t;
        vec3 gn = calcNormal(pos, drift, t);
        // Occlusion is a property of the shape, so it reads the geometric
        // normal, before the surface detail below tilts it.
        float ao = calcAO(pos, gn, drift);

        int hitSpecies = max(int(round(mat)) - 1, 0);
        bool onGround = mat < 0.5;

        // Reconstruct the generating tree once at the visible surface. Ground
        // pixels use its trunk position as the sink for the radial circuit web;
        // tree pixels use the same local coordinates as sceneMap so traces stay
        // painted on warped trunks and their arriving packet colors reach the
        // canopy.
        vec2 treeBase = vec2(0.0);
        float treeSeed = 0.0;
        float treeScale = 1.0;
        float treeFooting = 0.0;
        vec2 treeLean = vec2(0.0);
        // Beyond this range traces are sub-pixel, so they cannot contribute a
        // stable visible result and the reconstruction is not worth paying for.
        float circuitLod = 1.0 - smoothstep(26.0, 55.0, t);
        bool hasTree;
        if (circuitLod <= 0.0) {
            hasTree = false;
        } else if (onGround) {
            hasTree = nearestTrunk(pos.xz, drift, treeBase, treeSeed, treeScale);
        } else if (hitSpecies == 0) {
            hasTree = nearestTreeData(
                pos.xz, treeBase, treeSeed, treeScale, treeFooting, treeLean
            );
        } else {
            hasTree = interstitialTreeData(
                pos.xz, treeBase, treeSeed, treeScale, treeFooting, treeLean
            );
        }

        vec3 treeP = vec3(0.0);
        if (hasTree && !onGround) {
            vec3 tq = pos - vec3(treeBase.x, 0.0, treeBase.y);
            treeP = (tq - vec3(0.0, treeFooting, 0.0)) / treeScale;
            float treePhase = treeSeed * 6.2831853;
            vec2 warpFrequency = (hitSpecies == 0)
                ? vec2(0.55, 0.42)
                : vec2(0.62, 0.47);
            vec2 treeWarp = vec2(
                sin(treeP.y * warpFrequency.x + treePhase + drift),
                cos(treeP.y * warpFrequency.y + treePhase * 1.7 + drift)
            );
            float warpAmount = trunk_warp * ((hitSpecies == 0) ? 0.9 : 0.35);
            treeP.xz -= treeWarp * warpAmount + treeP.y * treeLean;
        }

        // ── Circuitry, resolved before the surface is shaded ──────────────
        //
        // The traces are part of the material, not a layer over it. That means
        // the channel has to darken the rock, stain its surroundings and tilt
        // the normal *before* the lighting runs. Adding the packet light on top
        // of a finished surface is exactly what makes a trace read as a decal.
        vec3 cd = vec3(1e9, 0.0, 0.0);
        vec2 grooveSlope = vec2(0.0);
        float channel = 0.0;
        float stain = 0.0;
        float spill = 0.0;
        bool doGround = onGround && circuitLod > 0.0 && circuit_intensity > 0.0;
        if (doGround) {
            // Etched traces belong on the shelves of the fold. Left running up
            // its vertical faces and overhangs they read as printed decoration.
            float lay = smoothstep(0.30, 0.78, gn.y);
            if (lay > 0.0) {
                // The board rides the same low-frequency warp that bends the
                // terrain fold, so the routing follows the land rather than
                // crossing it on the world axes.
                vec2 warp = vec2(
                    vnoise(pos.xz * 0.031),
                    vnoise(pos.zx * 0.031 + 5.1)
                ) * 2.6;
                cd = circuitField(pos.xz, warp, treeBase, treeSeed, drift, hasTree);
                channel = lay * (1.0 - smoothstep(
                    circuit_width * 0.8, circuit_width * 2.3, cd.x
                ));
                // Corrosion immediately around the cut, then a wider wash where
                // light escaping the channel lands on the surrounding stone.
                stain = lay * (1.0 - smoothstep(
                    circuit_width * 1.5, circuit_width * 8.0, cd.x
                ));
                spill = lay * (1.0 - smoothstep(
                    circuit_width * 2.0, circuit_width * 15.0, cd.x
                ));

                // Cross-slope of the channel walls, from two extra taps of the
                // field. Only worth sampling while a groove is wider than a
                // pixel; past that it is pure aliasing.
                float etch = clamp(circuit_etch, 0.0, 1.0)
                           * (1.0 - smoothstep(9.0, 24.0, t));
                if (etch > 0.0) {
                    float e = 0.02;
                    float dx = circuitField(
                        pos.xz + vec2(e, 0.0), warp, treeBase, treeSeed, drift, hasTree
                    ).x;
                    float dz = circuitField(
                        pos.xz + vec2(0.0, e), warp, treeBase, treeSeed, drift, hasTree
                    ).x;
                    // A V-channel's height rises with distance from its
                    // centreline, so the heightfield normal is the surface
                    // normal minus that gradient. The walls end up facing
                    // inward across the cut and catch the moon along its length.
                    grooveSlope = (vec2(dx, dz) - cd.x) / e * channel * etch * 1.7;
                }
            }
        }

        // Trunk conductors, needed here rather than at the emissive step so the
        // bark can be grooved and darkened along them too.
        float trunkTrace = 0.0;
        float trunkZone = 0.0;
        float climbPulse = 0.0;
        vec3 climbColor = vec3(0.0);
        float crownZone = 0.0;
        float arrival = 0.0;
        vec3 arrivalColor = vec3(0.0);
        bool doTree = hasTree && !onGround && circuitLod > 0.0
                    && (trunk_circuits > 0.0 || aura_intensity > 0.0);
        if (doTree) {
            float a = atan(treeP.z, treeP.x);
            // Four longitudinal conductors, with hashed quarter turns at each
            // height band and small horizontal bus rings joining them.
            float route = a * 0.6366198
                        + floor(treeP.y * 1.6 + treeSeed * 3.0) * 0.25;
            float longitudinal = abs(sin(route * 3.1415927)) * 0.22;
            float bus = abs(sin((treeP.y * 1.7 + treeSeed) * 3.1415927)) * 0.18;
            trunkTrace = traceMask(min(longitudinal, bus), circuit_width);

            // `drift - y*k = constant` moves packets upward as drift advances.
            float energyHeight = tree_height;
            if (hitSpecies > 0) energyHeight = interstitialHeight(hitSpecies);
            float climbPhase = drift - treeP.y * 0.31 + treeSeed;
            climbPulse = pulseTrain(climbPhase);
            climbColor = energyRGB(climbPhase);
            trunkZone = 1.0 - smoothstep(energyHeight * 0.82, energyHeight, treeP.y);

            // The color at the canopy is exactly the color of the packet after
            // traversing one tree height. Outer branches receive more aura than
            // the hub, so the silhouette glows at its tips instead of becoming
            // one flat emissive plate.
            float arrivalPhase = drift - energyHeight * 0.31 + treeSeed;
            arrival = pulseTrain(arrivalPhase);
            arrivalColor = energyRGB(arrivalPhase);
            vec3 crownP = treeP - vec3(0.0, energyHeight, 0.0);
            if (hitSpecies == 1) {
                // Facet edges and the terminal star hold the crystal's charge.
                crownZone = smoothstep(-energyHeight * 0.38, 0.08, crownP.y);
            } else if (hitSpecies == 2) {
                // Every fan tier can terminate a packet, producing alternating
                // bands of charged frond tips up the coral.
                float tier = pow(abs(sin(treeP.y * 8.3 + treeSeed * 6.0)), 5.0);
                crownZone = smoothstep(canopy_spread * 0.28,
                                       canopy_spread * 1.15,
                                       length(treeP.xz))
                          * (0.25 + 0.75 * tier);
            } else {
                crownZone = smoothstep(-0.35, 0.35, crownP.y)
                          * smoothstep(canopy_spread * 0.2,
                                       canopy_spread * 1.25,
                                       length(crownP.xz));
            }
        }

        vec3 n = gn;
        float strata = 0.0;
        if (onGround) {
            // Sedimentary banding, as surface detail rather than as geometry.
            // Bands in world y follow the land's contours for free, and they
            // show up strongest on the vertical faces, which is where strata
            // show in rock. Carving them for real would mean a fold fine enough
            // to multiply the march's step count several times over; this is the
            // detail that makes the surface read as stone, and none of it needs
            // to be in the silhouette to do that.
            float sy = pos.y * 6.5 + vnoise(pos.xz * 0.42) * 2.6;
            strata = fract(sy);
            float grain = vnoise(pos.xz * 9.0) - 0.5;
            n = normalize(gn + vec3(grain * 0.25, (strata - 0.5) * 1.1, grain * 0.25));
            n = normalize(n - vec3(grooveSlope.x, 0.0, grooveSlope.y));
        } else {
            // Trunk conductors sit in a shallow groove in the bark, so their
            // edges pick up the moon the same way the ground channels do.
            n = normalize(gn + normalize(vec3(treeP.x, 0.35, treeP.z) + 1e-5)
                             * trunkTrace * clamp(circuit_etch, 0.0, 1.0) * -0.45);
        }

        vec3 mdir = normalize(MOON_DIR);

        // Everything is lit from the moon, so foreground growth reads as a
        // silhouette and only its edges catch light.
        float toward = clamp(dot(n, mdir), 0.0, 1.0);
        float sky = 0.5 + 0.5 * n.y;
        float rim = pow(1.0 - abs(dot(n, rd)), 3.0);

        vec3 albedo;
        float fill;
        float key;
        // Rim light is edge light on branches. The ground is viewed almost edge
        // on, so an untamed rim term saturates it to a wet sheen.
        float rimK;
        if (!onGround) {
            float bark = vnoise(pos.xz * 6.0 + pos.y * 3.0) * 0.5 + 0.5;
            if (hitSpecies == 1) {
                // Crystal lattices borrow blue from the terrain and violet from
                // the vegetation so their facets are distinct before energy
                // reaches them.
                albedo = mix(terrain_color.rgb, tree_color.rgb, 0.35)
                       * (0.75 + 0.5 * bark);
                fill = 0.42;
                key = 2.2;
                rimK = 0.18;
            } else if (hitSpecies == 2) {
                albedo = tree_color.rgb * vec3(0.8, 1.25, 1.05)
                       * (0.65 + 0.65 * bark);
                fill = 0.36;
                key = 1.65;
                rimK = 0.12;
            } else {
                albedo = tree_color.rgb * (0.6 + 0.7 * bark);
                fill = 0.30;
                key = 1.8;
                rimK = 0.09;
            }
            // Bark is scarred where a conductor runs through it, so the trace
            // has a home in the material before any light comes out of it.
            albedo = mix(albedo, albedo * vec3(0.34, 0.40, 0.55),
                         trunkTrace * clamp(circuit_etch, 0.0, 1.0));
        } else {
            // Colour the ground from the same fractal that shaped it, so the
            // creases read as rock and the shelves as growth.
            // Height drives hue, so each stratum is a different shade and the
            // terracing reads as sediment rather than as one tint at different
            // exposures. Banding on the raw height rather than on the noise is
            // what makes the bands follow the shelves.
            float h = hillsAt(pos.xz);
            vec3 crest = terrain_color.rgb * vec3(1.7, 0.85, 1.15);
            // The same bands tint as well as tilt, so each stratum is its own
            // shade of rock instead of one tint at different exposures.
            albedo = mix(terrain_color.rgb, crest, smoothstep(0.25, 0.75, h))
                   * (0.7 + 0.8 * h + 0.5 * strata);
            // Weathering around the channel, then the channel itself: a dark,
            // cool, oxidised bed cut into the rock. The packet light is emitted
            // from inside this, which is what stops it looking painted on.
            albedo *= 1.0 - 0.22 * stain;
            albedo = mix(albedo, albedo * vec3(0.16, 0.24, 0.40)
                                 + terrain_color.rgb * 0.05, channel);
            // Ground is nearly parallel to the moonlight, so slope has to do
            // the work: without it low relief is invisible at a grazing angle.
            // The fold gives the ground real vertical faces now, so it is no
            // longer viewed almost edge-on everywhere and can take a fuller
            // share of the skylight without turning to sheen.
            fill = 0.5 + 0.7 * pow(clamp(n.y, 0.0, 1.0), 0.8);
            // A full key on ground this flat just puts a wet sheen along every
            // crest, which reads as water rather than forest floor.
            key = 1.0;
            rimK = 0.03;
        }

        // Under a moon the sky *is* the fill light and the moon *is* the key, so
        // both carry their own colour; leaving them white reads as grey. Neither
        // tint is allowed to reach zero, so pushing the sky to black dims the
        // scene without extinguishing it.
        vec3 fillTint = mix(vec3(1.0), horizon_color.rgb * 3.0, 0.55);
        vec3 keyTint = mix(vec3(1.0), moon_color.rgb, 0.85);

        // Occlusion belongs on the ambient term: the sky is a dome, so a
        // crevice sees less of it, while the moon is a point and is already
        // handled by its own shadow term through `toward`.
        col = albedo * sky * fill * fillTint * ao;
        col += albedo * toward * key * keyTint;
        col += moon_color.rgb * rim * rimK * moon_intensity;

        if (doGround) {
            float pulse = pulseTrain(cd.y);
            vec3 packet = energyRGB(cd.y);
            float gain = circuit_intensity * circuitLod;

            // Light from inside a cut escapes past the surrounding rock, so it
            // is occluded like any other light in a crevice. Only the packet
            // core is a source; the idle conductor is a faint standing bed. The
            // ring on a well's lip is held much brighter than that, because its
            // job is to outline the opening whether or not a packet is on it.
            float standing = (cd.z > 1.5) ? 0.45 : 0.03;
            col += packet * channel * (standing + 1.7 * pulse) * gain
                 * mix(0.55, 1.0, ao);
            // The escaping light washes the stone beside the channel, tinted by
            // that stone. This term is what actually seats the trace in the
            // surface: without a lit surround the brightest line in the frame
            // has no footprint in the material under it.
            col += packet * albedo * spill * spill
                 * (0.35 + 5.0 * pulse) * gain * ao;
        }
        if (doTree) {
            col += climbColor * trunkTrace * trunkZone * trunk_circuits
                 * circuitLod * (0.12 + 2.8 * climbPulse);
            float facet = (hitSpecies == 1) ? (0.45 + 0.55 * rim) : 1.0;
            float aura = crownZone * facet * (0.12 + 2.4 * arrival)
                       * (0.35 + 0.65 * rim);
            col += arrivalColor * aura * aura_intensity * circuitLod;
        }
    } else {
        col = skyColor(rd, spin, pxAng);
    }

    col = max(col, 0.0);
    fragColor = vec4(col, 1.0);
}
