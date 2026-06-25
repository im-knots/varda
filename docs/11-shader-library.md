# Shader Library

Varda ships with a catalog of ISF shaders, ready to drag from the **Library** panel into any deck (generators), onto a deck/channel/master (filters), or into a transition slot. All of them live in the `shaders/` directory and can be opened, edited, and hot-reloaded — see [ISF Shader Authoring](12-isf-authoring.md).

Shaders are classified automatically by type:

- **Generators** create visuals from scratch (no image input).
- **Filters** process an input image (blur, color grade, distort, key).
- **Transitions** blend two sources via a `progress` parameter.
- **Compute** shaders (`.comp`) run GPU-native simulations and particle systems.

## Generators

| Shader | Description |
|--------|-------------|
| `abstract_field.fs` | Abstract generative field — flowing organic patterns |
| `bars.fs` | Animated bars/stripes generator |
| `big_bang.fs` | Cyclical cosmic evolution with fluid-sim galaxy dust, stellar lifecycle, expansion/crunch |
| `black_hole.fs` | Particle-streak shell with emergent accretion disk, jets, orbiting crystals (black & white) |
| `char_cycle.fs` | Cycles through glyphs from a selected script |
| `checkerboard.fs` | Checkerboard pattern generator |
| `crystal_cave.fs` | Fly through a 3D cave filled with growing crystal formations |
| `cymatics.fs` | Chladni plate and Faraday wave vibration pattern generator |
| `dark_matter.fs` | Cosmic web filament network (neuro noise) |
| `fire.fs` | Procedural animated fire effect |
| `fractal.fs` | Mandelbrot / Julia set generator |
| `fractal_mandelbox.fs` | Raymarched 3D Mandelbox explorer with flythrough camera and orbit-trap coloring |
| `fractal_mandelbulb.fs` | Raymarched 3D Mandelbulb explorer with flythrough camera and orbit-trap coloring |
| `fractal_menger.fs` | Raymarched 3D Menger sponge explorer with flythrough camera and orbit-trap coloring |
| `game_of_life.fs` | Conway's Game of Life — cellular automaton with persistent state |
| `generative_feedback.fs` | Evolving patterns using a persistent feedback buffer |
| `gradient.fs` | Color gradient generator — linear, radial, or angular |
| `graph_network.fs` | Physics-driven floating nodes that connect by proximity |
| `grid.fs` | Dot/point grid generator |
| `hilbert_curve.fs` | Space-filling fractal growing outward from center |
| `lagrangian.fs` | Standard Model Lagrangian typed terminal-style with parallax layers |
| `lines.fs` | Animated geometric lines generator |
| `liquid_light.fs` | 1960s liquid light show — oil/water/dye overhead projector psychedelia |
| `noise.fs` | Procedural simplex-style animated noise |
| `oscilloscope.fs` | Audio-reactive waveform and shape visualizer with 2D/3D modes |
| `particle.fs` | Procedural particle field generator |
| `particle_collider.fs` | ATLAS/CERN-style collision with cascading fission tracks |
| `plasma.fs` | Simple plasma effect |
| `quantum_membrane.fs` | Rolling wave-mesh terrain with rainbow grid flyover |
| `radar.fs` | Radar sweep generator |
| `rings.fs` | Concentric animated rings generator |
| `sacred_geometry.fs` | Flower of Life, Metatron's Cube, Sri Yantra, Fibonacci spiral, and more |
| `shaper.fs` | Geometric shape generator — circle, triangle, square, star, polygon |
| `solid_color.fs` | Solid color fill generator |
| `starfield.fs` | Classic parallax star tunnel |
| `tas_psychedelic.fs` | Layered psychedelic bilateral ornamental art |
| `tunnelines.fs` | Infinite tunnel with animated lines |
| `turing_3d.fs` | Ray-marched volumetric reaction-diffusion |
| `turing_patterns.fs` | Brain-coral reaction-diffusion (Gray-Scott model) |
| `voronoi.fs` | Animated cellular/organic Voronoi pattern |

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
| `chroma_flow.fs` | Color-grouped displacement that makes similar-color regions flow apart |
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

> The catalog grows over time. The authoritative list is whatever sits in your workspace `shaders/` directory — open the Library panel to see exactly what's available in your build.

---

[← Prev: Resolution, Settings & Monitoring](10-resolution-and-monitoring.md) · [Home](README.md) · [Next: ISF Shader Authoring →](12-isf-authoring.md)
