# Shader Library

Varda ships with a catalog of ISF shaders. Drag them from the **Library** panel into a deck (generators), onto a deck, channel or master (filters), or into a transition slot. They are in the `shaders/` directory, and you can open, edit and hot-reload them. See [ISF Shader Authoring](14-isf-authoring.md).

Varda sorts shaders by type automatically:

- **Generators** create visuals with no image input.
- **Filters** process an input image (blur, color grade, distort, key).
- **Transitions** blend two sources using a `progress` parameter.
- **Compute** shaders (`.comp`) run simulations and particle systems on the GPU.

## Generators

| Shader | Description |
|--------|-------------|
| `abstract_field.fs` | Abstract generative field: flowing organic patterns |
| `alien_grove.fs` | Raymarched night forest flythrough of lacy umbel trees rising out of circular wells cut in the terrain, with smaller Menger crystal lattices and recursive fern-corals grown between them; RGB energy pulses run along circuit traces etched into the rock, which meander with it, ring the lip of every well, converge on webs centered under each trunk, climb the trunks and spars, and color their terminal auras beneath a cratered moon and log-periodic fractal halo |
| `apollonian_glow.fs` | Raymarched Kali-fold + Apollonian fractal tunnel lit entirely by an accumulated glow trail, with reflection pass |
| `aurora_borealis.fs` | Northern lights: fake-volumetric raymarch through folded noise sheets, green-to-violet curtains with starfield |
| `bars.fs` | Animated bars/stripes generator |
| `bicycle_day.fs` | Raymarched "Amazing Surface" fractal tunnel shaded by normals + dark edge lines, low sun, procedural rainbow trail, with fractal trees lining the roadside |
| `big_bang.fs` | Cyclical cosmic evolution with fluid-sim galaxy dust, stellar lifecycle, expansion/crunch |
| `biomine.fs` | Raymarched biotube lattice (gyroid surfaces) pumping fluid through a mine tunnel, with cellular bump mapping and fake reflective/refractive fluid glow |
| `black_hole.fs` | Particle-streak shell with emergent accretion disk, jets, orbiting crystals (black & white) |
| `char_cycle.fs` | Cycles through glyphs from a selected script |
| `checkerboard.fs` | Checkerboard pattern generator |
| `clouds.fs` | Raymarched volumetric cloud layer flythrough with sun glow and rim-lit shadowing |
| `crystal_cave.fs` | Fly through a 3D cave filled with growing crystal formations |
| `cube_subdivision.fs` | Raymarched cube recursively split into cuboid cells with holes and raised faces, over a polar tiled floor whose rings ripple with each turn, with depth of field and bloom |
| `cymatics.fs` | Chladni plate and Faraday wave vibration pattern generator |
| `dark_matter.fs` | Cosmic web filament network (neuro noise) |
| `digital_brain.fs` | Glowing voronoi-noise plasma with drifting camera and pulsing "moving electrons" octaves |
| `dull_skull.fs` | Raymarched skull with an animated jaw that sways, turns and drifts in front of a backdrop, with glowing eyes, fresnel rim light and fog. Controls include Mouth Open, Jaw Chatter, Head Turn and Sway Range |
| `eyes.fs` | Tiled grid of procedural cartoon eyes: autonomous blink, drifting gaze, IQ cosine-palette irises |
| `eyes_depth.fs` | The same eyes, tracking people seen by a Kinect: the gaze follows the motion-weighted centroid of whoever is in view, lids wake as someone approaches, pupils dilate on sudden movement. **Requires an attached depth sensor**. See [ISF authoring § `depth_sensor`](14-isf-authoring.md#depth_sensor-live-depth-camera) |
| `fire.fs` | Procedural animated fire effect |
| `fractal.fs` | Mandelbrot / Julia set generator |
| `fractal_explorer.fs` | Free flight through a six-slot hybrid 3D fractal: 22 Mandelbulb3D-style formulas (Box, Menger, Sierpinski, KIFS, Pseudo-Kleinian, Kaliset, Mandelbulb, Transform, Helispiral, Gnarl, and more) chained or combined, with a key light, fill, headlight, soft shadows, occlusion, fog, light shafts, depth of field, bloom and a grade. Fly with Throttle, Yaw and Pitch (speed follows the distance to the surface), or hand over to the Autopilot. Save locations and tour them. See [Fractal Explorer](09-fractal-explorer.md) |
| `game_of_life.fs` | Conway's Game of Life: cellular automaton with persistent state |
| `generative_feedback.fs` | Evolving patterns using a persistent feedback buffer |
| `gradient.fs` | Color gradient generator: linear, radial, or angular |
| `graph_network.fs` | Physics-driven floating nodes that connect by proximity |
| `grid.fs` | Dot/point grid generator |
| `hilbert_curve.fs` | Space-filling fractal growing outward from center |
| `lagrangian.fs` | Standard Model Lagrangian typed terminal-style with parallax layers |
| `lines.fs` | Animated geometric lines generator |
| `liquid_light.fs` | 1960s liquid light show: oil/water/dye overhead projector psychedelia |
| `liquid_light_depth.fs` | The same look driven by a live Kinect: bodies in the sensor's view push a real advected fluid and read as flowing dye outlines. **Requires an attached depth sensor**. See [ISF authoring § `depth_sensor`](14-isf-authoring.md#depth_sensor-live-depth-camera) |
| `mandelbrot_deco.fs` | Mandelbrot set with a decorative pattern overlay, adjustable zoom, color modes and vignette |
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
| `shaper.fs` | Geometric shape generator: circle, triangle, square, star, polygon |
| `solid_color.fs` | Solid color fill generator |
| `star_nest.fs` | Volumetric raymarched star-field/nebula tunnel via an iterated absolute-inversion fractal, with look_at camera rotation |
| `starfield.fs` | Classic parallax star tunnel |
| `steel_lattice.fs` | Raymarched gyroid-like lattice of interlocking steel tubes with cellular bump mapping and a subtle blackbody-tinted fire-reflection glow |
| `tas_psychedelic.fs` | Layered psychedelic bilateral ornamental art |
| `taste_of_noise.fs` | Organic fractal structures built from repeated folding and smooth blending, with trails that fade by **Trail Decay** |
| `truchet_kaleidoscope.fs` | Layered Truchet patterns seen through a rotating kaleidoscope tunnel, with color modes (black and white, custom, rainbow, neon, warm, cool) |
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
| `big_brother.fs` | Surveillance overlay: face detection with dossier info boxes |
| `block_distort.fs` | Scrambles image in blocky chunks |
| `blur.fs` | Gaussian blur |
| `boxinator.fs` | Grid of cells that churn simplex noise over the input |
| `brightness_contrast.fs` | Brightness and contrast adjustment |
| `channel_mixer.fs` | Reroute and mix RGB channels |
| `chroma_flow.fs` | Warps the previous frame through a drifting camera and grades the result into flat color groups, so the groups slither and morph like a Deforum animation. Dark ground acts as a boundary the flow moves around. An adjustable hardness sets how easily the flow crosses it. A circular mask can hold part of the frame still while the rest flows |
| `chroma_key.fs` | Keys a target color to a given opacity |
| `color_balance.fs` | Adjust shadows, midtones, highlights independently |
| `color_correction.fs` | Brightness, contrast, saturation, hue shift grading |
| `color_replace.fs` | Match a source color and replace with a target color |
| `colorize.fs` | Maps luminance to a color palette |
| `contour.fs` | Topographic lines that trace levels of brightness, flowing at a set speed |
| `crop.fs` | Mask/crop with adjustable edges |
| `curves.fs` | Master and per-channel tone curves, with highlight headroom kept above 1.0 |
| `displace.fs` | Luminance-based displacement mapping |
| `dither.fs` | Ordered and noise dithering with Mono, Two Color, Game Boy, CGA, PICO-8 and quantized source palettes. Also covers posterizing (Source Quantized, Spread 0) |
| `droste.fs` | The frame contains itself, nested forever, with an endless zoom and an optional Escher spiral |
| `duotone.fs` | Two-color toning based on luminance |
| `edge_detect.fs` | Clean Sobel edge detection with color options |
| `edge_glow.fs` | Edge detection with glow |
| `emboss.fs` | Relief/emboss convolution |
| `feedback_trails.fs` | Moving regions leave ghostly color-shifted trails that linger and fade |
| `film_grain.fs` | Analog film grain noise overlay |
| `flip.fs` | Mirror/flip horizontally or vertically |
| `freeze.fs` | Holds a frame, grabs a new one on demand, or re-grabs at a set rate for a stutter |
| `glow_bloom.fs` | Soft glow around bright areas |
| `goo.fs` | Goo / liquid distortion |
| `gradient_map.fs` | Maps luminance to a 4-stop color gradient |
| `halftone.fs` | Print-style dot pattern |
| `heat_distort.fs` | Rising heat-wave shimmer |
| `hue_key.fs` | Keys out pixels matching a target hue range |
| `hue_shift.fs` | Hue rotation / color cycling |
| `invert.fs` | Color inversion with blend control |
| `kaleidoscope.fs` | Kaleidoscope mirror effect |
| `lens_streaks.fs` | Anamorphic streaks and ghost reflections from bright areas |
| `levels.fs` | Input/output levels with gamma curve |
| `light_rays.fs` | Volumetric light shafts streaming from bright areas away from a light position |
| `luma_key.fs` | Keys out pixels based on brightness |
| `matte_tools.fs` | Erode, dilate, open, close, median and feather the alpha of a key. Place it after a key effect |
| `melt_drip.fs` | Makes the image look like it's melting and dripping down |
| `mirror.fs` | Mirror / flip with various modes |
| `mirror_kaleidoscope.fs` | Mirror and kaleidoscope with multiple reflection modes |
| `mosaic.fs` | Square, hexagon, Voronoi, triangle and diamond cell mosaics with grout and bevel. Square with no edge is a pixelate |
| `motion_blur.fs` | Directional blur along an angle |
| `old_film.fs` | Vintage projector look with scratches and flicker |
| `outline.fs` | Edge detection with filled or outline rendering |
| `paint.fs` | Oil paint look from an edge-preserving anisotropic Kuwahara filter |
| `pinch_bulge.fs` | Radial pinch or bulge distortion |
| `pixel_sort.fs` | Sorts runs of pixels inside a brightness band into streaks, horizontally or vertically |
| `point_cloud.fs` | Reprojects the image into a pseudo-3D cloud of soft splats (brightness = depth) with parallax orbit, depth fade, and Source/Depth/Thermal/Mono color modes; a persistent motion-reactive disturbance field lets live camera/video motion (wave a hand, Kinect/TouchDesigner style) scatter and recolor the points |
| `polar.fs` | Wraps the image around a center point or into an endless tunnel (Radius Mode: Tunnel), or unwraps it into a strip |
| `polkadot.fs` | Circular dot pattern overlay |
| `rgb_shift.fs` | Chromatic aberration / RGB shift |
| `ripple.fs` | Animated circular wave distortion |
| `rutt_etra.fs` | Scanlines lifted by brightness and drawn as glowing lines on black |
| `scanlines.fs` | CRT-style horizontal scan lines |
| `scatter_popup.fs` | Shrinks input into small copies that pop up randomly |
| `sepia.fs` | Warm vintage sepia tone |
| `shake.fs` | Handheld camera shake with smooth noise, rotation, zoom to cover and motion blur |
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
| `twist.fs` | Rotational twist/twirl from center |
| `vhs_crt.fs` | Retro video distortion with tracking errors |
| `vignette.fs` | Darkens edges of frame |
| `water_ripples.fs` | A simulated water surface, disturbed by drops or motion, that refracts the image |
| `wave_warp.fs` | Wave warp distortion |
| `zoom.fs` | Scales the image from a center point |
| `zoom_blur.fs` | Radial blur from center point |

## Transitions

| Shader | Description |
|--------|-------------|
| `transition_dissolve.fs` | Smooth crossfade dissolve between two sources |
| `transition_iris.fs` | Circular reveal from center |
| `transition_luma_key.fs` | Luma-based transition: brighter areas transition first |
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
| `cosmic_web.comp` | Dark matter cosmic web via the Zel'dovich approximation: analytic Fourier mode synthesis from a CDM power spectrum, cloud-in-cell density deposit, growth-factor collapse |

> The catalog grows over time. For the current list, see your workspace `shaders/` directory.

---

[← Prev: Control Surfaces & Macros](07-control-surfaces.md) · [Home](README.md) · [Next: Fractal Explorer →](09-fractal-explorer.md)
