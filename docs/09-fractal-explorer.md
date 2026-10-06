# Fractal Explorer

`fractal_explorer.fs` is a generator for flying through 3D fractals. You build the fractal from up to
six formulas, fly a camera through it, and light and grade it. Drag it from **Generators** in the
Library onto a channel.

Every control is an ordinary shader parameter, so you can map it to MIDI, OSC or a keyboard shortcut,
automate it in Arrangement mode, or modulate it.

It starts on a scale 2.2 Amazing Box turned a little in slot 1, a stack with open spaces to fly through.
Press **Find Inside** to drop into one.

## How it's built

The Fractal Explorer is an example of a full visual engine running inside Varda, built only from
the parts any shader author has. It is one ISF shader and one preprocessor:

- **The shader** (`shaders/fractal_explorer.fs`) draws. It is a multi-pass ISF generator: it
  marches the fractal into a G-buffer, computes shadows and occlusion, lights the scene, builds up
  detail over frames, and finishes with depth of field, bloom and the grade. Its formula stack is
  compiled per stack, its Look and Stack dropdowns write the sliders, and its camera buttons are
  momentary events. [Shader Authoring](14-isf-authoring.md) covers each of these features:
  multi-pass rendering with history, `SPECIALIZE` inputs, input presets and event inputs.
- **The `fractal_flight` preprocessor** holds the state and runs the logic. Every frame, before
  the shader draws, it reads the shader's inputs, flies the camera against its own copy of the
  fractal, runs Find Inside, the autopilot and the saved locations, and sets the render scale
  that holds your target frame rate. It hands the shader the camera as a texture, saves the camera
  and locations with your scene, and tells the shader which build it needs.

