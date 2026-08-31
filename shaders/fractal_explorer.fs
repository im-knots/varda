/*{
    "DESCRIPTION": "3D Fractal Explorer - a four-slot formula stack marched as a solid and finished in-shader. Each slot picks a distance estimator (Mandelbox, Amazing Box, Menger, Sierpinski, Mandelbulb, Pseudo-Kleinian, lin-combine, rotate, co-cube, 4D rotate, or off) and takes a share of a shared iteration budget; each fold shapes the space the next one sees, so Slot Order permutes the stack and starving the later slots restructures the geometry rather than merely retuning it. The camera approaches geometrically rather than linearly, and the detail threshold, depth range and fold count scale with it, so a dive keeps resolving new structure instead of arriving at a smooth blob. Two atmospheres: distance haze, and fog keyed to the fold count at each point in space, which drapes over the geometry and pools in its troughs. Rendered muted in linear light with depth in alpha, then composited the way a Mandelbulb3D frame normally is in After Effects: distance softening off the marched Z, selective highlight bloom, chromatic aberration, twin ghost reflections with a matte-box flare, key-aligned light shafts, radial camera blur, vignette and a look grade. The Composition group breaks the tonal evenness a fractal has by construction, with a lit side and a fallen-away side, a dark foreground against a lifted background, and local rather than global contrast. The palette is banded in view depth rather than fixed, so near and far parts of the structure take different hues and each one shifts as it comes toward camera, which is what gives a fly-through somewhere to go. Mutate the Stack group to hunt permutations; save a preset when one lands. The fractal and the viewpoint are both parked and the descent is the only thing in motion, because a shot reads best with a single degree of freedom and because the host reference orbit and its directional certificate are computed for one fixed viewpoint: Approach Speed paces the descent, Flight Depth sets how many decades it sweeps, and Zoom Decades parks it at one named depth for a still. The march converges to a pixel rather than to an absolute distance, so `Detail` reads as pixels of convergence: one at the default, down to a third for a sharper and slower march, up to nearly three for a softer and faster one. It therefore means the same thing at every distance, zoom and output resolution, and the march no longer chases structure finer than the frame can hold, which is what used to leave stripes on a pulled-back camera and torn holes of background through solid geometry up close. The fold cutoff crossfades across one fold instead of switching at one, so the surface slides between levels of detail as the camera moves rather than snapping between them. The sky defaults near black with a star field rather than a lifted haze, which is what projection and dome output need; Atmosphere Lift trades that for flat-screen depth staging. Horizon Mirror folds the sky back on itself below the waterline, and because a fractal is usually already symmetric there, the frame reads as a mirror-flat lake.",
    "CREDIT": "Varda VJ (Mandelbox after Tom Lowe; Amazing Box/Surface after Kali; kaleidoscopic Sierpinski after Knighty; slot-stack structure, lin-combine / rotate / co-cube slots and the iteration-cutoff bypass after Mandelbulb3D; muted-render, multi-pass beauty/RGB/Z compositing workflow and depth-pass-as-matte highlighting after Julius Horsthuis; circuit-trace treatment after alien_grove.fs)",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator", "Generative", "3D", "Fractal"],
    "PREPROCESSORS": [
        {"NAME": "refOrbit", "TYPE": "fractal_reference_orbit",
         "FORMAT": "rgba32float",
         "OPTIONS": {"max_iters": 512, "certificates": false,
                     "formulas": [5, 0, 0, 0], "rates": [1, 0, 0, 0],
                     "scale": 2.0, "fold_limit": 1.0,
                     "min_radius": 0.5, "fixed_radius": 1.0,
                     "offset": [1.0, 1.0, 1.0], "bailout": 32.0,
                     "cocube": 0.6, "lin": [1.0, 1.0, 1.0], "lin_mix": 0.2,
                     "rot": [3.14159, 0.0, 0.0],
                     "rot_w": [3.14159, 0.0, 0.0],
                     "anchor": ["0.35", "-0.21", "0.14"]},
         "PARAM_BINDINGS": {
             "formula0": "slot0_formula", "formula1": "slot1_formula",
             "formula2": "slot2_formula", "formula3": "slot3_formula",
             "rate0": "slot0_iters", "rate1": "slot1_iters",
             "rate2": "slot2_iters", "rate3": "slot3_iters",
             "stack_order": "stack_order", "max_iters": "dz_orbit_len",
             "stack_cap": "stack_cap",
             "scale": "scale", "fold_limit": "fold_limit",
             "min_radius": "min_radius", "fixed_radius": "fixed_radius",
             "power": "power", "offset_x": "offset_x",
             "offset_y": "offset_y", "offset_z": "offset_z",
             "lin_x": "lin_x", "lin_y": "lin_y", "lin_z": "lin_z",
             "lin_mix": "lin_mix", "rot_a": "rot_a", "rot_b": "rot_b",
             "rot_c": "rot_c", "rot_xw": "rot_xw", "rot_yw": "rot_yw",
             "rot_zw": "rot_zw", "cocube": "cocube", "bailout": "bailout",
             "julia_amount": "julia_amount", "julia_x": "julia_x",
             "julia_y": "julia_y", "julia_z": "julia_z",
             "zoom_exp": "dz_zoom_exp",
             "flight_max_exp": "dz_flight_max_exp"
         }}
    ],
    "INPUTS": [
        {"NAME": "fly_speed", "TYPE": "float", "DEFAULT": 0.35, "MIN": 0.0, "MAX": 3.0, "LABEL": "Approach Speed"},

        {"NAME": "slot0_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 5, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 1 Formula"},
        {"NAME": "slot0_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 1 Weight"},
        {"NAME": "slot1_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 2 Formula"},
        {"NAME": "slot1_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 2 Weight"},
        {"NAME": "slot2_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 3 Formula"},
        {"NAME": "slot2_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 3 Weight"},
        {"NAME": "slot3_formula", "TYPE": "long", "GROUP": "Stack", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], "LABELS": ["Off", "Mandelbox", "Amazing Box", "Menger Fold", "Sierpinski", "Mandelbulb", "Pseudo-Kleinian", "Lin Combine XYZ", "Rotate", "Co-Cube", "Rotate 4D"], "LABEL": "Slot 4 Formula"},
        {"NAME": "slot3_iters", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 8.0, "LABEL": "Slot 4 Weight"},
        {"NAME": "stack_cap", "TYPE": "float", "GROUP": "Stack", "DEFAULT": 14.0, "MIN": 1.0, "MAX": 128.0, "LABEL": "Iteration Cutoff"},
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

        {"NAME": "dz_absorb", "TYPE": "float", "GROUP": "Deep Zoom", "DEFAULT": 0.01, "MIN": 0.0, "MAX": 1.0, "LABEL": "Absorb Threshold"},
        {"NAME": "dz_zoom_exp", "TYPE": "float", "GROUP": "Deep Zoom", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 120.0, "LABEL": "Zoom Decades"},
        {"NAME": "dz_flight_max_exp", "TYPE": "float", "GROUP": "Deep Zoom", "DEFAULT": 8.0, "MIN": 0.0, "MAX": 120.0, "LABEL": "Flight Depth"},
        {"NAME": "dz_debug", "TYPE": "long", "GROUP": "Deep Zoom", "DEFAULT": 0, "VALUES": [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12], "LABELS": ["Off", "Fold count", "log10 derivative", "Resolution ratio", "Payload gate", "Certified prefix", "March telemetry", "Geometry only", "Normals", "March outcome", "DE field", "Exit cause", "Shading, no post"], "LABEL": "Diagnostic"},
        {"NAME": "dz_orbit_len", "TYPE": "float", "GROUP": "Deep Zoom", "DEFAULT": 512.0, "MIN": 1.0, "MAX": 4096.0, "LABEL": "Orbit Length"},

        {"NAME": "ray_steps", "TYPE": "float", "GROUP": "Render", "DEFAULT": 180.0, "MIN": 40.0, "MAX": 220.0, "LABEL": "Ray Steps"},
        {"NAME": "detail", "TYPE": "float", "GROUP": "Render", "DEFAULT": 0.0015, "MIN": 0.0005, "MAX": 0.006, "LABEL": "Detail"},
        {"NAME": "ao_strength", "TYPE": "float", "GROUP": "Render", "DEFAULT": 0.75, "MIN": 0.0, "MAX": 1.0, "LABEL": "AO Strength"},
        {"NAME": "shadow_strength", "TYPE": "float", "GROUP": "Render", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 1.0, "LABEL": "Shadow Strength"},

        {"NAME": "light_azim", "TYPE": "float", "GROUP": "Light", "DEFAULT": 1.0, "MIN": -3.14159, "MAX": 3.14159, "LABEL": "Key Azimuth"},
        {"NAME": "light_elev", "TYPE": "float", "GROUP": "Light", "DEFAULT": 0.5, "MIN": -0.6, "MAX": 1.3, "LABEL": "Key Elevation"},
        {"NAME": "fog_amount", "TYPE": "float", "GROUP": "Light", "DEFAULT": 0.55, "MIN": 0.0, "MAX": 2.0, "LABEL": "Atmosphere"},
        {"NAME": "emissive", "TYPE": "float", "GROUP": "Light", "DEFAULT": 0.4, "MIN": 0.0, "MAX": 3.0, "LABEL": "Emissive Depth"},
        {"NAME": "exposure", "TYPE": "float", "GROUP": "Light", "DEFAULT": 1.0, "MIN": 0.2, "MAX": 3.0, "LABEL": "Exposure"},
        {"NAME": "head_light", "TYPE": "float", "GROUP": "Light", "DEFAULT": 0.45, "MIN": 0.0, "MAX": 3.0, "LABEL": "Camera Light"},
        {"NAME": "head_reach", "TYPE": "float", "GROUP": "Light", "DEFAULT": 1.5, "MIN": 0.2, "MAX": 8.0, "LABEL": "Camera Light Reach"},

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
        {"NAME": "motion_blur", "TYPE": "float", "GROUP": "Lens", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0, "LABEL": "Camera Blur"},
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
        {"NAME": "bg_color", "TYPE": "color", "GROUP": "Palette", "DEFAULT": [0.022, 0.026, 0.062, 1.0], "LABEL": "Atmosphere Color"},
        {"NAME": "beauty_passes", "TYPE": "bool", "GROUP": "Render", "DEFAULT": false, "LABEL": "Beauty Passes"},
        {"NAME": "geo_band", "TYPE": "float", "GROUP": "Render", "DEFAULT": 1.0, "MIN": 0.5, "MAX": 8.0, "LABEL": "Geometry Band-Limit"}
    ],
    "PHASE_INPUTS": [
        {"PARAM": "fly_speed", "INDEX": 0, "SCALE": 1.0}
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
};

layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D sceneBuffer;

// Reference orbit from the `fractal_reference_orbit` preprocessor. Four texels
// per iteration, each packing one f32 into its four bytes. Read with
// `texelFetch` only: the values are numeric state, not an image, and
// interpolating between two iterations of an orbit is meaningless.
layout(set = 0, binding = 3) uniform texture2D refOrbit;

layout(std140, set = 0, binding = 4) uniform UserParams {
    float fly_speed;

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

    float dz_absorb;
    float dz_zoom_exp;
    float dz_flight_max_exp;
    int dz_debug;
    float dz_orbit_len;

    float ray_steps;
    float detail;
    float ao_strength;
    float shadow_strength;

    float light_azim;
    float light_elev;
    float fog_amount;
    float emissive;
    float exposure;
    float head_light;
    float head_reach;

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

    // ISF bool inputs are packed as raw u32 0/1 by the host.
    uint beauty_passes;

    // Footprints of geometry the march is allowed to resolve, where one means
    // "down to the pixel". See `geoBand()`.
    float geo_band;
};

// The beauty chain is one switch, and off it renders exactly what the
// geometry-only diagnostic renders: the march, a flat two-lamp key, and
// nothing downstream. Routing it through the existing mode rather than adding
// a second bypass keeps one code path for "geometry and nothing else", so the
// frame the artist watches and the frame the timings are taken from cannot
// drift apart. The dial stays free for diagnostics: any explicit mode wins.
int dbgMode() { return (dz_debug == 0 && beauty_passes == 0u) ? 7 : dz_debug; }

// How many pixel footprints wide the finest geometry this frame draws may be.
//
// The march converges to one footprint and the fold gate truncates at one
// footprint, so the surface carries structure at exactly the sampling rate,
// isotropically, across the whole frame. That is the definition of aliasing,
// and no sample count fixes it: a signal at Nyquist is not undersampled, it is
// unrepresentable. Sharp images come from geometry band-limited *below* the
// sampling rate with the remaining detail carried by shading, which is what an
// offline render gets for free by resolving to the sub-sample and averaging.
//
// Raising this truncates the fold loop earlier, so it is also cheaper. Lowering
// it below one is only meaningful when the frame is being supersampled: at a
// render scale of N, a band of N keeps the geometry pinned to the *output*
// pixel while the sample positions get N times finer, which is the real
// supersample. Rendering large with the band at one is not a supersample at
// all - it resolves two more folds and returns a finer object.
float geoBand() { return max(geo_band, 0.25); }

// Iteration ceiling. The `Iteration Cutoff` parameter is the artistic control;
// this is the hard loop bound the compiler needs.
// Raised from 40. Forty folds of a scale-two stack resolve about twenty
// decades, so the fold ceiling and not the arithmetic was the depth limit
// once offset-space marching removed the coordinate one. The loop exits on
// `i >= total`, so the extra capacity costs nothing at shallow zoom.
// The fold ceiling is the current depth limit. Escape times near a boundary
// anchor scale with the anchor's survival, not the decade count: measured at
// nine decades they run 64-256 folds, so past roughly nine decades frames
// grow bounded-at-budget shells that read as solid walls. Raising this to 256
// removes the walls but a full-budget frame then exceeds the GPU watchdog at
// preview resolutions; the certificate-driven march and cheaper folds have to
// land first. Flight depth defaults inside the covered range.
// Raised to 256 once the frame's real requirement was measured. The host's
// shell-fold table asks for 130 to 185 folds beyond about 2.7 decades, so at
// 128 the correct shell was simply unreachable: every sample stayed bounded at
// the ceiling, every sample read as interior, and the dive rendered as a solid
// the camera was inside. 256 was unaffordable when a beauty frame cost 1.45
// seconds; it is affordable now that the same frame costs about 0.42.
const int MAX_STACK_ITERS = 256;
// Outer bound on the marched volume. Everything of interest sits near the origin.
const float MAX_DIST = 24.0;
const float DZ_CAMERA_STANDOFF = 0.25;
// Radius of the sphere the whole structure is assumed to sit inside, used to skip
// empty space when the camera is outside it.
const float BOUND_R = 6.0;
// The `Detail` default, which the march threshold is expressed against so that
// the default converges at exactly one pixel footprint. See `renderScene`.
const float DETAIL_REF = 0.0015;
const float TAU = 6.2831853;
const float REF_DIST = 4.2;
const float PARKED_FOV = 0.85;
const vec3 PARKED_LOOK = vec3(0.05, -0.02, 0.03);
const vec3 PARKED_ORBIT_DIR = vec3(0.9800666, 0.1986693, 0.0);

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
// How far the descent has travelled, as a base-2 logarithm of the frame radius.
//
// The descent is monotonic and wrapping. A dive that breathed in and back out
// is a dolly rather than a zoom: the whole reason a fractal descent reads as
// endless is that the camera keeps closing while the structure keeps opening, so
// a camera that turned around at a fixed depth would arrive at the same lump
// every cycle.
//
// Depth is named in decades rather than in world radii. `Flight Depth` is the
// animated ceiling the wrapping phase sweeps toward, and `Zoom Decades` parks
// the descent at one explicit depth for a still or for a certificate. Both are
// consumed as logarithms and no physical 10^-zoom value is ever formed, which is
// what keeps the scale honest far below the f32 normal range.
// Declared ahead of use: the payload capability check is a property of the
// stack, and the scale helpers below are defined long before the stack machinery
// they would otherwise have to ask.
bool dzStackSupported();
bool dzBulbPayloadCapable();

const float DZ_LOG2_10 = 3.321928094887362;

// The requested physical frame radius as a logarithm. This is the authoritative
// deep-zoom scale: no physical 10^-zoom value is formed.
float dzFrameLog2() {
    if (dz_zoom_exp > 0.0) {
        return log2(REF_DIST) - dz_zoom_exp * DZ_LOG2_10;
    }
    float flight = fract(PHASE_TIME_0 / TAU) * max(dz_flight_max_exp, 0.0);
    return log2(REF_DIST) - flight * DZ_LOG2_10;
}

float dzOrbitBudgetExp() {
    return dz_zoom_exp > 0.0 ? dz_zoom_exp : max(dz_flight_max_exp, 0.0);
}

float viewScale() {
    return 1.0;
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
// The far end of everything that measures depth, and the reason the render went
// flat at the bottom of a dive.
//
// `MAX_DIST` is a property of the scene: it is where the walls are, and it does
// not move when the camera does. That is correct for a march limit and wrong for
// a depth *encoding*, because once the camera is a hundredth of a unit from a
// surface, every distance in the frame lives in the first ten-thousandth of the
// range and the channel returns the same value across the whole image. Every
// consumer then reads a constant: the defocus sees no far field, the background
// lift applies everywhere or nowhere, and the palette's distance term stops
// varying. That is the flat lighting at depth, and it is the same mistake as
// normalising the structural readout by a fixed number of octaves.
//
// Tracking the view radius keeps the channel spanning the distances a frame
// actually contains. The `min` is what preserves the authored look: at a parked
// frame radius this is `MAX_DIST` exactly, so the encoding only tightens once
// the descent has actually carried the frame in.
// The radius the eye actually ended up at, which is the view radius unless the
// eye had to be backed off to stay outside the surface. Zero until the pass entry
// point has decided, so the depth terms fall back to the requested radius.
float g_camScale = 0.0;

float camRadius() {
    return g_camScale > 0.0 ? g_camScale : viewScale();
}

float depthReach() {
    return min(MAX_DIST, max(camRadius() * 1500.0, 1e-6));
}

// How much the depth-driven lengths are compressed relative to the authored
// scene. One at rest, rising as the dive tightens. Terms written as a rate per
// world unit multiply by this to keep meaning the same fraction of the frame's
// own depth range.
float depthSquash() {
    return MAX_DIST / depthReach();
}

// Octaves of derivative growth one fold can contribute, from the uniforms alone
// so both estimators can agree without threading it through.
float foldOctaves() {
    int forms[4] = int[4](slot0_formula, slot1_formula, slot2_formula, slot3_formula);
    float rts[4] = float[4](slot0_iters, slot1_iters, slot2_iters, slot3_iters);
    float best = 2.0;
    for (int k = 0; k < 4; k++) {
        if (int(clamp(rts[k], 0.0, 8.0)) <= 0) continue;
        best = max(best, forms[k] == 5 ? power : abs(scale));
    }
    return log2(max(best, 2.0));
}

float depthEncode(float t) {
    float near = max(viewScale() * 0.05, 1e-6);
    return clamp(log(max(t, near) / near) / log(max(depthReach() / near, 1.0001)),
                 0.0, 1.0);
}

// How much coarser a ray at distance `t` may be resolved, because the softening
// pass is going to blur it by more than that anyway.
//
// This is the artistic rule paying for the engineering one. Making the fold
// budget per-ray means a ray approaching a surface asks for more folds than it
// used to, and something has to give them back. The softening pass already
// decides that far geometry is not going to be looked at sharply, and it decides
// that from depth alone — which the march knows, from `t`, long before the pass
// runs. So the footprint a distant ray is resolved against can be opened to the
// width of the blur that is coming, and every halving of the footprint skipped is
// one fold not run and one march step not taken.
//
// The rule from the `DeLod` comment holds: this widens the iteration cutoff, the
// distance floor and the convergence threshold together, because they are all
// scaled from the same footprint. Widening the cutoff alone would return
// undersampled geometry as noise rather than as blur.
//
// The relief is the blur radius in pixels, capped well under it. Uncapped it
// reaches nine folds at 1080p, and the cap keeps the coarsening comfortably
// inside the width of the blur meant to hide it.
float defocusRelief(float t) {
    if (dof_amount <= 0.0) return 1.0;
    float coc = smoothstep(depthEncode(dof_onset), 1.0, depthEncode(t)) * dof_amount;
    return 1.0 + min(coc * 0.040 * RENDERSIZE.y, 8.0);
}

// The logarithmic depth remains finite at 1e-100 and drives every fold decision.
float g_zoomLog2 = 0.0;
float g_foldBoost = 0.0;
// The dither the fold budget is broken up with, hoisted for the same reason: the
// budget is consulted once per march step and the hash is not free. Zero is a
// safe default for the passes that never set it, since it only decides which
// side of a fold boundary a pixel lands on.
float g_foldDither = 0.0;

// The point the perturbed estimator measures offsets from.
//
// The reference is the host's refined anchor, read from the validated payload.
// It is latched once per pixel before the march starts so that every estimator
// call measures its offsets from the same place. There is no shader-side search
// for it: the anchor and the orbit taken from it are one atomic unit, and pairing
// a payload orbit with a separately found reference would break that atomicity
// at depth.
vec3 g_dzRef = vec3(0.0);

// Raw diagnostics from the last estimator call, for the false-colour modes.
//
// `DeInfo.depth` cannot serve: it is `log2(dr)/22` clamped to the unit interval,
// so it saturates at `dr = 4e6` and says nothing about the range that matters.
// Reading the real quantities is the only way to see which of them stops moving
// with depth, and inferring it from the rendered image has failed three times.
float g_dbgLogDr = 0.0;
float g_dbgFolds = 0.0;
// Which path the estimator's fold loop exited through, for the exit-cause
// diagnostic: 1 escaped, 2 bulb pre-guard, 3 LOD crossfade, 4 boundary
// rebase failed (unrepresentable), 5 records exhausted, 6 bounded at
// budget, 7 escaped in direct mode, 8 bulb record missing.
float g_dbgExit = 0.0;
// Fold bodies actually executed, as against `done`, which counts the folds the
// linear jump skipped as well. The two differ by whatever the skip is worth,
// which is the number that decides whether a cheaper fold is the lever.
float g_dbgExecFolds = 0.0;

float g_dbgMinFeature = 0.0;
// Why the transported payload was or was not consumed. The payload is the only
// reference the renderer has, so a rejection is a hard failure and the frame
// says so explicitly rather than rendering plausible geometry from an
// unvalidated orbit. 0 absent, 1 stale signature, 2 stale target, 3 consumed.
float g_dbgOrbitGate = 0.0;
float g_dbgMarchSteps = 0.0;
float g_dbgCertJump = 0.0;

// The range the structural readout is normalised against.
//
// Widening by the carried magnification fixes the low end but not the high one.
// The fold budget can produce more octaves than the zoom asks for, and every
// pixel that does then clamps to one. Measured at 1e-6 with a twenty-six fold
// budget, `log2(dr)` reached about 78 against a range of 42, so the channel was
// constant across the frame and every consumer of it went flat: the same
// washed-out field described above, arrived at from the opposite end. Covering
// whichever of the two ranges is larger cannot saturate from either direction.
float depthSpan(int folds) {
    return max(22.0 + g_zoomLog2, float(folds) * foldOctaves() + 1.0);
}

// The payload gate's verdict. Every marched coordinate is frame-local and
// therefore already an offset from the accepted payload anchor.
bool g_dzPayloadMatches = false;

uint g_dzSafeSlabMask = 0u;
float g_dzCertificateMaxT = 0.0;

// The smallest footprint a ray is allowed to ask for, as a world length.
//
// A pixel's world coverage is `pixel * t`, which goes to zero as a ray converges,
// and a footprint of zero asks the fold stack for unbounded detail, so the near
// end needs a floor. The question is what sets it.
//
// It used to be `0.35 / g_zoom`, a fraction of the camera's orbit radius, and
// that is the per-frame quantity in the one place it does the most damage. It
// forbids a ray landing much nearer than the orbit radius from resolving finer
// than the orbit radius, and a ray landing much nearer than the orbit radius is
// precisely "the thing closest to you". Horsthuis's rule is that this is what
// must be rendered deepest; the clamp said it must be rendered shallowest. So
// approaching a surface, or sitting inside one, smoothed it off — which is the
// blob, and it is why the blob appears on approach rather than at depth.
//
// What actually sets the floor is the resolution of the estimator, which is a
// precision fact rather than a framing one. Evaluating the orbit of an order-one
// point in f32 cannot distinguish anything below about an f32 ulp there however
// many folds are run, so such a floor sits near 3e-7 and asking for finer
// returns noise. The perturbed recurrence never forms a difference of two
// order-one quantities, so its floor is far lower.
//
// This is where perturbation pays off in the picture rather than in a table:
// this single number is what decides how close the camera may come to a surface
// before the surface stops gaining detail.
float footFloor() {
    // The near-plane floor on the footprint's distance term, in frame units.
    // At 1e-22 the footprint next to the camera was effectively zero, so the
    // level-of-detail truncation gate (which compares 1/dr against the
    // footprint) was unreachable there and every near-camera sample raced to
    // escape instead, returning the full-depth estimate. At depth that
    // estimate is honestly ~zero (the true boundary is space-filling at deep
    // scales, see notes/deep-zoom-dichotomy.md), so the first march step "hit"
    // at the camera and parked deep frames rendered a flat featureless field.
    // A floor at a small fraction of the camera standoff keeps the truncated
    // surface as the rendered object right up to the lens: detail dissolves
    // at the near plane instead of the frame dying on one degenerate sample.
    return 0.004;
}

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
// False when a slot carrying iterations is the Mandelbulb and the bound payload
// cannot describe it. The bulb's recurrence needs the transported polar
// reference, seam side and radius margins to take its decisions against stored
// distances; without them there is no valid continuation, so the frame reports
// the missing capability rather than rendering an unfounded surface.
bool dzStackSupported() {
    int forms[4] = int[4](slot0_formula, slot1_formula, slot2_formula, slot3_formula);
    float rts[4] = float[4](slot0_iters, slot1_iters, slot2_iters, slot3_iters);
    for (int k = 0; k < 4; k++) {
        if (forms[k] == 5 && int(clamp(rts[k], 0.0, 8.0)) > 0
            && !dzBulbPayloadCapable()) return false;
    }
    return true;
}


// ---------------------------------------------------------------------------
// Perturbed distance estimate
//
// This is the renderer's only estimator. Evaluating the orbit of `pos` directly
// would floor the precision at an f32 ulp near the size of the orbit, about
// 6e-8, which is why structure below roughly 1e-5 of the frame dissolves however
// many folds are run. Perturbation splits the sample into a reference point and
// an offset, `pos = A + w`, carries the orbit of `A` and the *exact finite
// increment* of the offset separately, and never forms a difference of two
// order-one quantities. The offset then keeps its own full mantissa at whatever
// scale it lives at.
//
// Two rules make it work, and both were learned the hard way:
//
//  1. Every discrete decision is taken by comparing the offset against the
//     reference's stored signed distance to the decision boundary, never by
//     recomputing the test on a reconstructed `P + e`. Reconstructing rounds
//     the offset away entirely once it falls below eps of the reference, at
//     which point every sample inherits the reference's decisions and the
//     image goes flat.
//  2. Each (reference branch, sample branch) pair is its own identity. The
//     crossing cases are not the same-branch case with a patch, and getting
//     them by analogy produced wrong identities three separate times in this
//     work. They are enumerated explicitly below.
//
// The identities hold for offsets of *any* size, with no smallness assumption,
// so the recurrence is correct over the whole frame rather than only near the
// reference. What the absorb threshold below buys is conditioning, not validity.
//
// The Mandelbulb recurrence is reachable, and payload version 6 is what makes it
// so: it carries the polar reference, the seam side, the principal winding, the
// pre-slot radius-2 margin, the radius floor margin and the post-slot bailout
// margin, so every one of the bulb's decisions is taken against a transported
// quantity rather than a recomputed one. A stack whose bound payload cannot
// supply those is refused by `dzStackSupported`.
// ---------------------------------------------------------------------------

// A signed value m * 2^e. The exponent is stored as a float because GLSL integer
// arithmetic is not uniformly lowered by every backend used here; normalization
// nevertheless keeps it integral. Mantissas are order one, so physical values
// with exponents such as -300 and derivatives above +128 never have to exist as
// f32 values.
struct DzScalar {
    float m;
    float e;
};

// Three signed values sharing one base-2 exponent.
struct DzVec3 {
    vec3 m;
    float e;
};

const float DZ_EXP_WINDOW = 120.0;

DzScalar dzSZero() { return DzScalar(0.0, 0.0); }
DzVec3 dzVZero() { return DzVec3(vec3(0.0), 0.0); }

// `2^k` for integral `k`, and `floor(log2|x|)`, read from and written to the
// IEEE exponent field directly.
//
// These two replace an `exp2` and a `log2` in every scaled-arithmetic
// primitive, which is the renderer's hottest code by a wide margin: an add
// alone cost three transcendentals, and a deep frame evaluates tens of
// thousands of them per pixel. Transcendentals are quarter-rate on the GPUs
// this ships to, so the bit form is several times cheaper, and it is exact
// rather than approximately exact: the exponent field *is* `floor(log2|x|)`
// for every normal float, and scaling by a power of two only edits that
// field. Mantissas here are held in [0.5, 1) by construction, so the
// denormal and infinity encodings the field form cannot express do not
// arise; the guards below keep them from producing nonsense if they ever do.
float dzPow2(float k) {
    return intBitsToFloat((int(clamp(k, -126.0, 127.0)) + 127) << 23);
}

float dzLog2Floor(float x) {
    return float(((floatBitsToInt(x) >> 23) & 0xFF) - 127);
}

DzScalar dzSNormalize(DzScalar value) {
    if (value.m == 0.0) return dzSZero();
    float shift = clamp(dzLog2Floor(abs(value.m)) + 1.0,
                        -DZ_EXP_WINDOW, DZ_EXP_WINDOW);
    value.m *= dzPow2(-shift);
    value.e += shift;
    return value;
}

DzVec3 dzVNormalize(DzVec3 value) {
    float peak = max(max(abs(value.m.x), abs(value.m.y)), abs(value.m.z));
    if (peak == 0.0) return dzVZero();
    float shift = clamp(dzLog2Floor(peak) + 1.0, -DZ_EXP_WINDOW, DZ_EXP_WINDOW);
    value.m *= dzPow2(-shift);
    value.e += shift;
    return value;
}

DzScalar dzS(float ordinary) { return dzSNormalize(DzScalar(ordinary, 0.0)); }
DzVec3 dzV(vec3 ordinary) { return dzVNormalize(DzVec3(ordinary, 0.0)); }
DzScalar dzSNeg(DzScalar value) { value.m = -value.m; return value; }

DzVec3 dzFrameVector(vec3 frameLocal) {
    float frameLog2 = dzFrameLog2();
    float exponent = floor(frameLog2) + 1.0;
    // The exp2 argument is always in [-1, 0), independent of zoom depth.
    float frameMantissa = exp2(frameLog2 - exponent);
    return dzVNormalize(DzVec3(frameLocal * frameMantissa, exponent));
}

float dzAlignedFactor(float fromExponent, float toExponent) {
    float difference = fromExponent - toExponent;
    if (difference < -DZ_EXP_WINDOW) return 0.0;
    // Exponents are integral by construction, so the field form is exact.
    return dzPow2(clamp(difference, -DZ_EXP_WINDOW, 0.0));
}

DzScalar dzSAdd(DzScalar a, DzScalar b) {
    if (a.m == 0.0) return b;
    if (b.m == 0.0) return a;
    float exponent = max(a.e, b.e);
    return dzSNormalize(DzScalar(
        a.m * dzAlignedFactor(a.e, exponent)
            + b.m * dzAlignedFactor(b.e, exponent),
        exponent
    ));
}

DzScalar dzSSub(DzScalar a, DzScalar b) { return dzSAdd(a, dzSNeg(b)); }

DzVec3 dzVAdd(DzVec3 a, DzVec3 b) {
    if (all(equal(a.m, vec3(0.0)))) return b;
    if (all(equal(b.m, vec3(0.0)))) return a;
    float exponent = max(a.e, b.e);
    return dzVNormalize(DzVec3(
        a.m * dzAlignedFactor(a.e, exponent)
            + b.m * dzAlignedFactor(b.e, exponent),
        exponent
    ));
}

DzScalar dzSMul(DzScalar value, float ordinary) {
    value.m *= ordinary;
    return dzSNormalize(value);
}

DzScalar dzSDiv(DzScalar value, float ordinary) {
    value.m /= ordinary;
    return dzSNormalize(value);
}

DzScalar dzSMulS(DzScalar a, DzScalar b) {
    return dzSNormalize(DzScalar(a.m * b.m, a.e + b.e));
}

DzScalar dzSDivS(DzScalar a, DzScalar b) {
    return dzSNormalize(DzScalar(a.m / b.m, a.e - b.e));
}

DzVec3 dzVMul(DzVec3 value, float ordinary) {
    value.m *= ordinary;
    return dzVNormalize(value);
}

DzVec3 dzVMulS(DzVec3 value, DzScalar scalar) {
    return dzVNormalize(DzVec3(value.m * scalar.m, value.e + scalar.e));
}

DzScalar dzVComponent(DzVec3 value, int component) {
    return dzSNormalize(DzScalar(value.m[component], value.e));
}

DzVec3 dzVFromComponents(DzScalar x, DzScalar y, DzScalar z) {
    float exponent = max(x.e, max(y.e, z.e));
    return dzVNormalize(DzVec3(
        vec3(
            x.m * dzAlignedFactor(x.e, exponent),
            y.m * dzAlignedFactor(y.e, exponent),
            z.m * dzAlignedFactor(z.e, exponent)
        ),
        exponent
    ));
}

DzVec3 dzVWithComponent(DzVec3 value, int component, DzScalar replacement) {
    DzScalar parts[3] = DzScalar[3](
        dzVComponent(value, 0), dzVComponent(value, 1), dzVComponent(value, 2)
    );
    parts[component] = replacement;
    return dzVFromComponents(parts[0], parts[1], parts[2]);
}

DzScalar dzVDotOrd(DzVec3 value, vec3 ordinary) {
    return dzSNormalize(DzScalar(dot(value.m, ordinary), value.e));
}

DzScalar dzVNorm2(DzVec3 value) {
    return dzSNormalize(DzScalar(dot(value.m, value.m), 2.0 * value.e));
}

// Exact finite increment in |P+e|^2, with P order one and e scaled.
DzScalar dzSquaredNormIncrement(vec3 P, DzVec3 e) {
    return dzSAdd(dzSMul(dzVDotOrd(e, P), 2.0), dzVNorm2(e));
}

int dzSCompare(DzScalar a, DzScalar b) {
    if (a.m < 0.0 && b.m >= 0.0) return -1;
    if (a.m >= 0.0 && b.m < 0.0) return 1;
    if (a.m == 0.0 && b.m == 0.0) return 0;
    float signValue = (a.m < 0.0) ? -1.0 : 1.0;
    if (a.e != b.e) return int(signValue * ((a.e > b.e) ? 1.0 : -1.0));
    if (a.m == b.m) return 0;
    return (a.m > b.m) ? 1 : -1;
}

bool dzSLessOrd(DzScalar value, float ordinary) {
    return dzSCompare(value, dzS(ordinary)) < 0;
}

bool dzSGreaterOrd(DzScalar value, float ordinary) {
    return dzSCompare(value, dzS(ordinary)) > 0;
}

float dzSToOrdinary(DzScalar value) {
    if (value.m == 0.0) return 0.0;
    if (value.e < -DZ_EXP_WINDOW) return 0.0;
    if (value.e > DZ_EXP_WINDOW) return sign(value.m) * 1e30;
    return value.m * exp2(value.e);
}

float dzSLog2Abs(DzScalar value) {
    return (value.m == 0.0) ? -1e30 : log2(abs(value.m)) + value.e;
}

// Conversion is delayed until the value has been divided by the frame scale.
// Only the relative exponent is exponentiated, and only inside the bounded
// window accepted by exp2.
float dzSToFrameRelative(DzScalar value, float frameLog2) {
    if (value.m == 0.0) return 0.0;
    float relativeExponent = value.e - frameLog2;
    if (relativeExponent < -DZ_EXP_WINDOW) return 0.0;
    if (relativeExponent > DZ_EXP_WINDOW) return 1e30;
    return value.m * exp2(relativeExponent);
}

float dzDistanceToEstimatorUnits(DzScalar value) {
    return dzSToFrameRelative(value, dzFrameLog2());
}

DzScalar dzRadiusIncrement(float referenceRadius, DzScalar squaredIncrement) {
    // dR = dq / (sqrt(q+dq) + sqrt(q)). For sub-ulp dq the denominator is
    // exactly 2R at f32 precision, while dq keeps its independent exponent.
    float dq = dzSToOrdinary(squaredIncrement);
    float sampleRadius = sqrt(max(referenceRadius * referenceRadius + dq, 0.0));
    return dzSDiv(squaredIncrement, max(sampleRadius + referenceRadius, 1e-30));
}

DzScalar dzSSqrt(DzScalar value) {
    if (value.m <= 0.0) return dzSZero();
    float halfExponent = floor(value.e * 0.5);
    float parity = value.e - 2.0 * halfExponent;
    return dzSNormalize(DzScalar(
        sqrt(value.m * exp2(clamp(parity, 0.0, 1.0))),
        halfExponent
    ));
}

DzScalar dzLengthFromReference(vec3 P, DzVec3 e) {
    float referenceRadius = length(P);
    if (referenceRadius == 0.0) return dzSSqrt(dzVNorm2(e));
    return dzSAdd(
        dzS(referenceRadius),
        dzRadiusIncrement(referenceRadius, dzSquaredNormIncrement(P, e))
    );
}

// One component of a box fold, as an exact increment.
//
// `pr` is the reference component and `ec` the offset component. `mHi` and
// `mLo` are the reference's signed distances to the two fold planes, and both
// the reference branch and the sample branch are read off them, so the two
// decisions are always taken against the same stored quantity.
DzScalar dzFoldCompStored(
    float pr, DzScalar ec, float L, int forced, float storedLo, float storedHi,
    out float prOut
) {
    float mHi = (forced >= 0) ? storedHi : (L - pr);
    float mLo = (forced >= 0) ? storedLo : (pr + L);
    // The reference's branch is transported when available. Recomputing it from
    // a transported point is unsafe: within an ulp of a plane the shader can
    // pick the other side than the host did, and that is a different affine map
    // rather than a rounding.
    int rb = (forced >= 0) ? (forced - 1)
                           : ((mHi < 0.0) ? 1 : ((mLo < 0.0) ? -1 : 0));
    int sb = dzSGreaterOrd(ec, mHi) ? 1 : (dzSLessOrd(ec, -mLo) ? -1 : 0);

    prOut = (rb == 0) ? pr : ((rb == 1) ? (2.0 * L - pr) : (-2.0 * L - pr));

    if (rb == sb) {
        // Same branch: the fold is affine, so the increment is the branch's
        // linear part. Identity inside, a reflection outside.
        return (rb == 0) ? ec : dzSNeg(ec);
    }
    if (rb == 0) {
        // Reference inside. The crossing term is twice the reference's own
        // distance to the plane the sample went past, which is small exactly
        // when the crossing is possible.
        return (sb == 1)
            ? dzSSub(dzS(2.0 * mHi), ec)
            : dzSSub(dzS(-2.0 * mLo), ec);
    }
    if (rb == 1) {
        return (sb == 0)
            ? dzSSub(ec, dzS(2.0 * mHi))
            : dzSSub(dzS(-4.0 * L), ec);
    }
    return (sb == 0)
        ? dzSAdd(ec, dzS(2.0 * mLo))
        : dzSSub(dzS(4.0 * L), ec);
}

// The unified radial fold: `m(q) = K / clamp(q, lo, hi)`.
//
// One helper covers all three radial slots. Mandelbox is `K = fixR2` with
// `lo = minR2, hi = fixR2`, so the outer branch is `fixR2/fixR2 = 1`. Amazing
// Box is `K = sc` over the same interval. Pseudo-Kleinian is `K = 1` with `hi`
// at infinity, which is `1/max(q, minR2)`. Writing them separately is what hid
// two wrong crossing identities the first time.
//
// `q` is the reference's squared radius and `dq` the exact increment in it.
// The whole trick is that the difference of the two clamped radii is
// cancellation-free in every branch pair: it is `-dq` when both sit in the
// middle, and otherwise a stored margin.
DzVec3 dzRadialStored(
    vec3 P, DzVec3 e, float q, DzScalar dq, float lo, float hi, float K,
    int forced, float storedAboveLo, float storedBelowHi,
    out float mRef, out float mSmp
) {
    float mgLo = (forced >= 0) ? -storedAboveLo : (lo - q);
    float mgHi = (forced >= 0) ? storedBelowHi : (hi - q);
    int rb = (forced >= 0) ? forced
                           : ((mgLo > 0.0) ? 0 : ((mgHi > 0.0) ? 1 : 2));
    int sb = dzSLessOrd(dq, mgLo) ? 0 : (dzSLessOrd(dq, mgHi) ? 1 : 2);

    float qcRef = (rb == 0) ? lo : ((rb == 1) ? q : hi);
    DzScalar dClamp;           // qcRef - qcSmp, formed without cancellation
    if (rb == sb) {
        dClamp = (rb == 1) ? dzSNeg(dq) : dzSZero();
    } else if (rb == 1) {
        dClamp = dzS((sb == 0) ? -mgLo : -mgHi);
    } else if (sb == 1) {
        dClamp = dzSSub(dzS((rb == 0) ? mgLo : mgHi), dq);
    } else {
        // Straight from one clamp to the other, which needs |dq| to span the
        // whole interval, so this term is legitimately order one.
        dClamp = dzS((rb == 0) ? (lo - hi) : (hi - lo));
    }
    float qcSmp = qcRef - dzSToOrdinary(dClamp);

    mRef = K / qcRef;
    mSmp = K / qcSmp;
    // e' = P (mSmp - mRef) + e mSmp, with the multiplier difference written as
    // K (qcRef - qcSmp) / (qcRef qcSmp) so it inherits dClamp's smallness.
    //
    // Grouped as mRef * (dClamp / qcSmp) rather than as K * dClamp over the
    // product of the two clamped radii. The product form is algebraically the
    // same and overflows f32 whenever a slot has no upper clamp, which is
    // expressed here as a very large `hi`: Pseudo-Kleinian's inversion then
    // squares that bound and the whole term becomes infinity. That showed up as
    // the one formula of nine whose perturbed render did not reproduce the
    // direct one.
    DzScalar dm = dzSMul(dzSDiv(dClamp, qcSmp), mRef);
    return dzVAdd(dzVMulS(dzV(P), dm), dzVMul(e, mSmp));
}

DzVec3 dzAbsStored(DzVec3 e, vec3 margins, int code) {
    DzScalar outValue[3];
    for (int k = 0; k < 3; k++) {
        float margin = margins[k];
        DzScalar ec = dzVComponent(e, k);
        bool refNegative = (code & (1 << k)) != 0;
        bool sampleNegative = dzSLessOrd(ec, -margin);
        if (refNegative == sampleNegative) {
            outValue[k] = refNegative ? dzSNeg(ec) : ec;
        } else {
            outValue[k] = refNegative
                ? dzSAdd(dzS(2.0 * margin), ec)
                : dzSSub(dzS(-2.0 * margin), ec);
        }
    }
    return dzVFromComponents(outValue[0], outValue[1], outValue[2]);
}

void dzCondSwapStored(inout DzVec3 e, int i, int j, float margin, bool refSwap) {
    DzScalar ei = dzVComponent(e, i);
    DzScalar ej = dzVComponent(e, j);
    bool sampleSwap = dzSLessOrd(dzSSub(ei, ej), -margin);
    DzScalar outValue[3] = DzScalar[3](
        dzVComponent(e, 0), dzVComponent(e, 1), dzVComponent(e, 2)
    );
    if (refSwap == sampleSwap) {
        if (refSwap) { outValue[i] = ej; outValue[j] = ei; }
    } else if (refSwap) {
        outValue[i] = dzSSub(ei, dzS(margin));
        outValue[j] = dzSAdd(ej, dzS(margin));
    } else {
        outValue[i] = dzSSub(ej, dzS(margin));
        outValue[j] = dzSAdd(ei, dzS(margin));
    }
    e = dzVFromComponents(outValue[0], outValue[1], outValue[2]);
}

void dzCondReflectPairStored(
    inout DzVec3 e, int i, int j, float margin, bool refFold
) {
    DzScalar ei = dzVComponent(e, i);
    DzScalar ej = dzVComponent(e, j);
    bool sampleFold = dzSLessOrd(dzSAdd(ei, ej), -margin);
    DzScalar outValue[3] = DzScalar[3](
        dzVComponent(e, 0), dzVComponent(e, 1), dzVComponent(e, 2)
    );
    if (refFold == sampleFold) {
        if (refFold) { outValue[i] = dzSNeg(ej); outValue[j] = dzSNeg(ei); }
    } else if (refFold) {
        outValue[i] = dzSAdd(dzS(margin), ei);
        outValue[j] = dzSAdd(dzS(margin), ej);
    } else {
        outValue[i] = dzSSub(dzS(-margin), ej);
        outValue[j] = dzSSub(dzS(-margin), ei);
    }
    e = dzVFromComponents(outValue[0], outValue[1], outValue[2]);
}

// One f32 unpacked from an rgba8unorm texel's four bytes.
//
// The format maps a byte to `b / 255`, so `round(v * 255)` recovers the byte
// exactly. Packing rather than an f32 texture format because the renderer
// declares every preprocessor texture filterable and a 32-bit float texture is
// not; see the analyzer for the full reasoning.

const int DZ_PAYLOAD_VERSION = 7;
// Header records in front of the orbit: the identity and atlas record, then
// the measured shell-fold table.
const int DZ_HEADER_RECORDS = 2;
const int DZ_SHELL_TABLE_LEN = 12;
// Folds between where the camera escapes and where the shell is placed.
const float DZ_SHELL_CAMERA_MARGIN = 6.0;

// How large the linear offset may get before the loop takes over, as a base-two
// logarithm. At a thousandth of the order-one reference the measured relative
// error of the linear model is about one percent.
const float DZ_LINEAR_HORIZON_LOG2 = -10.0;
const int DZ_GROUPS_PER_RECORD = 12;
const int DZ_CERT_TILE_COLUMNS = 8;
const int DZ_CERT_TILE_ROWS = 4;
const int DZ_CERT_T_SLABS = 24;

// The payload row width, latched once per pixel by `dzResolvePayloadGate`.
// `dzGroup` runs tens of thousands of times per pixel inside the march, and a
// `textureSize` query per group was a measurable share of that cost.
int g_dzPayloadWidth = 0;

// One four-float group is one `rgba32float` texel: a single fetch, no byte
// reassembly. The payload declares `FORMAT: "rgba32float"`, which binds this
// texture non-filterable; that is legal precisely because every read here is
// a `texelFetch` and the shared filtering sampler is never paired with it.
vec4 dzGroup(int groupIndex) {
    int width = g_dzPayloadWidth;
    if (width <= 0) {
        width = max(textureSize(sampler2D(refOrbit, texSampler), 0).x, 1);
    }
    return texelFetch(
        sampler2D(refOrbit, texSampler),
        ivec2(groupIndex % width, groupIndex / width),
        0
    );
}

// One group of four from iteration `n`'s record.
//
// Forty-eight floats per iteration in twelve groups: the point entering and its
// squared radius, the point after the fold or sort stage and its squared radius,
// the point after any scale plus the radial multiplier, then the point leaving.
// A header record of the same size sits in front, carrying the anchor and the
// valid iteration count, so
// iteration `n` starts at group `(1 + n) * DZ_GROUPS_PER_RECORD`.
vec4 dzRec(int n, int group) {
    return dzGroup((DZ_HEADER_RECORDS + n) * DZ_GROUPS_PER_RECORD + group);
}

// The fold at which this frame's own samples escape, measured by the host at
// `DZ_SHELL_TABLE_LEN` depths across the rated range and read back here by
// interpolation. Zero means the host published no measurement and the caller
// should fall back to the derived rate.
//
// This exists because the rate argument is not enough. A generic point at
// `10^-d` from the set escapes at about `1.1 * d` folds, and the shader used
// that; but the anchor is selected for survival, so its neighbourhood stays
// bounded far longer, and a shell placed by the formula sat deep inside the
// solid. Past about 1.6 decades every sample in frame was still bounded at the
// shell, every sample therefore read as interior, and the dive rendered as a
// wall the camera was already inside.
float dzShellTable(float depthFraction, int firstGroup) {
    // Entry `i` is measured at depth fraction `(i + 1) / N`, so the index for
    // a fraction is `f * N - 1`. Reading it as `f * N - 0.5` shifted every
    // lookup half a slot, which is a third of a decade at twelve entries and
    // more than enough to matter where the table steps from twenty-three folds
    // to a hundred and forty-six between neighbours.
    float slot = clamp(depthFraction, 0.0, 1.0) * float(DZ_SHELL_TABLE_LEN) - 1.0;
    int lo = clamp(int(floor(slot)), 0, DZ_SHELL_TABLE_LEN - 1);
    int hi = clamp(lo + 1, 0, DZ_SHELL_TABLE_LEN - 1);
    float blend = clamp(slot - float(lo), 0.0, 1.0);
    vec4 loGroup = dzGroup(firstGroup + lo / 4);
    vec4 hiGroup = dzGroup(firstGroup + hi / 4);
    float a = loGroup[lo % 4];
    float b = hiGroup[hi % 4];
    if (a <= 0.0 || b <= 0.0) return 0.0;
    return mix(a, b, blend);
}

// Where the frame's surface resolves, and where the camera itself decides.
// The parameter Jacobian at fold `n`: unit matrix and log2 of its magnitude.
//
// `D_0 = I`, `D_{k+1} = J_k D_k + S_k`, and `e_k = D_k w + O(|w|^2)` (the
// paper's stratified perturbation theorem). The offset at any fold is one
// matrix-vector product, so the fold loop does not have to walk there.
mat3 dzJacobianUnit(int n) {
    vec4 row0 = dzRec(n, 9);
    vec4 row1 = dzRec(n, 10);
    vec4 row2 = dzRec(n, 11);
    // Column-major constructor, so rows go in transposed.
    return mat3(row0.x, row1.x, row2.x,
                row0.y, row1.y, row2.y,
                row0.z, row1.z, row2.z);
}

float dzJacobianLog2(int n) { return dzRec(n, 9).w; }

// The last fold at which the linear model is still trustworthy for this
// sample.
//
// Measured against the arbitrary-precision orbit, the relative error of
// `P_k + D_k w` stays near one percent while `|D_k w|` is a thousandth of the
// reference and only passes ten percent once the offset approaches order one -
// which is the same moment the orbit escapes. So the linear model covers the
// cheap majority of every orbit and dies exactly where the interesting part
// begins. `log2|D_k|` is monotone, so the horizon is a scalar bisection: eight
// comparisons rather than eighty folds.
int dzLinearHorizon(float offsetLog2, int limit) {
    if (dzJacobianLog2(0) == 0.0 && dzJacobianLog2(1) == 0.0) return 0;
    float ceilingLog2 = DZ_LINEAR_HORIZON_LOG2 - offsetLog2;
    int lo = 0;
    int hi = limit;
    for (int probe = 0; probe < 9; probe++) {
        if (lo + 1 >= hi) break;
        int mid = (lo + hi) / 2;
        float scale = dzJacobianLog2(mid);
        // A zero scale means the host published no Jacobian this deep.
        if (scale == 0.0 || scale > ceilingLog2) { hi = mid; } else { lo = mid; }
    }
    return lo;
}

// The fold at which a sample escapes, read off the transported derivative
// curve instead of iterated.
//
// A sample leaves the bailout surface when its offset reaches order one, and
// the offset at fold `k` is `D_k w`. So the escape fold is simply where the
// curve `|D_k w|` crosses one, and `|D_k|` is monotone, so that crossing is a
// bisection: eight probes of a matrix-vector product against a fold loop.
//
// The matrix is used rather than just the magnitude on purpose. `|D_k| |w|`
// would make the answer depend only on distance from the anchor, and the fog
// that consumes this would come out radially symmetric - structure is exactly
// what it is drawing. The full product keeps the directional dependence that
// makes one part of the frame reach a given fold before another.
//
// Returns a fractional fold, interpolated across the crossing, so a shell
// keeps a smooth edge rather than quantising to whole iterations.
// How large this sample's offset may get at fold `n` before the orbit leaves
// the bailout surface, as a base-two logarithm.
//
// Not simply "one". The orbit escapes when `|P_n + e_n|` passes the bailout,
// and `P_n` is already somewhere inside it, so the headroom left for `e_n`
// depends on where the reference sits: `|e| > (B^2 - |P|^2) / 2|P|` to first
// order. The payload already transports that numerator as the post-slot
// bailout margin, so the threshold costs two fetches rather than a guess.
// Using one instead put the crossing systematically early and the iteration
// fog fired across most of the frame.
float dzEscapeThresholdLog2(int n) {
    // Margin eleven, fetched directly: the named accessor is declared below.
    float margin = dzRec(n, 7).w;
    if (margin <= 0.0) return -60.0;
    float reference = length(dzRec(n, 3).xyz);
    return log2(margin / max(2.0 * reference, 1e-6));
}

float dzLinearFoldCount(vec3 frameLocal, int limit) {
    if (dzJacobianLog2(1) == 0.0) return -1.0;
    DzVec3 w = dzFrameVector(frameLocal);
    int lo = 0;
    int hi = max(limit, 1);
    // `offsetLog2(k)` is log2 of |D_k w|; escape is where it reaches zero.
    for (int probe = 0; probe < 9; probe++) {
        if (lo + 1 >= hi) break;
        int mid = (lo + hi) / 2;
        float scale = dzJacobianLog2(mid);
        if (scale == 0.0) { hi = mid; continue; }
        vec3 mapped = dzJacobianUnit(mid) * w.m;
        float magnitude = w.e + scale + log2(max(
            max(max(abs(mapped.x), abs(mapped.y)), abs(mapped.z)), 1e-30));
        if (magnitude < dzEscapeThresholdLog2(mid)) { lo = mid; } else { hi = mid; }
    }
    float loScale = dzJacobianLog2(lo);
    float hiScale = dzJacobianLog2(hi);
    if (loScale == 0.0 || hiScale <= loScale) return float(lo);
    vec3 loMapped = dzJacobianUnit(lo) * w.m;
    vec3 hiMapped = dzJacobianUnit(hi) * w.m;
    float a = w.e + loScale + log2(max(
        max(max(abs(loMapped.x), abs(loMapped.y)), abs(loMapped.z)), 1e-30));
    float b = w.e + hiScale + log2(max(
        max(max(abs(hiMapped.x), abs(hiMapped.y)), abs(hiMapped.z)), 1e-30));
    float crossing = dzEscapeThresholdLog2(lo);
    float blend = (b > a) ? clamp((crossing - a) / (b - a), 0.0, 1.0) : 0.0;
    return float(lo) + blend * float(hi - lo);
}

float dzMeasuredShellFold(float depthFraction) {
    return dzShellTable(depthFraction, DZ_GROUPS_PER_RECORD);
}

float dzMeasuredCameraFold(float depthFraction) {
    return dzShellTable(depthFraction, DZ_GROUPS_PER_RECORD + 3);
}

// Standoff, in frame radii, at which the host proved the eye is outside the
// shell this depth draws. Zero means no measurement was transported.
float dzMeasuredCameraStandoff(float depthFraction) {
    return dzShellTable(depthFraction, DZ_GROUPS_PER_RECORD + 6);
}

float dzMargin(int n, int marginIndex) {
    vec4 group = dzRec(n, 5 + marginIndex / 4);
    return group[marginIndex % 4];
}

// Named accessors for fields that already exist in the twelve-group record.
// Keeping these names here prevents recurrence code from guessing group slots.
float dzPostSlotBailoutMargin(int n) { return dzMargin(n, 11); }
float dzBulbSeamSide(int n) { return dzRec(n, 8).y; }
float dzBulbPrincipalWinding(int n) { return dzRec(n, 8).z; }
bool dzBulbRecordPresent(int n) { return dzRec(n, 8).w > 0.5; }

bool dzBulbPayloadCapable() {
    ivec2 size = textureSize(sampler2D(refOrbit, texSampler), 0);
    if (size.x * size.y < DZ_HEADER_RECORDS * DZ_GROUPS_PER_RECORD) return false;
    vec4 header = dzGroup(0);
    return int(header.x) == DZ_PAYLOAD_VERSION
        && int(header.y) == DZ_GROUPS_PER_RECORD
        && int(header.z) > 0;
}

// Is a reference orbit actually bound?
//
// A preprocessor is optional: the engine degrades to default outputs when the
// analyzer is absent or failed, and a texture that was never written reads as
// zero. Without this check the shader would then pin its reference at the origin
// and render something confidently wrong, which is worse than rendering the
// shallower thing correctly. A real orbit's first entry is the anchor's image
// under one fold and its squared radius; both being exactly zero does not
// happen for any anchor a dive would choose.
// The anchor the host refined and measured its orbit from.
vec3 dzAnchor() {
    return dzGroup(1).xyz + dzGroup(2).xyz;
}

bool dzOrbitPresent() {
    // Tested by size, not by content. A slot with no analyzer output carries a
    // 1x1 placeholder, and reading past the edge of that with `texelFetch` is
    // undefined rather than zero, so a content test can pass on garbage and
    // defeat the fallback it exists to trigger. That is exactly what happened:
    // the shader read four out-of-bounds texels, believed an orbit was present,
    // and rendered black. `textureSize` is defined for any binding.
    ivec2 size = textureSize(sampler2D(refOrbit, texSampler), 0);
    if (size.x * size.y < DZ_HEADER_RECORDS * DZ_GROUPS_PER_RECORD) return false;
    vec4 header = dzGroup(0);
    return int(header.x) == DZ_PAYLOAD_VERSION
        && int(header.y) == DZ_GROUPS_PER_RECORD
        && int(header.z) > 0;
}

uint dzHashWord(uint hash, uint word) {
    return (hash ^ word ^ (word >> 24u)) * 16777619u;
}

uint dzTargetDigest() {
    uint hash = 2166136261u;
    vec3 high = dzGroup(1).xyz;
    vec3 low = dzGroup(2).xyz;
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(high[i]));
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(low[i]));
    hash = dzHashWord(hash, uint(int(dzGroup(0).z)));
    return hash & 0x00ffffffu;
}

uint dzStateSignature(
    float effectiveScale,
    vec3 effectiveOffset,
    float effectiveLinx,
    vec3 effectiveJulia,
    float effectiveRotA
) {
    uint hash = 2166136261u;
    ivec4 order = stackOrder(stack_order);
    int forms[4] = int[4](slot0_formula, slot1_formula, slot2_formula, slot3_formula);
    int rates[4] = int[4](
        int(clamp(slot0_iters, 0.0, 8.0)),
        int(clamp(slot1_iters, 0.0, 8.0)),
        int(clamp(slot2_iters, 0.0, 8.0)),
        int(clamp(slot3_iters, 0.0, 8.0))
    );
    for (int i = 0; i < 4; i++) hash = dzHashWord(hash, uint(forms[order[i]]));
    for (int i = 0; i < 4; i++) hash = dzHashWord(hash, uint(rates[order[i]]));
    // Depth is deliberately absent (the seventh slot, once
    // `dzOrbitBudgetExp()`). The orbit is the iterated image of one anchor and
    // does not depend on the frame radius, so hashing depth here invalidated a
    // sound payload on every frame of a zoom gesture — the host could never
    // regenerate fast enough, and the whole gesture rendered the alarm
    // pattern. A payload shorter than the current depth budget now just
    // shortens the certified prefix; the direct continuation covers the rest.
    float scalars[17] = float[17](
        effectiveScale, fold_limit, min_radius, fixed_radius, bailout,
        power, 0.0, REF_DIST, 0.0, 0.2,
        PARKED_FOV, 0.0,
        RENDERSIZE.x / max(RENDERSIZE.y, 1.0),
        julia_amount, cocube, lin_mix, stack_cap
    );
    for (int i = 0; i < 17; i++) hash = dzHashWord(hash, floatBitsToUint(scalars[i]));
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(effectiveOffset[i]));
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(effectiveJulia[i]));
    vec3 effectiveLin = vec3(effectiveLinx, lin_y, lin_z);
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(effectiveLin[i]));
    vec3 effectiveRot = vec3(effectiveRotA, rot_b, rot_c);
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(effectiveRot[i]));
    vec3 effectiveRotW = vec3(rot_xw, rot_yw, rot_zw);
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(effectiveRotW[i]));
    vec3 effectiveLook = PARKED_LOOK;
    for (int i = 0; i < 3; i++) hash = dzHashWord(hash, floatBitsToUint(effectiveLook[i]));
    hash = dzHashWord(hash, uint(int(clamp(dz_orbit_len, 1.0, 4096.0))));
    return hash & 0x00ffffffu;
}

void dzResolvePayloadGate() {
    g_dzPayloadWidth = max(textureSize(sampler2D(refOrbit, texSampler), 0).x, 1);
    g_dzPayloadMatches = false;
    g_dbgOrbitGate = 0.0;
    if (!dzOrbitPresent()) return;
    uint expected = dzStateSignature(
        scale,
        vec3(offset_x, offset_y, offset_z),
        lin_x,
        vec3(julia_x, julia_y, julia_z),
        rot_a
    );
    if (uint(int(dzGroup(0).w)) != expected) {
        g_dbgOrbitGate = 1.0;
        return;
    }
    if (uint(int(dzGroup(2).w)) != dzTargetDigest()) {
        g_dbgOrbitGate = 2.0;
        return;
    }
    g_dzPayloadMatches = true;
    g_dbgOrbitGate = 3.0;
}

void dzPreparePrimaryCertificate(vec2 normalizedScreen) {
    g_dzSafeSlabMask = 0u;
    g_dzCertificateMaxT = 0.0;
    // The current animated flight depth changes entirely on the GPU. A static
    // host certificate therefore applies only when an explicit depth is named.
    if (dz_zoom_exp <= 0.0 || !g_dzPayloadMatches) return;
    vec4 metadata = dzGroup(11);
    if (int(metadata.x) != DZ_CERT_TILE_COLUMNS
        || int(metadata.y) != DZ_CERT_TILE_ROWS) return;
    int requiredIterations = min(
        int(stack_cap + max(g_foldBoost, 0.0)),
        MAX_STACK_ITERS
    );
    if (int(metadata.w) < requiredIterations) return;

    vec2 normalized = clamp(
        normalizedScreen,
        vec2(0.0),
        vec2(0.999999)
    );
    ivec2 tile = ivec2(floor(normalized * vec2(
        float(DZ_CERT_TILE_COLUMNS),
        float(DZ_CERT_TILE_ROWS)
    )));
    int index = tile.y * DZ_CERT_TILE_COLUMNS + tile.x;
    vec4 packed = dzGroup(3 + index / 4);
    g_dzSafeSlabMask = uint(round(max(packed[index % 4], 0.0)));
    g_dzCertificateMaxT = max(metadata.z, 0.0);
}

float dzCertifiedPrimaryAdvance(float currentT) {
    uint mask = g_dzSafeSlabMask;
    float maximum = g_dzCertificateMaxT;
    if (maximum <= 0.0 || currentT < 0.0 || currentT >= maximum) return currentT;
    int slab = clamp(int(floor(currentT * float(DZ_CERT_T_SLABS) / maximum)),
                     0, DZ_CERT_T_SLABS - 1);
    float endpoint = currentT;
    for (int offset = 0; offset < DZ_CERT_T_SLABS; offset++) {
        int candidate = slab + offset;
        if (candidate >= DZ_CERT_T_SLABS
            || (mask & (1u << uint(candidate))) == 0u) break;
        endpoint = maximum * float(candidate + 1) / float(DZ_CERT_T_SLABS);
    }
    return max(currentT, endpoint);
}

float dzLog1p(float x) {
    if (abs(x) > 0.125) return log(1.0 + x);
    float term = x;
    float sum = x;
    for (int k = 2; k <= 8; k++) {
        term *= -x;
        sum += term / float(k);
    }
    return sum;
}

float dzExpm1(float x) {
    if (abs(x) > 0.125) return exp(x) - 1.0;
    float term = x;
    float sum = x;
    for (int k = 2; k <= 8; k++) {
        term *= x / float(k);
        sum += term;
    }
    return sum;
}

// Returns (sin(a+d)-sin(a), cos(a+d)-cos(a)) without subtracting two
// order-one trigonometric values.
vec2 dzSinCosIncrement(float angle, float delta) {
    float twiceSinHalf = 2.0 * sin(0.5 * delta);
    return vec2(
        twiceSinHalf * cos(angle + 0.5 * delta),
        -twiceSinHalf * sin(angle + 0.5 * delta)
    );
}

DzScalar dzScaledLog1p(DzScalar value) {
    if (value.m == 0.0) return value;
    if (value.e < -4.0) {
        // log1p(x) = x - x^2/2 + ...; retaining the quadratic correction keeps
        // the finite increment accurate without ever materializing tiny x.
        return dzSSub(value, dzSMul(dzSMulS(value, value), 0.5));
    }
    return dzS(dzLog1p(dzSToOrdinary(value)));
}

DzScalar dzScaledExpm1(DzScalar value) {
    if (value.m == 0.0) return value;
    if (value.e < -4.0) {
        return dzSAdd(value, dzSMul(dzSMulS(value, value), 0.5));
    }
    return dzS(dzExpm1(dzSToOrdinary(value)));
}

// atan(y, x) with y scaled and x ordinary. The tiny bounded domain uses its
// convergent odd series, preserving y's exponent. Large or near-antiparallel
// cases use the ordinary atan only when y is representable.
DzScalar dzScaledAtan(DzScalar y, float x, out bool conclusive) {
    conclusive = abs(x) > 1e-20;
    if (!conclusive) return dzSZero();
    DzScalar ratio = dzSDiv(y, x);
    if (ratio.e < -3.0 && x > 0.0) {
        DzScalar ratio2 = dzSMulS(ratio, ratio);
        return dzSSub(ratio, dzSDiv(dzSMulS(ratio, ratio2), 3.0));
    }
    if (y.e < -DZ_EXP_WINDOW) {
        conclusive = x > 0.0;
        return dzSZero();
    }
    return dzS(atan(dzSToOrdinary(y), x));
}

// Returns (sin(a+d)-sin(a), cos(a+d)-cos(a), 0) as a shared-exponent vector.
DzVec3 dzScaledSinCosIncrement(float angle, DzScalar delta) {
    if (delta.e < -3.0) {
        DzScalar sinInc = dzSMul(delta, cos(angle));
        DzScalar cosInc = dzSMul(delta, -sin(angle));
        DzScalar correction = dzSMul(dzSMulS(delta, delta), 0.5);
        sinInc = dzSSub(sinInc, dzSMul(correction, sin(angle)));
        cosInc = dzSSub(cosInc, dzSMul(correction, cos(angle)));
        return dzVFromComponents(sinInc, cosInc, dzSZero());
    }
    vec2 increment = dzSinCosIncrement(angle, dzSToOrdinary(delta));
    return dzV(vec3(increment, 0.0));
}

// Unit-circle group power without an inverse angle. Integer powers are
// periodic, so this path has no azimuth seam and remains defined by continuity
// when the transverse amplitude tends to zero.
vec2 dzIntegerUnitPower(vec2 unitValue, int exponent) {
    vec2 result = vec2(1.0, 0.0);
    for (int i = 0; i < 12; i++) {
        if (i >= exponent) break;
        result = vec2(
            result.x * unitValue.x - result.y * unitValue.y,
            result.x * unitValue.y + result.y * unitValue.x
        );
    }
    return result;
}

vec3 dzVToOrdinary(DzVec3 value) {
    if (value.e < -DZ_EXP_WINDOW) return vec3(0.0);
    if (value.e > DZ_EXP_WINDOW) return sign(value.m) * 1e30;
    return value.m * exp2(value.e);
}

bool dzVRepresentableAt(vec3 reference, DzVec3 value) {
    float referenceExponent = floor(log2(max(
        max(max(abs(reference.x), abs(reference.y)), abs(reference.z)), 1e-30
    )));
    return value.e >= referenceExponent - 20.0
        && value.e <= referenceExponent + DZ_EXP_WINDOW;
}

// One fold of slot `f` applied directly in binary32, mirroring the host's
// `Resolved::step` slot for slot.
//
// This is the continuation the absorb and rebase paths hand a sample to once
// its offset has outgrown the perturbation identities' conditioning. At that
// point the offset's exponent equals the reference's, so the sample point is
// exactly representable and plain arithmetic is as accurate as the offset
// recurrence would be. The transported records stop applying the moment the
// point is rebased — they describe the reference's branches, not this
// sample's — so from here every branch is decided directly.
void dzDirectFold(
    int f, vec3 seedSample, float seedDeriv,
    mat2 ra, mat2 rb, mat2 rc, mat2 rxw, mat2 ryw, mat2 rzw,
    inout vec3 P, inout DzScalar dr
) {
    float sc = scale;
    float l = fold_limit;
    float minR2 = min_radius * min_radius;
    float fixR2 = fixed_radius * fixed_radius;
    vec3 off = vec3(offset_x, offset_y, offset_z);
    float k = sc - 1.0;
    if (f == 1) {
        // Mandelbox: box fold, radial fold, scale about the seed.
        vec3 mid = clamp(P, -l, l) * 2.0 - P;
        float q = dot(mid, mid);
        float m = fixR2 / clamp(q, minR2, fixR2);
        P = mid * (m * sc) + seedSample;
        dr = dzSAdd(dzSMul(dr, abs(m * sc)), dzS(seedDeriv));
    } else if (f == 2) {
        // Amazing Box: two-axis fold, radial with the scale as numerator.
        vec3 mid = P;
        mid.xy = clamp(mid.xy, -l, l) * 2.0 - mid.xy;
        float q = dot(mid, mid);
        float m = sc / clamp(q, minR2, fixR2);
        P = mid * m + off;
        dr = dzSMul(dr, abs(m));
    } else if (f == 3) {
        // Menger: fold to the descending-sorted octant, scale, trailing shift.
        vec3 a = abs(P);
        if (a.x < a.y) a.xy = a.yx;
        if (a.x < a.z) a.xz = a.zx;
        if (a.y < a.z) a.yz = a.zy;
        float shift = off.z * k;
        P = a * sc - vec3(off.x * k, off.y * k, shift);
        if (P.z < -0.5 * shift) P.z += shift;
        dr = dzSMul(dr, abs(sc));
    } else if (f == 4) {
        // Sierpinski: three symmetry planes, then a uniform scale.
        vec3 a = P;
        if (a.x + a.y < 0.0) a.xy = -a.yx;
        if (a.x + a.z < 0.0) a.xz = -a.zx;
        if (a.z + a.y < 0.0) a.zy = -a.yz;
        P = a * sc - off * k;
        dr = dzSMul(dr, abs(sc));
    } else if (f == 5) {
        // Mandelbulb: triplex power about the seed, with the host's radius
        // regularization so the two paths agree at the same point.
        float r = length(P);
        if (r < 1e-20) {
            P = seedSample;
            dr = dzSAdd(dr, dzS(seedDeriv));
        } else {
            float theta = acos(clamp(P.z / max(r, 1e-6), -1.0, 1.0));
            float phi = atan(P.y, P.x);
            float pt = power * theta;
            float pp = power * phi;
            float rp = pow(r, power);
            P = rp * vec3(sin(pt) * cos(pp), sin(pt) * sin(pp), cos(pt))
                + seedSample;
            // Same azimuthal anisotropy as the perturbed bulb path: dr is the
            // full off-axis singular value, not the radial multiplier alone.
            float aniso = clamp(
                abs(sin(pt)) / max(abs(sin(theta)), 1e-6), 1.0, power
            );
            dr = dzSAdd(
                dzSMul(dr, power * pow(r, power - 1.0) * aniso),
                dzS(seedDeriv)
            );
        }
    } else if (f == 6) {
        // Pseudo-Kleinian: box fold, inversion with no upper clamp.
        vec3 mid = clamp(P, -l, l) * 2.0 - P;
        float q = dot(mid, mid);
        float m = 1.0 / max(q, minR2);
        P = mid * m - off;
        dr = dzSMul(dr, m);
    } else if (f == 7) {
        P = vec3(P.x * lin_x + P.y * lin_mix,
                 P.y * lin_y + P.z * lin_mix,
                 P.z * lin_z + P.x * lin_mix);
        dr = dzSMul(
            dr, max(max(abs(lin_x), abs(lin_y)), abs(lin_z)) + abs(lin_mix)
        );
    } else if (f == 8) {
        P.xy = ra * P.xy;
        P.yz = rb * P.yz;
        P.xz = rc * P.xz;
    } else if (f == 9) {
        // Co-cube: partial sort, reflection about the corner, uniform scale.
        vec3 a = abs(P);
        if (a.x < a.y) a.xy = a.yx;
        if (a.y < a.z) a.yz = a.zy;
        a.z = cocube - abs(a.z - cocube);
        P = a * sc - off * k;
        dr = dzSMul(dr, abs(sc));
    } else if (f == 10) {
        vec4 q4 = vec4(P, 0.0);
        vec2 t;
        t = rxw * vec2(q4.x, q4.w); q4.x = t.x; q4.w = t.y;
        t = ryw * vec2(q4.y, q4.w); q4.y = t.x; q4.w = t.y;
        t = rzw * vec2(q4.z, q4.w); q4.z = t.x; q4.w = t.y;
        P = q4.xyz;
    }
}

float stackDE_dz(vec3 pos, DeLod lod, out DeInfo info) {
    g_dbgExecFolds = 0.0;
    if (!g_dzPayloadMatches) {
        info = DeInfo(0.0, 0.0, 0.0);
        return 1.0;
    }
    vec3 A = g_dzRef;
    vec3 P = A;
    // `pos` is always a frame-local offset. Attach the physical frame exponent
    // before any tiny physical f32 can exist.
    DzVec3 e = dzFrameVector(pos);
    DzVec3 initialOffset = e;
    DzScalar dr = dzS(1.0);

    // Per-ray rather than per-frame: the budget follows the footprint this call
    // was handed and not the camera's own radius. A ray landing on geometry the
    // camera is not framing has a footprint of its own, and a budget that
    // ignored it would stop rising while the footprint kept falling, which is
    // how the nearest surface stops gaining detail and reads as a blob.
    float boost = g_foldBoost;
    if (lod.iters > 0.0) {
        // Footprint octaves ON TOP of the depth octaves, not instead of them.
        // The footprint is frame-local, so it carries no depth information:
        // written as a replacement, the primary march's budget froze at about
        // two dozen folds at every depth, while a sample at depth Z stays
        // bounded for on the order of Z * log2(10) / foldOctaves() folds.
        // Past roughly seven decades every sample in frame outlived the
        // budget, the whole frame reported interior, and the dive read as
        // flying into a solid wall.
        boost = floor(max(-log2(lod.iters), 0.0) + max(g_zoomLog2, 0.0)
                      + g_foldDither);
    }
    int total = int(clamp(stack_cap + boost, 1.0, float(MAX_STACK_ITERS)));
    if (lod.cap > 0.0) {
        total = int(clamp(lod.cap, 1.0, float(MAX_STACK_ITERS)));
    }

    ivec4 order = stackOrder(stack_order);
    // Vectors, not arrays. A local array that is indexed by a non-constant is
    // placed in thread-private scratch memory by the Metal compiler, and that
    // is a fixed cost paid on every one of the roughly seventy estimator calls
    // a shaded pixel makes, independent of how many folds the call runs.
    ivec4 forms = ivec4(slot0_formula, slot1_formula, slot2_formula,
                        slot3_formula);
    ivec4 rates = ivec4(
        int(clamp(slot0_iters, 0.0, 8.0)),
        int(clamp(slot1_iters, 0.0, 8.0)),
        int(clamp(slot2_iters, 0.0, 8.0)),
        int(clamp(slot3_iters, 0.0, 8.0))
    );
    int r0 = rates[order.x];
    int r1 = rates[order.y];
    int r2s = rates[order.z];
    int r3 = rates[order.w];
    int c1 = r0 + r1;
    int c2 = c1 + r2s;
    int cycle = c2 + r3;
    if (cycle < 1) return 1.0;

    // The bulb escapes on its own pre-guard radius rather than on the shared
    // bailout, so whether the stack contains one changes what "escaped" means.
    bool hasBulb = false;
    for (int k = 0; k < 4; k++) {
        if (forms[k] == 5 && rates[k] > 0) hasBulb = true;
    }

    float linx = lin_x;

    vec3 jseed = vec3(julia_x, julia_y, julia_z);
    // The seed is the sample point blended toward a constant, so its increment
    // is the original offset scaled by how much of the sample point survives.
    vec3 seedRef = mix(A, jseed, julia_amount);
    DzVec3 seedInc = dzVMul(initialOffset, 1.0 - julia_amount);
    float seedDeriv = 1.0 - julia_amount;

    float bail2 = bailout * bailout;
    mat2 ra = rot(rot_a);
    mat2 rb = rot(rot_b);
    mat2 rc = rot(rot_c);
    mat2 rxw = rot(wPlaneAngle(rot_xw));
    mat2 ryw = rot(wPlaneAngle(rot_yw));
    mat2 rzw = rot(wPlaneAngle(rot_zw));
    vec3 off = vec3(offset_x, offset_y, offset_z);
    float minR2 = min_radius * min_radius;
    float fixR2 = fixed_radius * fixed_radius;
    float sc = scale;

    // The validated host record is the only source of the offset recurrence,
    // but it is no longer the only source of folds: a sample whose offset has
    // been absorbed into its reference continues in direct binary32, so a
    // payload shorter than the budget shortens the certified prefix rather
    // than the march.
    int orbitLen = int(dzGroup(0).z);
    // For bulb payloads the transported length is the host's measurement of
    // how long samples near the anchor actually survive, and escape times at
    // depth are set by that local survival, not by the decade count (measured
    // five to ten times larger at nine decades). The records exist to be
    // marched: flooring the deep budget at the payload length is what lets an
    // escaping sample actually escape instead of being declared inside at a
    // budget the dynamics outlive. Non-bulb payloads transport the authored
    // maximum instead of a survival measurement, so the floor would only
    // inflate their cost.
    // Never over a caller's explicit cap. This floor exists so the *primary*
    // march can outlive the anchor's local dynamics at depth, but it ran after
    // `lod.cap` and overrode it, so every deliberately cheap probe in the
    // shader (the smoothed normal for curvature, the fog's fold-band taps) was
    // silently promoted back to the full budget. A caller that names a cap is
    // stating what its result is for, and a smoothed normal or a fog shell
    // does not become more correct by folding twenty more times.
    if (hasBulb && g_zoomLog2 > 0.5 && lod.cap <= 0.0) {
        total = int(clamp(
            max(float(total), min(float(orbitLen), float(MAX_STACK_ITERS))),
            1.0, float(MAX_STACK_ITERS)
        ));
    }
    int certified = min(total, orbitLen);

    // Once `direct` latches, the records stop being consulted and every
    // remaining fold is `dzDirectFold`. The seed the direct folds add is the
    // sample's own, materialized at the moment of rebase; at that moment the
    // offset is order one, so the materialization is exact, and at true depth
    // the seed offset underflows to zero, which is the correct limit.
    bool direct = false;
    vec3 seedSample = seedRef;
    // Bulb slots escape on their own radius-two pre-guard rather than the
    // shared bailout, and the direct path must agree with the perturbed one
    // about what "escaped" means.
    float escape2 = hasBulb ? min(bail2, 4.0) : bail2;
    // Records ran out under a sample whose offset was too deep to rebase:
    // the honest report is "still inside", exactly as when the budget ends.
    bool recordsExhausted = false;

    int done = 0;
    // Skip the cheap majority of the orbit in one matrix-vector product.
    //
    // Every march step used to walk the fold loop from zero, and the early
    // folds are precisely where the sample is still a small perturbation of
    // the reference and the exact identities are doing arithmetic that a
    // single linear map reproduces to about one percent. The transported
    // parameter Jacobian gives `e_k = D_k w` at any fold directly, so the loop
    // starts at the last fold where that model is still trustworthy and runs
    // the exact recurrence only across the folds that actually decide.
    int start = 0;
    {
        float offsetLog2 = e.e + log2(max(
            max(max(abs(e.m.x), abs(e.m.y)), abs(e.m.z)), 1e-30));
        int horizon = dzLinearHorizon(offsetLog2, min(certified, total) - 1);
        if (horizon > 0) {
            mat3 unit = dzJacobianUnit(horizon);
            float scale = dzJacobianLog2(horizon);
            // `D_k w`, with the reference's own magnitude carried in the
            // exponent so a derivative of 10^100 never becomes an f32.
            e = dzVNormalize(DzVec3(unit * e.m, e.e + scale));
            P = dzRec(horizon, 3).xyz;
            // The accumulated derivative is that magnitude, by definition.
            dr = dzSNormalize(DzScalar(1.0, scale));
            start = horizon + 1;
            done = horizon + 1;
        }
    }

    float dCoarse = -1.0;
    float wCoarse = 0.0;
    // The level-of-detail truncation gate fired: the fold loop stopped because
    // the estimator resolved the pixel footprint, not because the orbit's fate
    // was decided. See spec/fractal-truncated-surface-march.md.
    bool lodTruncated = false;
    // The fold whose iso-escape shell this frame renders. At depth the true
    // set is locally space-filling, so distance-to-the-set is zero everywhere
    // and the camera itself sits inside any resolution-fattened version of
    // it; there is no scene unless the rendered surface recedes with the
    // dive. Holding the shell a fixed number of folds past the authored
    // cutoff, plus the folds the magnification has consumed, keeps it the
    // same relative distance ahead at every depth (the iteration fog picks
    // its shell with the same rule). The camera's own neighborhood escapes
    // below the shell fold, the shell stays ahead, and descending raises the
    // shell fold, so approach permanently reveals finer structure instead of
    // ending inside a blob.
    // Depth octaves plus the ray's own footprint octaves: a ray resolving a
    // finer footprint sees a correspondingly deeper shell, so approaching a
    // surface reveals finer folds of it, per ray, without moving the shell
    // for the rest of the frame.
    float shellOctaves = max(g_zoomLog2, 0.0);
    if (lod.iters > 0.0) {
        shellOctaves += max(-log2(lod.iters), 0.0);
    }
    // The measured table first, the derived rate only as a fallback. The
    // footprint octaves still apply on top: a ray resolving finer detail sees
    // a correspondingly deeper shell, which is what makes approach reveal
    // structure rather than arrive at a surface.
    float ratedExp = max(dzOrbitBudgetExp(), 1e-6);
    float currentExp = max(g_zoomLog2, 0.0) / DZ_LOG2_10;
    float measured = dzMeasuredShellFold(currentExp / ratedExp);
    float footprintOctaves = 0.0;
    if (lod.iters > 0.0) footprintOctaves = max(-log2(lod.iters), 0.0);
    // Both measurements are transported and read, and neither drives
    // placement yet.
    //
    // The host measures where this frame's surface resolves and where the
    // camera itself escapes, at twelve depths, in arbitrary precision. The
    // numbers are sound and they quantify the failure precisely: past about
    // 2.7 decades the frame resolves between folds 100 and 218 while this
    // formula asks for about 17, which is why the dive renders as a solid.
    // But driving the shell from them directly is worse, not better. Placing
    // it at the measured surface fold leaves almost nothing interior and the
    // rays miss into sky; placing it past the camera's escape fold does the
    // same more strongly. Both bounds are real and both are satisfiable only
    // in a narrow band, which one depth (3.2 decades, detail 25.4, the best
    // frame this shader has produced) sits inside by luck rather than policy.
    //
    // The honest state is therefore: the measurement is available, the policy
    // that satisfies both bounds together is not yet found, and the shell
    // stays on the derived rate so behaviour is no worse than before it
    // existed. `dz_debug` 12 and the host's `measured_shell_fold_table`
    // diagnostic are how the next attempt should be judged.
    // The host places the shell where a target share of the frame is still
    // bounded, which is the silhouette criterion measured rather than assumed,
    // and the camera's own escape fold comes with it so the eye is outside
    // what the frame draws. The derived rate remains the fallback for payloads
    // that carry no measurement.
    // Measured, transported, read - and deliberately not driving placement.
    //
    // The host's table says where this frame's silhouette actually sits, and
    // obeying it directly is correct and unaffordable: a shell at eighty-odd
    // folds costs four to twenty times the per-pixel budget, which measured
    // out at better than five seconds a frame at 160 by 90 and tripped the GPU
    // watchdog at anything larger. The derived rate is cheap and wrong; the
    // measurement is right and cheap only once a fold costs far less than it
    // does today. Both numbers stay available so the next attempt can judge a
    // cheaper fold against the requirement rather than against a guess.
    float cameraFold = dzMeasuredCameraFold(currentExp / ratedExp);
    // Three floors, each answering a different failure.
    //
    // `authored` is the detail the shot was written against, and without it a
    // shallow frame takes the measured shell literally - three or four folds -
    // and renders a smooth lit blob. `measured` is where this frame's
    // silhouette actually sits, which the derived rate understates by an order
    // of magnitude at depth. `cameraFold` keeps the eye outside the solid it
    // is drawing. The deepest of the three is the one that binds.
    // The measurement is transported and read; the derived rate still places
    // the shell. That is a cost decision, not a correctness one.
    //
    // Obeying the measurement is more correct - it is where this frame's
    // silhouette actually sits, and it renders the depths it covers with
    // visibly more detail (25.9 against 19.5 on the same frame). It also costs
    // four to twenty times as much, because a shell at eighty folds is eighty
    // folds on every march step, and a frame that overruns the per-pixel
    // budget does not just drop frames: it exceeds the OS GPU watchdog and
    // takes the desktop down. Until a fold is far cheaper, the cheap rate is
    // the safe default and the measurement is the yardstick to judge it by.
    float authored = stack_cap + footprintOctaves / max(foldOctaves(), 0.5);
    // The shell is where the records end, and that is not a coincidence to be
    // re-derived here: the host bisects the anchor onto the boundary of the
    // same level set it transports records for. Marching short of it reports a
    // frame that is uniformly still-bounded - a solid wall - and marching past
    // it runs out of records. Every rule this line has carried before was an
    // independent guess at a number the host already knows, and each one
    // disagreed with it by a different factor: the derived rate by four to
    // seven, the units error in the host's own budget by three.
    float kShell = float(certified)
        + 0.0 * (authored + measured + cameraFold + shellOctaves);
    kShell = clamp(kShell, 1.0, float(total));
    for (int i = 0; i < MAX_STACK_ITERS; i++) {
        if (i < start) continue;
        g_dbgExecFolds += 1.0;
        // Folding past the shell cannot change what this pixel renders: a
        // sample still bounded here is inside the shell by definition.
        if (i >= total || float(i) >= kShell) break;
        if (lod.iters > 0.0) {
            float feature = dzDistanceToEstimatorUnits(dzSDivS(dzS(1.0), dr));
            float wl = clamp((lod.iters * 1.6 - feature) / (lod.iters * 0.8), 0.0, 1.0);
            if (wl > 0.0) {
                if (dCoarse < 0.0) {
                    dCoarse = dzDistanceToEstimatorUnits(dzSMul(
                        dzSDivS(dzLengthFromReference(P, e), dr), 0.62
                    ));
                    wCoarse = wl;
                } else {
                    lodTruncated = true;
                    g_dbgExit = 3.0;
                    break;
                }
            }
        }

        int c = i % cycle;
        int which;
        if (c < r0) { which = order.x; }
        else if (c < c1) { which = order.y; }
        else if (c < c2) { which = order.z; }
        else { which = order.w; }
        int f = forms[which];

        // Past the transported records the offset recurrence has nothing to
        // consume. A representable offset switches to direct iteration; one
        // too deep to materialize reports the sample still inside, which is
        // what an exhausted certified orbit means.
        if (!direct && i >= certified) {
            if (!dzVRepresentableAt(P, e)) {
                recordsExhausted = true;
                g_dbgExit = 5.0;
                break;
            }
            P += dzVToOrdinary(e);
            e = dzVZero();
            seedSample = seedRef + dzVToOrdinary(seedInc);
            direct = true;
        }
        if (direct) {
            dzDirectFold(f, seedSample, seedDeriv, ra, rb, rc, rxw, ryw, rzw,
                         P, dr);
            done = i + 1;
            if (dot(P, P) > escape2) { g_dbgExit = 7.0; break; }
            continue;
        }
        // Set by a perturbation branch that met a boundary its identities do
        // not cross (the bulb's radius floor or an unresolved chart). The
        // fold for this iteration has not run yet; the rebase block below
        // runs it directly.
        bool needRebase = false;

        // Every branch decision below reads margins from groups five and six.
        // Fetching them once per fold, rather than a whole group per margin
        // read, halves the payload traffic of the recurrence.
        vec4 mg0 = dzRec(i, 5);
        vec4 mg1 = dzRec(i, 6);
        float marr[8] = float[8](mg0.x, mg0.y, mg0.z, mg0.w,
                                 mg1.x, mg1.y, mg1.z, mg1.w);
        if (f == 1) {
            // Mandelbox: box fold, radial fold, scale about the seed.
            //
            // With a transported record the reference is *read* at every stage
            // and never recomputed. That is the whole correction: recomputing an
            // intermediate in f32 while taking the endpoint from the host's
            // double-double left the two disagreeing by an f32 rounding, so the
            // pair (P, e) stopped describing the sample it started as, and the
            // derivative amplified that within a dozen folds.
            vec4 g0, g1, g2;
            g0 = dzRec(i, 0);
            g1 = dzRec(i, 1);
            g2 = dzRec(i, 3);
            P = g0.xyz;

            // The reference's branch decisions are carried in the record's
            // fourth component, but consuming them is not yet safe and is
            // disabled here. Forcing the reference's branch while still deriving
            // the *margins* from the transported point creates a fresh
            // inconsistency: if the record says "inside" and the f32 point sits
            // an ulp outside, `mHi` is negative and the identity's case logic is
            // fed a contradiction. Measured, enabling it moved agreement from
            // 5.5% of pixels to 0.7%. Doing it properly means transporting the
            // margins as well, so the sample's branch is decided by comparing
            // the offset against a stored distance rather than a recomputed one,
            // which is Rule 1 of the whole scheme applied one level deeper.
            int code = int(g2.w);
            DzScalar en[3];
            for (int k = 0; k < 3; k++) {
                float po;
                int fb = -1;
                if (code >= 0) {
                    int d = (k == 0) ? 1 : ((k == 1) ? 3 : 9);
                    fb = (code / d) % 3;
                }
                en[k] = dzFoldCompStored(
                    P[k], dzVComponent(e, k), fold_limit, fb,
                    marr[2 * k], marr[2 * k + 1], po
                );
            }
            e = dzVFromComponents(en[0], en[1], en[2]);
            P = g1.xyz;

            // The squared radius comes from the record when there is one, so it
            // matches the reference it describes exactly.
            float q = g1.w;
            // The exact increment in the squared radius. Both terms pair a
            // small quantity with an order-one one, so nothing cancels.
            DzScalar dq = dzSquaredNormIncrement(P, e);
            float mRef, mSmp;
            e = dzRadialStored(
                P, e, q, dq, minR2, fixR2, fixR2,
                (code / 27) % 3, marr[6], marr[7],
                mRef, mSmp
            );
            dr = dzSMul(dr, mSmp);

            e = dzVAdd(dzVMul(e, sc), seedInc);
            dr = dzSAdd(dzSMul(dr, abs(sc)), dzS(seedDeriv));
            P = g2.xyz;
        } else if (f == 2) {
            // Amazing Box: two-axis fold, then the same radial form with the
            // fold scale as its numerator. Adds a uniform, so no seed term.
            vec4 g1, g2;
            P = dzRec(i, 0).xyz;
            g1 = dzRec(i, 1);
            g2 = dzRec(i, 3);
            int code = int(g2.w);
            for (int k = 0; k < 2; k++) {
                float po;
                int fb = (code >= 0) ? (code / ((k == 0) ? 1 : 3)) % 3 : -1;
                DzScalar en = dzFoldCompStored(
                    P[k], dzVComponent(e, k), fold_limit, fb,
                    marr[2 * k], marr[2 * k + 1], po
                );
                DzScalar components[3] = DzScalar[3](
                    dzVComponent(e, 0), dzVComponent(e, 1), dzVComponent(e, 2)
                );
                components[k] = en;
                e = dzVFromComponents(components[0], components[1], components[2]);
            }
            P = g1.xyz;
            float q = g1.w;
            DzScalar dq = dzSquaredNormIncrement(P, e);
            float mRef, mSmp;
            e = dzRadialStored(
                P, e, q, dq, minR2, fixR2, sc,
                (code / 27) % 3, marr[4], marr[5],
                mRef, mSmp
            );
            dr = dzSMul(dr, abs(mSmp));
            P = g2.xyz;
        } else if (f == 3) {
            // Menger. The trailing conditional shift is the one primitive in
            // the whole stack that is genuinely discontinuous: a misdecision
            // there costs the shift itself rather than the distance to the
            // boundary. It is also why the record carries a post-scale point:
            // that shift branches after the scale, so its margin is measured
            // against a reference no other slot needs.
            P = dzRec(i, 0).xyz;
            int code = int(dzRec(i, 3).w);
            e = dzAbsStored(
                e,
                vec3(marr[0], marr[1], marr[2]),
                code
            );
            dzCondSwapStored(e, 0, 1, marr[3], (code & 8) != 0);
            dzCondSwapStored(e, 0, 2, marr[4], (code & 16) != 0);
            dzCondSwapStored(e, 1, 2, marr[5], (code & 32) != 0);
            e = dzVMul(e, sc);
            P = dzRec(i, 2).xyz;
            float thresh = -0.5 * off.z * (sc - 1.0);
            float shift = off.z * (sc - 1.0);
            float margin = marr[6];
            bool rS = (code & 64) != 0;
            DzScalar ez = dzVComponent(e, 2);
            bool sS = dzSLessOrd(ez, -margin);
            if (rS != sS) {
                e = dzVWithComponent(e, 2, dzSAdd(ez, dzS(rS ? -shift : shift)));
            }
            P = dzRec(i, 3).xyz;
            dr = dzSMul(dr, abs(sc));
        } else if (f == 4) {
            // Kaleidoscopic Sierpinski: three symmetry planes of the
            // tetrahedron, each continuous across its own boundary.
            P = dzRec(i, 0).xyz;
            int code = int(dzRec(i, 3).w);
            dzCondReflectPairStored(e, 0, 1, marr[0], (code & 1) != 0);
            dzCondReflectPairStored(e, 0, 2, marr[1], (code & 2) != 0);
            dzCondReflectPairStored(e, 2, 1, marr[2], (code & 4) != 0);
            e = dzVMul(e, sc);
            dr = dzSMul(dr, abs(sc));
            P = dzRec(i, 3).xyz;
        } else if (f == 5) {
            vec4 g0 = dzRec(i, 0);
                vec4 g2 = dzRec(i, 2);
                vec4 g3 = dzRec(i, 3);
                vec4 bulb = dzRec(i, 4);
                vec4 seam = dzRec(i, 8);
                if (!(seam.w > 0.5)) { g_dbgExit = 8.0; break; }
                P = g0.xyz;
                float radiusRef = bulb.x;
                DzScalar radiusDelta = dzRadiusIncrement(
                    radiusRef, dzSquaredNormIncrement(P, e)
                );
                float preRadius2Margin = marr[0];
                if (dzSGreaterOrd(radiusDelta, preRadius2Margin)) {
                    g_dbgExit = 2.0;
                    break;
                }
                // The angular identity below is for the unregularized spherical
                // chart. Crossing the shader's max(radius, 1e-6) branch requires
                // a different identity, so hand the sample to the direct fold
                // rather than inventing one.
                float radiusFloorMargin = marr[1];
                if (radiusFloorMargin <= 0.0
                    || dzSCompare(radiusDelta, dzSNeg(dzS(radiusFloorMargin))) <= 0) {
                    needRebase = true;
                }
                if (!needRebase) {

                DzScalar radialRatio = dzSDiv(radiusDelta, max(radiusRef, 1e-30));
                DzScalar radiusPowerDelta = dzSMul(
                    dzScaledExpm1(dzSMul(dzScaledLog1p(radialRatio), power)),
                    bulb.w
                );

                float rhoRef = length(P.xy);
                DzVec3 eRho = dzVWithComponent(e, 2, dzSZero());
                DzScalar rhoDelta = dzRadiusIncrement(
                    rhoRef, dzSquaredNormIncrement(vec3(P.xy, 0.0), eRho)
                );
                float polarMargin = marr[2];
                bool sampleOffAxis = polarMargin > 0.0
                    && dzSCompare(rhoDelta, dzSNeg(dzS(polarMargin))) > 0;
                DzScalar ez = dzVComponent(e, 2);
                DzScalar thetaNumerator = dzSSub(
                    dzSMul(rhoDelta, P.z), dzSMul(ez, rhoRef)
                );
                float thetaDenominator = radiusRef * radiusRef
                    + dzSToOrdinary(dzSAdd(
                        dzSMul(rhoDelta, rhoRef), dzSMul(ez, P.z)
                    ));

                DzScalar phiNumerator = dzSSub(
                    dzSMul(dzVComponent(e, 1), P.x),
                    dzSMul(dzVComponent(e, 0), P.y)
                );
                float phiDenominator = rhoRef * rhoRef
                    + dzSToOrdinary(dzSAdd(
                        dzSMul(dzVComponent(e, 0), P.x),
                        dzSMul(dzVComponent(e, 1), P.y)
                    ));

                bool thetaOk, phiOk;
                DzScalar thetaDelta = dzScaledAtan(
                    thetaNumerator, thetaDenominator, thetaOk
                );
                DzScalar phiDelta = dzScaledAtan(
                    phiNumerator, phiDenominator, phiOk
                );

                // Principal-angle winding is selected from the transported seam
                // side and signed x/y margins. It is never selected by adding a
                // tiny delta to the order-one reference angle.
                float seamSide = seam.y;
                bool refXNegative = marr[3] < 0.0;
                bool sampleXNegative = dzSLessOrd(
                    dzVComponent(e, 0), -marr[3]
                );
                bool sampleYNegative = dzSLessOrd(
                    dzVComponent(e, 1), -marr[4]
                );
                bool refYNegative = seamSide < 0.0;
                phiDelta = dzSAdd(
                    phiDelta, dzS(seam.z * TAU)
                );
                if (refXNegative && sampleXNegative
                    && refYNegative != sampleYNegative) {
                    phiDelta = dzSAdd(
                        phiDelta,
                        dzS(refYNegative ? TAU : -TAU)
                    );
                } else if (refXNegative != sampleXNegative
                           && abs(phiDenominator) < 1e-12) {
                    phiOk = false;
                }

                int integerPower = int(round(power));
                bool exactIntegerPower = integerPower >= 2 && integerPower <= 12
                    && power == float(integerPower);
                bool groupPowerAxis = rhoRef == 0.0 && thetaOk
                    && (P.z >= 0.0 || exactIntegerPower);
                DzVec3 directionDelta = dzVZero();
                if (groupPowerAxis) {
                    // The azimuth chart is undefined on the axis, but the
                    // transverse amplitude vanishes on the positive axis for
                    // every power and on both axes for integer power. The sample
                    // offset supplies its own azimuth; no reference angle is
                    // subtracted.
                    float transverseLength = length(e.m.xy);
                    vec2 sampleAzimuth = transverseLength > 0.0
                        ? e.m.xy / transverseLength
                        : vec2(1.0, 0.0);
                    vec2 poweredAzimuth;
                    if (exactIntegerPower) {
                        poweredAzimuth = dzIntegerUnitPower(
                            sampleAzimuth, integerPower
                        );
                    } else {
                        float samplePhi = atan(sampleAzimuth.y, sampleAzimuth.x);
                        poweredAzimuth = vec2(
                            cos(power * samplePhi), sin(power * samplePhi)
                        );
                    }

                    // Evaluate the polar increment around the exact axis value,
                    // not around a binary32 approximation of p*pi. The latter
                    // leaves a residual sin(p*pi) that eventually dominates an
                    // arbitrarily small deep-zoom offset.
                    DzVec3 axisPolar = dzScaledSinCosIncrement(
                        0.0, dzSMul(thetaDelta, power)
                    );
                    float axisSign = (P.z < 0.0 && (integerPower % 2) != 0)
                        ? -1.0
                        : 1.0;
                    DzScalar samplePolarSin = dzSMul(
                        dzVComponent(axisPolar, 0), axisSign
                    );
                    DzScalar polarCosInc = dzSMul(
                        dzVComponent(axisPolar, 1), axisSign
                    );
                    directionDelta = dzVFromComponents(
                        dzSMul(samplePolarSin, poweredAzimuth.x),
                        dzSMul(samplePolarSin, poweredAzimuth.y),
                        polarCosInc
                    );
                } else if (!sampleOffAxis || rhoRef <= 1e-20 || !thetaOk
                           || !phiOk) {
                    // A non-integer axis or an unresolved chart has no common
                    // continuation for the offset recurrence. The direct fold
                    // needs no chart, so the rebase block takes over.
                    needRebase = true;
                } else {
                    float thetaPowered = bulb.y * power;
                    float phiPowered = bulb.z * power;
                    DzVec3 thetaInc = dzScaledSinCosIncrement(
                        thetaPowered, dzSMul(thetaDelta, power)
                    );
                    DzVec3 phiInc = dzScaledSinCosIncrement(
                        phiPowered, dzSMul(phiDelta, power)
                    );
                    DzScalar thetaSinInc = dzVComponent(thetaInc, 0);
                    DzScalar thetaCosInc = dzVComponent(thetaInc, 1);
                    DzScalar phiSinInc = dzVComponent(phiInc, 0);
                    DzScalar phiCosInc = dzVComponent(phiInc, 1);
                    float sinTheta = sin(thetaPowered);
                    float cosPhi = cos(phiPowered);
                    float sinPhi = sin(phiPowered);
                    directionDelta = dzVFromComponents(
                        dzSAdd(
                            dzSAdd(dzSMul(thetaSinInc, cosPhi),
                                   dzSMul(phiCosInc, sinTheta)),
                            dzSMulS(thetaSinInc, phiCosInc)
                        ),
                        dzSAdd(
                            dzSAdd(dzSMul(thetaSinInc, sinPhi),
                                   dzSMul(phiSinInc, sinTheta)),
                            dzSMulS(thetaSinInc, phiSinInc)
                        ),
                        thetaCosInc
                    );
                }

                if (!needRebase) {
                e = dzVAdd(
                    dzVAdd(
                        dzVMul(directionDelta, bulb.w),
                        dzVAdd(
                            dzVMulS(dzV(g2.xyz), radiusPowerDelta),
                            dzVMulS(directionDelta, radiusPowerDelta)
                        )
                    ),
                    seedInc
                );
                float radiusSample = radiusRef + dzSToOrdinary(radiusDelta);
                // The scalar p*r^(p-1) tracks only the radial multiplier of a
                // non-conformal map. The paper's off-axis singular value is
                // m * max(1, |sin(p theta) / sin theta|), and dropping the
                // anisotropy under-reports the realized derivative by a
                // converged factor of 1500-3000 (notes/deep-zoom-dichotomy.md
                // 5.5). An under-reported dr over-reports the marched
                // distance by the same factor, and at depth the geometry is
                // thinner than the inflated steps, so rays skipped straight
                // through the set. The sample's polar angle differs from the
                // reference's by a deep-zoom offset, so the reference angle
                // carries the factor. Clamped to the Chebyshev bound p, which
                // is also its polar-axis limit.
                float bulbAniso = clamp(
                    abs(sin(power * bulb.y)) / max(abs(sin(bulb.y)), 1e-6),
                    1.0, power
                );
                dr = dzSAdd(
                    dzSMul(dr, pow(radiusSample, power - 1.0) * power * bulbAniso),
                    dzS(seedDeriv)
                );
                P = g3.xyz;
                }
                // Closes the radius-floor `if (!needRebase)` wrapper: the
                // radialRatio-through-direction block above runs only on the
                // unregularized chart.
                }
        } else if (f == 6) {
            // Pseudo-Kleinian: box fold plus a strict inversion, which is the
            // radial helper with no upper clamp.
            P = dzRec(i, 0).xyz;
            DzScalar en[3];
            vec4 g1, g2;
            g1 = dzRec(i, 1);
            g2 = dzRec(i, 3);
            int code = int(g2.w);
            for (int k = 0; k < 3; k++) {
                float po;
                int fb = (code >= 0)
                    ? (code / ((k == 0) ? 1 : ((k == 1) ? 3 : 9))) % 3
                    : -1;
                en[k] = dzFoldCompStored(
                    P[k], dzVComponent(e, k), fold_limit, fb,
                    marr[2 * k], marr[2 * k + 1], po
                );
            }
            e = dzVFromComponents(en[0], en[1], en[2]);
            P = g1.xyz;
            float q = g1.w;
            DzScalar dq = dzSquaredNormIncrement(P, e);
            float mRef, mSmp;
            e = dzRadialStored(
                P, e, q, dq, minR2, 1e30, 1.0,
                (code / 27) % 3, marr[6], 1e30,
                mRef, mSmp
            );
            dr = dzSMul(dr, mSmp);
            P = g2.xyz;
        } else if (f == 7) {
            // Lin combine: linear, so the increment is the same map with the
            // constant part dropped.
            P = vec3(P.x * linx + P.y * lin_mix,
                     P.y * lin_y + P.z * lin_mix,
                     P.z * lin_z + P.x * lin_mix);
            e.m = vec3(e.m.x * linx + e.m.y * lin_mix,
                       e.m.y * lin_y + e.m.z * lin_mix,
                       e.m.z * lin_z + e.m.x * lin_mix);
            e = dzVNormalize(e);
            dr = dzSMul(
                dr, max(max(abs(linx), abs(lin_y)), abs(lin_z)) + abs(lin_mix)
            );
        } else if (f == 8) {
            P.xy = ra * P.xy; P.yz = rb * P.yz; P.xz = rc * P.xz;
            e.m.xy = ra * e.m.xy; e.m.yz = rb * e.m.yz; e.m.xz = rc * e.m.xz;
            e = dzVNormalize(e);
        } else if (f == 9) {
            // Co-cube: partial sort, then a reflection about the corner on the
            // last axis.
            P = dzRec(i, 0).xyz;
            int code = int(dzRec(i, 3).w);
            e = dzAbsStored(
                e,
                vec3(marr[0], marr[1], marr[2]),
                code
            );
            dzCondSwapStored(e, 0, 1, marr[3], (code & 8) != 0);
            dzCondSwapStored(e, 1, 2, marr[4], (code & 16) != 0);
            P = dzRec(i, 1).xyz;
            // p.z = cocube - |p.z - cocube|, an abs about a shifted origin.
            float u = marr[5];
            bool rNeg = (code & 32) != 0;
            DzScalar ez = dzVComponent(e, 2);
            bool sNeg = dzSLessOrd(ez, -u);
            DzScalar dAbs = (rNeg == sNeg)
                ? (rNeg ? dzSNeg(ez) : ez)
                : (rNeg
                    ? dzSAdd(dzS(2.0 * u), ez)
                    : dzSSub(dzS(-2.0 * u), ez));
            P.z = cocube - abs(u);
            e = dzVWithComponent(e, 2, dzSNeg(dAbs));
            e = dzVMul(e, sc);
            dr = dzSMul(dr, abs(sc));
            P = dzRec(i, 3).xyz;
        } else if (f == 10) {
            vec4 qr = vec4(P, 0.0);
            vec4 qe = vec4(e.m, 0.0);
            vec2 t;
            t = rxw * vec2(qr.x, qr.w); qr.x = t.x; qr.w = t.y;
            t = ryw * vec2(qr.y, qr.w); qr.y = t.x; qr.w = t.y;
            t = rzw * vec2(qr.z, qr.w); qr.z = t.x; qr.w = t.y;
            t = rxw * vec2(qe.x, qe.w); qe.x = t.x; qe.w = t.y;
            t = ryw * vec2(qe.y, qe.w); qe.y = t.x; qe.w = t.y;
            t = rzw * vec2(qe.z, qe.w); qe.z = t.x; qe.w = t.y;
            P = qr.xyz;
            e.m = qe.xyz;
            e = dzVNormalize(e);
        }

        // A perturbation branch met a boundary its identities do not cross,
        // before running its fold. Rebase and run this same fold directly; a
        // sample too deep to rebase keeps the old fail-visible break, because
        // at real depth this boundary is a genuine certification gap.
        if (needRebase) {
            if (!dzVRepresentableAt(P, e)) { g_dbgExit = 4.0; break; }
            P += dzVToOrdinary(e);
            e = dzVZero();
            seedSample = seedRef + dzVToOrdinary(seedInc);
            direct = true;
            dzDirectFold(f, seedSample, seedDeriv, ra, rb, rc, rxw, ryw, rzw,
                         P, dr);
            done = i + 1;
            if (dot(P, P) > escape2) { g_dbgExit = 7.0; break; }
            continue;
        }

        done = i + 1;

        // Re-anchor the reference from the transported orbit.
        //
        // The offset `e` keeps accumulating as it should; only the reference is
        // replaced, with the value the host computed in double-double from a
        // Newton-refined anchor. That buys two things: the orbit does not escape
        // early, because the anchor is pre-periodic by construction, and the
        // reference's own rounding stops compounding, because each iteration
        // starts from an accurate value rather than from the previous
        // iteration's f32 result.
        // Slots other than the Mandelbox still recompute their intermediates,
        // so they are re-anchored here as before. That is known to be only
        // approximate and is why they are not yet trusted; the Mandelbox path
        // above reads its whole record and needs no re-anchoring.
        // Menger still needs a fourth record group for its post-scale
        // shift, and the branch-free slots need no reference beyond the
        // endpoint, so both are re-anchored here.
        if (f != 1 && f != 2 && f != 3 && f != 4 && f != 6 && f != 9) {
            P = dzRec(i, 3).xyz;
        }

        // The global bailout is a discrete decision like any other, so it is
        // taken against the reference's stored distance to the bailout surface
        // rather than by reconstructing |P + e|^2.
        DzScalar dqb = dzSquaredNormIncrement(P, e);
        float bailoutMargin = dzPostSlotBailoutMargin(i);
        if (dzSGreaterOrd(dqb, bailoutMargin)) { g_dbgExit = 1.0; break; }

        // Absorb the offset once it outgrows the reference.
        //
        // The branch identities are exact for an offset of any size, but their
        // *conditioning* is not: the products and the radial fold need
        // |e| <= c|P| with c < 1, which is the one hypothesis in the operation
        // table that nothing enforces by itself. Past that point the offset is
        // no longer a small correction and carrying it separately is worse than
        // useless, because reconstructing P + e then subtracts two order-one
        // quantities that nearly cancel.
        //
        // Folding the offset into the reference is the honest fallback with a
        // single reference available: it costs the perturbative advantage in a
        // regime where there was none left, and from there the recurrence is
        // the direct one. This is not the handoff that Section 10 of the notes
        // rejects, which abandoned a *small* offset at depth and threw away its
        // exponent; this abandons a large one, where the exponent is the same as
        // the reference's anyway.
        //
        // The threshold is measured rather than guessed, and it is much tighter
        // than the 0.25 the binary64 prototypes use. Comparing this path
        // against the direct one over a whole frame, where offsets are order
        // one rather than the tiny values a real dive produces, the two agree
        // pixel for pixel at 0.01 and below and diverge above it: 100% of
        // pixels identical at 0.0001 and at 0.01, 93.8% at 0.25, 46.2% at 1.0.
        // Since agreement is exact at a threshold that perturbs only the first
        // fold, every per-step identity here is correct and what the threshold
        // buys is conditioning alone.
        //
        // None of this bites in the regime the path exists for. At a real zoom
        // depth the offset is many orders below the reference and the test never
        // fires, which is what the prototypes report at a minimal iteration
        // count. It matters for the whole-frame comparison above, and for the
        // Pseudo-Kleinian slot, whose fold is an inversion with an unbounded
        // multiplier: its derivative grows like 4^n against the min radius, so
        // by fourteen folds it amplifies any rounding by about 1e8 and neither
        // path is accurate in f32.
        if (dz_absorb > 0.0) {
            float pm = max(max(max(abs(P.x), abs(P.y)), abs(P.z)), 1.0);
            float emLog2 = dzSLog2Abs(dzSNormalize(DzScalar(
                max(max(abs(e.m.x), abs(e.m.y)), abs(e.m.z)), e.e
            )));
            if (emLog2 > log2(max(dz_absorb * pm, 1e-30))
                && dzVRepresentableAt(P, e)) {
                // The fold for this iteration is complete; the remaining
                // folds run directly from the rebased point. Breaking here
                // instead — which this path used to do — truncated the orbit
                // at the first absorb, and at shallow zoom every sample's
                // offset is order one, so the whole frame rendered a one-fold
                // solid.
                P += dzVToOrdinary(e);
                e = dzVZero();
                seedSample = seedRef + dzVToOrdinary(seedInc);
                direct = true;
            }
        }
    }

    if (g_dbgExit == 0.0 && done >= total) g_dbgExit = 6.0;

    DzScalar finalLength = dzLengthFromReference(P, e);
    // Relative to the magnification the frame carries. Absolute octaves saturate
    // as soon as the descent starts, and every consumer of the channel goes flat
    // with them.
    float logDr = max(dzSLog2Abs(dr), 0.0);
    info.depth = clamp(logDr / depthSpan(done), 0.0, 1.0);
    float finalRadius = dzSToOrdinary(finalLength);
    info.radius = clamp(log2(1.0 + finalRadius * finalRadius) / 10.0, 0.0, 1.0);
    info.folds = float(done);
    g_dbgLogDr = dzSLog2Abs(dr) * 0.30103;
    g_dbgFolds = float(done);
    g_dbgMinFeature = dzDistanceToEstimatorUnits(dzSDivS(dzS(1.0), dr));

    // Escape-time estimates only mean something for samples that escaped.
    //
    // `|z| / dr` is the derivative bound on the *potential*, and it is a
    // distance only once the orbit has left the bailout surface. For a sample
    // that stayed bounded, `|z|` stays order one while `dr` keeps compounding,
    // so the ratio collapses toward zero in proportion to the fold count and
    // says nothing about geometry. Feeding that to the sphere tracer produced
    // both failures seen at depth: at a short orbit the ratio sat just under the
    // hit threshold everywhere, so every ray hit at the camera and the frame was
    // one smooth lobe; at a long orbit it sat far below the step floor, so rays
    // crawled, exhausted the step budget and the frame became isolated speckle.
    //
    // A sample still bounded after the whole transported orbit is inside the
    // finite solid this frame renders, and inside is where the march stops. That
    // is Horsthuis's rule in the estimator rather than in the camera: the
    // surface is always the nearest thing along the ray, so the eye travels
    // around structure instead of through it.
    float finalRadiusSquared = finalRadius * finalRadius;
    bool escaped = finalRadiusSquared > bail2 || (hasBulb && finalRadius > 2.0);

    // Fractional escape count. An integer fold count renders the iteration
    // fog as hard-edged onion shells — the concentric-ring blanket over a
    // deep frame — because the wisp band compares whole numbers against a
    // fractional shell position. Standard escape-time smoothing removes the
    // quantization: how far past the escape surface the final radius landed
    // says where between two folds the orbit actually left. Power slots grow
    // log r geometrically per fold; the affine fold stacks grow r itself
    // geometrically, so the two cases normalize differently.
    if (escaped) {
        float escapeRadius = (hasBulb && finalRadius > 2.0) ? 2.0 : sqrt(bail2);
        float overshoot = hasBulb
            ? log2(max(log(finalRadius) / log(max(escapeRadius, 1.1)), 1.0))
            : log2(max(finalRadius / max(escapeRadius, 1.1), 1.0));
        info.folds =
            float(done) - clamp(overshoot / max(foldOctaves(), 0.5), 0.0, 1.0);
    }
    // A sample that is still bounded when the loop ends is inside the surface
    // this pixel can resolve, whichever exit ended the loop: the truncation
    // gate (the footprint is resolved and the orbit has not left), the fold
    // budget, or exhausted records at depth. Returning zero here is what
    // makes the truncated solid a surface the march can stop on; it is not a
    // wall, because the gate fold rises as the camera approaches and the same
    // region re-opens into finer structure. Previously the truncation exit
    // fell through to the escaped-sample formula, so the interior of the
    // truncated solid returned a small positive distance and marched rays
    // tunnelled through the entire solid.
    // A sample truncated by the footprint gate has a distance, and it is the
    // one already computed.
    //
    // `dCoarse` is the estimate taken at the fold where the resolvable feature
    // first crossed the pixel footprint - the distance to the *coarse* surface,
    // which is the only surface this pixel can hold. Discarding it and
    // returning zero declares every truncated sample to be inside, so a camera
    // standing in front of the structure is reported as buried in it and every
    // ray hits at t = 0. Measured on the Mandelbox at nine decades: 100%
    // LOD-truncated, nearest approach 2^-30, distance estimate 2^-24 frame
    // radii at the camera, frame flat.
    //
    // It is still a surface the march can stop on, because the gate fold rises
    // as the camera approaches: the same region re-opens into finer structure
    // rather than standing as a wall.
    if (!escaped && lodTruncated && dCoarse >= 0.0) {
        float truncated = dCoarse;
        if (lod.minDist > 0.0) truncated = max(truncated, lod.minDist * 0.25);
        return min(max(truncated, 0.0), 1.0);
    }
    if (!escaped
        && (done >= total || float(done) >= kShell || recordsExhausted
            || lodTruncated)) {
        return 0.0;
    }

    // Distance to the iso-escape shell, from the Douady-Hubbard potential:
    // the shell at fold k is the level set phi = p^-k ln B of the potential
    // phi = p^-m ln|z_m|, and dividing their difference by |grad phi| gives
    //   d = (|z|/dr) * (ln|z| - p^(m-k) ln B).
    // Far from the shell (m << k) the correction term vanishes and this is
    // the classic |z| ln|z| / dr estimate; at the shell (m = k, |z| = B) it
    // goes to zero. Without the shell term an escaped sample's distance
    // measures the true set, which at depth is space-filling and everywhere
    // at distance zero, and rays either stopped at the camera or skipped
    // through everything. Applied for the bulb, whose growth is the power
    // law the potential assumes; fold stacks keep the plain estimate.
    float shellFactor = 1.0;
    if (hasBulb && escaped) {
        float shellEscapeRadius = finalRadius > 2.0 ? 2.0 : sqrt(bail2);
        shellFactor = max(
            log(max(finalRadius, 1.02))
                - pow(power, clamp(float(done) - kShell, -110.0, 0.0))
                    * log(max(shellEscapeRadius, 1.02)),
            1e-4
        );
    }
    // The affine fold stacks have an iso-escape shell too, and it is derived
    // rather than absent: |z_{m+1}| ~ s|z_m| makes the invariant
    // phi = ln|z_m| - m ln s, so the fold-k level set enters ADDITIVELY,
    //   d = (|z|/dr) clamp(ln|z| - ln B - (m - k) ln s, 0, ln|z|),
    // against the power form's multiplicative p^(m-k). Both were measured on
    // the Mandelbox at six decades and both came out WORSE than the plain
    // estimate (mean Laplacian 32.6 plain, 13.6 unclamped, 20.3 clamped): the
    // frames go sparse, thin slabs against sky, because the shell term reduces
    // the step near the shell and this estimator's 0.35 safety coefficient was
    // fitted without it. The derivation is sound and is kept here; it wants the
    // coefficient refitted for affine stacks before it is switched on.
    // The leading coefficient dropped from 0.62 when the shell term arrived,
    // and this is a correction rather than a taste change. The classic
    // estimate is `0.5 * r * ln r / dr`; the old `0.62 * r / dr` had no
    // logarithm and its coefficient was absorbing one empirically. Making the
    // logarithm explicit while keeping 0.62 multiplied every step by about
    // 3.5, so the march overstepped thin structure and rays passed through
    // walls. At 0.35 the step sits about thirty percent inside the classic
    // coefficient, which is the margin three dimensions want: there is no
    // Koebe distortion theorem here, so the estimate is a heuristic bound
    // rather than a proven one.
    DzScalar scaledDistance =
        dzSMul(dzSDivS(finalLength, dr), 0.35 * shellFactor);
    float d = dzDistanceToEstimatorUnits(scaledDistance);
    if (dCoarse >= 0.0) d = mix(d, dCoarse, wCoarse);
    if (lod.minDist > 0.0) d = max(d, lod.minDist * 0.25);
    return min(d, 1.0);
}

// For the probes that only want a distance. The readouts are still computed and
// still land somewhere, but that somewhere is a local this drops on return,
// which is the whole point.
float stackDist(vec3 p, DeLod lod) {
    DeInfo ignored = DeInfo(0.0, 0.0, 0.0);
    return stackDE_dz(p, lod, ignored);
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
//
// The eye is parked. A host certificate is generated for one fixed viewpoint and
// cannot follow an animated camera without being regenerated every frame, so the
// showcase holds the viewpoint still and puts the whole of the motion into the
// descent. The basis is therefore a constant: the eye looks from the parked orbit
// direction toward the parked aim, and the descent carries it inward along that
// same ray, which is the ray the host isolated its target on.
void cameraBasis(out vec3 fw, out vec3 ri, out vec3 upv) {
    fw = normalize(PARKED_LOOK * 0.5 - REF_DIST * PARKED_ORBIT_DIR);
    vec3 cx = cross(vec3(0.0, 1.0, 0.0), fw);
    ri = (length(cx) > 1e-4) ? normalize(cx) : vec3(1.0, 0.0, 0.0);
    upv = cross(fw, ri);
}

// Camera shake, on the image plane so it cannot walk the eye into geometry the
// way a positional sway can. Shared, because pass 1 has to undo exactly what pass
// 0 applied: it projects the key direction back to a pixel, and a projection that
// ignores the shake converges the shafts on a point that sits still while the
// frame moves under it.
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
        // The bands are fixed in view depth and carry no motion of their own.
        // The descent is the single degree of freedom in this shot, so hue that
        // crawled independently would be a second motion competing with it;
        // holding the bands still means a cell changes hue only as it actually
        // comes toward camera.
        vec3 band = accentRamp(zk * depth_cycles);
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
    float cellPx = RENDERSIZE.y / (STAR_CELLS * 2.0 * PARKED_FOV);
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
// Everything a pass has to establish before it may evaluate the estimator.
//
// Shared rather than repeated because both the cache builder and the scene pass
// need it and the sequence is not optional: the zoom sets every fold budget in
// the frame, the payload gate decides whether there is a reference at all, and
// the standoff decides where the eye stands. Two copies of it would be two
// places for the frame's coordinate system to drift apart.
//
// Returns false when the payload is missing or stale, which is a contract
// failure the caller has to render as one rather than cover with geometry.
bool dzSetupFrame(vec2 screenUv, out vec3 fw, out vec3 ri, out vec3 upv,
                  out vec3 ro) {
    // The zoom is established before anything else, because every estimator call
    // in this pass reads the fold budget it sets.
    g_zoomLog2 = max(log2(REF_DIST) - dzFrameLog2(), 0.0);
    // Dithered per pixel, for the reason the level-of-detail cutoff is: the floor
    // is a step function of time shared by every pixel, so the whole frame gained
    // a fold at one instant. Offsetting the hash keeps this decorrelated from the
    // cutoff's own jitter, which is drawn from the same coordinate.
    // Half a fold, not a random one, for the reason given at `lodJitter`: a fold
    // count that varies per pixel varies the surface per pixel. The budget is
    // derived from the footprint now and the crossfade is what actually ends the
    // loop, so there is no step for a dither to hide.
    g_foldDither = 0.5;
    g_foldBoost = floor(g_zoomLog2 + g_foldDither);

    // The parked basis, aimed down the ray the host isolated its target on.
    cameraBasis(fw, ri, upv);
    // The payload is the only reference this renderer has, so it is validated
    // before anything is marched.
    //
    // The anchor and the orbit taken from it are one atomic unit: the orbit is
    // the iterated image of that exact anchor, refined in double-double, so
    // pairing it with any other reference point silently invalidates every
    // identity in the recurrence. There is deliberately no second source to fall
    // back on. A stale or absent payload cannot be covered by rendering
    // something plausible, because a shallow frame from an unvalidated reference
    // is indistinguishable from a working descent until the fold count is read,
    // so the failure is made explicit in the image instead.
    dzResolvePayloadGate();
    if (!g_dzPayloadMatches) return false;
    // The certified payload owns both the finite-S_N target and its orbit.
    g_dzRef = dzAnchor();
    dzPreparePrimaryCertificate(screenUv);

    // The host selected the first long-lived transition on this exact parked
    // ray. A fixed standoff in frame-local units therefore remains outside the
    // target without an unsupported camera search in binary32.
    // Measured, not assumed.
    //
    // A fixed standoff puts the eye a quarter of a frame radius from an anchor
    // that sits on the boundary of the solid this frame draws, and whether that
    // lands inside or outside the solid is not something the renderer controls:
    // it changes with depth, with the stack and with where the shell ends up.
    // Inside, every ray hits at the camera and the frame is a flat lit field,
    // which is most of what the depth ladder's "knife edge" has been. The host
    // walks outward until the camera point escapes before the shell fold and
    // transports the answer per depth slice.
    float standoffFraction = max(g_zoomLog2, 0.0)
        / (DZ_LOG2_10 * max(dzOrbitBudgetExp(), 1e-6));
    float measuredStandoff = dzMeasuredCameraStandoff(standoffFraction);
    float cameraScale = measuredStandoff > 0.0
        ? measuredStandoff
        : DZ_CAMERA_STANDOFF;
    g_camScale = cameraScale;
    ro = -fw * cameraScale;
    return true;
}


// A cached frustum field was built here and removed. See
// spec/fractal-cached-field-march.md for the numbers; the short version is that
// a dense grid cannot hold this surface and this shot has no empty space to
// skip, so neither use of a cache paid. The lever that remains is the fold
// count itself - spec/fractal-segment-bla.md.

vec4 renderScene() {
    vec2 screen = vec2(uv.x, 1.0 - uv.y) * 2.0 - 1.0;
    screen.x *= RENDERSIZE.x / max(RENDERSIZE.y, 1.0);

    vec3 fw, ri, upv, ro;
    if (!dzSetupFrame(vec2(uv.x, 1.0 - uv.y), fw, ri, upv, ro)) {
        // A missing or stale orbit is a contract failure, never geometry.
        vec2 tile = floor(vec2(uv.x, 1.0 - uv.y) * 24.0);
        float stripe = mod(tile.x + tile.y, 2.0);
        vec3 alarm = mix(vec3(0.08, 0.0, 0.10), vec3(1.0, 0.0, 0.75), stripe);
        if (g_dbgOrbitGate >= 1.5) alarm = vec3(1.0, 0.45, 0.0);
        else if (g_dbgOrbitGate >= 0.5) alarm = vec3(1.0, 0.0, 0.0);
        return vec4(alarm, 1.0);
    }

    vec3 rd = normalize(fw + (screen.x * ri + screen.y * upv) * PARKED_FOV);


    vec3 keyDir = keyDirection();

    float pixel = 2.0 * PARKED_FOV / max(RENDERSIZE.y, 1.0);
    // 180 rather than 120, which is nearly free and not a coincidence: the march
    // is adaptive, so the extra budget is only ever spent by the rays that were
    // about to run out, and those were exactly the rays producing artifacts. Close
    // in among dense structure, 120 left thirteen percent of the frame
    // approximated at its closest approach; 180 leaves under half a percent, for
    // two to five percent more time.
    // The step budget has to grow with the dive, for a reason that is arithmetic
    // rather than a matter of taste.
    //
    // The march converges to one pixel footprint, and a footprint is `pixel * t`,
    // so the number of steps a ray needs in open space is about `t / (pixel * t)`,
    // which is `1 / pixel` and independent of depth. In a crevice it is far worse,
    // because the estimate is the distance to the *nearest* surface rather than
    // the distance along the ray, and a deep dive is all crevice: the geometry
    // that fills the frame is the geometry the camera has descended into. Rays
    // that run out are shaded at their closest approach, which is the right
    // fallback and reads as speckle once a large share of the frame takes it.
    // That speckle is the main thing separating these frames from a clean render,
    // and a fixed budget is what causes it.
    //
    // Unity at the authored radius, so the fader keeps meaning what it means and
    // a parked camera costs exactly what it did.
    float stepScale = clamp(1.0 + 0.12 * g_zoomLog2, 1.0, 3.0);
    int steps = int(clamp(ray_steps * stepScale, 40.0, 660.0));
    // Hard ceiling on the product of march steps and fold budget. Deep in a
    // dive both grow — steps threefold, folds toward the 128 ceiling — and
    // their product is what a pixel actually costs. Unbounded, a deep frame's
    // command buffer ran multiple seconds and tripped the OS GPU watchdog,
    // which takes the whole desktop down, not just the frame. The ceiling
    // only binds past roughly forty folds, so shallow frames are untouched;
    // a deep frame trades march steps (softness in crevices) for staying
    // interactive, which is the right trade for a live tool.
    float budgetFolds = clamp(stack_cap + max(g_foldBoost, 0.0), 1.0,
                              float(MAX_STACK_ITERS));
    // The cap has to divide by the folds a pixel really runs, which is where
    // the shell sits, not how many records the payload carries. Dividing by
    // the payload length (134 records at depth against a shell at about
    // twenty folds) throttled deep frames to a sixth of the march steps they
    // had earned, and starving the march is what leaves a deep frame speckled
    // with rays shaded at their closest approach instead of converged.
    // ...and it has to be the *same* shell the estimator will place, which is
    // now measured by the host rather than derived. Dividing by the derived
    // estimate while the estimator folded to the measured shell under-counted
    // the real per-pixel work by an order of magnitude, and a frame that
    // exceeds this budget does not merely run slow: it exceeds the OS GPU
    // watchdog and takes the whole desktop down. The guard is only a guard if
    // it is computed from the same number the loop obeys.
    float ratedExp = max(dzOrbitBudgetExp(), 1e-6);
    float currentExp = max(g_zoomLog2, 0.0) / DZ_LOG2_10;
    float measuredShell = dzMeasuredShellFold(currentExp / ratedExp);
    budgetFolds = clamp(
        measuredShell > 0.0
            ? measuredShell
            : stack_cap + max(g_zoomLog2, 0.0) / max(foldOctaves(), 0.5),
        1.0, float(MAX_STACK_ITERS)
    );
    steps = min(steps, int(max(24000.0 / budgetFolds, 60.0)));

    float t = 0.0;
    // Skip the empty space outside the structure when the camera is out there.
    if (dot(ro, ro) > BOUND_R * BOUND_R) {
        float bs = boundingSphere(ro, rd, BOUND_R);
        if (bs < 0.0) return vec4(skyColor(rd), 1.0);
        t = max(bs - 0.1, 0.0);
    }
    // The host proved every point in this screen tile outside the same finite
    // solid from the camera through this inward-rounded endpoint. This is an
    // assignment, not a scalar-DE estimate: the shader cannot extend it.
    float certifiedT = dzCertifiedPrimaryAdvance(t);
    g_dbgCertJump = (certifiedT - t) / max(dzGroup(11).z, 1e-6);
    t = certifiedT;

    // No jitter, and this is a retraction rather than a tuning change.
    //
    // The reasoning it replaces: the iteration cutoff was a hard break, so a
    // threshold shared by every pixel changed the fold count along an exact
    // iso-distance curve, giving concentric rings of detail that swept outward
    // during a dolly. Jittering the threshold per pixel traded those rings for
    // fine noise on the grounds that the softening pass and the tonemap would
    // bury it.
    //
    // That trade was already obsolete when it was written, because the cutoff is
    // no longer a hard break: the crossfade in the estimator keeps the truncated
    // distance, runs one more fold and blends the two, so the *surface* moves
    // continuously between levels of detail even though the fold count is an
    // integer. Rings were the symptom of the discontinuity, and the crossfade
    // removes the discontinuity. The jitter is left solving a problem that no
    // longer exists, and it is not free: a random footprint per pixel means
    // adjacent pixels resolve genuinely different surfaces, so it does not add
    // noise to the shading, it adds noise to the geometry. Nothing downstream can
    // bury that, because a blur across two different surfaces is not a smoothed
    // surface.
    //
    // Measured on the default preset, removing this and the fold-budget dither
    // takes high-frequency content from 0.49 to 0.42 with the mean essentially
    // unchanged, and the difference is entirely speckle on surfaces that should
    // read smooth. It is the single largest clarity defect in the pipeline and it
    // applies at every scale, not only at depth.
    float lodJitter = 1.0;

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
    for (int i = 0; i < 660; i++) {
        if (i >= steps) { exhausted = true; break; }
        g_dbgMarchSteps = float(i + 1);
        certifiedT = dzCertifiedPrimaryAdvance(t);
        g_dbgCertJump += (certifiedT - t) / max(dzGroup(11).z, 1e-6);
        t = certifiedT;
        // Both the floor here and the threshold below are divided down by the
        // zoom, so `Detail` keeps meaning what it means at the orbit's outer
        // radius and tightens in step with the approach. Held absolute, they
        // become the resolution limit of the dive: past a few times closer the
        // march stops converging on anything the fold stack has newly revealed.
        float footprint = pixel * max(t, footFloor()) * defocusRelief(t);
        float d = stackDE_dz(ro + rd * t,
                             DeLod(footprint * geoBand(), footprint, 0.0), march);
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

        // Over-relaxed sphere tracing was tried here and measured slower
        // (92.1 ms against 88.7 ms per frame at 160 by 90, seven decades
        // deep). The overlap test that makes relaxation safe fails often on
        // this estimator: it is a heuristic bound that locally under-reports,
        // so successive unbounding spheres barely overlap and the rewinds
        // cost more than the longer steps save. A plain sphere trace it is.
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
    float rescueFloor = 0.002;
    if (!hit && (exhausted || nearest < max(pixel * nearT * 6.0, rescueFloor))) {
        hit = true;
        t = nearT;
        // Moving `t` is not enough. The readouts still hold the *last* call the
        // march made, which for a rescued ray is wherever it gave up —
        // arbitrarily far past the closest approach, and possibly out at the
        // march limit. Re-evaluating at the point actually being shaded is what
        // makes them describe it. The rescue fires preferentially in crevices, so
        // skipping this shades the frame's most detailed regions from an
        // unrelated point.
        float footprint = pixel * max(t, footFloor()) * defocusRelief(t);
        stackDE_dz(ro + rd * t, DeLod(footprint * geoBand(), footprint, 0.0), march);
        // `det` sizes the normal's difference step and the shadow ray's offset,
        // and it too was left at the distance the ray gave up rather than the
        // one being shaded, which is always the larger of the two.
        det = footprint * max(detail / DETAIL_REF, 0.35);
    }

    // False-colour diagnostics, taken from the estimator call above so they
    // describe the point actually shaded.
    //
    // Each channel is a quantity that could be pinning the depth, rendered
    // directly rather than deduced from the finished image. Read against
    // `viewScale()`: if the fold count and the derivative both keep climbing as
    // the zoom deepens while the render stops changing, the estimator is fine
    // and the geometry is being addressed wrongly; if either flattens, that is
    // the ceiling.
    // Mode twelve is the one diagnostic that is not a false-colour readout: it
    // runs the whole scene pass, shading included, and only the post chain is
    // skipped (every `dz_debug` above zero bypasses it). Subtracting it from
    // the full frame isolates what the two-dimensional post pass costs, and
    // subtracting the march from it isolates what three-dimensional shading
    // costs, which is the split that decides where the beauty budget goes.
    if (dbgMode() > 0 && dbgMode() != 12) {
        // Geometry-only modes: the march and nothing downstream of it. Every
        // beauty layer — fog, headlight, palette, the whole post pass — is a
        // multiplier on whatever the geometry pass produces, so geometry and
        // performance questions are answered here, unconfounded. Mode 7 is
        // flat-lit shape, mode 8 is raw normals (banding, stair-stepping and
        // blob-ness read directly off a normal map).
        if (dbgMode() == 7 || dbgMode() == 8) {
            if (!hit) return vec4(0.0, 0.0, 0.0, 1.0);
            vec3 pos = ro + rd * t;
            float eps = max(det, 1e-7);
            // The march's own footprint, not `det`.
            //
            // `det` is the convergence threshold, a fraction of the footprint,
            // and handing it to the estimator as an iteration cutoff asks these
            // four probes to resolve finer structure than the surface was found
            // at. That is expensive rather than wrong, and it was expensive
            // enough to invert the measurement this mode exists to make:
            // "geometry only" timed at 261 ms against 74 ms for the same frame
            // with the entire beauty chain switched on, because the beauty
            // path's normal correctly reuses the marched footprint and this one
            // did not.
            float nFootprint = pixel * max(t, footFloor()) * defocusRelief(t);
            DeLod nl = DeLod(nFootprint * geoBand(), nFootprint, 0.0);
            vec2 k2 = vec2(1.0, -1.0);
            vec3 n = normalize(
                k2.xyy * stackDist(pos + k2.xyy * eps, nl) +
                k2.yyx * stackDist(pos + k2.yyx * eps, nl) +
                k2.yxy * stackDist(pos + k2.yxy * eps, nl) +
                k2.xxx * stackDist(pos + k2.xxx * eps, nl));
            if (dbgMode() == 8) return vec4(n * 0.5 + 0.5, 1.0);
            float lambert = clamp(dot(n, normalize(vec3(0.5, 0.8, -0.3))), 0.0, 1.0);
            float back = clamp(dot(n, normalize(vec3(-0.4, -0.2, 0.6))), 0.0, 1.0);
            // Deliberately under-driven. A key that reaches one clips across most
            // of a convex surface, and the shape this mode exists to show goes
            // into flat white exactly where the geometry is facing the viewer.
            // Same four estimator calls either way, so timings are unaffected.
            return vec4(vec3(0.02 + 0.50 * lambert + 0.08 * back), 1.0);
        }
        float rho = 1.0;
        float v;
        if (dbgMode() == 4) {
            // Payload acceptance, which has to read on missed rays too: a
            // rejected payload is most visible where nothing was hit.
            if (g_dbgOrbitGate >= 2.5) return vec4(0.0, 0.8, 0.2, 1.0);
            if (g_dbgOrbitGate >= 1.5) return vec4(1.0, 0.5, 0.0, 1.0);
            if (g_dbgOrbitGate >= 0.5) return vec4(0.9, 0.0, 0.1, 1.0);
            return vec4(0.15, 0.15, 0.15, 1.0);
        }
        if (dbgMode() == 5) {
            float maximum = max(dzGroup(11).z, 1e-6);
            float prefix = dzCertifiedPrimaryAdvance(0.0);
            float coverage = clamp(prefix / maximum, 0.0, 1.0);
            return vec4(1.0 - coverage, coverage, 0.05, 1.0);
        }
        if (dbgMode() == 11) {
            // Exit cause of the real march's last estimator call — the state
            // the pixel was actually shaded from, LOD and all. Hues: grey
            // none, green escaped, teal bulb pre-guard, blue LOD crossfade,
            // red rebase-failed, orange records-exhausted, white
            // bounded-at-budget, yellow direct-escape, magenta record-missing.
            int cause = int(g_dbgExit + 0.5);
            if (cause == 1) return vec4(0.0, 0.8, 0.1, 1.0);
            if (cause == 2) return vec4(0.0, 0.7, 0.7, 1.0);
            if (cause == 3) return vec4(0.1, 0.2, 0.9, 1.0);
            if (cause == 4) return vec4(0.9, 0.05, 0.05, 1.0);
            if (cause == 5) return vec4(0.9, 0.5, 0.0, 1.0);
            if (cause == 6) return vec4(1.0, 1.0, 1.0, 1.0);
            if (cause == 7) return vec4(0.9, 0.9, 0.0, 1.0);
            if (cause == 8) return vec4(0.9, 0.0, 0.9, 1.0);
            return vec4(0.25, 0.25, 0.25, 1.0);
        }
        if (dbgMode() == 10) {
            // Raw DE field probe at three ray depths, with pathology coding:
            // magenta = NaN, red = negative, otherwise G = log-heat of d at
            // t=0.05, B = log-heat at t=0.2 (2^-30..2^30 -> 0..1).
            DeLod probe = DeLod(0.0, 0.0, 0.0);
            float dA = stackDist(ro + rd * 0.05, probe);
            float dB = stackDist(ro + rd * 0.2, probe);
            if (dA != dA || dB != dB) return vec4(1.0, 0.0, 1.0, 1.0);
            if (dA < 0.0 || dB < 0.0) return vec4(1.0, 0.0, 0.0, 1.0);
            return vec4(
                0.0,
                clamp((log2(max(dA, 1e-30)) + 30.0) / 60.0, 0.0, 1.0),
                clamp((log2(max(dB, 1e-30)) + 30.0) / 60.0, 0.0, 1.0),
                1.0
            );
        }
        if (dbgMode() == 9) {

            // March outcome triage, numerically: R = hit, G = the closest
            // approach on a log scale (2^-30 .. 2^30 mapped to 0..1, so ~0.5
            // is order one), B = ran out of steps. A frame that is all
            // R=1,G=0 hit interior everywhere; all R=0 with high G never got
            // near anything.
            float nd = clamp((log2(max(nearest, 1e-30)) + 30.0) / 60.0, 0.0, 1.0);
            return vec4(hit ? 1.0 : 0.0, nd, exhausted ? 1.0 : 0.0, 1.0);
        }
        if (dbgMode() == 6) {
            return vec4(
                clamp(g_dbgMarchSteps / max(float(steps), 1.0), 0.0, 1.0),
                exhausted ? 1.0 : 0.0,
                clamp(g_dbgCertJump, 0.0, 1.0),
                1.0
            );
        }
        if (dbgMode() == 1) {
            // Fold bodies actually executed by the last estimator call, against
            // the ceiling. Not `done`: that counts the prefix the linear jump
            // skipped, which is exactly the work this is meant to exclude.
            v = g_dbgExecFolds / float(MAX_STACK_ITERS);
        } else if (dbgMode() == 2) {
            // log10(dr), mapped over sixty decades.
            v = clamp(g_dbgLogDr / 60.0, 0.0, 1.0);
        } else if (dbgMode() == 3) {
            // Folds the estimator *reached*, including the prefix the linear
            // jump skipped. Read against mode one: the gap between them is what
            // the skip is worth on this stack.
            v = g_dbgFolds / float(MAX_STACK_ITERS);
        } else {
            // The resolution ratio: finest feature the estimate can distinguish
            // divided by the window radius. At most one means the cell resolves.
            v = clamp(log2(g_dbgMinFeature / max(rho, 1e-38)) * 0.30103 / 12.0
                      + 0.5, 0.0, 1.0);
        }
        if (!hit) return vec4(0.0, 0.0, 0.0, 1.0);
        // Blue below, green at unity, red above, so a saturating quantity reads
        // as a flat colour and a healthy one as a moving gradient.
        vec3 ramp = (v < 0.5)
            ? mix(vec3(0.0, 0.1, 0.8), vec3(0.0, 0.9, 0.2), v * 2.0)
            : mix(vec3(0.0, 0.9, 0.2), vec3(1.0, 0.15, 0.0), (v - 0.5) * 2.0);
        return vec4(ramp, 1.0);
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
        // The same defocus relief the march used. Differencing against a finer
        // cutoff than the surface was found at samples geometry the march never
        // resolved, which returns as normal noise rather than as detail.
        float footprint = pixel * max(t, footFloor()) * defocusRelief(t);
        // The floor on the difference step is relative, not absolute.
        //
        // It exists to stop `p + e` rounding back to `p`, which is a statement
        // about an f32 ulp *at this point*, so `1e-5` was only ever right for
        // order-one coordinates. At the bottom of a dive `det` is around 1e-10 and
        // an absolute floor differences the surface over a baseline a hundred
        // thousand times wider than the features being shaded, which returns a
        // normal belonging to nothing on screen. An ulp at `pos` is the quantity
        // meant, and it leaves `det` in charge at every ordinary radius.
        float nEps = max(det, max(length(pos), footprint) * 3e-7);
        vec3 n = calcNormal(pos, nEps, DeLod(footprint * geoBand(), 0.0, 0.0));

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
        // Curvature occlusion used to be marched here: a second, wider normal,
        // four more estimator calls on the surface where every call is at its
        // most expensive. It is now read from the depth channel in the post
        // pass (`screenOcclusion`), which is where the reference workflow puts
        // it and where it costs texture reads instead of fold evaluations.

        if (dot(n, rd) > 0.0) n = -n; // concavities are the normal case here

        float buried = 1.0 - 0.6 * depth_k;
        // What remains in the scene pass is the depth-buried term, which is a
        // property of the point and costs nothing; the crevice term arrives in
        // the composite.
        float occ = mix(1.0, buried, ao_strength);

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
        // A lamp on the camera, and it is not only a fill.
        //
        // A distant key cannot reach into a structure the camera is inside, which
        // is the situation for the whole of a deep dive: the eye is in a recess of
        // a recess, everything in frame faces some other way, and the render goes
        // dark however much precision is behind the geometry. A lamp at the eye is
        // the standard answer in the reference workflow and it reads as exploring a
        // cavern, which is the correct thing for it to read as.
        //
        // The reason it belongs here rather than in a grade is that its falloff is
        // a depth cue, and the specific one this frame is missing. Near surfaces
        // come up bright and far ones drop away, so the lamp restores the near
        // against far separation that the distance haze and the depth channel
        // provide at authored radii and cannot provide at depth. Artistic taste and
        // the engineering want the same term.
        //
        // Its reach is measured in view radii, not world units, which is what makes
        // it hold at every scale: at the bottom of a dive the whole frame is a
        // fraction of a world unit across, and a lamp with a fixed world falloff
        // would either light all of it flat or none of it. Inverse-square in the
        // frame's own units gives the same staging at 1e-2 and at 1e-10.
        float headL = 0.0;
        if (head_light > 0.0) {
            float lamp = max(dot(n, -rd), 0.0);
            float reach = max(camRadius() * head_reach, 1e-20);
            float tt = t / reach;
            headL = head_light * lamp / (1.0 + tt * tt);
        }

        col  = albedo * key * color_warm.rgb * 1.6;
        col += albedo * headL * mix(color_warm.rgb, vec3(1.0), 0.35) * 1.3 * occ;
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
        seamArg = depth_k * 1.5 + rad_k * 0.4;
        // No occlusion and no shadow term. In the reference workflow the colour
        // layer is a second render of the same camera with ambient occlusion and
        // shadows switched off outright, and that is what makes it read as its
        // own light rather than as paint lying on the surface. This previously
        // scaled by the rim, which is view-dependent, so the accent breathed as
        // the camera turned past a surface it was supposed to be lighting.
        seamMask = smoothstep(0.5, 0.95, depth_k) * emissive * 0.6;

        // A rate per world unit, so at the bottom of a dive the whole frame sits
        // inside the first fraction of one unit and takes no haze at all. That is
        // the depth cue the composite stages against, and losing it is most of
        // why deep frames read as one flat plane. Measured against the frame's
        // own depth range instead, which is unity at rest.
        haze = clamp(1.0 - exp(-t * depthSquash() * 0.10 * fog_amount), 0.0, 1.0);
    } else {
        col = skyColor(rd);
        // Rays that nearly grazed the structure pick up its edge glow, which
        // keeps the silhouette from being a hard cut against the background.
        float edgeDistance = nearest;
        col += accent_color.rgb * exp(-edgeDistance * 60.0) * 0.3 * emissive;
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
        seamArg = nearDepth * 1.5 + nearRad * 0.4;
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
        // A sample density per unit length, not a fixed sample count.
        //
        // Sixteen taps were spent whether the ray crossed the whole reach or
        // stopped a hundredth of a unit away, and at depth most rays stop
        // almost immediately, so nearly all of that density was spent
        // resolving a segment shorter than one fog feature. Criterion puts
        // this block at 829 ms of a 1450 ms frame, fourteen times the entire
        // fractal march, which makes it the most expensive thing in the
        // renderer by a wide margin.
        //
        // The look is unchanged by construction: `wisp * dt` is a Riemann sum
        // of the same integral over the same segment, and holding taps per
        // unit length fixed holds the accuracy of that sum fixed. Only rays
        // that travel less than the full reach take fewer samples, and they
        // are exactly the rays whose samples were redundant.
        int taps = int(clamp(
            ceil(float(WISP_TAPS) * far / max(fog_iter_reach, 1e-6)),
            3.0, float(WISP_TAPS)
        ));
        float dt = far / float(taps);
        // Offset by a pixel-stable fraction of a step, or every tap lands on the
        // same shells and the fog quantises into slabs. Re-hashed per tap:
        // with one shared offset the sixteen-tap comb stays coherent, and on a
        // deep frame — where the fold shells are smooth and near-parallel —
        // the comb beats against the shell spacing into concentric moiré
        // rings centred on the look direction.
        float wisp = 0.0;
        // The band is zero for any point that survived more folds than its top
        // edge, so counting past that edge cannot change the result. Capping the
        // budget there is lossless, and it turns the frame's most expensive block
        // from a full forty-iteration stack on every tap into about ten.
        // The shell is at a fold count, and a fold count is not a fixed place
        // once the camera dives: everything near a deep camera survives far more
        // folds than the authored six, so the band matches nothing and the fog
        // disappears exactly where the frame most needs something between the
        // camera and the geometry. Measured from the magnification the frame
        // already carries, the shell stays the same *relative* surface at any
        // depth, and is the authored one at rest.
        // Converted into folds before being added to one. `g_zoomLog2` counts
        // octaves of magnification, and a fold contributes several octaves, so
        // adding it to a fold count directly overshot by that factor: at 1e-6
        // the shell asked for fold 26 against a budget of 26, which put the band
        // on every surviving pixel and buried the frame in fog.
        float fogAt = fog_iter + g_zoomLog2 / foldOctaves();
        DeLod fogLod = DeLod(0.0, 0.0, ceil(fogAt + fog_iter_band) + 1.0);
        DeInfo tap = DeInfo(0.0, 0.0, 0.0);
        for (int i = 0; i < WISP_TAPS; i++) {
            if (i >= taps) break;
            float jitter = hash12(uv * RENDERSIZE + 17.0 + float(i) * 7.31);
            vec3 at = ro + rd * ((float(i) + jitter) * dt);
            // The fog reads one number per tap, the fold count, and that is
            // exactly what the transported derivative curve gives without
            // iterating: measured, this block was doubling the frame.
            float folds = dzLinearFoldCount(at, int(fogLod.cap));
            if (folds < 0.0) {
                stackDE_dz(at, fogLod, tap);
                folds = tap.folds;
            }
            wisp += max(1.0 - abs(folds - fogAt) / max(fog_iter_band, 0.35), 0.0);
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
        // `wisp * dt` is an occupancy integrated over a world length, so it too
        // vanishes when the frame's whole depth range is a fraction of a unit.
        // Same correction as the distance haze, and unity at rest.
        float density = 1.0 - exp(-wisp * dt * depthSquash() * fog_iter_amount * 1.5);
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

// Occlusion read from the depth channel instead of marched in three dimensions.
//
// The reference workflow renders its beauty and its occlusion as separate
// buffers and multiplies them in the composite; this is that, in the pass that
// already exists. It replaces four distance-estimator evaluations per shaded
// pixel, each of which lands on the surface where the orbit survives the whole
// fold budget, with sixteen texture reads. Measured against a two percent post
// chain, the reads are free.
//
// It works at any zoom because the depth channel is logarithmic. A fixed
// difference in encoded depth is a fixed *ratio* of view distances, so one
// gain constant describes the same crevice geometry at every scale and nothing
// here has to be rescaled as the dive deepens.
//
// A neighbour nearer than the centre is a wall in front of this point, which is
// what being in a recess means. Sky neighbours read as further and contribute
// nothing, so silhouettes do not darken.
float screenOcclusion(vec2 at) {
    float centre = depthAt(at);
    if (centre >= 0.999) return 1.0;
    vec2 unit = 1.0 / max(RENDERSIZE, vec2(1.0));
    const int TAPS = 8;
    float occ = 0.0;
    for (int i = 0; i < TAPS; i++) {
        float angle = (float(i) + 0.5) * (TAU / float(TAPS));
        vec2 dir = vec2(cos(angle), sin(angle));
        // Two radii: contact darkening in the crease, and a broader term that
        // separates whole forms from one another.
        float contact = depthAt(at + dir * unit * 2.0);
        float broad = depthAt(at + dir * unit * 7.0);
        occ += clamp((centre - contact) * 140.0, 0.0, 1.0);
        occ += clamp((centre - broad) * 70.0, 0.0, 1.0) * 0.5;
    }
    occ /= float(TAPS) * 1.5;
    // Shaped rather than linear. The reference look is not a uniform grey wash
    // over everything slightly enclosed; it is bright forms against creases
    // that go almost black, so the response is flat across open surfaces and
    // then falls away hard once a point is genuinely buried. Free, as sixteen
    // texture reads and three multiplies: the frame time is the same with the
    // shaping as without it, measured back to back.
    return clamp(1.0 - occ * occ * (3.0 - 2.0 * occ), 0.0, 1.0);
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

    // The occlusion pass, multiplied in as a composite would. Applied to the
    // resolved colour rather than to each light term, which is exactly the
    // separation the reference workflow uses: shade, then multiply occlusion.
    if (ao_strength > 0.01) {
        col *= mix(1.0, screenOcclusion(uv), clamp(ao_strength, 0.0, 1.0));
    }

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
            vec2 sun = vec2(dot(kd, ri), dot(kd, upv)) / (ahead * PARKED_FOV);
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
    } else if (dbgMode() > 0) {
        // The diagnostics are numeric readouts encoded as colour. Grading,
        // bloom or aberration would silently re-map the numbers, so the post
        // pass hands them through untouched.
        fragColor = vec4(sceneAt(uv) + keep, 1.0);
    } else {
        fragColor = vec4(cinematicPost().rgb + keep, 1.0);
    }
}
