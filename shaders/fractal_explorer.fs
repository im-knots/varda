/*{
    "DESCRIPTION": "3D Fractal Explorer - a four-slot formula stack marched as a solid and finished in-shader. Each slot picks a distance estimator (Mandelbox, Amazing Box, Menger, Sierpinski, Mandelbulb, Pseudo-Kleinian, lin-combine, rotate, co-cube, 4D rotate, or off) and takes a share of a shared iteration budget; each fold shapes the space the next one sees, so Slot Order permutes the stack and starving the later slots restructures the geometry rather than merely retuning it. The camera approaches geometrically rather than linearly, and the detail threshold, depth range and fold count scale with it, so a dive keeps resolving new structure instead of arriving at a smooth blob. Two atmospheres: distance haze, and fog keyed to the fold count at each point in space, which drapes over the geometry and pools in its troughs. Rendered muted in linear light with depth in alpha, then composited the way a Mandelbulb3D frame normally is in After Effects: distance softening off the marched Z, selective highlight bloom, chromatic aberration, twin ghost reflections with a matte-box flare, key-aligned light shafts, radial camera blur, vignette and a look grade. The Composition group breaks the tonal evenness a fractal has by construction, with a lit side and a fallen-away side, a dark foreground against a lifted background, and local rather than global contrast. The palette is banded in view depth rather than fixed, so near and far parts of the structure take different hues and each one shifts as it comes toward camera, which is what gives a fly-through somewhere to go. Mutate the Stack group to hunt permutations; save a preset when one lands Ships with the fractal parked and only the camera moving, because a shot reads best with a single degree of freedom in motion; Formula / Energy Speed is the beat you bring in when you want the structure itself to move, and Evolve Target picks the single parameter it drives, defaulting to the fold scale that every folding formula reads. The march converges to a pixel rather than to an absolute distance, so `Detail` reads as pixels of convergence: one at the default, down to a third for a sharper and slower march, up to nearly three for a softer and faster one. It therefore means the same thing at every distance, zoom and output resolution, and the march no longer chases structure finer than the frame can hold, which is what used to leave stripes on a pulled-back camera and torn holes of background through solid geometry up close. The fold cutoff crossfades across one fold instead of switching at one, so the surface slides between levels of detail as the camera moves rather than snapping between them. The sky defaults near black with a star field rather than a lifted haze, which is what projection and dome output need; Atmosphere Lift trades that for flat-screen depth staging. Horizon Mirror folds the sky back on itself below the waterline, and because a fractal is usually already symmetric there, the frame reads as a mirror-flat lake.",
    "CREDIT": "Varda VJ (Mandelbox after Tom Lowe; Amazing Box/Surface after Kali; kaleidoscopic Sierpinski after Knighty; slot-stack structure, lin-combine / rotate / co-cube slots and the iteration-cutoff bypass after Mandelbulb3D; muted-render, multi-pass beauty/RGB/Z compositing workflow and depth-pass-as-matte highlighting after Julius Horsthuis; circuit-trace treatment after alien_grove.fs)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "INPUTS": [
        {"NAME": "fly_speed", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 3.0, "LABEL": "Approach Speed"},
        {"NAME": "evolve", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 2.0, "LABEL": "Formula / Energy Speed"},
        {"NAME": "orbit_speed", "TYPE": "float", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 2.0, "LABEL": "Orbit Speed"},
        {"NAME": "sway_speed", "TYPE": "float", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 2.0, "LABEL": "Camera Sway Speed"},

        {"NAME": "slot0_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 1, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 1 Formula"},
        {"NAME": "slot0_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 2.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 1 Weight"},
        {"NAME": "slot1_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 5, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 2 Formula"},
        {"NAME": "slot1_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 2 Weight"},
        {"NAME": "slot2_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 8, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 3 Formula"},
        {"NAME": "slot2_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 3 Weight"},
        {"NAME": "slot3_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 9, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 4 Formula"},
        {"NAME": "slot3_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 4 Weight"},
        {"NAME": "stack_cap", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 14.0, "MIN": 1.0, "MAX": 40.0, "LABEL": "Iteration Cutoff"},
        {"NAME": "stack_order", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7], "LABELS": ["1 2 3 4", "2 1 3 4", "1 2 4 3", "2 1 4 3", "3 4 1 2", "2 3 4 1", "4 1 2 3", "4 3 2 1"], "LABEL": "Slot Order"},

        {"NAME": "scale", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 2.0, "MIN": -3.0, "MAX": 3.0, "LABEL": "Scale"},
        {"NAME": "fold_limit", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": 0.3, "MAX": 2.0, "LABEL": "Fold Limit"},
        {"NAME": "min_radius", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.5, "MIN": 0.1, "MAX": 1.0, "LABEL": "Min Radius"},
        {"NAME": "fixed_radius", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 2.0, "LABEL": "Fixed Radius"},
        {"NAME": "power", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 8.0, "MIN": 2.0, "MAX": 12.0, "LABEL": "Bulb Power"},
        {"NAME": "offset_x", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": -3.0, "MAX": 3.0, "LABEL": "Offset X"},
        {"NAME": "offset_y", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": -3.0, "MAX": 3.0, "LABEL": "Offset Y"},
        {"NAME": "offset_z", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": -3.0, "MAX": 3.0, "LABEL": "Offset Z"},
        {"NAME": "lin_x", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": -2.0, "MAX": 2.0, "LABEL": "Lin Combine X"},
        {"NAME": "lin_y", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": -2.0, "MAX": 2.0, "LABEL": "Lin Combine Y"},
        {"NAME": "lin_z", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 1.0, "MIN": -2.0, "MAX": 2.0, "LABEL": "Lin Combine Z"},
        {"NAME": "lin_mix", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.2, "MIN": -1.0, "MAX": 1.0, "LABEL": "Lin Combine Bleed"},
        {"NAME": "rot_a", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 3.14159, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate XY"},
        {"NAME": "rot_b", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate YZ"},
        {"NAME": "rot_c", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate XZ"},
        {"NAME": "rot_xw", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 3.14159, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate XW"},
        {"NAME": "rot_yw", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate YW"},
        {"NAME": "rot_zw", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Rotate ZW"},
        {"NAME": "cocube", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.6, "MIN": 0.0, "MAX": 2.0, "LABEL": "Co-Cube Corner"},
        {"NAME": "bailout", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 32.0, "MIN": 1.5, "MAX": 32.0, "LABEL": "Bailout Radius"},
        {"NAME": "julia_amount", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Julia Seed"},
        {"NAME": "julia_x", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.35, "MIN": -2.0, "MAX": 2.0, "LABEL": "Julia X"},
        {"NAME": "julia_y", "TYPE": "float", "GROUP": "Formula", "DEFAULT": -0.15, "MIN": -2.0, "MAX": 2.0, "LABEL": "Julia Y"},
        {"NAME": "julia_z", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.2, "MIN": -2.0, "MAX": 2.0, "LABEL": "Julia Z"},
        {"NAME": "evolve_amount", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 0.18, "MIN": 0.0, "MAX": 0.6, "LABEL": "Evolve Depth"},
        {"NAME": "evolve_target", "TYPE": "long", "GROUP": "Formula", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4], "LABELS": ["Fold Scale", "Offset X", "Rotate XY", "Lin Combine X (needs slot)", "Julia Seed X (needs seed)"], "LABEL": "Evolve Target"},

        {"NAME": "cam_dist", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 4.2, "MIN": 0.6, "MAX": 12.0, "LABEL": "Distance"},
        {"NAME": "cam_azim", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Azimuth"},
        {"NAME": "cam_elev", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.2, "MIN": -1.4, "MAX": 1.4, "LABEL": "Elevation"},
        {"NAME": "zoom_cycle", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 24.0, "LABEL": "Zoom Cycle"},
        {"NAME": "look_x", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.0, "MIN": -1.5, "MAX": 1.5, "LABEL": "Aim X"},
        {"NAME": "look_y", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.0, "MIN": -1.5, "MAX": 1.5, "LABEL": "Aim Y"},
        {"NAME": "look_z", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.0, "MIN": -1.5, "MAX": 1.5, "LABEL": "Aim Z"},
        {"NAME": "fov", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.85, "MIN": 0.35, "MAX": 1.8, "LABEL": "FOV"},
        {"NAME": "sway_amount", "TYPE": "float", "GROUP": "Camera", "DEFAULT": 0.08, "MIN": 0.0, "MAX": 1.0, "LABEL": "Sway Amount"},

        {"NAME": "ray_steps", "TYPE": "float", "GROUP": "Render", "DEFAULT": 180.0, "MIN": 40.0, "MAX": 220.0, "LABEL": "Ray Steps"},
        {"NAME": "detail", "TYPE": "float", "GROUP": "Render", "DEFAULT": 0.0015, "MIN": 0.0005, "MAX": 0.006, "LABEL": "Detail"},
        {"NAME": "ao_strength", "TYPE": "float", "GROUP": "Render", "DEFAULT": 0.75, "MIN": 0.0, "MAX": 1.0, "LABEL": "AO Strength"},
        {"NAME": "shadow_strength", "TYPE": "float", "GROUP": "Render", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 1.0, "LABEL": "Shadow Strength"},

        {"NAME": "light_azim", "TYPE": "float", "GROUP": "Light", "DEFAULT": 1.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Key Azimuth"},
        {"NAME": "light_elev", "TYPE": "float", "GROUP": "Light", "DEFAULT": 0.5, "MIN": -0.6, "MAX": 1.3, "LABEL": "Key Elevation"},
        {"NAME": "fog_amount", "TYPE": "float", "GROUP": "Light", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 2.0, "LABEL": "Atmosphere"},
        {"NAME": "emissive", "TYPE": "float", "GROUP": "Light", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 3.0, "LABEL": "Emissive Depth"},
        {"NAME": "exposure", "TYPE": "float", "GROUP": "Light", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 3.0, "LABEL": "Exposure"},

        {"NAME": "fog_iter_amount", "TYPE": "float", "GROUP": "Atmosphere", "DEFAULT": 0.9, "MIN": 0.0, "MAX": 2.0, "LABEL": "Iteration Fog"},
        {"NAME": "fog_iter", "TYPE": "float", "GROUP": "Atmosphere", "DEFAULT": 6.0, "MIN": 1.0, "MAX": 24.0, "LABEL": "Fog Iteration"},
        {"NAME": "fog_iter_band", "TYPE": "float", "GROUP": "Atmosphere", "DEFAULT": 1.5, "MIN": 0.35, "MAX": 8.0, "LABEL": "Fog Iteration Width"},
        {"NAME": "fog_iter_reach", "TYPE": "float", "GROUP": "Atmosphere", "DEFAULT": 7.0, "MIN": 1.0, "MAX": 24.0, "LABEL": "Fog Reach"},
        {"NAME": "atmos_lift", "TYPE": "float", "GROUP": "Atmosphere", "DEFAULT": 0.06, "MIN": 0.0, "MAX": 0.5, "LABEL": "Atmosphere Lift"},
        {"NAME": "star_amount", "TYPE": "float", "GROUP": "Atmosphere", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 1.0, "LABEL": "Stars"},
        {"NAME": "horizon_mirror", "TYPE": "float", "GROUP": "Atmosphere", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Horizon Mirror"},

        {"NAME": "dof_onset", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 1.8, "MIN": 0.2, "MAX": 8.0, "LABEL": "Softening Onset"},
        {"NAME": "dof_amount", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 1.0, "LABEL": "Distance Softening"},
        {"NAME": "bloom", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 2.0, "LABEL": "Bloom"},
        {"NAME": "bloom_thresh", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.55, "MIN": 0.2, "MAX": 3.0, "LABEL": "Bloom Threshold"},
        {"NAME": "aberration", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.18, "MIN": 0.0, "MAX": 1.0, "LABEL": "Chromatic Aberration"},
        {"NAME": "ghost", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.14, "MIN": 0.0, "MAX": 1.0, "LABEL": "Ghost Reflection"},
        {"NAME": "motion_blur", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.15, "MIN": 0.0, "MAX": 1.0, "LABEL": "Camera Blur"},
        {"NAME": "vignette", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 1.0, "LABEL": "Vignette"},

        {"NAME": "look_contrast", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 1.0, "LABEL": "Look Contrast"},
        {"NAME": "saturation", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 1.15, "MIN": 0.0, "MAX": 1.5, "LABEL": "Saturation"},
        {"NAME": "palette_mute", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 0.2, "MIN": 0.0, "MAX": 1.0, "LABEL": "Render Mute"},

        {"NAME": "depth_palette", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 0.7, "MIN": 0.0, "MAX": 1.0, "LABEL": "Depth Colour Shift"},
        {"NAME": "depth_cycles", "TYPE": "float", "GROUP": "Palette", "DEFAULT": 2.5, "MIN": 0.25, "MAX": 6.0, "LABEL": "Depth Colour Cycles"},

        {"NAME": "light_side", "TYPE": "float", "GROUP": "Composition", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Light Side"},
        {"NAME": "depth_stage", "TYPE": "float", "GROUP": "Composition", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0, "LABEL": "Depth Staging"},
        {"NAME": "clarity", "TYPE": "float", "GROUP": "Composition", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0, "LABEL": "Local Contrast"},
        {"NAME": "shafts", "TYPE": "float", "GROUP": "Composition", "DEFAULT": 0.25, "MIN": 0.0, "MAX": 1.0, "LABEL": "Light Shafts"},

        {"NAME": "color_cool", "TYPE": "color", "GROUP": "Palette", "DEFAULT": [0.06, 0.16, 0.44, 1.0], "LABEL": "Cool"},
        {"NAME": "color_warm", "TYPE": "color", "GROUP": "Palette", "DEFAULT": [0.52, 0.10, 0.42, 1.0], "LABEL": "Warm"},
        {"NAME": "accent_color", "TYPE": "color", "GROUP": "Palette", "DEFAULT": [0.10, 0.95, 0.86, 1.0], "LABEL": "Accent"},
        {"NAME": "bg_color", "TYPE": "color", "GROUP": "Palette", "DEFAULT": [0.022, 0.026, 0.062, 1.0], "LABEL": "Atmosphere Color"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "fly_speed", "INDEX": 0, "SCALE": 1.0},
        {"PARAM": "evolve", "INDEX": 1, "SCALE": 1.0},
        {"PARAM": "orbit_speed", "INDEX": 2, "SCALE": 1.0},
        {"PARAM": "sway_speed", "INDEX": 3, "SCALE": 1.0}
    ],
    "PASSES": [
        {"TARGET": "sceneBuffer", "FLOAT": true},
        {}
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

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D sceneBuffer;

layout(std140, set = 0, binding = 3) uniform UserParams {
    float fly_speed;
    float evolve;
    float orbit_speed;
    float sway_speed;

    int slot0_formula;
    float slot0_iters;
    int slot1_formula;
    float slot1_iters;
    int slot2_formula;
    float slot2_iters;
    int slot3_formula;
    float slot3_iters;
    float stack_cap;
    int stack_order;

    float scale;
    float fold_limit;
    float min_radius;
    float fixed_radius;
    float power;
    float offset_x;
    float offset_y;
    float offset_z;
    float lin_x;
    float lin_y;
    float lin_z;
    float lin_mix;
    float rot_a;
    float rot_b;
    float rot_c;
    float rot_xw;
    float rot_yw;
    float rot_zw;
    float cocube;
    float bailout;
    float julia_amount;
    float julia_x;
    float julia_y;
    float julia_z;
    float evolve_amount;
    int evolve_target;

    float cam_dist;
    float cam_azim;
    float cam_elev;
    float zoom_cycle;
    float look_x;
    float look_y;
    float look_z;
    float fov;
    float sway_amount;

    float ray_steps;
    float detail;
    float ao_strength;
    float shadow_strength;

    float light_azim;
    float light_elev;
    float fog_amount;
    float emissive;
    float exposure;

    float fog_iter_amount;
    float fog_iter;
    float fog_iter_band;
    float fog_iter_reach;
    float atmos_lift;
    float star_amount;
    float horizon_mirror;

    float dof_onset;
    float dof_amount;
    float bloom;
    float bloom_thresh;
    float aberration;
    float ghost;
    float motion_blur;
    float vignette;

    float look_contrast;
    float saturation;
    float palette_mute;

    float depth_palette;
    float depth_cycles;

    float light_side;
    float depth_stage;
    float clarity;
    float shafts;

    vec4 color_cool;
    vec4 color_warm;
    vec4 accent_color;
    vec4 bg_color;
};

// Iteration ceiling. The `Iteration Cutoff` parameter is the artistic control;
// this is the hard loop bound the compiler needs.
const int MAX_STACK_ITERS = 40;
// Outer bound on the marched volume. Everything of interest sits near the origin.
const float MAX_DIST = 24.0;
// Radius of the sphere the whole structure is assumed to sit inside, used to skip
// empty space when the camera is outside it.
const float BOUND_R = 6.0;
// The `Detail` default, which the march threshold is expressed against so that
// the default converges at exactly one pixel footprint. See `renderScene`.
const float DETAIL_REF = 0.0015;
const float TAU = 6.2831853;
// The radius the scene's absolute lengths were authored against. Zoom is measured
// from here rather than from `cam_dist`, which is what makes closing `Distance` by
// hand buy the same tightened thresholds and the same extra folds that the dive
// does. Measured against `cam_dist` the two cancelled — `viewScale` is
// `cam_dist * exp(-descent)` — so the whole scale apparatus was blind to how close
// the camera actually was and only ever responded to the animation.
const float REF_DIST = 4.2;

// ── Scale ────────────────────────────────────────────────────────────────────
//
// The camera approaches geometrically rather than linearly, and once it does,
// almost nothing in the renderer can go on being an absolute world distance.
//
// A linear approach is what conventional 3D does, and it has two problems that
// only show up on a fractal. It walks through walls, because a step sized for
// the room is enormous next to the detail near a surface. And it is the wrong
// pacing: closing from 4.0 to 3.0 reveals a great deal, closing from 0.2 to 0.0
// reveals nothing and then ends. What makes a fractal dive read as endless is
// that equal time buys an equal *ratio* of distance, so detail keeps arriving at
// a constant rate and the surface is never actually reached.
//
// The catch is that a geometric approach only pays off if the thresholds come
// with it. Detail held at a fixed world size stops resolving as the camera
// closes and the last stretch of the dive goes to mush; a depth channel
// normalised to a fixed range crushes flat, taking the softening, the depth
// palette and the staging with it; and a fixed fold count runs out of structure
// and arrives at a smooth blob. So distance is expressed against the camera's
// own radius from here on, and `viewZoom` is the conversion factor for the
// handful of controls that are still authored in world units.
//
// Not everything converts, and getting the split wrong is worse than not
// converting at all. Two things here are properties of the *scene* rather than
// of the camera, because the structure does not shrink — only the camera does.
// The march's reach is one: the walls around a camera deep in a dive still sit a
// unit or several away, and a reach tied to the camera cuts them out of frame.
// The haze is the other, and it was the more instructive mistake. Converted, it
// reached full density within a fraction of a unit, so everything but the one
// body the camera was diving at washed out to flat fog and the dive appeared to
// arrive in an empty room. Atmosphere is measured through the world.
// How far the descent has travelled, in natural logs of distance.
//
// Monotonic and wrapping, where this used to be `0.5 - 0.5 * cos(PHASE_TIME_0)`:
// a dive that breathed in and back out, bottoming out at seventy-nine times closer
// and then reversing. That is a dolly, not a zoom, and it cannot produce the
// effect this is for. The whole reason a fractal dive reads as endless is that the
// camera keeps closing and the structure keeps opening, and a camera that turns
// around at a fixed depth is instead guaranteed to arrive at the same lump every
// cycle.
//
// It wraps rather than running forever, and the wrap is the interesting part. A
// self-similar structure repeats under scaling by its own fold scale, so cutting
// the descent after a whole number of those steps lands on geometry that matches
// where it started as closely as the formula allows. `Zoom Cycle` is that whole
// number: eight steps of a scale-2 stack is a descent of two hundred and
// fifty-six, wrapping to a frame that should read as continuous. Higher is a
// longer fall before the seam.
//
// Bounding the descent this way also keeps float32 honest. A dive aimed at a
// general point in space is limited by how precisely that point can be written
// down, which caps a single-precision descent at a few tens of thousands; a cycle
// is far shorter than that, so precision never enters into it.
float diveLog() {
    float stepLog = log(max(abs(scale), 1.2));
    return fract(PHASE_TIME_0 / TAU) * zoom_cycle * stepLog;
}

float viewScale() {
    return max(cam_dist * exp(-diveLog()), 1e-5);
}

// 1.0 at the authored radius, rising as the camera closes, whether it closes by
// diving or because `Distance` was pulled in. Dividing a world distance by this
// converts it to "the same fraction of the view as it was at the authored
// radius", which is what every inherited absolute wants to mean.
float viewZoom() {
    return max(REF_DIST / viewScale(), 1.0);
}

// The depth channel, logarithmic in distance.
//
// Two linear encodings failed here before this one. Normalising by MAX_DIST
// crammed every surface into the bottom fifth of the channel, which left the
// depth of field nothing to separate. Normalising by the camera radius instead,
// `1 / (2 * viewScale())`, fixed that while the camera was parked and broke
// worse once it dove: the walls do not shrink when the camera does, so MAX_DIST
// stays at 24 while the radius falls two decades, and at full dive depth the
// channel saturated past about a tenth of a unit. All three consumers then read
// a solid clamp across most of the frame — the circle of confusion pinned at
// maximum, `back` pinned at 1.0 so the whole frame took the background lift and
// nothing took the foreground darkening, and the depth palette no longer
// banding.
//
// Log holds at every scale, and for the same reason the approach itself is
// geometric: equal *ratios* of distance get equal spans of channel, so the near
// shell and the far wall both keep room however deep the dive has gone. The near
// end is tied to the camera radius, which is the one length in the scene that
// tracks the dive; the far end is the march limit, which is a property of the
// scene and stays put.
float depthEncode(float t) {
    float near = max(viewScale() * 0.05, 1e-6);
    return clamp(log(max(t, near) / near) / log(MAX_DIST / near), 0.0, 1.0);
}

// Cached per pixel by the pass entry points. The estimator is called upward of a
// hundred times per pixel and `viewScale` costs a logarithm and an exponential,
// so recomputing it in there would be paid for on every march step.
float g_zoom = 1.0;
float g_foldBoost = 0.0;

mat2 rot(float a) {
    float c = cos(a), s = sin(a);
    return mat2(c, -s, s, c);
}

// A w-plane angle at exactly ±π/2 collapses its spatial axis.
//
// Working the 4D rotate's spatial block through from q = (p, 0), it comes out
// lower-triangular with cos(xw), cos(yw), cos(zw) on the diagonal, so a zero
// there sends that axis to zero for every point in space. The derivative chain
// never sees it — a rotation is an isometry, so `dr` is deliberately left alone —
// and the estimator therefore goes on reporting distances for a solid while the
// march is in a space that has been flattened to a plane, and oversteps through
// it.
//
// Nudged out of a narrow band rather than clamped to it, because the determinant
// is that same product of cosines and so changes sign across ±π/2: the slot flips
// between a proper rotation and a reflection there whatever we do, and there is
// no continuous way through.
float wPlaneAngle(float a) {
    const float HALF_PI = 1.5707963;
    const float MARGIN = 0.06;
    float m = abs(a);
    if (m > HALF_PI - MARGIN && m < HALF_PI + MARGIN) {
        m = (m < HALF_PI) ? HALF_PI - MARGIN : HALF_PI + MARGIN;
    }
    return (a < 0.0) ? -m : m;
}

// Which authored slot runs in each position of the cycle. Eight of the
// twenty-four permutations, chosen so that adjacent swaps, the two-swap, both
// single rotations and the full reversal are all one step away.
ivec4 stackOrder(int which) {
    if (which == 1) return ivec4(1, 0, 2, 3);
    if (which == 2) return ivec4(0, 1, 3, 2);
    if (which == 3) return ivec4(1, 0, 3, 2);
    if (which == 4) return ivec4(2, 3, 0, 1);
    if (which == 5) return ivec4(1, 2, 3, 0);
    if (which == 6) return ivec4(3, 0, 1, 2);
    if (which == 7) return ivec4(3, 2, 1, 0);
    return ivec4(0, 1, 2, 3);
}

// Pixel-stable dither for the level-of-detail threshold. See its use in the
// march for why the threshold is jittered rather than shared by every pixel.
float hash12(vec2 p) {
    return fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
}

// How much of the fold stack one estimator call is asked to resolve. Passed in,
// not set on a global, and the reason is a bug category rather than a style
// preference: every probe in the frame calls the estimator, so a global protocol
// means each probe silently inherits whatever the previous one left and silently
// overwrites it for the next. That produced three separate bugs here — a rescued
// ray shaded from the point where it gave up, surface readouts clobbered by the
// normal and shadow probes, and a shadow footprint left coarse for everything
// after it. Threading it through costs nothing at runtime and makes all three
// unwritable.
//
// The two controls are deliberately separate because they do different things
// and the passes want different combinations of them.
//
// `iters` stops the fold stack once its features are finer than the given
// footprint. That is what chooses the isosurface: a truncated stack describes a
// smoother solid than a full one.
//
// `minDist` additionally refuses to report a distance finer than the footprint,
// which is what stops a ray creeping along a surface at sub-pixel steps until it
// runs out of budget.
//
// The march wants both. Normals want the cutoff *without* the floor: the floor
// clamps the estimate to a constant just inside the surface, and differencing a
// constant is how a normal comes back flat. Shadows want both, and coarser than
// the march, because a penumbra resolves nothing fine.
//
// The rule that ties them together: a differencing baseline and the iteration
// cutoff always move as a pair. Widening one without the other samples fine
// geometry at scattered points and calls the result smooth, which is not a
// smoothed value but an undersampled one, and it returns as noise rather than as
// blur. The coarse shadow footprint and the curvature normal's wide cutoff are
// both this rule.
//
// `cap` overrides the iteration budget, for a caller that only needs the fold
// count up to a known depth. Zero leaves the authored budget alone.
struct DeLod {
    float iters;
    float minDist;
    float cap;
};

// What the estimator learned about the point it was asked about. Returned to the
// one caller that asked, so no probe can reach another's copy.
//
// `depth` is not an orbit trap: a stack of conformal folds lets the orbit run
// away, and normalising a trap by a derivative that grows like
// scale-to-the-iteration drives it to zero, so the minimum stops describing the
// structure and merely re-describes the solid. That was diagnosed the hard way
// once already.
//
// Nor is it the raw escape count, which was the first attempt here and rendered
// flat: a point on the boundary is by definition one whose orbit does *not*
// escape, so the count sits at the cap across nearly the whole visible surface.
//
// What does vary, point to point, is how hard the stack magnified the
// neighbourhood — deep in a crevice is many more folds of magnification than out
// on a face. That is `log2(dr)`, and it is the smooth structural depth below.
//
// `folds` is how many folds the point survived before escaping. Unlike the
// others it is not a shading term; it is what the iteration fog integrates
// against.
struct DeInfo {
    float depth;
    float radius;
    float folds;
};

// The formula stack: one switch, one loop, slots visited round-robin.
//
// The slots interleave rather than running one after another, and that is the
// whole design. Applying them in sequence — all of slot 1, then all of slot 2 —
// makes the last slot the only one that shapes the surface and reduces the rest
// to a pre-transform, which renders as a smooth lump with texture painted on it.
// Cycling through them is what Mandelbulb3D does and what makes the *order* of a
// stack matter, which in turn is what makes permuting slots a creative act
// rather than a reshuffle. Slot weights are repeat counts within one cycle, so a
// heavy slot crowds the others out of the iteration budget: the graduated form of
// the trick where an absurd count on one slot bypasses those after it.
float stackDE(vec3 pos, DeLod lod, out DeInfo info) {
    vec3 p = pos;
    float dr = 1.0;

    // The cutoff is a floor on feature size as much as a count, and each fold is
    // roughly a halving of it, so a doubling of zoom needs one more fold to keep
    // arriving at new structure instead of at a smoothed version of the old.
    int total = int(clamp(stack_cap + g_foldBoost, 1.0, float(MAX_STACK_ITERS)));
    if (lod.cap > 0.0) {
        total = int(clamp(lod.cap, 1.0, float(MAX_STACK_ITERS)));
    }

    // Slot order as a permutation, not as a rewrite of the slots.
    //
    // Reordering a stack is not a variation on it. Each formula folds the space
    // the next one sees, so the same four in a different order build a different
    // object. In the reference archaeology the interesting mutation between two
    // files an hour and a half apart is that the first two slots are swapped and
    // nothing else changed. Expressed only as four dropdowns, that move requires
    // reading four values and writing them back in a new order, which makes it
    // awkward by hand and unreachable by a randomize — so it is a control.
    ivec4 order = stackOrder(stack_order);
    int forms[4] = int[4](slot0_formula, slot1_formula, slot2_formula, slot3_formula);
    int rates[4] = int[4](
        int(clamp(slot0_iters, 0.0, 8.0)),
        int(clamp(slot1_iters, 0.0, 8.0)),
        int(clamp(slot2_iters, 0.0, 8.0)),
        int(clamp(slot3_iters, 0.0, 8.0))
    );
    int r0 = rates[order.x];
    int r1 = rates[order.y];
    int r2 = rates[order.z];
    int r3 = rates[order.w];
    int c1 = r0 + r1;
    int c2 = c1 + r2;
    int cycle = c2 + r3;
    if (cycle < 1) return 1.0; // every slot silenced: nothing to march

    // Julius animates the lin-combine X multiplier to get the liquid stacking
    // motion; this is that, as a phase accumulator so it survives tempo changes.
    //
    // A triangle rather than a sine. A sine's excursion is slowest at its
    // extremes, so it parks there for most of the cycle — and the extremes are
    // exactly where a formula animation stops being useful: the reference read on
    // this run is that it goes liquid, then flattens, then turns noisy, and "here
    // it gets very noisy, which is basically where the animation ends". A
    // triangle gives every value on the way equal time, and `Evolve Depth` is
    // bounded short of the flat end rather than reaching it.
    float tri = asin(sin(PHASE_TIME_1)) * 0.63662; // ±1, uniform dwell
    float swing = tri * evolve_amount;

    // The swing drives exactly one parameter, and `Evolve Target` picks which.
    //
    // Every shot in the reference work has a single degree of freedom in motion:
    // "just the Julia parameter that is animating", "just the polyfold is moving
    // and that's actually all that's animated here", "the only thing animated
    // except the camera is the X multiplier from the first slot". So one target,
    // not several.
    //
    // Which one has to be a control rather than a fixed choice, because the
    // parameter has to be live in the stack that is actually loaded. This drove
    // the lin-combine multiplier unconditionally at first, matching the reference
    // run directly, and the result was a speed slider that did nothing at all:
    // `lin_x` is read only by the Lin Combine slot, no default slot is one, so the
    // whole evolve path was dead until the user happened to load that formula.
    // Fold scale is the default target instead, since every folding formula in
    // the stack reads it. The two targets that still carry a precondition say so
    // in their labels: `lin_x` needs a Lin Combine slot loaded and the seed needs
    // `Julia Seed` off zero, and a target that silently does nothing is the whole
    // bug being fixed here.
    float sScale  = (evolve_target == 0) ? swing : 0.0;
    float sOffset = (evolve_target == 1) ? swing : 0.0;
    // Radians want a wider throw than a length does to read as the same amount of
    // motion.
    float sRot    = (evolve_target == 2) ? swing * 3.0 : 0.0;
    float sLin    = (evolve_target == 3) ? swing : 0.0;
    float sJulia  = (evolve_target == 4) ? swing : 0.0;

    float linx = lin_x + sLin;

    // Julia: the folding formulas add a constant back each iteration instead of
    // the sample point. The escape-time set of a fixed seed rather than the
    // parameter-space set, which is the axis he animates when he wants the
    // structure itself to move.
    vec3 jseed = vec3(julia_x + sJulia, julia_y, julia_z);
    vec3 seed = mix(pos, jseed, julia_amount);
    // The `+ 1.0` on those two derivatives is the derivative of the term added
    // back, which is one for the sample point and zero for a constant. Same
    // reasoning as the Amazing Box below, which adds a uniform and so carries no
    // constant term at all.
    float seedDeriv = 1.0 - julia_amount;

    // Squared, because the test is against `dot(p, p)`. The default of 32 is the
    // 1024 this used to hardcode, so an existing preset is unchanged. Generous
    // by default, because a stack of folds can throw a point a long way out and
    // still bring it back; pulling it in cuts the set off early, which is a
    // different knob from the iteration count and moves the surface rather than
    // its detail.
    float bail2 = bailout * bailout;

    mat2 ra = rot(rot_a + sRot);
    mat2 rb = rot(rot_b);
    mat2 rc = rot(rot_c);
    mat2 rxw = rot(wPlaneAngle(rot_xw));
    mat2 ryw = rot(wPlaneAngle(rot_yw));
    mat2 rzw = rot(wPlaneAngle(rot_zw));
    vec3 off = vec3(offset_x + sOffset, offset_y, offset_z);
    float minR2 = min_radius * min_radius;
    float fixR2 = fixed_radius * fixed_radius;
    // Animated through the same value the derivative chain uses, so a moving
    // scale stays consistent with the distance bound rather than drifting away
    // from it.
    float sc = scale + sScale;
    float pw = max(power, 2.0);

    int done = 0;
    // The coarse answer, frozen at the fold where the cutoff starts to bite, and
    // how much of it to keep. See the blend after the loop.
    float dCoarse = -1.0;
    float wCoarse = 0.0;
    for (int i = 0; i < MAX_STACK_ITERS; i++) {
        if (i >= total) break;
        // Level of detail: once a fold's features are finer than a pixel, further
        // iterations are aliasing rather than detail.
        //
        // Faded across a fold rather than cut at one, and this is what the camera
        // stutter was.
        //
        // A hard break makes the *number of folds* a step function of the
        // viewpoint, and a fold is not a small change: the solid a truncated stack
        // describes is visibly smoother than the one with a further fold on it. So
        // every point in the frame jumped between two different surfaces as the
        // camera moved, and no amount of dithering the threshold fixes that,
        // because dithering only decides *where* the jump lands, never that it is a
        // jump. That is why the artifact read as the geometry snapping rather than
        // as noise: it was the geometry snapping.
        //
        // Instead: when the cutoff first touches this point, keep the distance the
        // truncated stack would have reported, let exactly one more fold run, and
        // blend the two by how far into the transition the point sits. The fold
        // count is still an integer; the surface no longer is, so it slides from
        // one level of detail to the next instead of arriving at it.
        if (lod.iters > 0.0) {
            float feature = 1.0 / max(abs(dr), 1e-10);
            float w = clamp((lod.iters * 1.6 - feature) / (lod.iters * 0.8), 0.0, 1.0);
            if (w > 0.0) {
                if (dCoarse < 0.0) {
                    dCoarse = 0.62 * length(p) / max(abs(dr), 1e-8);
                    wCoarse = w;
                } else {
                    break;
                }
            }
        }

        int c = i % cycle;
        int f;
        int which;
        if (c < r0) { which = order.x; }
        else if (c < c1) { which = order.y; }
        else if (c < c2) { which = order.z; }
        else { which = order.w; }
        f = forms[which];

        if (f == 1) {
            // Mandelbox. Box fold, inverse sphere fold, scale about the sample
            // point. Tom Lowe's, and the workhorse of architectural fractals.
            p = clamp(p, -fold_limit, fold_limit) * 2.0 - p;
            float r2 = dot(p, p);
            if (r2 < minR2) {
                float m = fixR2 / minR2;
                p *= m; dr *= m;
            } else if (r2 < fixR2) {
                float m = fixR2 / r2;
                p *= m; dr *= m;
            }
            p = p * sc + seed;
            dr = dr * abs(sc) + seedDeriv;
        } else if (f == 2) {
            // Amazing Box, Kali's. Folding two axes rather than three is what
            // opens it out into colonnades instead of closed cells.
            //
            // No `+ 1.0` on the derivative, unlike the Mandelbox above. That
            // term is the derivative of whatever is added back after the scale,
            // and it is one only when that is the sample point: here it is
            // `off`, a uniform, whose derivative is zero. Carrying it anyway was
            // conservative (an inflated `dr` shortens the step, so nothing
            // oversteps) but it was paid for in march steps, and it skewed
            // `log2(dr)` — which is the palette and the buried term of the AO,
            // so the error was visible as well as slow. Classic Amazing Box adds
            // the seed rather than a constant; swapping `off` for `pos` here
            // would restore the `+ 1.0` and give a genuinely different formula.
            p.xy = clamp(p.xy, -fold_limit, fold_limit) * 2.0 - p.xy;
            float r2 = dot(p, p);
            float m = sc / clamp(r2, minR2, fixR2);
            p = p * m + off;
            dr = dr * abs(m);
        } else if (f == 3) {
            // Menger sponge as a fold: sort the components into one octant, then
            // scale about a corner. Reads as lattice and coffering.
            p = abs(p);
            if (p.x < p.y) p.xy = p.yx;
            if (p.x < p.z) p.xz = p.zx;
            if (p.y < p.z) p.yz = p.zy;
            p = p * sc - off * (sc - 1.0);
            if (p.z < -0.5 * off.z * (sc - 1.0)) p.z += off.z * (sc - 1.0);
            dr *= abs(sc);
        } else if (f == 4) {
            // Sierpinski tetrahedron, Knighty's kaleidoscopic form. The three
            // conditional folds are the symmetry planes of the tetrahedron.
            if (p.x + p.y < 0.0) p.xy = -p.yx;
            if (p.x + p.z < 0.0) p.xz = -p.zx;
            if (p.y + p.z < 0.0) p.zy = -p.yz;
            p = p * sc - off * (sc - 1.0);
            dr *= abs(sc);
        } else if (f == 5) {
            // Mandelbulb, spherical form at arbitrary power. Not conformal, so
            // the derivative is the usual scalar approximation.
            float r = length(p);
            if (r > 2.0) { done = i; break; }
            float th = acos(clamp(p.z / max(r, 1e-6), -1.0, 1.0));
            float ph = atan(p.y, p.x);
            dr = pow(r, pw - 1.0) * pw * dr + seedDeriv;
            float zr = pow(r, pw);
            th *= pw;
            ph *= pw;
            p = zr * vec3(sin(th) * cos(ph), sin(th) * sin(ph), cos(th)) + seed;
        } else if (f == 6) {
            // Pseudo-Kleinian. Box fold plus a strict inversion, which is what
            // produces the deep nested cells. `off` is a uniform, so as with
            // the Amazing Box above the derivative carries no constant term.
            p = clamp(p, -fold_limit, fold_limit) * 2.0 - p;
            float r2 = dot(p, p);
            float m = 1.0 / max(r2, minR2);
            p = p * m - off;
            dr = dr * m;
        } else if (f == 7) {
            // Lin combine XYZ. A linear recombination with cross-axis bleed;
            // animating the X multiplier is what gives the liquid stacking.
            //
            // The bound below is the max row sum, the infinity norm. That is
            // *not* in general a Euclidean Lipschitz constant — three rows of
            // (1,0,0) give an infinity norm of 1 against a spectral norm of
            // sqrt(3) — and it is only valid here because of the matrix's exact
            // shape: one diagonal and one bleed term per row *and* per column,
            // so the one norm and the infinity norm are equal, and
            // ‖A‖₂ ≤ sqrt(‖A‖₁‖A‖∞) closes the gap. Give `lin_mix` a per-axis
            // value, or add a second off-diagonal term, and that equality fails
            // along with the bound, and the march starts overstepping.
            p = vec3(
                p.x * linx + p.y * lin_mix,
                p.y * lin_y + p.z * lin_mix,
                p.z * lin_z + p.x * lin_mix
            );
            dr *= max(max(abs(linx), abs(lin_y)), abs(lin_z)) + abs(lin_mix);
        } else if (f == 8) {
            // Ordered plane rotations. An isometry, so the derivative is
            // untouched; its whole effect is on what the next slot folds.
            p.xy = ra * p.xy;
            p.yz = rb * p.yz;
            p.xz = rc * p.xz;
        } else if (f == 9) {
            // Co-cube. A partial sort toward the cube diagonal rather than
            // Menger's full sort, plus a reflection on the last axis, which is
            // what gives it cells that interlock instead of nesting.
            p = abs(p);
            if (p.x < p.y) p.xy = p.yx;
            if (p.y < p.z) p.yz = p.zy;
            p.z = cocube - abs(p.z - cocube);
            p = p * sc - off * (sc - 1.0);
            dr *= abs(sc);
        } else if (f == 10) {
            // Rotate 4D. The reference stack runs one of these with two of its
            // six rotation planes at exactly 180 degrees, and it is not a
            // variation on the 3D rotate above.
            //
            // Three of the six planes involve a fourth coordinate the estimator
            // never sees, so what the geometry gets is the spatial block of the
            // rotation, and that block is not itself a rotation. At 180 degrees
            // it negates a single axis — a reflection, which SO(3) cannot
            // produce at all, since every 3D rotation flips axes in pairs. In
            // between it contracts along one, collapsing a direction rather than
            // turning it. Either way it folds space in a way the 3D rotate has
            // no access to, which is why his stack keeps one.
            //
            // Discarding the fourth component can only ever shorten `p`, so
            // leaving `dr` alone overstates the derivative. That is the safe
            // direction: an inflated derivative shortens the march step, so
            // nothing oversteps.
            //
            // Only the three planes through the hidden axis. The spatial three
            // belong to the 3D rotate, and between the two slots all six of a 4D
            // rotation's planes are available — stack both to get the full
            // group. Sharing the spatial angles with the 3D rotate instead was
            // the first attempt and it was worse than redundant: with `Rotate XY`
            // already sitting at 180 the two negations cancelled the two the 4D
            // rotate contributed, and the whole slot evaluated to the identity.
            vec4 q = vec4(p, 0.0);
            vec2 xw = rxw * vec2(q.x, q.w); q.x = xw.x; q.w = xw.y;
            vec2 yw = ryw * vec2(q.y, q.w); q.y = yw.x; q.w = yw.y;
            vec2 zw = rzw * vec2(q.z, q.w); q.z = zw.x; q.w = zw.y;
            p = q.xyz;
        }
        // f == 0 is Off, and falls through untouched so a slot can be silenced
        // without losing the parameters set on it.

        done = i + 1;
        if (dot(p, p) > bail2) break;
    }

    info.depth = clamp(log2(max(abs(dr), 1.0)) / 22.0, 0.0, 1.0);
    info.radius = clamp(log2(1.0 + dot(p, p)) / 10.0, 0.0, 1.0);
    info.folds = float(done);

    // The solid, not a shell: this is an object to fly around and through, so the
    // escape-time readout is what is wanted. A stack mixing conformal folds with
    // the non-conformal bulb has no exact estimator, which is also true of
    // Mandelbulb3D; the fudge factor is what keeps it from overstepping.
    float d = 0.62 * length(p) / max(abs(dr), 1e-8);
    // The level-of-detail crossfade. `wCoarse` slides from 0 to 1 as the point
    // moves into the transition, so the returned surface slides with it.
    if (dCoarse >= 0.0) d = mix(d, dCoarse, wCoarse);
    // Sub-pixel detail is aliasing noise, so the estimate never claims to
    // resolve finer than the footprint it was given.
    if (lod.minDist > 0.0) d = max(d, lod.minDist * 0.25);
    return min(d, 1.0);
}

// For the probes that only want a distance. The readouts are still computed and
// still land somewhere, but that somewhere is a local this drops on return,
// which is the whole point.
float stackDist(vec3 p, DeLod lod) {
    DeInfo ignored = DeInfo(0.0, 0.0, 0.0);
    return stackDE(p, lod, ignored);
}

vec3 calcNormal(vec3 p, float eps, DeLod lod) {
    // The floor guards against catastrophic cancellation when the difference step
    // is small enough that `p + e` rounds back to `p`, so it belongs to the
    // magnitude of `p` rather than to the world. Held absolute at 2e-5 it was
    // correct near unit distance and wrong at both ends: it overrode the intended
    // step at the bottom of a dive, where the marched footprint reaches about
    // 1.5e-5, and it was too small to guard anything out near the march limit.
    float e = max(eps, max(length(p), 1e-2) * 2e-6);
    vec2 h = vec2(e, -e);
    vec3 n = h.xyy * stackDist(p + h.xyy, lod)
           + h.yyx * stackDist(p + h.yyx, lod)
           + h.yxy * stackDist(p + h.yxy, lod)
           + h.xxx * stackDist(p + h.xxx, lod);
    float len = length(n);
    return (len > 1e-20) ? n / len : vec3(0.0, 1.0, 0.0);
}

// The start offset and the step bounds are fractions of the ray's own length,
// not world constants.
//
// This was the last absolute length left in the file, and it failed the same way
// everything else did before conversion. `tmax` is camera-scaled, so at full
// approach depth it is around 0.07 while the ray still started at 0.02 and
// refused to step below 0.01: a quarter of its own length before the first
// sample, then five steps of thirteen percent. Shadows degraded to noise exactly
// where the dive is most detailed. The fractions below are the old constants
// divided by `tmax` at the orbit's outer radius, so a parked camera renders as it
// did. `h / t` is a ratio and needed no conversion.
// The `32` is the penumbra hardness: the ratio of blocker distance to travelled
// distance at which a tap counts as fully shadowing. It was 12, which spreads
// every edge into a wide gradient, and a fractal's shadows are the one place its
// structure reads as structure rather than as texture — the reference frames get
// their sense of mass from shadows with edges. Higher is harder.
float softShadow(vec3 origin, vec3 dir, float tmax, DeLod lod) {
    float res = 1.0;
    float t = tmax * 0.0034;
    for (int i = 0; i < 20; i++) {
        float h = stackDist(origin + dir * t, lod);
        res = min(res, 32.0 * h / t);
        t += clamp(h, tmax * 0.0017, tmax * 0.051);
        if (res < 0.02 || t > tmax) break;
    }
    return clamp(res, 0.0, 1.0);
}

// The camera, shared by both passes. Pass 1 needs the basis too: it has to know
// where the key light falls in frame to put its bright side on the same side the
// shading lit, and a second copy of this arithmetic would drift out of step with
// this one the first time either was touched.
// The direction the eye sits in from the point it is diving at.
vec3 orbitDir(float azOffset, float elOffset) {
    float az = cam_azim + azOffset;
    float el = clamp(cam_elev + elOffset, -1.45, 1.45);
    return vec3(cos(az) * cos(el), sin(el), sin(az) * cos(el));
}

// Where the descent converges.
//
// This is the change that makes an endless zoom possible at all, and the bug it
// fixes is the same one that made a deep dive arrive at a blob. The eye position
// used to be `viewScale() * orbitDir`, which converges on the world origin no
// matter what is authored, and the origin is the symmetry centre of every folding
// formula in the stack. So every dive on every structure fell toward the same
// mirror-symmetric point and bottomed out on the same kaleidoscopic lump, and
// `Aim` could not help because it only rotated the view. Julius' cathedrals are
// off-centre boundary points, where the folds are not mirrored about the axis of
// travel.
//
// The target has to sit on the boundary. Inside the solid the descent ends buried;
// out in open space it ends in nothing. So it is not authored directly: a probe
// ray is cast from the parked eye toward the aim point and the target is the first
// surface it meets, which is on the boundary by construction. `Aim` steers that
// probe, so it is both the automatic behaviour and the manual override, and there
// is no combination of settings that produces an invalid target.
//
// Two details matter. The probe starts from `cam_dist` and from `cam_azim`
// and `cam_elev` *without* the orbit or the sway, so the target holds still while
// the camera revolves around it and descends toward it — a target recomputed from
// the live eye would chase itself. And the probe carries no per-pixel jitter and a
// fixed coarse footprint, so every pixel in the frame agrees on where it is.
vec3 diveTarget() {
    vec3 ro0 = cam_dist * orbitDir(0.0, 0.0);
    vec3 aim = vec3(look_x, look_y, look_z) * 0.5;
    vec3 rd0 = aim - ro0;
    float len = length(rd0);
    if (len < 1e-5) return aim;
    rd0 /= len;

    float eps = max(cam_dist, 0.1) * 0.002;
    DeLod lod = DeLod(eps * 4.0, eps, 0.0);
    float t = 0.0;
    for (int i = 0; i < 48; i++) {
        float d = stackDist(ro0 + rd0 * t, lod);
        if (d < eps) return ro0 + rd0 * t;
        t += d;
        if (t > MAX_DIST) break;
    }
    // The probe found nothing. Falling back to the aim point keeps the frame
    // sensible rather than parking the camera at an arbitrary distance.
    return aim;
}

// The basis alone, which needs no target: the eye sits along `orbitDir` from the
// target and looks straight back down it, so forward is that direction negated.
// Pass 1 wants only the basis, to find where the key light falls in frame, and
// keeping the target out of here is what stops the composite paying for a probe
// march it has no use for.
void cameraBasis(out vec3 fw, out vec3 ri, out vec3 upv) {
    fw = -orbitDir(PHASE_TIME_2, sin(PHASE_TIME_3 * 0.31) * sway_amount * 0.2);
    vec3 cx = cross(vec3(0.0, 1.0, 0.0), fw);
    ri = (length(cx) > 1e-4) ? normalize(cx) : vec3(1.0, 0.0, 0.0);
    upv = cross(fw, ri);
}

// Camera shake, on the image plane so it cannot walk the eye into geometry the
// way a positional sway can. Shared, because pass 1 has to undo exactly what pass
// 0 applied: it projects the key direction back to a pixel, and a projection that
// ignores the shake converges the shafts on a point that sits still while the
// frame moves under it.
vec2 swayOffset() {
    return vec2(sin(PHASE_TIME_3), cos(PHASE_TIME_3 * 0.73)) * sway_amount * 0.06;
}

vec3 keyDirection() {
    return normalize(vec3(
        cos(light_azim) * cos(light_elev),
        sin(light_elev),
        sin(light_azim) * cos(light_elev)
    ));
}

float boundingSphere(vec3 ro, vec3 rd, float r) {
    float b = dot(ro, rd);
    float c = dot(ro, ro) - r * r;
    float h = b * b - c;
    if (h < 0.0) return -1.0;
    return -b - sqrt(h);
}

// The high-energy channel. This is the RGB/Colorama pass of the reference
// workflow: a saturated ramp that exists to be a matte for the bloom in pass 1,
// not to be looked at directly.
vec3 accentRamp(float k) {
    return 0.5 + 0.5 * cos(6.28318 * (k + vec3(0.0, 0.33, 0.67)));
}

// Band-limited form, given the ramp argument's screen-space rate of change.
//
// A cyclic hue remap fed a quantity that moves by a whole cycle or more between
// adjacent pixels does not merely alias, it aliases far worse than the geometry
// it decorates: a sub-pixel shift in the surface swings the hue right around the
// wheel, so a smooth structure comes back as coloured stipple. The ramp here is
// driven by the log of the estimator's running derivative, which is about the
// noisiest signal in the shader, and the reference workflow renders its Colorama
// layer at double resolution for precisely this reason.
//
// Supersampling converges on the mean of whatever cycles a pixel straddles, and
// the mean of this ramp over a full cycle is a flat 0.5 in every channel. Fading
// there as the rate approaches a cycle arrives at the same answer without the
// extra samples. The rate has to be measured by the caller, in uniform control
// flow: a screen-space derivative taken inside the surface-hit branch is
// undefined for any quad straddling a silhouette.
vec3 accentRampBandLimited(float k, float rate) {
    return mix(accentRamp(k), vec3(0.5), smoothstep(0.20, 0.80, rate));
}

// Muted albedo. Deliberately desaturated and kept off both ends of the range:
// the grade in pass 1 is where saturation and contrast are meant to come from,
// and a render that has already clipped has nothing left to grade.
vec3 surfaceAlbedo(float depth, float rad, float zk) {
    vec3 c = mix(color_cool.rgb, color_warm.rgb, smoothstep(0.08, 0.85, depth));
    // How far the orbit travelled is a second, largely independent axis, so it
    // separates structures that happen to sit at the same fold depth.
    c = mix(c, accent_color.rgb, smoothstep(0.45, 0.95, rad) * 0.3);
    // There was a third axis here, a light tint from the slot the orbit ended in,
    // and it was removed rather than retuned. The slot the orbit *ended* in is
    // very nearly a constant: a point that runs its whole iteration budget always
    // exits at the same position in the cycle, and that is most of the visible
    // surface. Only the level-of-detail cutoff, firing at varying iterations, gave
    // it any variation at all — which means the axis was reporting the render
    // budget more than the structure, and paying for a per-point readout to do it.

    // Colour banded in view depth, which is what makes the object worth flying
    // into rather than a single scheme repeated all the way in. The three
    // authored colours are keyed to structure — fold depth and orbit radius —
    // and structure is self-similar, so they recur at every scale and the whole
    // approach is one palette however far the camera travels. Banding on
    // distance instead cuts across that: near and far cells of the same
    // structure take different hues, and each one changes as it comes toward
    // camera, so a fly-through moves through the spectrum.
    if (depth_palette > 0.0) {
        // The bands drift at a fixed fraction of the evolve accumulator, which
        // means they are parked whenever formula motion is. That is deliberate
        // now that `Formula / Energy Speed` defaults to zero: at defaults the
        // camera is the only thing moving, and hue that crawled under a still
        // structure would be a second motion competing with it. Bring evolve up
        // and the structure and the bands move together, as one gesture.
        //
        // A fader on this rate would have to multiply an ever-growing phase,
        // which jumps the bands whenever it moves; all four phase slots are
        // already spoken for, so there is no accumulator to fold such a rate
        // into with MULTIPLY_BY. See docs/12-isf-authoring.md § Combining two
        // rates.
        vec3 band = accentRamp(zk * depth_cycles + PHASE_TIME_1 * 0.04);
        // Applied at matched luminance. Mixing straight to the ramp would drag
        // the render's tonal structure around with the hue, and this pass is
        // meant to hand the grade a muted image whose luminance already carries
        // the lighting — a shading term is not the place to also decide
        // exposure.
        float lum = dot(c, vec3(0.299, 0.587, 0.114));
        float bandLum = max(dot(band, vec3(0.299, 0.587, 0.114)), 1e-3);
        c = mix(c, band * (lum / bandLum), depth_palette);
    }

    float grey = dot(c, vec3(0.299, 0.587, 0.114));
    return mix(c, vec3(grey), palette_mute);
}

// Below the horizon the sky is the sky from above it, folded back.
//
// The closing shot of the reference film does this by copying the star field top
// to bottom, and the reason it works is that a fractal is very often already
// symmetric about the same plane: nothing in frame is actually reflecting
// anything, but with the background agreeing, the whole thing reads as a
// mirror-flat lake. Interpolated rather than switched, so it can be brought in.
vec3 mirrorDir(vec3 rd) {
    return vec3(rd.x, mix(rd.y, abs(rd.y), horizon_mirror), rd.z);
}

float hash13(vec3 p) {
    p = fract(p * 0.1031);
    p += dot(p, p.yzx + 33.33);
    return fract((p.x + p.y) * p.z);
}

// Stars, so that negative space reads as space rather than as absence.
//
// This is the other half of why the reference frames survive projection. Domes
// and projectors cannot hold a bright field — "big bright swaths are just not
// very pretty", and almost every planetarium in the world is projection-based —
// so the background has to stay near black. A near-black background with points
// of light in it is legible; a near-black background with nothing in it is a
// dead screen.
//
// Cells on the direction sphere, at most one star each. The cell scale is chosen
// so a cell subtends several pixels at 1080p: a star smaller than a pixel does
// not read as a star, it crawls, and it would crawl in exactly the way the
// accent ramp used to.
const float STAR_CELLS = 120.0;

float starField(vec3 rd) {
    // A cell spans about 1/STAR_CELLS of a radian, and the frame height covers
    // roughly twice the field of view, so this is how many pixels wide a cell is.
    // Under a pixel or two the field stops being stars and becomes per-pixel
    // twinkle that crawls as the camera turns, so it is faded out instead. Fading
    // the amplitude rather than scaling the cells with resolution keeps the
    // pattern fixed to the sky: tying it to RENDERSIZE would slide every star the
    // moment a window was resized.
    float cellPx = RENDERSIZE.y / (STAR_CELLS * 2.0 * max(fov, 1e-3));
    float resolved = smoothstep(1.5, 3.0, cellPx);
    if (resolved <= 0.0) return 0.0;

    vec3 d = rd * STAR_CELLS;
    vec3 cell = floor(d);
    float h = hash13(cell);
    if (h < 0.975) return 0.0;
    // Jittered inside the cell, or the field reads as a lattice.
    vec3 j = vec3(hash13(cell + 7.1), hash13(cell + 19.3), hash13(cell + 31.7));
    float r = length(fract(d) - 0.5 - (j - 0.5) * 0.55);
    return smoothstep(0.42, 0.06, r) * (0.25 + 0.75 * fract(h * 137.0)) * resolved;
}

vec3 atmosphere(vec3 rd) {
    // Never pure black, and the gradient is a vertical one so the silhouette has
    // something to sit against. The floor is not decoration: the grade's shadow
    // lift and the bloom threshold both need a background above zero to act on,
    // and an audience reads a clipped black as a dead pixel.
    float h = 0.5 + 0.5 * mirrorDir(rd).y;
    return bg_color.rgb * (0.55 + 0.95 * h) + vec3(0.004, 0.005, 0.010);
}

// What a long path through the atmosphere converges to.
//
// The distinction from `atmosphere` matters more than it looks. Haze that mixes
// toward an unlit background only removes contrast, so distance reads as
// *vanishing* — which is how the frame ended up uniformly dark, with the near
// structure and the far structure both sitting on black and nothing separating
// the planes. Real atmosphere in-scatters: a longer path adds light. Giving the
// far field its own lift is what buys the dark-foreground-against-bright-
// background staging the reference look is built on, and it is the only reason
// the depth staging in pass 1 has anything to act against.
// The lift is its own control rather than a multiple of the haze amount, because
// the two answer different questions and the right answers disagree. How far the
// haze reaches is a depth cue. How bright it gets is an output decision: a lifted
// background is what stages depth on a flat screen, and it is the one thing a
// dome or a projector cannot take, since almost every planetarium in the world is
// projection-based and big bright swaths do not survive there. The default here
// is the projection-safe end; raise it for flat-screen work and the frame gains a
// great deal of depth.
vec3 hazeColor(vec3 rd) {
    return atmosphere(rd)
         + mix(color_cool.rgb, color_warm.rgb, 0.25) * atmos_lift * 4.0;
}

// What an escaped ray sees. Stars live here and not in `atmosphere`, which is the
// target the haze mixes surfaces toward: a star showing through hazed geometry is
// a star behind something opaque.
vec3 skyColor(vec3 rd) {
    return hazeColor(rd)
         + vec3(0.72, 0.82, 1.0) * starField(mirrorDir(rd)) * star_amount;
}

// Pass 0 — the beauty render, muted linear light with depth in alpha.
vec4 renderScene() {
    vec2 screen = vec2(uv.x, 1.0 - uv.y) * 2.0 - 1.0;
    screen.x *= RENDERSIZE.x / max(RENDERSIZE.y, 1.0);
    screen += swayOffset();

    // The zoom is established before anything else, because the probe march that
    // finds the dive target runs the estimator and the estimator reads the fold
    // budget this sets.
    g_zoom = viewZoom();
    // Dithered per pixel, for the reason the level-of-detail cutoff is: the floor
    // is a step function of time shared by every pixel, so the whole frame gained
    // a fold at one instant. Offsetting the hash keeps this decorrelated from the
    // cutoff's own jitter, which is drawn from the same coordinate.
    g_foldBoost = floor(log2(max(g_zoom, 1.0)) + hash12(uv * RENDERSIZE + 61.3));

    // An orbit around the point being dived at, rather than around the origin.
    // The structure stays framed by construction, which is what makes a randomize
    // land on something worth looking at instead of on the inside of a wall.
    vec3 fw, ri, upv;
    cameraBasis(fw, ri, upv);
    vec3 ro = diveTarget() - fw * viewScale();
    vec3 rd = normalize(fw + (screen.x * ri + screen.y * upv) * fov);

    vec3 keyDir = keyDirection();

    float pixel = 2.0 * fov / max(RENDERSIZE.y, 1.0);
    // 180 rather than 120, which is nearly free and not a coincidence: the march
    // is adaptive, so the extra budget is only ever spent by the rays that were
    // about to run out, and those were exactly the rays producing artifacts. Close
    // in among dense structure, 120 left thirteen percent of the frame
    // approximated at its closest approach; 180 leaves under half a percent, for
    // two to five percent more time.
    int steps = int(clamp(ray_steps, 40.0, 220.0));

    float t = 0.0;
    // Skip the empty space outside the structure when the camera is out there.
    if (dot(ro, ro) > BOUND_R * BOUND_R) {
        float bs = boundingSphere(ro, rd, BOUND_R);
        if (bs < 0.0) return vec4(skyColor(rd), 1.0);
        t = max(bs - 0.1, 0.0);
    }

    // The iteration cutoff is a hard break, so a threshold shared by every pixel
    // changes the iteration count along an exact iso-distance curve: concentric
    // rings of detail that sweep outward through the frame during a dolly.
    // Jittering it by a pixel-stable amount trades those rings for fine noise,
    // which the softening pass and the tonemap both bury.
    float lodJitter = mix(0.75, 1.3, hash12(uv * RENDERSIZE));

    float det = 0.0;
    float nearest = 1e6;
    float nearT = t;
    // Palette readouts at the closest approach, kept so a missed ray has a
    // continuous value to report where the accent ramp needs one. See the miss
    // branch below.
    float nearDepth = 0.0;
    float nearRad = 0.0;
    bool hit = false;
    // Whether the loop ended because the budget ran out rather than because the
    // ray left the volume. See the rescue below.
    bool exhausted = false;
    // Declared outside the loop so the readouts from the converging call survive
    // the break, which is what the surface branch shades from.
    DeInfo march = DeInfo(0.0, 0.0, 0.0);
    for (int i = 0; i < 220; i++) {
        if (i >= steps) { exhausted = true; break; }
        // Both the floor here and the threshold below are divided down by the
        // zoom, so `Detail` keeps meaning what it means at the orbit's outer
        // radius and tightens in step with the approach. Held absolute, they
        // become the resolution limit of the dive: past a few times closer the
        // march stops converging on anything the fold stack has newly revealed.
        float footprint = pixel * max(t, 0.35 / g_zoom);
        float d = stackDE(ro + rd * t, DeLod(footprint * lodJitter, footprint, 0.0), march);
        if (d < nearest) {
            nearest = d;
            nearT = t;
            nearDepth = march.depth;
            nearRad = march.radius;
        }
        // Converge to a pixel, and `Detail` says how many.
        //
        // This was an absolute length with two correction factors bolted on — an
        // exponential relaxation in `t` and a division by the zoom — and the
        // result was a march chasing detail below what the frame can hold. At
        // 1080p the threshold came out around a third of a pixel footprint out in
        // the far field, so every distant ray was asked to resolve structure three
        // times finer than the pixel it would be written to. It could not, it spent
        // its whole step budget failing, and the striping visible on a pulled-back
        // camera is the boundary between the rays that happened to converge and
        // the rays that ran out.
        //
        // The footprint already carries both corrections, and carries them
        // correctly: it is `pixel * t`, so it opens linearly with distance the way
        // a pixel's own world coverage does, and its near clamp is written against
        // the zoom. Expressing the threshold as a multiple of it collapses three
        // scale rules into one and makes `Detail` mean the same thing at every
        // distance, every zoom and every resolution: pixels of convergence, one at
        // the default, down to a third for a sharper and slower march, up to
        // nearly three for a softer and faster one.
        //
        // The floor the estimator puts under `d` is a quarter of a footprint, so
        // the threshold stays above it even at the sharpest setting, which is what
        // stops a ray creeping at the floor value until its budget runs out.
        det = footprint * max(detail / DETAIL_REF, 0.35);
        if (d < det) { hit = true; break; }
        t += max(d, det * 0.5);
        if (t > MAX_DIST) break;
    }

    // A ray that ran out of budget in a crevice got arbitrarily close without
    // converging. Shading it at its closest approach is much closer to the truth
    // than returning background through the middle of solid geometry.
    //
    // Every ray that ran out of budget, not only the ones that got close. The
    // proximity test alone left a large share of them reported as misses, and a
    // miss is sky: whole regions of solid structure returned background, and the
    // boundary between the rays that converged and the rays that gave up moved
    // with the camera. That is the difference between an artifact that reads as
    // noise and one that reads as geometry tearing, and it is why running out of
    // budget looked like clipping and stutter rather than like softness. A ray
    // that stopped with the volume still in front of it is inside the structure by
    // construction, so its closest approach is the best answer available and is
    // always a better one than sky.
    if (!hit && (exhausted || nearest < max(pixel * nearT * 6.0, 0.002 / g_zoom))) {
        hit = true;
        t = nearT;
        // Moving `t` is not enough. The readouts still hold the *last* call the
        // march made, which for a rescued ray is wherever it gave up —
        // arbitrarily far past the closest approach, and possibly out at the
        // march limit. Re-evaluating at the point actually being shaded is what
        // makes them describe it. The rescue fires preferentially in crevices, so
        // skipping this shades the frame's most detailed regions from an
        // unrelated point.
        float footprint = pixel * max(t, 0.35 / g_zoom);
        stackDE(ro + rd * t, DeLod(footprint * lodJitter, footprint, 0.0), march);
        // `det` sizes the normal's difference step and the shadow ray's offset,
        // and it too was left at the distance the ray gave up rather than the
        // one being shaded, which is always the larger of the two.
        det = footprint * max(detail / DETAIL_REF, 0.35);
    }

    vec3 col;
    float depth = MAX_DIST;
    // Written by the surface branch and consumed after it, so that the accent
    // ramp's band limiting can measure a derivative in uniform control flow.
    // Defined for every fragment, including the ones that missed.
    float seamArg = 0.0;
    float seamMask = 0.0;
    float haze = 0.0;
    if (hit) {
        depth = t;
        vec3 pos = ro + rd * t;
        float depth_k = march.depth;
        float rad_k = march.radius;

        // The normal is differenced from the same solid the march converged
        // against, so the iteration cutoff stays at the marched footprint.
        // Running normals at full depth instead puts the sub-pixel structure the
        // cutoff exists to drop straight back into the shading, which is the
        // crawling noise the level of detail was added to remove. The distance
        // floor is dropped for the same evaluation: it clamps the estimate to a
        // constant just inside the surface, and a difference across a constant
        // is a flat normal.
        float footprint = pixel * max(t, 0.35 / g_zoom);
        vec3 n = calcNormal(pos, max(det, 1e-5),
                            DeLod(footprint * lodJitter, 0.0, 0.0));

        // Occlusion from surface curvature: the fine normal against a normal
        // differenced over a much wider baseline. On anything locally flat the
        // two agree and this is zero; in a crevice the wide one has already
        // turned to follow the walls while the fine one still faces out, and the
        // angle between them is how enclosed the point is.
        //
        // This replaces a step-count proxy, `used / steps`, and the reason is
        // temporal rather than aesthetic. That proxy was a function of the whole
        // path the ray took to arrive, so it changed as the camera moved and the
        // shading swam over the geometry instead of belonging to it — and it also
        // moved when the `Ray Steps` performance fader was touched, which is a
        // render budget silently acting as a look control. It is the same failure
        // the reference workflow's rule about screen-space occlusion in animation
        // warns of: an occlusion term must be a property of the point, or it will
        // crawl. Curvature is such a property, so the same point returns the same
        // value from any camera.
        // The wide normal needs a wide iteration cutoff as well as a wide
        // baseline. Left at the marched footprint, the four tetrahedron probes sit
        // ten pixels apart and each one still resolves full sub-pixel structure,
        // so the result is not a smoothed normal, it is an undersampled one, and
        // `curve` inherits that as noise — at 2.2x gain, in exactly the crevices
        // the term exists to darken. Four cheaper estimator calls, too.
        vec3 nWide = calcNormal(pos, max(det, 1e-5) * 10.0,
                                DeLod(footprint * 10.0 * lodJitter, 0.0, 0.0));
        // Compared before the outward flip below, since flipping one and not the
        // other would read every surface as a crevice.
        float curve = clamp(1.0 - dot(n, nWide), 0.0, 1.0);

        if (dot(n, rd) > 0.0) n = -n; // concavities are the normal case here

        float buried = 1.0 - 0.6 * depth_k;
        float occ = mix(1.0, clamp(1.0 - 2.2 * curve, 0.0, 1.0) * buried, ao_strength);

        float key = clamp(dot(n, keyDir), 0.0, 1.0);
        // Shadows are only marched where they can be resolved; past that the
        // penumbra is finer than a pixel and the march is pure cost.
        // Faded out rather than switched off. A hard cutoff on distance is a
        // hard cutoff along an iso-distance surface, which draws a visible band
        // straight across the frame wherever it crosses geometry.
        // Shadows reach most of the frame now rather than the near third. The old
        // range faded them out past roughly twice the orbit radius, which on a
        // parked camera is a little beyond the structure's own front face, so the
        // shot had cast shadows only in its foreground and read flat behind that.
        float shadowFade = 1.0 - smoothstep(viewScale() * 2.5, viewScale() * 5.0, t);
        if (shadow_strength > 0.01 && shadowFade > 0.01) {
            // Twenty taps of the full stack is the most expensive block in the
            // pass, so it is marched coarser than the surface — but only twice as
            // coarse, not six times. Six was set on the reasoning that a penumbra
            // keeps no fine detail, which is true of the penumbra and false of the
            // *blocker*: the occluder's own silhouette is what a shadow is a
            // picture of, and a stack truncated six footprints early is casting
            // from a smoothed lump instead of from the structure on screen. That
            // is most of why these shadows carried no detail.
            //
            // The ray also starts twice `det` off the surface rather than four
            // times. `det` is around three times larger than it was now that the
            // march converges to a pixel, and the old multiplier on top of that
            // lifted the origin far enough to let light under the contact.
            float shadowFp = footprint * 2.0;
            key *= mix(1.0, softShadow(pos + n * det * 2.0, keyDir, viewScale() * 2.2,
                                       DeLod(shadowFp, shadowFp, 0.0)),
                       shadow_strength * shadowFade);
        }
        float fill = 0.5 + 0.5 * n.y;
        float rim = pow(clamp(1.0 + dot(rd, n), 0.0, 1.0), 3.0);

        vec3 albedo = surfaceAlbedo(depth_k, rad_k, depthEncode(t));

        // A key, a sky fill, a flat ambient that keeps the deepest recesses off
        // pure black, and a rim that separates near detail from far. Gains are
        // modest on purpose: the reference method's whole point is that the
        // render leaves headroom and the grade spends it.
        col  = albedo * key * color_warm.rgb * 1.6;
        col += albedo * fill * color_cool.rgb * 0.9 * occ;
        col += albedo * bg_color.rgb * 1.4;
        col += albedo * occ * 0.18;
        vec3 rimColor = mix(vec3(dot(color_cool.rgb, vec3(0.299, 0.587, 0.114))),
                            color_cool.rgb, 0.5);
        col += rimColor * rim * 0.24 * occ;

        // Selective highlights: the shader-native form of masking an RGB pass
        // with Colorama and multiplying its exposure. Only the deepest escapes
        // emit, so a small and structurally meaningful share of the frame carries
        // the high-energy colour and the bloom in pass 1 has something to find.
        //
        // Carried out of the branch rather than added here, so the ramp's
        // screen-space rate can be measured where derivatives are defined.
        seamArg = depth_k * 1.5 + rad_k * 0.4 + PHASE_TIME_1 * 0.05;
        // No occlusion and no shadow term. In the reference workflow the colour
        // layer is a second render of the same camera with ambient occlusion and
        // shadows switched off outright, and that is what makes it read as its
        // own light rather than as paint lying on the surface. This previously
        // scaled by the rim, which is view-dependent, so the accent breathed as
        // the camera turned past a surface it was supposed to be lighting.
        seamMask = smoothstep(0.5, 0.95, depth_k) * emissive * 0.6;

        haze = clamp(1.0 - exp(-t * 0.10 * fog_amount), 0.0, 1.0);
    } else {
        col = skyColor(rd);
        // Rays that nearly grazed the structure pick up its edge glow, which
        // keeps the silhouette from being a hard cut against the background.
        col += accent_color.rgb * exp(-nearest * 60.0 * g_zoom) * 0.3 * emissive;
        // The ramp argument continued across the silhouette rather than dropped
        // to zero, because the band limiting below measures `fwidth(seamArg)`
        // over the quad. At zero, any quad straddling an edge saw a
        // full-magnitude jump: the rate went large, the smoothstep saturated, and
        // the *hit* pixels in that quad were flattened to the ramp's mean. That
        // is a one-pixel desaturated fringe along every silhouette in frame, and
        // it landed on the brightest accents, since `seamMask` keys off the same
        // `depth_k` that peaks at edges.
        //
        // The closest-approach readouts are the right continuation: for a ray
        // grazing an edge, the nearest point is all but the surface point its
        // neighbour hit, so the rate the quad measures is the ramp's real
        // variation across the surface. Rays nowhere near the structure report
        // something arbitrary, and it costs nothing, because their `seamMask` is
        // zero and only a straddling quad's hit pixels read this at all.
        seamArg = nearDepth * 1.5 + nearRad * 0.4 + PHASE_TIME_1 * 0.05;
    }

    // Uniform control flow resumes here, which is the only place a screen-space
    // derivative of the ramp argument is well defined.
    vec3 seam = mix(accent_color.rgb,
                    accentRampBandLimited(seamArg, fwidth(seamArg)), 0.5);
    col += seam * seamMask;
    // Haze last, so it washes over the accent the way it does over everything
    // else. Zero on a missed ray, which already carries the haze colour.
    col = mix(col, hazeColor(rd), haze);

    // ── Fog on iteration ─────────────────────────────────────────────────────
    //
    // Distance fog is a function of how far light travelled and so is featureless
    // by construction: it can only ever be a smooth gradient. The wisps and
    // hanging banks in the reference renders come from a different quantity
    // entirely — fog keyed to the *fold count* at each point in space. Iso-fold
    // surfaces are the level sets wrapped around the structure at successive
    // removes, so a band picks out a shell that drapes over the geometry and
    // pools in its troughs, and moving the band one fold along lands on a
    // visibly different shape. There is no way to fake that from depth.
    //
    // On its own taps, not piggybacked onto the march, which was the first
    // attempt and does not work: the march runs with the iteration cutoff
    // engaged, so the fold count reports where the cutoff fired rather than how deep
    // the point actually is, and every band collapses onto the two or three folds
    // that open space escapes in — a flat wash at low settings and nothing at all
    // above them. Running these at full depth is also what makes the field a
    // property of space rather than of the camera, so the fog holds still while
    // the camera moves through it. For the same reason the iteration cutoff
    // cannot be coarsened here as a cost saving the way it is for shadows: the
    // cutoff is what corrupts the fold count, so coarsening it does not coarsen this
    // pass, it destroys it. The budget below is the saving that is available
    // instead, and it is exact rather than approximate.
    //
    // What the band picks out is a shell wrapped around the structure, so where
    // it sits relative to the surface is the whole look. Sitting on the surface
    // is the one placement to avoid: the reference note on this is "I usually
    // don't like it when they really, as I call it, hug the surface of something",
    // and a shell hugging the geometry renders as a halo around it rather than as
    // fog in the space. `Fog Iteration` is that placement, and the default was
    // originally set far too low. Sweeping it reproduces the reference experience
    // closely: below about four every point in open space qualifies and the fog
    // floods the frame, around six or seven it hangs in the space and pools in the
    // troughs, and by nine it has gone — "iteration six is another interesting
    // one. Seven is gone." A wide band reaches down into the flooding range from a
    // usable centre, so it is narrow by default too.
    if (fog_iter_amount > 0.0) {
        const int WISP_TAPS = 16;
        // Scene-scale reach, and the ray stops at whatever it hit. Both halves
        // of that matter: the fog is a volume the light crossed, so the only
        // honest length is the distance actually travelled through it.
        float far = min(hit ? t : MAX_DIST, fog_iter_reach);
        float dt = far / float(WISP_TAPS);
        // Offset by a pixel-stable fraction of a step, or every tap lands on the
        // same shells and the fog quantises into slabs.
        float jitter = hash12(uv * RENDERSIZE + 17.0);
        float wisp = 0.0;
        // The band is zero for any point that survived more folds than its top
        // edge, so counting past that edge cannot change the result. Capping the
        // budget there is lossless, and it turns the frame's most expensive block
        // from a full forty-iteration stack on every tap into about ten.
        DeLod fogLod = DeLod(0.0, 0.0, ceil(fog_iter + fog_iter_band) + 1.0);
        DeInfo tap = DeInfo(0.0, 0.0, 0.0);
        for (int i = 0; i < WISP_TAPS; i++) {
            stackDE(ro + rd * ((float(i) + jitter) * dt), fogLod, tap);
            wisp += max(1.0 - abs(tap.folds - fog_iter) / max(fog_iter_band, 0.35), 0.0);
        }
        // Composited over what the ray found, not added to it.
        //
        // Adding was the original reasoning — in-scattered light hanging in front
        // of a surface should only ever brighten it — and it is half of the
        // physics. A medium that scatters light toward the camera also blocks the
        // light coming from behind it, and dropping the extinction half is what
        // bleached this render: the fold shells sit directly in front of every
        // surface, so the brightest fog in frame landed on the structure and was
        // added to it, flattening the contrast that makes the geometry read.
        // That is the hazy halo, and mixing rather than adding is most of the
        // cure, because a blend toward the fog colour cannot exceed it.
        // The integral along the ray, `mean occupancy times distance travelled`,
        // and not the mean alone.
        //
        // The mean was tried, to stop the density falling away as a camera-scaled
        // reach shrank through a dive, and it is wrong in a way that is worth
        // recording: it makes the fog independent of how far the light actually
        // travelled through it. A ray that hits a wall a hundredth of a unit away
        // then receives exactly as much in-scattered light as one that crossed
        // seven units of open space, so every surface in frame gets the same
        // additive wash laid over it. That reads as a haze halo clinging to the
        // geometry and spilling past its silhouette, and it costs the render most
        // of its crispness — the structure is what the wash is brightest on.
        //
        // The reach is scene-scale for the same reason, which also settles the
        // inconsistency with the distance haze rather than arguing for it: fog is
        // a property of the world, so a dive really does leave less of it between
        // the camera and a surface a hundredth of a unit away. Thin fog on close
        // geometry is the correct answer, not a regression to be normalised out.
        float density = 1.0 - exp(-wisp * dt * fog_iter_amount * 1.5);
        col = mix(col, mix(color_cool.rgb, accent_color.rgb, 0.25) * 0.55, density);
    }

    return vec4(max(col, 0.0), depthEncode(depth));
}

vec3 sceneAt(vec2 c) {
    return texture(sampler2D(sceneBuffer, texSampler), clamp(c, vec2(0.0), vec2(1.0))).rgb;
}

float depthAt(vec2 c) {
    return texture(sampler2D(sceneBuffer, texSampler), clamp(c, vec2(0.0), vec2(1.0))).a;
}

float luma(vec3 c) {
    return dot(c, vec3(0.299, 0.587, 0.114));
}


// Pass 1 — the compositing pass. Every step here is one from the reference After
// Effects stack, done against a single marched frame rather than three separate
// renders: the beauty, the highlight matte and the Z-buffer are the channels of
// the buffer this reads.
vec4 cinematicPost() {
    vec2 aspect = vec2(RENDERSIZE.x / max(RENDERSIZE.y, 1.0), 1.0);
    vec2 centered = uv - 0.5;
    float radius = length(centered * aspect);

    // Softening that increases with distance, not a focus plane.
    //
    // This is deliberately *not* a lens model, and two lens models were built and
    // thrown away before landing here. A focus plane is bidirectional and mobile:
    // everything nearer than it blurs as well as everything beyond it, and the
    // plane itself has to be placed. A manual focus distance drifts off the
    // subject as soon as the camera orbits or breathes. Autofocusing on the middle
    // of frame replaces that with pumping, and cannot be fixed by sampling more
    // central pixels, because the middle of frame simply does not know about
    // something close in a corner.
    //
    // What the reference workflow actually does with a Z-buffer is closer to haze
    // than to focus: near is crisp, far is soft, monotonically. There is no plane
    // to place and none to drift, so nothing pumps, everything close reads sharp
    // wherever it sits in frame, and the aliasing and jitter on fine distant
    // geometry (the real reason the pass exists) still gets buried.
    float depth = depthAt(uv);
    // Straight through the same encoding the channel was written with, so the
    // onset is simply the world distance it reads as. It used to need a further
    // correction through the zoom, because a fixed world distance measured
    // against a linear, shrinking depth range walks off the end of the channel as
    // the camera closes and the far field snaps back to full sharpness. A log
    // channel never runs out of range, so there is nothing left to correct.
    float onset = depthEncode(dof_onset);
    float coc = smoothstep(onset, 1.0, depth) * dof_amount;
    vec3 focal = sceneAt(uv);
    float weight = 1.0;
    float spread = coc * 0.040;
    for (int i = 0; i < 12; i++) {
        float fi = float(i);
        float ang = fi * 2.39996; // golden angle, so 12 taps land evenly
        float r = sqrt((fi + 0.5) / 12.0) * spread;
        vec2 at = uv + vec2(cos(ang), sin(ang)) * r / aspect;
        // A tap only contributes if it is at least as far away as this pixel.
        // Because the blur is now monotonic in depth, "is behind" and "is at least
        // as soft" are the same test, so this one comparison is what stops crisp
        // near geometry from smearing outward and haloing over what is behind it.
        float w = step(depth - 0.004, depthAt(at));
        focal += sceneAt(at) * w;
        weight += w;
    }
    vec3 col = focal / weight;

    // How much of this pixel's sharpness the gather left intact.
    //
    // Everything below re-samples `sceneBuffer`, which is the untouched pass-0
    // image, so wherever the gather softened the frame they would paste crisp
    // detail straight back over it. On fine far geometry that detail *is* the
    // aliasing the gather exists to bury, and the aberration splits it into two
    // colour channels, so it returns as coloured sparkle at the frame edge —
    // exactly where the aberration's own radial weighting peaks. Scaling each of
    // them by what survived leaves them at full strength on sharp geometry and
    // out of the way where the frame has already given up its resolution.
    float sharp = 1.0 - coc;

    // Chromatic aberration, split along the radial direction and weighted to the
    // frame edge, so it reads as a lens rather than as a filter.
    if (aberration > 0.0) {
        vec2 dir = centered * (aberration * 0.012 * (0.25 + radius * radius));
        float edge = smoothstep(0.15, 0.75, radius) * sharp;
        col.r = mix(col.r, sceneAt(uv + dir).r, edge);
        col.b = mix(col.b, sceneAt(uv - dir).b, edge);
    }

    // Ghost reflection: the same frame at low opacity, slightly offset and
    // scaled. The duplicate-layer trick that gives the glassy, layered feel.
    //
    // Two copies, because the reference comp has two. One offset copy reads as a
    // smear of the image; a second at a different scale and the opposite offset
    // is what makes it read as glass, since real internal reflections come in
    // families rather than singly.
    if (ghost > 0.0) {
        col += sceneAt((uv - 0.5) * 0.985 + 0.5 + vec2(0.006, 0.003)) * ghost * 0.30 * sharp;
        col += sceneAt((uv - 0.5) * 1.022 + 0.5 - vec2(0.011, 0.005)) * ghost * 0.16 * sharp;

        // Matte box flare: a highlight thrown to the *opposite* side of frame.
        // A lens shade that catches light reflects some of it back in, and the
        // reflection lands point-mirrored through the optical axis. Distinct
        // from the offset copies above, which are near-coincident with the
        // image; this one is nowhere near it, and that is what reads as a
        // physical lens rather than as a double exposure. Highlights only, or it
        // is just the frame upside down.
        vec3 opposite = sceneAt(1.0 - uv);
        col += opposite * max(luma(opposite) - bloom_thresh, 0.0) * ghost * 0.55;
    }

    // Camera blur. The motion is an orbiting approach, so its screen-space smear
    // is radial out of the frame centre; no motion vectors needed to know that.
    if (motion_blur > 0.0) {
        vec3 streak = vec3(0.0);
        vec2 step_uv = centered * (motion_blur * 0.03);
        for (int i = 1; i <= 5; i++) {
            streak += sceneAt(uv - step_uv * (float(i) / 5.0));
        }
        col = mix(col, streak / 5.0, motion_blur * 0.45 * sharp);
    }

    // Local contrast, not global. A fractal frame is large swathes of similar
    // tone, and a contrast curve on the whole image only stretches the swathes
    // apart; what gives the structure definition is contrast *within* them. So
    // this is a high pass of the source added back, which is the clarity slider
    // of the reference workflow and the reason its stills read as detailed
    // without being sharpened to bits.
    //
    // Taken off the source rather than off `col`, because a high pass needs a
    // matching low pass and `col` has already been gathered — and weighted by
    // the sharpness that survived, or it would hand back the far-field aliasing
    // the gather just buried.
    if (clarity > 0.0) {
        vec3 low = vec3(0.0);
        for (int i = 0; i < 6; i++) {
            float ang = float(i) * 1.0471976;
            low += sceneAt(uv + vec2(cos(ang), sin(ang)) * 0.013 / aspect);
        }
        col += (sceneAt(uv) - low / 6.0) * clarity * sharp;
    }

    // Selective highlight bloom. The threshold is the luma matte and the gain is
    // the fourfold exposure multiply, which drives glow off the accents without
    // blowing out the base image under them.
    if (bloom > 0.0) {
        vec3 glow = vec3(0.0);
        for (int ring = 0; ring < 2; ring++) {
            float rr = (ring == 0) ? 0.011 : 0.030;
            for (int i = 0; i < 8; i++) {
                float ang = float(i) * 0.7853982 + float(ring) * 0.3927;
                vec3 s = sceneAt(uv + vec2(cos(ang), sin(ang)) * rr / aspect);
                glow += s * max(luma(s) - bloom_thresh, 0.0);
            }
        }
        // Weighted by sharpness like the other post terms, and for a stronger
        // reason than they have. Those merely undo the softening; this one
        // manufactures highlights that were never there, because thresholding
        // luminance on the *unblurred* buffer picks out exactly the single-pixel
        // aliasing on fine far geometry that the softening gather just buried.
        col += glow * (bloom * 4.0 / 16.0) * sharp;
    }

    // Light shafts from the key.
    //
    // There is no volumetric light in the render and this does not pretend to be
    // one: it is the radial-blur trick, smearing whatever is bright along the
    // line to the source. What makes it work is that the source is the actual key
    // direction projected into frame, so the shafts converge where the shading
    // says the light is and swing round with it as the camera orbits.
    //
    // Masked to the frame's negative space, because a shaft is atmosphere lying
    // *between* the camera and the source. Laid over lit geometry it reads
    // immediately as a filter, which is why the reference workflow mattes it.
    if (shafts > 0.0) {
        vec3 fw, ri, upv;
        cameraBasis(fw, ri, upv);
        vec3 kd = keyDirection();
        float ahead = dot(kd, fw);
        // Behind the camera there is nothing to converge on, and the projection
        // below would put the source at a mirrored phantom position in front.
        // Ramped rather than switched, so the term does not pop as the key
        // crosses the view plane.
        float front = smoothstep(0.05, 0.35, ahead);
        if (front > 0.0) {
            // Less the shake, since a pixel's ray was built from the shaken
            // image plane and this is that mapping run backwards.
            vec2 sun = vec2(dot(kd, ri), dot(kd, upv)) / (ahead * max(fov, 1e-3))
                     - swayOffset();
            vec2 sunUV = vec2((sun.x / aspect.x + 1.0) * 0.5, (1.0 - sun.y) * 0.5);

            const int SHAFT_TAPS = 14;
            vec2 delta = (sunUV - uv) / float(SHAFT_TAPS);
            vec2 at = uv;
            vec3 shaft = vec3(0.0);
            float w = 1.0;
            float wsum = 0.0;
            for (int i = 0; i < SHAFT_TAPS; i++) {
                at += delta;
                vec3 s = sceneAt(at);
                // A ramp rather than a hard threshold, and at a fifth of the
                // bloom's bar. Bloom picks out the few pixels that should flare;
                // a shaft is seeded by anything brighter than the fog it travels
                // through. This render is muted linear light by design and sits
                // an order of magnitude below the bloom threshold almost
                // everywhere, so seeding at that threshold produced a shaft term
                // small enough to be invisible at any gain.
                // Taps that leave the frame contribute nothing.
                //
                // `sceneAt` clamps its coordinate, so a tap past the edge returns
                // the border pixel — and when the source sits outside the view,
                // which is most of the time, an entire shaft's taps clamp to the
                // same border pixel. Normalising that by the weight it accumulated
                // scaled one edge pixel up to full brightness and smeared it the
                // length of the frame: the streaks, running whichever way the
                // off-screen source lay, and flaring as it moved.
                //
                // Dropped from the numerator while still counting toward the
                // total, so partial coverage fades the shaft out instead of
                // normalising a fragment of it back up, and the source leaving
                // frame is a fade rather than a smear.
                vec2 edge = step(vec2(0.0), at) * step(at, vec2(1.0));
                float inside = edge.x * edge.y;
                shaft += s * smoothstep(0.0, bloom_thresh * 0.20, luma(s)) * w * inside;
                wsum += w;
                w *= 0.90;
            }
            shaft /= max(wsum, 1e-4);
            // Admitted in proportion to how dim the pixel already is. A shaft
            // is only ever visible against something darker than itself; laid
            // over lit geometry it just brightens it, and gating on depth
            // instead put it exactly where the render is busiest and hid it.
            // Dark pixels are the negative space, which is where the reference
            // workflow masks it to as well.
            float room = 1.0 - smoothstep(0.02, 0.30, luma(col));
            col += shaft * shafts * 1.5 * room * front;
        }
    }

    // ── Composition ──────────────────────────────────────────────────────────
    //
    // A fractal is evenly lit and evenly busy by construction: the same kind of
    // structure at the same kind of brightness occurs everywhere in frame, at
    // every scale. That evenness, not any missing filter, is what makes a
    // straight fractal render read as amateur — there is no light side and dark
    // side, no near and far, nothing for an eye to be led around. The two terms
    // below exist purely to break it, and they are the same two the reference
    // workflow reaches for first.

    // Dark foreground against a lifted background. Cheap, and it does more for
    // the sense of depth than the softening does, because it separates planes by
    // tone rather than by focus.
    if (depth_stage > 0.0) {
        // Retuned for the log channel: surfaces land around 0.3 to 0.75 across
        // the whole dive and only the march limit reaches 1.0.
        float back = smoothstep(0.35, 0.90, depth);
        col *= mix(1.0 - 0.70 * depth_stage, 1.0 + 0.45 * depth_stage, back);
    }

    // A bright warm side and a fallen-away cool side, placed where the key light
    // actually is rather than at a fixed corner. Projecting the key direction
    // onto the camera basis is what ties it to the shading: the gradient sweeps
    // around the frame as the camera orbits, so it reads as a light in the scene
    // instead of a gradient laid over the lens.
    if (light_side > 0.0) {
        vec3 fw, ri, upv;
        cameraBasis(fw, ri, upv);
        vec3 kd = keyDirection();
        // `uv` runs downward here while the ray basis has up positive, hence the
        // flip. Only the direction matters, so this is normalised rather than
        // properly projected through the frustum.
        vec2 axis = vec2(dot(kd, ri), -dot(kd, upv));
        float axisLen = length(axis);
        axis = (axisLen > 1e-4) ? axis / axisLen : vec2(1.0, 0.0);

        float lit = smoothstep(-0.30, 0.35, dot(centered, axis));
        // Both tints are normalised to unit luminance so they shift hue without
        // shifting level, and the brightening and darkening are then stated
        // explicitly. Multiplying by the palette colours raw instead makes both
        // sides darker than neutral — these are dark, saturated colours — so the
        // control reads as a dimmer rather than as a light and a shadow.
        vec3 warmTint = color_warm.rgb / max(luma(color_warm.rgb), 1e-3);
        vec3 coolTint = color_cool.rgb / max(luma(color_cool.rgb), 1e-3);
        vec3 warmSide = mix(vec3(1.0), warmTint, 0.35) * (1.0 + 0.90 * light_side);
        vec3 coolSide = mix(vec3(1.0), coolTint, 0.25) * (1.0 - 0.45 * light_side);
        col *= mix(coolSide, warmSide, lit);
    }

    col *= exposure;

    // The look filter: violet lift in the shadows, warmth held in the highlights,
    // then a contrast curve. Applied in linear light and left unclamped, so the
    // highlights keep their headroom for Varda's tonemap.
    float l = luma(col);
    float shadowMask = 1.0 - smoothstep(0.0, 0.35, l);
    col = mix(col, col * vec3(0.95, 0.96, 1.12) + bg_color.rgb * 0.08,
              shadowMask * look_contrast);
    col *= mix(vec3(1.0), color_warm.rgb, smoothstep(0.5, 1.6, l) * look_contrast * 0.5);
    // Contrast about a mid-grey pivot rather than a bare gamma. A bare
    // `pow(col, 1.3)` is not a contrast control, it is a darkener: it holds 1.0
    // fixed and drags everything below it down, which took the atmosphere to
    // roughly a thousandth of its authored value and put pure black in frame —
    // the one thing the reference method's muted rule forbids.
    const float PIVOT = 0.18;
    col = PIVOT * pow(max(col, 0.0) / PIVOT, vec3(mix(1.0, 1.35, look_contrast)));
    col = mix(vec3(luma(col)), col, saturation);
    col *= mix(1.0, smoothstep(1.15, 0.2, radius), vignette);

    return vec4(max(col, 0.0), 1.0);
}

void main() {
    // Keeps the automatic uniforms this shader does not otherwise read from being
    // stripped out of the module, which would break the bind group.
    float keep = (audio_level + audio_bass + audio_mid + audio_treble
                + audio_bpm + audio_beat_phase + TIME + TIMEDELTA
                + float(FRAMEINDEX) + DATE.x + DATE.y + DATE.z + DATE.w) * 1e-9;

    if (PASSINDEX == 0) {
        vec4 scene = renderScene();
        fragColor = vec4(scene.rgb + keep, scene.a);
    } else {
        fragColor = vec4(cinematicPost().rgb + keep, 1.0);
    }
}