[Analyzers & Preprocessors](16-analyzers-and-preprocessors.md#preprocessors) explains the preprocessor system
this relies on and walks through the explorer as its example. The same split, a shader that draws
and a preprocessor that keeps state, is how to build other instruments in Varda.

## Flying

The five controls above the sections fly the camera:

| Control | Does |
|---|---|
| **Throttle** | Forward (positive) and back (negative) |
| **Speed** | How fast full throttle is: scene lengths per second in Walk, distances to the surface per second in Dive |
| **Flight** | **Walk** moves at a steady pace through a place; **Dive** zooms into detail |
| **Heading** | Turns the view left and right. Drag it 30 degrees and the view turns 30 degrees and stays |
| **Pitch** | Turns the view up and down, the same way |

Heading and Pitch turn the camera by as much as you move them, so they work alongside the
autopilot and saved locations, which turn it too. For a view that keeps turning while you hold a
control, use **Yaw Speed** and **Pitch Speed**.

**Walk** is for moving through a place, the way you would film it: the pace and the fog stay the
same for the whole shot, and the camera slows only when it comes close to a wall. **Dive** is for
falling into the fractal: speed follows the distance to the surface, so the camera slows as it
approaches and the detail keeps growing, and fog and lighting scale with it. Switching from Dive to
Walk keeps the scale you dove to. In both, the camera cannot fly into the solid.

**Find Inside** (in **Camera**) drops the camera into open space surrounded by the fractal, looking
down the longest way that still ends in structure. Press it after changing formulas, then fly from
there. Some stacks are solid lumps with no rooms at all; Find Inside then shows the most open spot
it found and says so in a notification. Saved locations remember their scale, so recalling one brings back its pace and fog.

**Camera** holds Roll, Yaw Speed, Pitch Speed, Roll Speed, Strafe X and Y, Field of View, Reset
Camera and Find Inside.

### Autopilot and locations

In **Motion**:

- **Autopilot** (0 to 1) hands the steering over. It looks ahead in a cone and turns toward
  directions with room to fly, structure to look at, and detail where it lands, and away from walls.
  Between 0 and 1 it blends with your stick.
- **Save Location** stores the current camera position and direction. Map it to a button.
- **Location** picks a saved location. Changing it flies the camera there over **Recall Seconds**
  (0 cuts).
- **Tour Locations** flies through the saved locations in order, **Seconds per Stop** at each, and
  loops. A tour that ends where it started makes a seamless loop.

Locations are saved with the scene and with deck presets.

## Building the fractal

**Stack**, at the top of **Form**, starts you from a ready-made formula stack: picking one sets
every slot, the hybrid settings, the iteration cap and Julia, then moves the camera to Find
Inside. Tune the sliders from there. The stacks after the default come from Julius Horsthuis's
tutorials:

| Stack | Formulas | Speed |
|---|---|---|
| Turned Box | Amazing Box scale 2.2, turned (the default) | Fast |
| Goldcape | Lin Combine, Rotate 4D, Amazing Box x4, Koch Cube, JCube, Reciprocal | Medium |
| Blue Sphere Temple | Goldcape with Koch Cube x16 | Medium |
| Goldcape Temple 2016 | Rotate 4D, Amazing Box x4, Koch Cube, JCube, Reciprocal, Transform | Medium |
| Dark Gate | Rotate 4D, ModKali box x4, SinY, Koch Cube, Reciprocal | Slow |

Medium stacks run at about half the frame rate of the default; Dark Gate at about a seventh,
since its Sine slot bends space in a way that costs more to draw.

**Slot 1** to **Slot 6** each hold one formula:

| Formula | Variants | A | B | C | D |
|---|---|---|---|---|---|
| Box | Amazing Box, Amazing Surf, Surf Cylinder, ModKali, Kalibox | Scale | Min Radius | Fold Limit | Fixed Radius |
| Menger | | Scale | Center Offset | | |
| Sierpinski | | Scale | Offset | | |
| KIFS | Tetrahedral, Octahedral, Icosahedral folds | Scale | Offset | Abs First (tetra) | Fold Intensity |
| Pseudo-Kleinian | | Box Size | Inversion Size | End Plane Z | |
| Kaliset | | Scale | Radius Offset | | |
| Mandelbulb | | Power | Z Multiplier | | |
| Transform | | Scale | Offset X | Offset Y | Offset Z |
| Helispiral | | Twist per Radius | Twist per Height | Fixed Twist | |
| Gnarl | | Step | Alpha | Beta | Scale |
| Bulbox P-2 | Box on, Box off | Scale | Inner Radius | Inner Scale | Fold Limit |
| Sphere Inversion | Point, Whole space | Radius | Center X | Center Y | Center Z |
| Polyfold Sym | | Order | Angle Shift (degrees) | Shift X | Shift Y |
| Sine | Y, X, Z | Offset 1 | Scale 1 | Scale 2 | Offset 2 |
| Reciprocal | X, Y, Z | Limiter | | | |
| Repeat | Mirror, Plain | Cell Size | Copies Each Way (0: endless) | Warp | Phase |
| Koch Cube | | Post Scale (1) | XY Stretch (1) | Z Fold (1) | X Add (0) |
| JCube | | Alpha (0.414) | GScale (3) | Edge Center (1) | Corner Center (1) |
| Lin Combine | | X Multiplier (1) | Y Multiplier (1) | Z Multiplier (1) | |
| Rotate 4D | | XW Angle (half turns) | YW Angle | ZW Angle | |
| ABoxMod2 | | Scale (2) | Min R (0.5) | Fold XY (1) | Fold Z (1.5) |
| msltoe Sym4 | | XZ Sym-Mul (1) | XY Sym-Mul (1) | YZ Sym-Mul (1) | |

Each slot also has **Iterations** (how many times it runs per visit, 1 to 16) and **Rotate X, Y,
Z**. To turn a slot off, set its Formula to Empty.

The renderer is compiled for the stack you build: Formula, Variant and Iterations, and the
Hybrid, Repeat From Slot and Part 2 From Slot settings. Changing one recompiles it, which takes a
moment the first time; going back to a stack you used recently is instant. These controls are
not modulated. For motion, map and modulate the A to D parameters and the rotations, which are free
to change every frame.

Helispiral and Gnarl are the symmetry breakers: put one after a Box or Menger slot, in the part
that repeats, to bend its straight lines. A slot's Rotate X to Z break symmetry too, at no extra
cost; the default stack uses them. Small values go far: Gnarl step 0.1 to 0.2 with alpha,
beta and scale at 1, or Helispiral 0.1 per radius and 0.2 per height. A Gnarl scale of 0 counts
as 1.

Sine, Reciprocal, Bulbox, KIFS with a fold intensity other than 0 or 1, Repeat with a warp, and
whole-space Sphere Inversion cost much more per frame when they are in the part that repeats, and
more again close to a surface. In a slot that runs once, such as Repeat in slot 1, they cost
little.

**Koch Cube, JCube, Lin Combine, Rotate 4D, ABoxMod2 and msltoe Sym4** are MB3D formulas. Each slot
keeps its own parameter values when you change its formula, so after choosing one of these, dial in
the defaults shown in brackets above. Koch Cube and JCube never add the seed: they are folding IFS
shapes on their own (Koch Cube needs at least 10 iterations). Lin Combine scales each axis before
the slot rotation. Rotate 4D turns the point through a fourth axis: two angles at 1 (half turns, so
180 degrees) mirror two axes. ABoxMod2 is the Amazing Box with a cylinder in place of the sphere,
and Sym4 is a power-2 bulb with a four-fold symmetry.

**Bulbox P-2** applies a box fold, then an inverse power near the center, blended to plain scaling
further out. It is discontinuous. It works well after an Amazing Surf slot in Julia mode.

**Polyfold Sym** folds space into Order slices around the z axis, like a kaleidoscope. In slot 1 it
makes the whole world symmetric; in a later slot it acts only on parts. Animating Angle Shift from 0
to 360 / Order is a seamless loop. **Sine** bends one axis through a sine wave (SinY). Animating
Offset 1 slides along an endless chain. **Reciprocal** pulls one axis toward a limit, which cleans
up and stretches forms.

**Repeat** tiles the world into cells so it never ends: put it in slot 1, the fractal in slot 2,
and set **Repeat From Slot** to 2. A cell a little smaller than the fractal packs copies into
each other; larger leaves open space between them. **Mirror** flips every other cell so the joins
match. The slot's rotation turns the whole lattice.

**Warp** bends the grid before tiling, so no two cells match and the rows stop lining up. Its
waves have golden-ratio and silver-ratio lengths, so the pattern never repeats. Try 0.1 to 0.3.
**Phase** shifts the waves to get a different world with the same cell size.

**Sphere Inversion** turns space inside out around a sphere. With **Whole space**, in slot 1 with
**Repeat From Slot** set to 2, it bends everything the other slots build. Its radius and center are
free to animate, so mapping or modulating them moves the sphere through the fractal.

**Form** holds the settings for the whole stack:

- **Hybrid: Alternate** runs the slots in order, each its Iterations times, then repeats from
  **Repeat From Slot**. Slots before that run once, as a set-up transform.
- **Hybrid: Combine** splits the slots at **Part 2 From Slot** into two fractals and combines them:
  Union, Intersect, Subtract (part 2 with part 1 carved out), Chamfer or Fillet. **Combine Width**
  sets the bevel, in pixels. A Box temple in part 1 and an organic hybrid in part 2 is a common
  use.
- **Iterations** caps the total, **Bailout** is the escape radius, and **Julia** replaces the
  sample point with a fixed seed (**Julia X, Y, Z**). Turning Julia on or off moves the camera to
  Find Inside, because the Julia set is a different shape.

## Look

**Look**, at the top of **Lighting**, sets the whole look at once: **Stone Hall** (the default,
gray stone with a moss accent under a soft sun), **Desert Sunbeams** (a low warm sun with light
shafts), **Moonlit** (a cold light from behind and a warm lantern at the camera) and **Teal &
Gold**. Picking one moves the lighting, palette, material, fog, sky and grade sliders to its
values. Adjust any of them from there; picking the same look again keeps your changes, and picking
another replaces them. A saved scene or preset keeps the sliders as you left them. Look is a
parameter like any other, so you can map it to a MIDI control and change it with the music.

| Section | Controls |
|---|---|
| **Lighting** | Sun direction, color, intensity; shadow softness and length; a fill light; a headlight at the camera; sky and ground ambient; occlusion; bounce light |
| **Palette** | Color source (iterations, orbit trap, stretch, slope, or which combined part), four colors, offset and scale, roughness, metallic, specular, and a glow band with width and gain. Scale is how fast the colors cycle; the default 0.3 changes color slowly across a structure. Higher values give finer rings, which are averaged inside each pixel rather than shimmering |
| **Atmosphere** | Fog and its color; iteration fog, which gathers where rays graze the surface; light shafts |
| **Lens** | Depth of field: Focus (with Autofocus on what the camera faces) or Ramp (far things blur more); aperture |
| **Grade** | Exposure, contrast, saturation, bloom, vignette, chromatic aberration, grain |
| **Surface** | A real material on the fractal: Stone, Mossy Rock, Weathered Metal, Bark, Forest Ground or Ice; how it is mapped, its scale, amount and bump, and the Depth of the orbit that places it |

The Surface materials are scanned textures wrapped onto the fractal by its own orbit, so the
pattern follows the structure instead of sliding across it. They show on smooth surfaces a few
pixels or more across; on detail finer than the texture can show, the surface takes the
material's average color instead of shimmering. Raise **Depth** for texture that follows finer
structure, lower it for larger, calmer patterns. The textures are from ambientCG, public domain
(CC0).

Fog, shadow length and the headlight are measured in the scene scale: fixed while walking, so the
fog belongs to the place, and following the distance to the surface while diving, so the look
holds as you fly into smaller structure. For music sync, modulate **Glow Gain**
or a light's intensity from an audio band.

## Quality and speed

In **Detail**:

- **Detail (px)**: how close a ray must come to the surface to count as a hit. Higher is faster and
  softer.
- **Geometry Band (px)**: stops iterating where detail is finer than this many pixels. Off (0)
  by default. Try 1 to 2 on solid fractals that shimmer; porous ones turn blobby.
- **Ray Steps** and **Step Size**: the march budget. Lower Step Size if thin structure disappears.
- **Temporal Smoothing**: blends frames to remove noise. Turn it off only to compare.
- **Tile Prepass**: skips empty space in 8x8 tiles.
- **Target FPS** and **Max Render Scale**: the explorer picks its render resolution every frame
  to hold Target FPS, up to Max Render Scale of the deck's resolution, and rebuilds full
  resolution with Temporal Smoothing. Pixel cost falls with the square of the scale: at 0.5 the
  fractal costs about a quarter. Set Target FPS to 0 to render at Max Render Scale always. The
  live scale is published as `render_scale_live`. It follows the whole app's frame time, so a heavy
  deck elsewhere also lowers it.
- **Sharpness**: contrast-adaptive sharpening after temporal smoothing (AMD RCAS). 0 is off. The
  default is 0.85.
- **Still Samples**: how many jittered samples a pixel averages while the camera holds still.
  Detail improves for that many frames, past the deck's resolution; moving pixels average
  8. Geometry looks the same at every render scale.

**Diagnostic** shows what the renderer is doing: steps and work, normals, depth, exit cause
(green hit, yellow closest approach, blue miss, red work limit), iterations, shadow and
occlusion, and shade reuse (white where shadow and occlusion were computed this frame).

A magenta and black checkerboard means the shader and the running Varda do not agree on the
camera data format. Restart Varda after updating the shader file.

---

[← Prev: Shader Library](08-shader-library.md) · [Home](README.md) · [Next: Outputs →](10-outputs.md)
