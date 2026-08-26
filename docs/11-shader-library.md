# Shader Library

Varda ships with a catalog of ISF shaders, ready to drag from the **Library** panel into any deck (generators), onto a deck/channel/master (filters), or into a transition slot. All of them live in the `shaders/` directory and can be opened, edited, and hot-reloaded. See [ISF Shader Authoring](12-isf-authoring.md).

Shaders are classified automatically by type:

- **Generators** create visuals from scratch (no image input).
- **Filters** process an input image (blur, color grade, distort, key).
- **Transitions** blend two sources via a `progress` parameter.
- **Compute** shaders (`.comp`) run GPU-native simulations and particle systems.

## Generators

| Shader | Description |
|--------|-------------|
| `abstract_field.fs` | Abstract generative field — flowing organic patterns |
| `alien_grove.fs` | Raymarched night forest flythrough of lacy umbel trees rising out of circular wells cut in the terrain, with smaller Menger crystal lattices and recursive fern-corals grown between them; RGB energy pulses run along circuit traces etched into the rock, which meander with it, ring the lip of every well, converge on webs centred under each trunk, climb the trunks and spars, and color their terminal auras beneath a cratered moon and log-periodic fractal halo |
| `apollonian_glow.fs` | Raymarched Kali-fold + Apollonian fractal tunnel lit entirely by an accumulated glow trail, with reflection pass |
| `aurora_borealis.fs` | Northern lights — fake-volumetric raymarch through folded noise sheets, green-to-violet curtains with starfield |
| `bars.fs` | Animated bars/stripes generator |
| `bicycle_day.fs` | Raymarched "Amazing Surface" fractal tunnel shaded by normals + dark edge lines, low sun, procedural rainbow trail, with fractal trees lining the roadside |
| `big_bang.fs` | Cyclical cosmic evolution with fluid-sim galaxy dust, stellar lifecycle, expansion/crunch |
| `biomine.fs` | Raymarched biotube lattice (gyroid surfaces) pumping fluid through a mine tunnel, with cellular bump mapping and fake reflective/refractive fluid glow |
| `black_hole.fs` | Particle-streak shell with emergent accretion disk, jets, orbiting crystals (black & white) |
| `char_cycle.fs` | Cycles through glyphs from a selected script |
| `checkerboard.fs` | Checkerboard pattern generator |
| `clouds.fs` | Raymarched volumetric cloud layer flythrough with sun glow and rim-lit shadowing |
| `crystal_cave.fs` | Fly through a 3D cave filled with growing crystal formations |
| `cymatics.fs` | Chladni plate and Faraday wave vibration pattern generator |
| `dark_matter.fs` | Cosmic web filament network (neuro noise) |
| `digital_brain.fs` | Glowing voronoi-noise plasma with drifting camera and pulsing "moving electrons" octaves |
| `eyes.fs` | Tiled grid of procedural cartoon eyes — autonomous blink, drifting gaze, IQ cosine-palette irises |
| `eyes_depth.fs` | The same eyes, tracking people seen by a Kinect: the gaze follows the motion-weighted centroid of whoever is in view, lids wake as someone approaches, pupils dilate on sudden movement. **Requires an attached depth sensor** — see [ISF authoring § `depth_sensor`](12-isf-authoring.md#depth_sensor--live-depth-camera) |
| `fire.fs` | Procedural animated fire effect |
| `fractal.fs` | Mandelbrot / Julia set generator |
| `fractal_explorer.fs` | Raymarched 3D fractal explorer built on a four-slot formula stack. Each slot picks a distance estimator (Mandelbox, Amazing Box, Menger, Sierpinski, Mandelbulb, Pseudo-Kleinian, lin-combine, rotate, co-cube, 4D rotate, or off) and takes a weighted share of the iteration budget; the slots interleave, so order matters, and `Slot Order` permutes them without retyping the dropdowns. The camera approaches geometrically rather than linearly, with the detail threshold, depth range and fold count all scaling with it, so a dive keeps resolving new structure instead of arriving at a blob. Two kinds of atmosphere: distance haze, and fog keyed to the fold count at each point in space (`Atmosphere` group), which hangs in the space and pools in the troughs. `Fog Iteration` places the shell that band picks out, and it is the whole look: below about four every point in open space qualifies and the fog floods the frame, six or seven hangs it in the space, and by nine it has gone. Finished in a second pass with distance softening off the marched Z, selective highlight bloom, chromatic aberration, twin ghost reflections with a matte-box flare, key-aligned light shafts and a look grade. The `Composition` group exists to break the tonal evenness a fractal has by construction: a bright side and a fallen-away side placed where the key light actually is, a dark foreground against a lifted background, and local contrast rather than global. The palette is banded in view depth rather than fixed, so near and far parts of the structure take different hues and each shifts as it comes toward camera — `Depth Colour Shift` in the `Palette` group, at zero for the fixed three-colour scheme. `Julia Seed` swaps the sample point for a fixed seed in the folding formulas, which is a whole second family of shapes and the axis to animate when you want the structure itself to move. Ships with the fractal parked and only the camera moving, since a shot reads best with one degree of freedom in motion: `Formula / Energy Speed` is the beat you bring in, and `Evolve Target` picks the single parameter it drives (fold scale by default, since every folding formula reads it). The march converges to a pixel rather than to an absolute distance, so `Detail` reads as pixels of convergence: one at the default, down to a third for a sharper and slower march, up to nearly three for a softer and faster one. It therefore means the same thing at every distance, zoom and output resolution, and the march no longer chases structure finer than the frame can hold, which is what used to leave stripes on a pulled-back camera and torn holes of background through solid geometry up close. The fold cutoff crossfades across one fold instead of switching at one, so the surface slides between levels of detail as the camera moves rather than snapping between them. The sky defaults near black with a star field rather than a lifted haze, which is what projection and dome output need — raise `Atmosphere Lift` to trade that for flat-screen depth staging. `Horizon Mirror` folds the sky back on itself below the waterline; because a fractal is usually already symmetric there, the frame reads as a mirror-flat lake. Mutate the `Stack` group to hunt looks — see [Finding a Look](04-performance.md#finding-a-look-random-and-mutate) |
| `game_of_life.fs` | Conway's Game of Life — cellular automaton with persistent state |
| `generative_feedback.fs` | Evolving patterns using a persistent feedback buffer |
| `gradient.fs` | Color gradient generator — linear, radial, or angular |
| `graph_network.fs` | Physics-driven floating nodes that connect by proximity |
| `grid.fs` | Dot/point grid generator |
| `hilbert_curve.fs` | Space-filling fractal growing outward from center |
| `lagrangian.fs` | Standard Model Lagrangian typed terminal-style with parallax layers |
| `lines.fs` | Animated geometric lines generator |
| `liquid_light.fs` | 1960s liquid light show — oil/water/dye overhead projector psychedelia |
| `liquid_light_depth.fs` | The same look driven by a live Kinect: bodies in the sensor's view push a real advected fluid and read as flowing dye outlines. **Requires an attached depth sensor** — see [ISF authoring § `depth_sensor`](12-isf-authoring.md#depth_sensor--live-depth-camera) |
| `noise.fs` | Procedural simplex-style animated noise |
| `oscilloscope.fs` | Audio-reactive waveform and shape visualizer with 2D/3D modes |
| `particle.fs` | Procedural particle field generator |
| `particle_collider.fs` | ATLAS/CERN-style collision with cascading fission tracks |
| `plasma.fs` | Simple plasma effect |
| `plasma_globe.fs` | Raymarched electrical arcs writhing inside a glowing plasma sphere, seen through a reflective outer shell |
| `quantum_membrane.fs` | Rolling wave-mesh terrain with rainbow grid flyover |
| `radar.fs` | Radar sweep generator |
| `rings.fs` | Concentric animated rings generator |
| `sacred_geometry.fs` | Flower of Life, Metatron's Cube, Sri Yantra, Fibonacci spiral, and more |
| `shaper.fs` | Geometric shape generator — circle, triangle, square, star, polygon |
| `solid_color.fs` | Solid color fill generator |
| `star_nest.fs` | Volumetric raymarched star-field/nebula tunnel via an iterated absolute-inversion fractal, with look_at camera rotation |
| `starfield.fs` | Classic parallax star tunnel |
| `steel_lattice.fs` | Raymarched gyroid-like lattice of interlocking steel tubes with cellular bump mapping and a subtle blackbody-tinted fire-reflection glow |
| `tas_psychedelic.fs` | Layered psychedelic bilateral ornamental art |
| `truchet_tube.fs` | Raymarched superquadric truchet-tube tunnel flythrough with randomly-oriented arc cells |
| `tunnelines.fs` | Infinite tunnel with animated lines |
| `turing_3d.fs` | Ray-marched volumetric reaction-diffusion |
| `turing_patterns.fs` | Brain-coral reaction-diffusion (Gray-Scott model) |
| `voronoi.fs` | Animated cellular/organic Voronoi pattern |
| `warped_grid.fs` | Raymarched pinwheel-skewed extruded grid of pylons along a warped/twisted tunnel path, early-2000s demoscene style, with per-cell glow-blink trail |

## Filters

| Shader | Description |
|--------|-------------|
| `add_subtract.fs` | Add/subtract RGB values |
| `ascii_art.fs` | Renders image using real font glyph atlases |
| `big_brother.fs` | Surveillance overlay — face detection with dossier info boxes |
| `block_distort.fs` | Scrambles image in blocky chunks |
| `blur.fs` | Gaussian blur |
| `brightness_contrast.fs` | Brightness and contrast adjustment |
| `channel_mixer.fs` | Reroute and mix RGB channels |
| `chroma_flow.fs` | Warps the previous frame through a drifting camera and grades the result into flat color groups, so the groups slither and morph like a Deforum animation. Dark ground acts as a boundary the flow crawls around, with an adjustable hardness that lets it burst its banks. A circular mask can hold part of the frame still while the rest flows |
| `chroma_key.fs` | Keys a target color to a given opacity |
| `color_balance.fs` | Adjust shadows, midtones, highlights independently |
| `color_correction.fs` | Brightness, contrast, saturation, hue shift grading |
| `color_replace.fs` | Match a source color and replace with a target color |
| `colorize.fs` | Maps luminance to a color palette |
| `crop.fs` | Mask/crop with adjustable edges |
| `displace.fs` | Luminance-based displacement mapping |
| `duotone.fs` | Two-color toning based on luminance |
| `edge_detect.fs` | Clean Sobel edge detection with color options |
| `edge_glow.fs` | Edge detection with glow |
| `emboss.fs` | Relief/emboss convolution |
| `feedback_trails.fs` | Moving regions leave ghostly color-shifted trails that linger and fade |
| `film_grain.fs` | Analog film grain noise overlay |
| `flip.fs` | Mirror/flip horizontally or vertically |
| `freeze.fs` | Holds/freezes the current frame |
| `glow_bloom.fs` | Soft glow around bright areas |
| `goo.fs` | Goo / liquid distortion |
| `gradient_map.fs` | Maps luminance to a 4-stop color gradient |
| `halftone.fs` | Print-style dot pattern |
| `heat_distort.fs` | Rising heat-wave shimmer |
| `hue_key.fs` | Keys out pixels matching a target hue range |
| `hue_shift.fs` | Hue rotation / color cycling |
| `invert.fs` | Color inversion with blend control |
| `kaleidoscope.fs` | Kaleidoscope mirror effect |
| `levels.fs` | Input/output levels with gamma curve |
| `luma_key.fs` | Keys out pixels based on brightness |
| `melt_drip.fs` | Makes the image look like it's melting and dripping down |
| `mirror.fs` | Mirror / flip with various modes |
| `mirror_kaleidoscope.fs` | Mirror and kaleidoscope with multiple reflection modes |
| `motion_blur.fs` | Directional blur along an angle |
| `old_film.fs` | Vintage projector look with scratches and flicker |
| `outline.fs` | Edge detection with filled or outline rendering |
| `pinch_bulge.fs` | Radial pinch or bulge distortion |
| `pixelate.fs` | Pixelation / mosaic |
| `point_cloud.fs` | Reprojects the image into a pseudo-3D cloud of soft splats (brightness = depth) with parallax orbit, depth fade, and Source/Depth/Thermal/Mono color modes; a persistent motion-reactive disturbance field lets live camera/video motion (wave a hand, Kinect/TouchDesigner style) scatter and recolor the points |
| `polkadot.fs` | Circular dot pattern overlay |
| `posterize.fs` | Reduces color count for a graphic poster look |
| `rgb_shift.fs` | Chromatic aberration / RGB shift |
| `ripple.fs` | Animated circular wave distortion |
| `scanlines.fs` | CRT-style horizontal scan lines |
| `scatter_popup.fs` | Shrinks input into small copies that pop up randomly |
| `sepia.fs` | Warm vintage sepia tone |
| `shape_mask.fs` | Mask area with selectable shape, position, size, feather |
| `sharpen.fs` | Unsharp mask sharpening |
| `shift_glitch.fs` | Digital glitch / shift glitch |
| `sphere.fs` | Spherical/fisheye lens distortion |
| `strobe.fs` | Flash to solid color on beat or timer |
| `threshold.fs` | Reduces to black and white or limited colors |
| `tile.fs` | Repeat/tile the image in a grid |
| `tilt_shift.fs` | Fake miniature/selective focus blur |
| `tint.fs` | Color tint overlay |
| `transform.fs` | 2D translate, rotate, scale |
| `tunnel.fs` | Infinite zoom tunnel distortion |
| `twist.fs` | Rotational twist/twirl from center |
| `vhs_crt.fs` | Retro video distortion with tracking errors |
| `vignette.fs` | Darkens edges of frame |
| `wave_warp.fs` | Wave warp distortion |
| `zoom.fs` | Scales the image from a center point |
| `zoom_blur.fs` | Radial blur from center point |

## Transitions

| Shader | Description |
|--------|-------------|
| `transition_dissolve.fs` | Smooth crossfade dissolve between two sources |
| `transition_iris.fs` | Circular reveal from center |
| `transition_luma_key.fs` | Luma-based transition — brighter areas transition first |
| `transition_push.fs` | Slides one image, pushing the other off |
| `transition_wipe_down.fs` | Vertical wipe from top to bottom |
| `transition_wipe_left.fs` | Horizontal wipe from left to right |
| `transition_wipe_right.fs` | Horizontal wipe from right to left |
| `transition_wipe_up.fs` | Vertical wipe from bottom to top |
| `transition_zoom.fs` | Zooms into source revealing destination |

## Compute

| Shader | Description |
|--------|-------------|
| `black_hole_sim.comp` | N-body black hole with 65,536 persistent shell particles, Schwarzschild lensing, accretion disk, Hawking glow |
| `compute_gradient.comp` | Simple animated gradient (compute shader) |
| `cosmic_web.comp` | Dark matter cosmic web via the Zel'dovich approximation — analytic Fourier mode synthesis from a CDM power spectrum, cloud-in-cell density deposit, growth-factor collapse |

> The catalog grows over time. The authoritative list is whatever sits in your workspace `shaders/` directory.

---

[← Prev: Resolution, Settings & Monitoring](10-resolution-and-monitoring.md) · [Home](README.md) · [Next: ISF Shader Authoring →](12-isf-authoring.md)
