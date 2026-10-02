# Fractal Explorer

`fractal_explorer.fs` is a generator for flying through 3D fractals. You build the fractal from up to
six formulas, fly a camera through it, and light and grade it. Drag it from **Generators** in the
Library onto a channel.

Every control is an ordinary shader parameter, so you can map it to MIDI, OSC or a keyboard shortcut,
automate it in Arrangement mode, or modulate it.

It starts on a scale 2.2 Amazing Box turned a little in slot 1, a stack with rooms to fly through.
Press **Find Inside** to drop into one.

## Flying

The four controls above the sections fly the camera:

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

Each slot also has **Iterations** (how many times it runs per visit) and **Rotate X, Y, Z**.

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

These formulas stretch space unevenly, so the explorer tracks how the whole space is stretched
(the Jacobian) to measure distance. This also applies to Sine, Reciprocal, Bulbox, KIFS with a fold
intensity other than 0 or 1, Repeat with a warp, and whole-space Sphere Inversion, when they are
in the part that repeats. There it costs three to four times the frame time of a stack without
them, and close up to a surface many times more; Target FPS lowers the render scale, but only so
far. In a slot that runs once, such as Repeat in slot 1, it
costs almost nothing.

**Koch Cube, JCube, Lin Combine, Rotate 4D, ABoxMod2 and msltoe Sym4** are the MB3D formulas of
Julius Horsthuis's named stacks. Each slot keeps its own parameter values when you change its
formula, so after choosing one of these, dial in the defaults shown in brackets above. Koch Cube and
JCube never add the seed: they are folding IFS shapes on their own (Koch Cube wants at least 10
iterations), the Goldcape temples' building blocks. Lin Combine scales each axis before the slot
rotation; Julius animates its X multiplier. Rotate 4D turns the point through a fourth axis: two
angles at 1 (half turns, so 180 degrees), as in his Goldcape stacks, mirror two axes. ABoxMod2 is the Amazing Box with a
cylinder in place of the sphere, and Sym4 is a power-2 bulb with a four-fold symmetry; both are in
his forest piece.

**Bulbox P-2** is the lace formula: a box fold, then an inverse power near the center, blended to
plain scaling further out. It is discontinuous on purpose. After an Amazing Surf slot in Julia mode
it gives the intricate lace surfaces of Julius Horsthuis's HDR piece.

**Polyfold Sym** folds space into Order slices around the z axis, like a kaleidoscope. In slot 1
it makes the whole world symmetric; in a later slot it acts only on parts. Animating Angle Shift
from 0 to 360 / Order is a seamless loop. **Sine** bends one axis through a sine wave (Julius's
SinY); animating Offset 1 slides along an endless chain. **Reciprocal** pulls one axis toward a
limit, which cleans up and stretches forms.

**Repeat** tiles the world into cells so it never ends: put it in slot 1, the fractal in slot 2,
and set **Repeat From Slot** to 2. A cell a little smaller than the fractal packs copies into
each other; larger leaves open space between them. **Mirror** flips every other cell so the joins
are seamless. The slot's rotation turns the whole lattice.

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
  sets the bevel, in pixels. A Box temple in part 1 and an organic hybrid in part 2 is the classic
  use.
- **Iterations** caps the total, **Bailout** is the escape radius, and **Julia** replaces the
  sample point with a fixed seed (**Julia X, Y, Z**).

## Look

**Look**, at the top of **Lighting**, switches the whole look at once: **Stone Hall** (the
default, grey stone with a moss accent under a soft sun), **Desert Sunbeams** (a low warm sun with
light shafts), **Moonlit** (a cold light from behind and a warm lantern at the camera) and **Teal &
Gold**. Each sets the lighting, the palette colors and materials, the fog, the sky and the grade.
While one is selected those sliders do nothing; choose **Custom** to use them. The sliders start at
Stone Hall's values, so Custom begins from the default picture. Look is a parameter like any other,
so you can map it to a MIDI control and change it with the music.

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
- **Geometry Band (px)**: stops iterating where detail gets finer than this many pixels. Off (0)
  by default: the cut fills the holes it skips, so porous fractals turn into rounded blobs and
  can even render slower. Try 1 to 2 for solid fractals that shimmer.
- **Ray Steps** and **Step Size**: the march budget. Lower Step Size if thin structure disappears.
- **Temporal Smoothing**: blends frames to remove noise. Turn it off only to compare.
- **Tile Prepass**: skips empty space in 8x8 tiles.
- **Target FPS** and **Max Render Scale**: the explorer picks its render resolution every frame
  to hold Target FPS, up to Max Render Scale of the deck's resolution, and rebuilds full
  resolution with Temporal Smoothing. Pixel cost falls with the square of the scale: at 0.5 the
  fractal costs about a quarter. Set Target FPS to 0 to render at Max Render Scale always. The
  live scale is published as `render_scale_live`. It follows the whole app's frame time, so a heavy
  deck elsewhere also lowers it.
- **Sharpness**: contrast-adaptive sharpening after temporal smoothing (AMD's RCAS). 0 is off; the
  default 0.85 brings fine detail close to a supersampled render without halos.
- **Still Samples**: how many jittered samples a pixel averages while the camera holds still.
  Detail keeps sharpening for that many frames, past the deck's resolution; moving pixels average
  8. Hits are measured in output pixels, so geometry looks the same at every render scale.

**Diagnostic** shows what the renderer is doing: steps and work, normals, depth, exit cause
(green hit, yellow closest approach, blue miss, red work limit), iterations, shadow and
occlusion, and shade reuse (white where shadow and occlusion were computed this frame; held still,
about one 8x8 block in four).

A magenta and black checkerboard means the shader and the running Varda do not agree on the
camera data format. Restart Varda after updating the shader file.
