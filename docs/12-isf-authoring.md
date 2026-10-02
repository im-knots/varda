# Shader Authoring

Varda shaders are **GLSL 450 (Vulkan)** with an [ISF](https://isf.video)-style JSON metadata header that declares parameters, inputs, and passes.

> **If you have existing ISF shaders, read this first.** Varda uses ISF's metadata format but a different shader language. A shader downloaded from isf.video, VDMX, or the ISF Editor will **not** load as-is. Original ISF is GLSL ES with uniforms injected implicitly; Varda needs explicit Vulkan declarations. Porting is mechanical and usually takes a few minutes. See [Porting an ISF Shader](#porting-an-isf-shader).

The dialect also supports features ISF lacks: [compute shaders](#compute-shaders) with persistent storage buffers, [analyzer preprocessors](#analyzer-preprocessors) that inject ML and sensor data as textures, and [phase accumulators](#phase-accumulators) for speed changes without jumps.

## Shader Types

| Type | Detection | Purpose |
|------|-----------|---------|
| **Generator** | No `image` type inputs | Creates visuals from scratch (patterns, fractals, color fields) |
| **Filter** | Has at least one `image` input | Processes an input image (blur, color grade, distort) |
| **Transition** | Has `Transition` category + image inputs | Blends two images via a `progress` parameter (dissolve, wipe, push) |

Varda classifies shaders automatically from their metadata.

## Metadata Format

Every ISF shader starts with a JSON block in a block comment:

```glsl
/*{
    "DESCRIPTION": "A solid color fill",
    "CREDIT": "Author Name",
    "CATEGORIES": ["Generator"],
    "INPUTS": [
        { "NAME": "color", "TYPE": "color", "DEFAULT": [1.0, 0.0, 0.5, 1.0] }
    ]
}*/
```

### Input Types

| Type | GLSL Type | Properties | Description |
|------|-----------|------------|-------------|
| `float` | `float` | MIN, MAX, DEFAULT | Slider control |
| `bool` | `uint` | DEFAULT (true/false) | Toggle switch |
| `long` | `int` | VALUES, LABELS, DEFAULT | Dropdown / enum selector |
| `color` | `vec4` | DEFAULT [R,G,B,A] | Color picker (0.0–1.0 per channel) |
| `point2D` | `vec2` | DEFAULT [x,y] | Paired X and Y number drags, bounded by MIN and MAX |
| `image` | texture2D | (none) | Input texture (filters and transitions) |

Every input also accepts an optional `GROUP`, which puts it in a section of the inspector. See
[Grouping parameters](#grouping-parameters).

Every numeric parameter can be modulated, including individual color channels and point2D axes, and
all of them are reachable over OSC and the HTTP API. The inspector shows modulation and learn
controls on `float` sliders only. To assign a modulator to a point2D axis or a color channel, use the
API.

### Example: Float Parameter

```json
{ "NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 10.0, "LABEL": "Speed" }
```

### Example: Enum Parameter

```json
{ "NAME": "mode", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2], "LABELS": ["Normal", "Mirror", "Tile"] }
```

The shader receives `VALUES`; the performer sees `LABELS`. If you declare only `LABELS`, the index
becomes the value. If you declare neither, the input is a plain number stepper.

### Grouping parameters

Give an input a `GROUP` and the inspector puts it in a collapsible section. This helps once a shader
has more than about a dozen parameters.

```json
"INPUTS": [
    { "NAME": "brightness", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 2.0 },
    { "NAME": "fold_scale", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 2.0, "MIN": 1.0, "MAX": 4.0 },
    { "NAME": "iterations", "TYPE": "float", "GROUP": "Formula", "DEFAULT": 8.0, "MIN": 1.0, "MAX": 24.0 },
    { "NAME": "fog", "TYPE": "float", "GROUP": "Grade", "DEFAULT": 0.3, "MIN": 0.0, "MAX": 1.0 }
]
```

The inspector lays groups out by three rules:

- **Ungrouped inputs come first**, with no header, and cannot be collapsed. Put the controls a
  performer needs mid-set here so they stay in view.
- **Named groups follow in first-appearance order.** A group sits where its first member appears in
  `INPUTS`, so the order of `INPUTS` sets the order of sections.
- **The first named group starts open; the rest start closed.** A large shader opens as a short list
  of section headers.

### Columns

A shader with many groups can lay some of them out as columns of their own in the deck detail bar,
next to the params column, with the top-level `COLUMNS` key. Each column lists groups by name:

```json
"COLUMNS": [
    { "TITLE": "Light", "GROUPS": ["Lighting", "Palette"] },
    { "TITLE": "Detail", "GROUPS": ["Detail"] }
]
```

- Columns follow the params column in `COLUMNS` order. Clicking a column's title collapses it to a
  strip, like the other columns in the bar.
- A column shows its groups as sections in the order listed, the first open.
- Ungrouped inputs and any group no column names stay in the params column.
- Without `COLUMNS` the inspector is unchanged. Other ISF hosts ignore the key.
- Varda rejects the shader at load when a column names a group no input declares, puts a group in
  two columns, or has an empty `TITLE` or `GROUPS`.

### Group names in the shipped library

`GROUP` accepts any string. The shaders Varda ships use a fixed set of names, because group names
also fill the Random and Mutate scope selector:

`Camera`, `Motion`, `Form`, `Detail`, `Lighting`, `Palette`, `Grade`, `Mask`, `Audio`.

- `Form`: the geometry or formula that defines the subject.
- `Detail`: quality and stability controls (ray steps, iteration caps, epsilon).
- `Grade`: post treatment (brightness, contrast, saturation, bloom, vignette).

Most shaders use four to six of the nine. Use the same names in your own shaders to match the
built-in ones. Other names work too.

`tests/shader_param_grouping_guard.rs` holds the shipped library to three conventions:

- **A shader with fourteen or more parameters declares groups.** Below that, a flat list is fine.
- **Two to five parameters stay ungrouped.** They render first and cannot be collapsed, so they form
  the mid-set row. With nothing ungrouped, every control is behind a collapsed section.
- **No group has a single member.** A header over one row adds a click.

If you use the `<prefix>_mode` hide convention, **put the gate bool in the same group as the
parameters it hides.** Otherwise the performer sees a section that will not open, and the switch that
opens it is in another section.

`GROUP` affects presentation only. It does not change parameter names, modulation keys, MIDI or OSC
paths, or anything that is saved, so you can add groups to an existing shader without affecting
scenes or presets that use it. A shader with no groups renders as one flat list. A shader with
`GROUP` still loads in other ISF hosts, which ignore metadata keys they do not recognize.

## Built-in Uniforms

Varda injects these uniforms automatically at `set = 0, binding = 0`:

```glsl
layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;              // Elapsed seconds since shader start
    float TIMEDELTA;         // Frame delta in seconds
    uint FRAMEINDEX;         // Frame counter
    int PASSINDEX;           // Current render pass index
    vec2 RENDERSIZE;         // Output resolution [width, height]
    float audio_level;       // Overall RMS level (0.0–1.0)
    float audio_bass;        // 20–250 Hz energy
    float audio_mid;         // 250–2000 Hz energy
    float audio_treble;      // 2000–20000 Hz energy
    float audio_bpm;         // Detected BPM (0.0 if unavailable)
    float audio_beat_phase;  // Phase in beat cycle (0.0–1.0)
    vec4 DATE;               // [year, month, day, seconds_since_midnight]
    float PHASE_TIME_0;      // Phase accumulator 0
    float PHASE_TIME_1;      // Phase accumulator 1
    float PHASE_TIME_2;      // Phase accumulator 2
    float PHASE_TIME_3;      // Phase accumulator 3
    vec2 JITTER;             // Sub-pixel offset in pixels, [-0.5, 0.5), Halton (2, 3)
    int JITTERINDEX;         // Index in the 16-frame jitter cycle
    int HISTORYVALID;        // 0 on the first frame after HISTORY targets were cleared
};
```

You can stop the block after any field. A shader that declares only up to `PHASE_TIME_3` still
binds.

### Phase Accumulators

`PHASE_TIME_0` through `PHASE_TIME_3` are phase accumulators driven by user parameters. Each frame they add `dt * param_value * scale`. `TIME * speed` jumps when speed changes; an accumulator changes speed smoothly.

Declare them in the metadata:

```json
"PHASE_INPUTS": [
    { "PARAM": "rotation_speed", "INDEX": 0, "SCALE": 1.0 }
]
```

Then use it in the shader, for example `float angle = PHASE_TIME_0 * 6.28318;` for rotation that does not jump when the user adjusts speed.

The accumulator integrates the parameter's **modulated** value, the same value the shader reads from the user-parameter buffer. If you route an audio band or LFO to `rotation_speed`, the animation speeds up and slows down without jumping. Read `PHASE_TIME_N` for anything that advances over time. Multiplying the raw parameter by `TIME` brings the jump back.

#### Combining two rates

`PHASE_TIME_0 * rot_speed` also jumps, because it scales a growing phase by a live value. To put a per-element rate on top of a master speed, fold both into one accumulator with `MULTIPLY_BY`:

```json
"PHASE_INPUTS": [
    { "PARAM": "speed", "INDEX": 0, "SCALE": 1.0 },
    { "PARAM": "speed", "MULTIPLY_BY": "rot_speed", "INDEX": 1, "SCALE": 0.2 }
]
```

`PHASE_TIME_1` now accumulates `dt × speed × rot_speed × 0.2`. The shader writes `float rotAngle = PHASE_TIME_1;`, and both parameters stay smooth and modulatable. For a rate that depends on three parameters, `MULTIPLY_BY` takes an array: `"MULTIPLY_BY": ["time_scale", "flow_speed"]`.

Two rules:

- Never multiply `PHASE_TIME_N` by a user parameter.
- Never apply a parameter that already drives an accumulator a second time in the shader body. The phase already contains it, so the response becomes quadratic in that parameter.

You can break the first rule without writing the multiply. Adding phase to a coordinate that is scaled later has the same effect:

```glsl
coord += PHASE_TIME_0;
float pattern = fract(coord * line_count);   // = fract(coord*n + PHASE_TIME_0*n)
```

`line_count` looks purely spatial, but it multiplies the scroll phase, so changing it slides the whole field. Scale the position only, then add a phase that has the count inside the integral:

```glsl
float pattern = fract(coord * line_count + PHASE_TIME_1);   // MULTIPLY_BY: line_count
```

The lines then move at the same screen speed however many there are, without the jump.

`tests/shader_param_contract_guard.rs` fails the build on all of these patterns. It follows the whole multiplicative chain, so it catches a parameter behind a constant (`PHASE_TIME_0 * 0.5 * look_speed`). It follows local aliases within a function, so `float t = PHASE_TIME_0;` is also caught. It stops at function calls, because `sin(PHASE_TIME_0) * amount` is valid. It cannot see a phase passed into a function as an argument and scaled inside that function.

#### Accumulators are not position-deterministic

An accumulator's value depends on the path taken to reach it, not on the current show position. A
shader that declares `PHASE_INPUTS` resumes from its accumulated value after a jump; it does not
recompute its phase for the new position.

Modulators, automation, and cues are deterministic from position. Accumulators are the exception. If
a shader's look depends on accumulated phase, it will look different after a jump. This is expected
behavior.

#### Affine rates

`MULTIPLY_BY` covers `speed × amount`. It does not cover `speed × (1 + k · amount)`, which you need when the base motion should keep running at `amount` = 0. The product form stops the animation there.

Integration is linear, so split the term across two accumulators and add them in the shader:

```json
"PHASE_INPUTS": [
    { "PARAM": "flow_speed", "INDEX": 0 },
    { "PARAM": "flow_speed", "MULTIPLY_BY": "agitation", "INDEX": 1, "SCALE": 0.8 }
]
```

```glsl
float t = PHASE_TIME_0 + PHASE_TIME_1;   // = ∫ flow_speed·(1 + 0.8·agitation) dt
```

This is exact and continuous in both parameters. `big_bang.fs` uses it.

A factor that varies across the image but not over time, such as a per-cell hash, stays outside the integral. Only the parameter needs to be inside it. `char_cycle.fs` gives every cell its own rate with `PHASE_TIME_0 + h * PHASE_TIME_1`.

Each affine term uses one slot, and there are four slots.

#### Bounding an accumulator

An accumulator grows without limit. That suits values that cycle forever, such as a hue, a scroll offset, or a wrapping angle. It does not suit values that must stay in a range. For example, an unbounded phase fed straight into a camera angle can orbit the camera behind the scene's backdrop and render black.

Wrap the phase in a periodic function and scale that result by the amplitude parameter:

```glsl
float swayAngle = sin(PHASE_TIME_1) * sway_range;
```

This is allowed: the sine is bounded, so `sway_range` scales a value in [-1, 1]. `sway_range` is an amplitude, so it stays a plain uniform.

#### Rates and amplitudes under automation

A shader can pass the guard and still look bad under an LFO. An amplitude parameter sets where something is, so automating it moves that thing back and forth at the LFO's rate, which looks like sloshing or stutter. A rate parameter only makes motion faster or slower, so automating it cannot move anything to a new position, however fast or often it changes direction.

`liquid_light.fs`'s Agitation shows the difference. As an amplitude on the domain-warp gain, a 1 Hz triangle LFO raised per-frame change to 13.5× the baseline with the fader parked. As a mixing rate on accumulator slot 1 (advancing the inner warp stages against the outer one), the same LFO measures 0.97× the baseline, while the control still spans a 10.5× range in mixing speed.

When a control needs more range, add another rate before adding an amplitude. Ask whether the parameter names a speed or a position. Only speeds hold up under automation.

#### Parameters that belong in `PHASE_INPUTS`

Only parameters that express a rate. A parameter that sets a static angle, scale, threshold, or count must stay a plain uniform; integrating it would ramp it to its limit and hold it there. Shaders that step a simulation into a persistent buffer are already continuous and need no accumulator for their step-rate coefficients.

## Binding Layout

| Binding | Content |
|---------|---------|
| `set=0, binding=0` | ISFUniforms (all shaders) |
| `set=0, binding=1` | Sampler (if shader has textures) |
| `set=0, binding=2+` | Textures (inputImage, pass buffers, imported images) |
| Last binding | UserParams (if shader has parameters) |

Fragment input: `layout(location = 0) in vec2 uv;`, normalized coordinates (0.0–1.0).

Fragment output: `layout(location = 0) out vec4 fragColor;`

## Shader Examples

### Generator

```glsl
/*{ "CATEGORIES": ["Generator"], "INPUTS": [
    { "NAME": "color", "TYPE": "color", "DEFAULT": [1.0, 0.0, 0.5, 1.0] }
] }*/
#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;
layout(set = 0, binding = 0) uniform ISFUniforms { float TIME; /* ... */ };
layout(set = 0, binding = 1) uniform UserParams { vec4 color; };
void main() { fragColor = color; }
```

### Filter

```glsl
/*{ "CATEGORIES": ["Filter"], "INPUTS": [
    { "NAME": "inputImage", "TYPE": "image" },
    { "NAME": "amount", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0 }
] }*/
#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;
layout(set = 0, binding = 0) uniform ISFUniforms { float TIME; /* ... */ };
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform UserParams { float amount; };
void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);
    fragColor = mix(src, vec4(1.0) - src, amount);  // invert by amount
}
```

### Transition

```glsl
/*{ "CATEGORIES": ["Transition"], "INPUTS": [
    { "NAME": "progress", "TYPE": "float", "DEFAULT": 0.0, "MIN": 0.0, "MAX": 1.0 },
    { "NAME": "startImage", "TYPE": "image" },
    { "NAME": "endImage", "TYPE": "image" }
] }*/
#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in vec2 uv;
layout(set = 0, binding = 0) uniform ISFUniforms { /* ... */ };
layout(set = 0, binding = 1) uniform sampler texSampler;
layout(set = 0, binding = 2) uniform texture2D startImage;
layout(set = 0, binding = 3) uniform texture2D endImage;
layout(set = 0, binding = 4) uniform TransitionParams { float progress; };
void main() {
    vec4 from = texture(sampler2D(startImage, texSampler), uv);
    vec4 to = texture(sampler2D(endImage, texSampler), uv);
    fragColor = mix(from, to, progress);
}
```

## Porting an ISF Shader

Varda's JSON header is ISF-compatible, so the metadata usually needs no changes. The work is in the GLSL body.

### What differs

| | Real ISF | Varda |
|---|---|---|
| Language | GLSL ES (no `#version`) | GLSL 450 Vulkan, `#version 450` required |
| Uniforms | Injected implicitly by name from `INPUTS` | Declared explicitly in a `UserParams` block |
| Automatic vars | Injected implicitly (`TIME`, `RENDERSIZE`, …) | Declared explicitly in the `ISFUniforms` block |
| Textures | Combined `sampler2D` | Separate `texture2D` + `sampler` (WebGPU has no combined samplers) |
| Sampling | `IMG_THIS_PIXEL()`, `IMG_NORM_PIXEL()`, `texture2D()` | `texture(sampler2D(tex, texSampler), uv)` |
| Fragment coords | `isf_FragNormCoord`, **bottom-left** origin | `uv` varying, **top-left** origin |
| Output | `gl_FragColor` | `layout(location = 0) out vec4 fragColor` |
| Bindings | Host-managed, invisible | Explicit `layout(set = 0, binding = N)` |
| Output range | Effectively `[0,1]` (clamped at an 8-bit target) | **Unbounded**: linear-light float all the way to the tonemap |

### Don't clamp your output

Varda composites in linear-light float from the deck stage onward. Values above 1.0 survive to the
tonemap, which rolls them off (ACES by default). A final clamp throws them away:

```glsl
col = clamp(col, 0.0, 1.0);   // ← don't
```

In a generator, the clamp flattens emissive highlights. In a **filter**, it also removes headroom
produced by the deck upstream, so one clamping filter anywhere in a chain limits HDR for everything
before it.

Negative light has no meaning and some blend modes propagate it. To keep negatives out of the blend
math, floor without a ceiling:

```glsl
col = max(col, 0.0);          // ← floor only, no ceiling
```

Alpha is the exception. It is coverage, so keep it in `[0, 1]`.

Two related points when porting:

- **Don't apply your own gamma.** `col = sqrt(col)` or `pow(col, 1.0/2.2)` at the end of a
  Shadertoy port is display encoding, which Varda applies at the output. Doing it in the shader
  encodes twice. A few bundled shaders still do this and are flagged for review.
- **`IMPORTED` textures are sRGB-tagged**, so sampling them already decodes to linear. If a port
  applies `pow(tex, 1.0/2.2)` to an imported atlas, it is compensating for that decode on purpose.
  Leave it alone.

### Steps

1. **Keep the JSON header.** `DESCRIPTION`, `CREDIT`, `CATEGORIES`, `INPUTS`, `PASSES`, `IMPORTED` all parse as-is.
2. **Add `#version 450`** as the first line after the header, and delete any existing `#version`.
3. **Add the standard prologue**: `in vec2 uv`, `out vec4 fragColor`, the `ISFUniforms` block, the sampler, your textures, and a `UserParams` block listing every non-image `INPUTS` entry **in declaration order**. Copy the layout from the [Filter example](#filter) above.
4. **Delete any `varying` declarations.** They are not valid in GLSL 450 core.
5. **Replace `gl_FragColor`** with `fragColor`, and remove any final
   `clamp(col, 0.0, 1.0)`. See [Don't clamp your output](#dont-clamp-your-output).
6. **Rewrite sampling calls:**
   ```glsl
   texture2D(inputImage, c)      →  texture(sampler2D(inputImage, texSampler), c)
   IMG_NORM_PIXEL(inputImage, c) →  texture(sampler2D(inputImage, texSampler), c)
   IMG_THIS_PIXEL(inputImage)    →  texture(sampler2D(inputImage, texSampler), uv)
   IMG_PIXEL(inputImage, px)     →  texture(sampler2D(inputImage, texSampler), px / RENDERSIZE)
   IMG_SIZE(inputImage)          →  vec2(textureSize(sampler2D(inputImage, texSampler), 0))
   ```
7. **Fix the vertical orientation** (see below). This step is the one most often missed.

### The vertical flip

**ISF's `isf_FragNormCoord` has `(0,0)` at the bottom-left. Varda's `uv` has `(0,0)` at the top-left.** Substituting one for the other renders the shader upside down.

A vertically symmetric shader looks the same either way, so the error shows up only on asymmetric content such as text or a logo. Test with an asymmetric source.

To port ISF coordinate math unchanged, compute a flipped coordinate once at the top of `main` and use it everywhere ISF used `isf_FragNormCoord`:

```glsl
void main() {
    vec2 p = vec2(uv.x, 1.0 - uv.y);   // ISF/GL orientation
    // ... original ISF body, using p wherever it used isf_FragNormCoord
}
```

**Do not use the flipped coordinate for texture sampling.** Varda stores textures with a top-left origin, so sample `inputImage` and pass buffers with raw `uv`. Mixing the two produces a shader that generates correctly but samples mirrored, or the reverse.

`gl_FragCoord` needs the same treatment. Its origin is upper-left in Vulkan and lower-left in OpenGL:

```glsl
vec2 fc = vec2(gl_FragCoord.x, RENDERSIZE.y - gl_FragCoord.y);
```

About a dozen shaders in `shaders/` are ports that do this. `star_nest.fs`, `apollonian_glow.fs`, `truchet_tube.fs`, and `mandelbrot_deco.fs` are good references.

### Worked example

Original ISF:

```glsl
/*{ "CATEGORIES": ["Filter"], "INPUTS": [
    { "NAME": "inputImage", "TYPE": "image" },
    { "NAME": "amount", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0 }
] }*/
void main() {
    vec4 src = IMG_THIS_PIXEL(inputImage);
    float v = isf_FragNormCoord.y;
    gl_FragColor = mix(src, vec4(v), amount);
}
```

Ported:

```glsl
/*{ "CATEGORIES": ["Filter"], "INPUTS": [
    { "NAME": "inputImage", "TYPE": "image" },
    { "NAME": "amount", "TYPE": "float", "DEFAULT": 0.5, "MIN": 0.0, "MAX": 1.0 }
] }*/
#version 450
layout(location = 0) out vec4 fragColor;
layout(location = 0) in  vec2 uv;
layout(set = 0, binding = 0) uniform ISFUniforms { /* full block — see below */ };
layout(set = 0, binding = 1) uniform sampler   texSampler;
layout(set = 0, binding = 2) uniform texture2D inputImage;
layout(set = 0, binding = 3) uniform UserParams { float amount; };

void main() {
    vec4 src = texture(sampler2D(inputImage, texSampler), uv);  // raw uv — sampling
    float v = 1.0 - uv.y;                                       // flipped — ISF coord
    fragColor = mix(src, vec4(v), amount);
}
```

### Not supported

| ISF feature | Status |
|---|---|
| Vertex shaders (`.vs`, `isf_vertShaderInit()`) | Not supported |
| Filters with two or more `image` inputs | Not supported. Each effect takes one input image. To blend two sources, use two decks in a channel with a [blend mode](04-performance.md). |
| `audio` / `audioFFT` image inputs | Not bound. Use the `audio_*` scalars in `ISFUniforms`. |
| `.frag` / `.glsl` extensions | Only `.fs` and `.comp` are discovered |

## Multi-Pass Rendering

For feedback effects, simulations, and post-processing chains, declare multiple render passes:

```json
"PASSES": [
    { "TARGET": "feedbackBuffer", "PERSISTENT": true },
    {}
]
```

- A pass with a `TARGET` renders to a named buffer, which later passes can read as a texture.
- **Persistent** buffers keep their contents across frames. Use them for feedback loops and simulations (Game of Life, reaction-diffusion).
- The final pass (empty `{}`) renders to the output.
- Read pass buffers as `texture2D` samplers with the target name.
- Optional `WIDTH`/`HEIGHT` expressions: `"$WIDTH/2"` for half-resolution buffers. Only `$WIDTH`, `$HEIGHT`, `$WIDTH/N` and `$WIDTH*N` with integer `N` are parsed. `$WIDTH/2.0` and arithmetic like `max($WIDTH,$HEIGHT)` are not parsed and fall back to full resolution. A bare integer literal (`"WIDTH": "32"`) sets a fixed size, which you can use to build a reduction pyramid.
- Optional `FLOAT: true`. Pass buffers are 16-bit float (`rgba16float`) with or without it; use
  `FORMAT` for 32-bit.
- Optional `FORMAT`, `TARGETS` and `HISTORY`, described below.

> **`RENDERSIZE` is the size of the pass being rendered**, not the deck's. In a
> `"WIDTH": "1", "HEIGHT": "1"` pass, `RENDERSIZE` is `(1, 1)`. Anything that needs the deck's
> dimensions or aspect ratio, such as a letterbox fit or a screen-space offset, must be computed in a
> full-size pass. For this reason `eyes_depth.fs` carries its gaze target in sensor space through a
> 1x1 pass and converts it to deck space in the final pass.

**Every pass buffer is double-buffered.** Each pass reads the last value written to its target and
writes to the other texture. (Every pass binds all pass buffers as sampled textures, and wgpu rejects
a texture used as both color attachment and sampled resource.) Budget two textures per declared
target when sizing large buffers. A pass that needs its own previous output declares `HISTORY`;
reading your own target from any other pass is not guaranteed to keep working.

### `HISTORY`: last frame's output

```json
{ "TARGET": "taa", "HISTORY": true }
```

A `HISTORY` pass runs once per frame and reads its own target to get what it wrote in the previous
frame. Use it for temporal accumulation (TAA, denoising, shadows or fog built up over frames).

History starts at zero whenever the pass buffers are allocated: when the deck is created, when the
shader reloads, and when the deck is resized. On that first frame `HISTORYVALID` is 0, so the shader
can ignore the empty history; on every later frame it is 1.

`HISTORY` and `PERSISTENT` cannot be combined. `PERSISTENT` runs four substeps a frame (below), which
makes it four times as expensive as `HISTORY` for the same pass.

### `FORMAT` and `TARGETS`: G-buffers

```json
"PASSES": [
    { "TARGETS": ["gA", "gB"], "FORMATS": ["rgba32float", "rgba16float"] },
    { "TARGET": "depth", "FORMAT": "r32float" },
    {}
]
```

| Format | Use |
|---|---|
| `rgba16float` (default) | Color, normals, anything you want to sample with filtering |
| `rgba32float` | Depth and data that need full float precision |
| `r32float` | One channel of full precision at a quarter of the memory |
| `rg32float` | Two channels of full precision, for example texture coordinates |

- `TARGETS` writes up to four buffers in one pass. Declare one output per target:
  `layout(location = 0) out vec4 out0; layout(location = 1) out vec4 out1;`. All targets of a pass
  share its `WIDTH` and `HEIGHT`. Give their formats as `FORMATS`, one per target.
- **Read 32-bit targets with `texelFetch`**, never `texture()`. They are not filterable:
  `texelFetch(sampler2D(gA, texSampler), ivec2(gl_FragCoord.xy), 0)`.
- The final pass renders to the deck and has no `FORMAT` or `TARGETS`.
- **A pass's targets may total at most 32 bytes per pixel**, the portable GPU limit: `rgba32float`
  is 16, `rgba16float` and `rg32float` are 8, `r32float` is 4. Beyond it the pipeline fails to
  build.

Half-float depth has an 11-bit mantissa. A hit position rebuilt from it at 1080p is off by about
half a pixel's footprint, which is enough to make shadow rays start inside the surface. Store depth
in `rgba32float`, `r32float` or `rg32float`.

### Pass sizes from inputs

`WIDTH` and `HEIGHT` may multiply or divide by an input, so a shader can render its expensive
passes below the deck's resolution under a user control:

```json
{ "TARGET": "gbuf", "WIDTH": "$WIDTH*$render_scale", "HEIGHT": "$HEIGHT*$render_scale" }
```

An expression is `$WIDTH` or `$HEIGHT` followed by `*f` or `/f` steps, left to right, where `f` is
a number or `$name` of a `float` or `long` input. The result is rounded down and never below one
pixel. Varda reads the input's base value each frame (modulation does not reach it) and resizes
the pass's buffers when the size changes; history restarts that frame. A reduced pass that feeds
a full-size one must say how to upsample, for example a temporal pass that reads the reduced
image and accumulates it at full size, as `fractal_explorer.fs` does.

### `JITTER`: sub-pixel offsets

`JITTER` is an offset in pixels, in `[-0.5, 0.5)`, from the Halton (2, 3) sequence over 64 frames.
It changes once per rendered frame and is the same for every pass of that frame. Add it to the pixel
position before building a ray, and a `HISTORY` pass that accumulates the result gets
supersampled edges. `JITTERINDEX` is the position in the cycle.

### `SPECIALIZE`: inputs as compile-time constants

A `long`, `bool` or `float` input marked `"SPECIALIZE": true` also reaches the shader as a
specialization constant. The GPU compiler then treats it as a literal and drops the code it rules
out, which a uniform cannot do: a shader choosing between ten formulas with a uniform keeps all ten
live and pays for the choice on every call.

```json
{"NAME": "formula", "TYPE": "long", "DEFAULT": 1, "VALUES": [1, 2, 3], "SPECIALIZE": true}
```

```glsl
layout(constant_id = 0) const int SPEC_FORMULA = 1;

// Read the constant through a function. Varda's shader translator cannot
// handle an expression on a specialization constant, such as SPEC_FORMULA == 2,
// and a function call keeps it out of one. The compiler still folds it.
int specialized(int value) {
    return value;
}
#define FORMULA specialized(SPEC_FORMULA)
```

- Constants are numbered by the inputs' order among the specialized inputs: the first specialized
  input is `constant_id = 0`. A shader whose constants do not match fails to load.
- The value is the input's base value. A float rounds to the nearest integer. Modulation does not
  reach it.
- Changing the input builds a new pipeline, which takes a moment. Varda keeps the last few, so
  switching back is instant. Use it for structure (which formula, which mode), not for anything
  that animates.
- The input is still in `UserParams`, so the shader may read either.

`"SPECIALIZE_PASSES": true` at the top level does the same for the pass: each pass gets its own
pipeline, with `PASSINDEX` as the constant after the inputs' constants. Each pass then compiles
alone and runs with the registers it needs, instead of those of the largest pass in the file. In
`fractal_explorer.fs` that was worth 1 to 4 ms at 1080p.

### Texture limit

Every pass binds every pass buffer, plus imported and preprocessor textures. A shader that binds
more sampled textures than the GPU allows fails to load, and the error names both numbers.

**Reductions.** A fragment shader cannot reduce an image to one value in a single pass, and doing it
inline in the final pass repeats the whole scan for every output pixel. Use fixed-size passes as a
pyramid. `eyes_depth.fs` tallies the sensor image into a 32x32 buffer, reduces that to a 1x1 gaze
target, and reads one texel in the final pass: about 110k texture fetches per frame, against billions
for the inline version.

### Persistent pass substeps and pass-buffer sampling

**`PERSISTENT` passes in generators run four times per frame** (in effects, once). Varda substeps persistent passes for numerical stability: 4 iterations at `TIMEDELTA / 4`, with `FRAMEINDEX` advancing once per substep. Time-based simulations integrate correctly (4 × dt/4 == dt). Anything that steps once per invocation regardless of time (cellular automata, fixed-step reaction-diffusion, `FRAMEINDEX`-gated logic) advances **four generations per frame**.

Drive state changes from `TIMEDELTA`, or rate-limit against `FRAMEINDEX` explicitly, as `game_of_life.fs` does. A persistent multi-pass shader also costs roughly 4× its apparent GPU budget, which matters when you stack decks.

**`texSampler` filters linearly.** `rgba16float` pass buffers, imported images and `inputImage`
are all sampled with bilinear filtering. For an exact texel, use `texelFetch`, or sample at texel
centers:

```glsl
vec2 texel = 1.0 / RENDERSIZE;
vec2 snapped = (floor(uv * RENDERSIZE) + 0.5) * texel;
```

### Beauty pass plus cinematic post

A raymarched generator can include its own compositing chain instead of relying on downstream
effect decks, using two non-persistent passes:

```json
"PASSES": [
    { "TARGET": "sceneBuffer", "FLOAT": true },
    {}
]
```

Pass 0 marches the scene and writes **HDR color in rgb and normalized depth in alpha**. Pass 1 reads
that buffer and applies depth of field, threshold bloom, chromatic aberration, radial blur and the
grade. Before copying the pattern:

- **An intermediate pass's alpha is free to use.** A generator's final alpha is deck coverage and
  must be 1.0, but a pass buffer's alpha can hold anything. Packing depth there lets a single-buffer
  post pass do focus and haze without a second target. Normalize depth by a constant the shader also
  uses to convert a world-space focus parameter, so the two agree.
- **Pass buffers default to `Rgba16Float`**, with or without `FLOAT: true`. That covers HDR emission,
  so bloom can threshold above 1.0. Depth that later passes rebuild positions from belongs in a
  32-bit `FORMAT` target.
- **Keep the post pass unclamped and linear.** Varda tonemaps the composite downstream, and a grade
  that clamps to 1.0 removes the highlight headroom the tonemap uses. Put saturation and contrast in
  the shader; leave the display transform to Varda.
- **Sampling is bilinear** (see above), so a post tap between texels blends its neighbors. Divide uv
  offsets by the aspect ratio, or radial effects come out elliptical.


### Raymarch and post traps

None of these produce an error:

- **The hit threshold must exceed any level-of-detail floor the map puts under `d`.** If the map
  ends with `d = max(d, g_pix * 0.2)` and the loop tests `d < detail * exp(k * t)`, then once the
  pixel footprint outgrows `detail` no ray can register a hit. It creeps along the surface at the
  floor value until the step budget runs out and is reported as a miss. A fractal estimator hides
  this because it sometimes overshoots to a negative distance. An exact analytic surface converges
  to zero from above and never does, so the problem appears only when you add designed geometry.
  Derive the threshold from the same footprint: `max(detail * exp(k * t), g_pix * 0.35)`.
- **`calcNormal` and `softShadow` re-enter the map and overwrite its output globals.** Capture the
  orbit trap and the material id immediately after the march, before taking a normal. If you read
  them afterwards, every surface is shaded with the values from the last normal probe, which shows
  up as flat facets that slide as the camera moves.
- **Escape iteration count is nearly constant on the surface you are shading**, so a palette keyed
  on it renders flat. A point on the boundary is one whose orbit does not escape, so the count sits
  at the iteration cap across almost the whole visible surface. Key the palette off `log2(dr)`
  instead, which measures how much the map magnified the neighborhood and varies point to point.
  Orbit traps are the other option, but they fail on a stack of conformal folds: dividing the trap
  minimum by a derivative that grows like scale-to-the-iteration drives it to zero.
- **Normalize the depth channel to the depth range the subject occupies, not to the march limit.**
  Dividing by a large `MAX_DIST` puts every surface in the bottom fifth of the channel. A
  depth-of-field pass reading it then has almost no range to separate anything, and DoF appears to
  do nothing at any aperture setting.
- **For a moving camera, increase blur steadily with depth instead of modeling a focus plane.** A
  focus plane blurs in both directions (everything nearer blurs too) and has to be placed. With a
  moving camera, a constant focus distance drifts off the subject within seconds, and autofocusing on
  frame center causes pumping. Sampling more central pixels does not help, because the center of
  frame cannot detect something close in a corner. Ramp the blur with distance (near always crisp,
  far always soft). There is no plane to place, drift, or pump, and the pass still hides aliasing and
  jitter on fine distant geometry.
- **With blur that increases with depth, one comparison guards a gather.** A tap should contribute
  only if it is at least as far away as the pixel gathering it; otherwise crisp near geometry smears
  outward and halos over what is behind it. With a monotonic ramp, "is behind" and "is at least as
  soft" are the same test, so `step(depth - eps, tapDepth)` is the whole guard.
- **`pow(col, 1.3)` darkens; it is not a contrast control.** It keeps 1.0 fixed and pulls everything
  below it down, which can reduce an authored atmosphere value to a thousandth of itself and put pure
  black in frame. Apply contrast around a mid-grey pivot: `P * pow(col / P, g)` with `P` around 0.18.

### Previewing while you author

`examples/shader_preview.rs` renders a shader headless and writes a PNG. It is the fastest way to
iterate on a generator without launching the app:

```sh
LIBRARY_PATH="/opt/homebrew/lib:${LIBRARY_PATH:-}" cargo run --release --example shader_preview -- \
    shaders/alien_grove.fs /tmp/frame.png --size 960x540 --frame 300
```

It steps a fixed 60 fps clock up to `--frame`, so phase accumulators integrate as they would live
and a given frame index is reproducible between runs. `--set NAME=VALUE` overrides any float, bool or
long input. The frame is taken from the mixer composite, so it has gone through the real compositing
and tonemap path.

| Option | Does |
|---|---|
| `--set NAME=VALUE` | Override an input, clamped to its `MIN`/`MAX` with a warning |
| `--warmup N`, `--settle MS` | Extra frames at time 0, each followed by a pause, so background preprocessors can publish first |
| `--time N` | After the capture, render N more frames and print ms/frame for the render loop alone |
| `--probe` | Per-channel mean and percentiles of the linear values, read before the PNG's sRGB encode |
| `--pair` | Also write frame N-1 (from the same process) and print their difference at 1x, 4x, 8x and 16x downsampling |
| `--sweep`, `--sweep2`, `--grid` | A contact sheet with one or two inputs walked across it |

Every capture prints the frame's mean Laplacian. Near zero means a flat frame. A high value means
detail or noise; compare the `--pair` rows to tell which.

## Compute Shaders

Varda also supports **GLSL 450 compute shaders** for work that does not fit one invocation per output pixel: particle systems, N-body simulations, cellular automata, and other GPU-native generators. Compute shaders use the **same language and compilation pipeline** as fragment shaders, with an ISF-style JSON header for metadata.

Compute shaders are **generators**. Each one renders into its own output image, which becomes the deck's source. A compute shader cannot be an effect and does not receive an upstream input texture. To process an incoming frame, use a fragment-shader filter (see [Shader Types](#shader-types)).

### Anatomy of a Compute Shader

A compute shader uses the `.comp` extension and requires `"TYPE": "compute"` plus a `"COMPUTE"` block in the header. Three things must line up:

1. The JSON `"COMPUTE".WORKGROUP_SIZE` must equal the GLSL `layout(local_size_*)` declaration.
2. The output is **always** a write-only `rgba16f` storage image at **`binding = 2`**.
3. Every `INPUTS` entry maps, in order, into the `UserParams` uniform block at `binding = 1`.

### Compute Metadata Fields

Standard ISF fields (`DESCRIPTION`, `CREDIT`, `CATEGORIES`, `INPUTS`, `PHASE_INPUTS`, `IMPORTED`, `PREPROCESSORS`) work the same way. Compute adds:

| Field | Required | Description |
|-------|----------|-------------|
| `"TYPE": "compute"` | Yes | Distinguishes compute from fragment shaders |
| `"COMPUTE".WORKGROUP_SIZE` | Yes | `[x, y, z]`; must match the GLSL `layout(local_size_*)` declaration |
| `"COMPUTE".DISPATCH` | Yes | Only `"resolution"` is implemented (workgroup count derived from the output size). `"custom"` is reserved and currently does nothing; do not use it. |
| `"COMPUTE".NUM_PASSES` | No | Number of sequential dispatches per frame (default `1`). See [Multi-Pass Compute](#multi-pass-compute). |
| `"BUFFERS"` | No | Typed storage buffers (SSBOs). See [Storage Buffers](#storage-buffers). |

### Binding Layout

Compute bindings are fixed and assigned in this order:

| Binding | Resource | Notes |
|---------|----------|-------|
| `set=0, binding=0` | `ISFUniforms` | Same fields as fragment shaders (`TIME`, `RENDERSIZE`, audio, `PHASE_TIME_*`, etc.) |
| `set=0, binding=1` | `UserParams` | Your `INPUTS`, packed in declaration order |
| `set=0, binding=2` | Output image | `rgba16f`, `writeonly`; the deck displays this |
| `set=0, binding=3 …` | Storage buffers | One per `BUFFERS` entry, in declaration order |

The output format is fixed at `rgba16f`. Declare it exactly as `rgba16f` in the layout qualifier and write with `imageStore`.

> **Changed in 0.1.12.** The output format changed from `rgba8` to `rgba16f`, because the color
> path composites in linear-light `Rgba16Float` (see
> [Core Concepts → Signal Flow](02-concepts.md)). **Existing `.comp` shaders need one edit:** change
> `rgba8` to `rgba16f` in the `binding = 2` layout qualifier. `imageStore` values above 1.0 are not
> clamped, so additive and accumulation sims keep their headroom and roll off through the tonemap.

### Dispatch Model

In `"resolution"` mode the engine launches `ceil(RENDERSIZE / WORKGROUP_SIZE)` workgroups in X and Y (Z is always `1`):

```
dispatch_x = ceil(width  / local_size_x)
dispatch_y = ceil(height / local_size_y)
dispatch_z = 1
```

The count is rounded **up**, so the last row and column of workgroups extend past the image. **Every kernel must bounds-check** its invocation and return early, or it will write out of range. A per-pixel generator checks against `RENDERSIZE`; a buffer sim checks against the element count (below).

### Worked Example 1: Per-Pixel Generator

The smallest useful compute generator: one invocation per output pixel, no storage buffers. This is `shaders/compute_gradient.comp` in full.

```glsl
/*{
    "DESCRIPTION": "Simple animated gradient (compute shader)",
    "CREDIT": "Varda VJ",
    "ISFVSN": "2.0",
    "CATEGORIES": ["Generator"],
    "TYPE": "compute",
    "COMPUTE": {
        "WORKGROUP_SIZE": [16, 16, 1],
        "DISPATCH": "resolution"
    },
    "INPUTS": [
        {"NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 5.0, "LABEL": "Speed"}
    ]
}*/

#version 450

layout(local_size_x = 16, local_size_y = 16, local_size_z = 1) in;

// Binding 0: ISF automatic uniforms (identical field order to fragment shaders).
layout(set = 0, binding = 0) uniform ISFUniforms {
    float TIME;
    float TIMEDELTA;
    uint  FRAMEINDEX;
    int   PASSINDEX;
    vec2  RENDERSIZE;
    float audio_level;
    float audio_bass;
    float audio_mid;
    float audio_treble;
    float audio_bpm;
    float audio_beat_phase;
    vec4  DATE;
    float PHASE_TIME_0;
    float PHASE_TIME_1;
    float PHASE_TIME_2;
    float PHASE_TIME_3;
};

// Binding 1: your INPUTS, in declaration order.
layout(set = 0, binding = 1) uniform UserParams {
    float speed;
};

// Binding 2: the output image (always rgba16f, writeonly).
layout(set = 0, binding = 2, rgba16f) uniform writeonly image2D outputImage;

void main() {
    ivec2 pixel = ivec2(gl_GlobalInvocationID.xy);
    ivec2 size  = ivec2(RENDERSIZE);

    // Mandatory bounds guard — the last workgroup overruns the image.
    if (pixel.x >= size.x || pixel.y >= size.y) {
        return;
    }

    vec2 uv = vec2(pixel) / vec2(size);
    float t = TIME * speed * 0.2;

    float r = 0.5 + 0.5 * sin(uv.x * 3.14159 + t);
    float g = 0.5 + 0.5 * sin(uv.y * 3.14159 + t * 1.3);
    float b = 0.5 + 0.5 * sin((uv.x + uv.y) * 3.14159 + t * 0.7);

    imageStore(outputImage, pixel, vec4(r, g, b, 1.0));
}
```

Copy the `ISFUniforms` block exactly into every compute shader. The field order is part of the ABI.

### Storage Buffers

Storage buffers (SSBOs) give compute shaders **writable memory that persists across frames**, which fragment shaders do not have. Simulations depend on it.

```json
"BUFFERS": [
    { "NAME": "particles", "TYPE": "storage", "STRUCT": "Particle", "COUNT": 65536, "STRIDE": 32, "PERSISTENT": true }
]
```

| Field | Description |
|-------|-------------|
| `NAME` | Label used for the GPU allocation (not referenced from GLSL; see below) |
| `TYPE` | `"storage"` (read-write) or `"read-only-storage"` |
| `STRUCT` | Documentation only; names the conceptual element type. The engine does **not** parse it. |
| `COUNT` | Number of elements |
| `STRIDE` | Bytes per element |
| `PERSISTENT` | `true` keeps contents across frames; `false` is zeroed before pass 0 every frame |

**Sizing.** The engine allocates exactly `COUNT × STRIDE` bytes and zero-fills it once at creation. It does not inspect your GLSL struct; `STRUCT` and `STRIDE` only size the allocation. In GLSL, declare a struct array or, like the bundled simulations, a flat `vec4[]`, and make the total match. The example above reserves `65536 × 32 = 2 MiB`: two `vec4`s (32 bytes) per particle.

**GLSL declaration.** Always use `std430` layout, at the next binding after the output image:

```glsl
// First BUFFERS entry → binding 3. 32-byte stride = 2 vec4 per particle.
layout(std430, set = 0, binding = 3) buffer ParticleBuffer {
    vec4 particle_data[];   // [2*i] = position/extra, [2*i+1] = velocity/extra
};
```

`std430` is tightly packed, but a `vec3` still takes 16 bytes. Pack data as `vec4` to keep `STRIDE` predictable.

**Lifecycle.** A `PERSISTENT: true` buffer keeps state from frame to frame. Use it for particle positions, Game-of-Life grids, or feedback. A `PERSISTENT: false` buffer is cleared to zero before pass 0 each frame. Use it for per-frame scratch space such as a spatial binning grid.

### Worked Example 2: Buffer-Backed Simulation

A simulation updates *N* elements instead of *W×H* pixels, but dispatch is still resolution-based. The pattern, taken from `shaders/black_hole_sim.comp`, is to **convert the 2D dispatch grid into a 1D element index** and check it against the element count. Choose a render resolution where `width × height ≥ COUNT`, or some elements never get a thread.

```glsl
#version 450

layout(local_size_x = 256, local_size_y = 1, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform ISFUniforms { /* ...full block as in Example 1... */ };
layout(set = 0, binding = 1) uniform UserParams { float gravity; };
layout(set = 0, binding = 2, rgba16f) uniform writeonly image2D outputImage;

// Persistent particle state: 2 vec4 per particle (pos.xyz + vel.xyz).
layout(std430, set = 0, binding = 3) buffer ParticleBuffer {
    vec4 particle_data[];
};

const uint NUM_PARTICLES = 65536u;

void main() {
    // Linearize the (possibly oversized) 2D dispatch grid into a 1D index.
    uint row_width = gl_NumWorkGroups.x * 256u;          // 256 == local_size_x
    uint idx = gl_GlobalInvocationID.y * row_width + gl_GlobalInvocationID.x;
    if (idx >= NUM_PARTICLES) return;                    // mandatory guard

    // Initialize on the first frame, otherwise integrate.
    if (FRAMEINDEX == 0u) {
        particle_data[2u * idx]      = vec4(/* spawn position */ vec3(0.0), 0.0);
        particle_data[2u * idx + 1u] = vec4(/* initial velocity */ vec3(0.0), 0.0);
        return;
    }

    vec3 pos = particle_data[2u * idx].xyz;
    vec3 vel = particle_data[2u * idx + 1u].xyz;

    vel += vec3(0.0, -gravity, 0.0) * TIMEDELTA;          // step the sim
    pos += vel * TIMEDELTA;

    particle_data[2u * idx]      = vec4(pos, 0.0);         // write back (persists)
    particle_data[2u * idx + 1u] = vec4(vel, 0.0);
}
```

The required lines are the `idx` computation and the `if (idx >= NUM_PARTICLES) return;` guard. The rest is your simulation. To turn particle state into pixels, add a second pass that reads this buffer and writes `outputImage` (next section).

### Multi-Pass Compute

Set `"COMPUTE".NUM_PASSES` to run several dispatches per frame. The engine runs them **in sequence**: each pass completes on the GPU before the next begins. The `PASSINDEX` uniform holds the current pass. Non-persistent buffers are zeroed once, before pass 0; persistent buffers carry through every pass.

```glsl
void main() {
    if (PASSINDEX == 0) {
        simulate();   // update persistent particle buffer, bin into a scratch grid
    } else {
        render();     // read buffers, imageStore() into outputImage
    }
}
```

`black_hole_sim.comp` uses this split: pass 0 advances 65536 persistent particles and bins them into a non-persistent screen grid; pass 1 reads both and ray-traces the final image.

### Limitations

- **Generators only.** There is no compute effect (input-texture) path. Use a fragment filter to process upstream frames.
- **Generators write float.** Output is `rgba16f`; values above 1.0 reach the compositor and the tonemap. There is no clamping at the deck boundary.
- **`DISPATCH: "custom"` is not implemented.** Only `"resolution"` works.

### See Also

Two reference compute shaders ship with Varda:

- `shaders/black_hole_sim.comp`: a **stateful N-body** simulation. It uses a `PERSISTENT: true` particle buffer integrated with leapfrog each frame, a non-persistent scratch grid for atomic spatial binning, a two-pass simulate/render split, `PHASE_INPUTS`, and audio reactivity. It uses every feature in this section.
- `shaders/cosmic_web.comp`: a **stateless, analytic** simulation of a dark-matter cosmic web based on the *Zel'dovich approximation*. Pass 0 builds a Gaussian displacement field as plane-wave modes drawn from a CDM (BBKS) power spectrum. Pass 1 displaces a grid of Lagrangian particles (`x = q + D·Ψ(q)`) and deposits them into a fixed-resolution density buffer with cloud-in-cell. Pass 2 tone-maps that field into a void→filament→node colormap. Positions are recomputed each frame from a fixed seed with no persistent state, so it can be scrubbed, and the growth factor `D` animates the collapse of structure.

Read `black_hole_sim.comp` for persistence and binning. Read `cosmic_web.comp` for the multi-pass "generate → deposit → render" split and for keeping a sim deterministic and safe to scrub.

## Analyzer Preprocessors

Some effects need **structured data about the input frame** that plain GLSL cannot compute: face detection bounding boxes, depth maps, segmentation masks, optical flow fields. A **preprocessor** runs an analyzer and binds its output to your shader as an extra texture, which you read with ordinary texture samples.

> This section covers **authoring**: declaring preprocessors and reading their textures in GLSL. For the analyzer engine itself (how it runs, the two output paths, the full type catalog, the depth-sensor performer controls, and the HTTP API), see [Frame Analysis & Preprocessors](14-frame-analysis.md).

This is an advanced feature for shader authors building ML integrations, sensor-driven effects, or data processing pipelines.

### Declaring Preprocessors

Add a `PREPROCESSORS` array to your ISF JSON header:

```json
{
  "DESCRIPTION": "Surveillance overlay with face detection",
  "CATEGORIES": ["Filter", "Analysis"],
  "INPUTS": [
    {"NAME": "inputImage", "TYPE": "image"},
    {"NAME": "overlay_opacity", "TYPE": "float", "DEFAULT": 0.8, "MIN": 0.0, "MAX": 1.0}
  ],
  "PREPROCESSORS": [
    {"NAME": "landmarks", "TYPE": "face_detect"},
    {"NAME": "face_data", "TYPE": "face_detect"},
    {"NAME": "dossier_text", "TYPE": "face_detect"}
  ]
}
```

Each preprocessor entry declares:

| Key | Default | Meaning |
|---|---|---|
| `NAME` | required | The texture binding name your shader uses, and the output it receives |
| `TYPE` | required | Which preprocessor to run (e.g. `face_detect`, `depth_sensor`) |
| `OPTIONS` | `{}` | A JSON object passed to the preprocessor as configuration |
| `PARAM_BINDINGS` | `{}` | Preprocessor value name to one of your `INPUTS`. The preprocessor reads the live, modulated value every frame |
| `PHASE_BINDINGS` | `{}` | Preprocessor value name to a phase accumulator index (0 to 3) |
| `FORMAT` | `rgba8unorm` | Texture format of the output. `rgba32float` holds raw floats; read it with `texelFetch`, never `texture()` |

Several entries with the same `TYPE` share one running instance, one entry per output.

`"OPTIONS": {"bind_all_inputs": true}` binds every one of your `INPUTS` under its own name, for a
preprocessor that needs most of them.

### How It Works

1. Varda parses `PREPROCESSORS` from your shader's ISF header.
2. The engine starts the requested analyzers on dedicated background threads.
3. Analyzers receive downscaled input frames and produce data textures asynchronously.
4. Data textures are uploaded to the GPU and bound as `texture2D` samplers alongside your other inputs.
5. Your shader reads them with standard `texture()` calls.

Preprocessor textures are bound **after** imported textures and **before** user params. They never block the render loop. If analysis is slower than the frame rate, the shader uses the most recent result.

Some preprocessors are **host-inline**: instead of a background thread, they step once per rendered frame on the render thread, just before your shader, so their outputs always belong to the frame being drawn. They take no frame input, only bound parameters and time. A host-inline preprocessor can also keep state that is saved with the scene and with deck presets.

### Available Analyzer Types

You can request two analyzers as preprocessors:

| Type | Outputs | Description |
|------|---------|-------------|
| `face_detect` | `landmarks` (wireframe overlay), `face_data` (bbox/scores), `dossier_text` (character indices) | ONNX-based face detection with 478-point mesh landmarks |
| `depth_sensor` | `depth`, `mask`, `motion`, `rgb` | Live depth camera (Kinect v1). **Required** (see below) |

More analyzer types (`depth_estimate`, `segmentation`, `optical_flow`, `edge_detect`) are planned. See [Frame Analysis & Preprocessors](14-frame-analysis.md#whats-implemented) for the current implemented and planned list, and for the scalar outputs the same analyzers expose to modulation.

### `depth_sensor` (live depth camera)

`depth_sensor` reads a physical device instead of your deck's own frame. It runs entirely on the
GPU; the sensor's pixels never pass through host memory. Declare one entry per output you want:

```json
"PREPROCESSORS": [
  {"NAME": "depth",  "TYPE": "depth_sensor"},
  {"NAME": "mask",   "TYPE": "depth_sensor"},
  {"NAME": "motion", "TYPE": "depth_sensor"},
  {"NAME": "rgb",    "TYPE": "depth_sensor", "OPTIONS": {"device": 0}}
]
```

All four are at the sensor's native resolution (640×480 on Kinect v1) and are filterable. Sample
them with normalized UVs:

| `NAME` | Format | Contents |
|---|---|---|
| `depth` | `R16Float` | Distance normalized to `0..1` across the deck's near/far range. **`0.0` means invalid**: out of range, or a hole the sensor could not resolve. Hole-filled and temporally smoothed |
| `mask` | `R8Unorm` | Feathered silhouette occupancy: `1.0` on a subject, `0.0` on background |
| `motion` | `RG16Float` | Approximate screen-space velocity of the depth surface, signed, in UV units per second. Use it to react to movement instead of presence |
| `rgb` | color path | The sensor's color stream. Only approximately aligned with `depth`, because the IR and color cameras are physically offset |

`OPTIONS: {"device": N}` selects a specific sensor. Omit it to use the first one detected.

**This preprocessor is required.** A shader that declares `depth_sensor` **refuses to load** if no
depth sensor is attached, and an error toast names the shader. Every other preprocessor falls back
to a black texture instead. The `depth` feature is compiled out on Windows and macOS Intel, so these
shaders never load there.

Runtime framing (near/far clip, smoothing, hole fill, mask feather, motion gain, and mirror) is set
per deck in the bottom bar and can be mapped over MIDI/OSC at `deck/<uuid>/depth_prepro/<param>`. See
[Frame Analysis → Depth Sensor](14-frame-analysis.md#depth-sensor-performers) for the full control
reference and guidance on framing performers.

`shaders/liquid_light_depth.fs` is a worked example: an advected fluid whose flow is driven by
`mask` gradients and `motion`, rendering performers as flowing dye outlines.

### Shader Access

Read preprocessor textures like any other texture. They appear after imported textures in the standard binding layout:

```glsl
layout(set = 0, binding = N) uniform texture2D landmarks;    // wireframe overlay
layout(set = 0, binding = N+1) uniform texture2D face_data;  // packed bbox/score data
layout(set = 0, binding = N+2) uniform texture2D dossier_text; // character indices

void main() {
    // Read face bounding box from data texture
    vec4 bbox = texelFetch(sampler2D(face_data, texSampler), ivec2(0, 0), 0);
    float x = bbox.r;  // normalized x position
    float y = bbox.g;  // normalized y position
    float w = bbox.b;  // normalized width
    float h = bbox.a;  // normalized height
    // ...
}
```

### Lifecycle

- Analyzers start automatically when a shader that declares them is loaded onto a deck.
- Shaders requesting the same analyzer type share one instance (refcounted).
- When the last shader using an analyzer is removed, the analyzer stops and frees its resources.
- If an analyzer fails to initialize (missing model file, unsupported platform), the shader still loads and its preprocessor textures fall back to 1×1 black. The exception is `depth_sensor`, which is required: if the device cannot be acquired, the shader does not load.

## Hot-Reload

Varda watches shaders in the `shaders/` directory. When you save a `.fs` file, Varda:

1. Detects the file change
2. Recompiles GLSL → SPIR-V
3. On success: replaces the running shader and resets parameters to defaults
4. On error: keeps the old shader running and shows an error notification

You do not need to restart. Edit shaders in any external editor and see the result immediately.

## File Location

Varda loads shaders from a fixed hierarchy, lowest to highest precedence:

1. Bundled shaders (shipped inside the `.app`, the AppImage, the Flatpak or the Windows ZIP)
2. `./shaders/` in the working directory
3. The workspace `.varda/shaders/`
4. The platform user shader dir (`~/.local/share/varda/shaders`, `~/Library/Application Support/Varda/Shaders`, `%APPDATA%\Varda\Shaders`)
5. Any `--shader-dir <DIR>` flags (repeatable), in the order given

On a name collision the higher-precedence directory wins, so a `--shader-dir` shader overrides a built-in of the same name. The order holds for the whole session: shaders hot-reload as you edit them, and deleting an override restores the built-in it replaced. A `--shader-dir` that does not exist is skipped with a warning; Varda does not create it.

Varda discovers shaders at startup from every directory in the hierarchy. They appear in the **Library** panel under Generators, Effects, or Transitions based on their type.

---

[← Prev: Shader Library](11-shader-library.md) · [Home](README.md) · [Next: HTTP API & Headless Mode →](13-api.md)
