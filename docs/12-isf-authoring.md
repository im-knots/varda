# Shader Authoring

Varda shaders are **GLSL 450 (Vulkan)** with an [ISF](https://isf.video)-style JSON metadata header that declares parameters, inputs, and passes.

> **Read this first if you have existing ISF shaders.** Varda uses ISF's *metadata format*, not its *shader language*. A shader downloaded from isf.video, VDMX, or the ISF Editor will **not** load as-is. OG ISF is GLSL ES with implicitly injected uniforms, and Varda needs explicit Vulkan declarations. Porting is mechanical and usually takes a few minutes. See [Porting an ISF Shader](#porting-an-isf-shader).

In exchange for not being drop-in ISF, the dialect gets you things ISF can't express: [compute shaders](#compute-shaders) with persistent storage buffers, [analyzer preprocessors](#analyzer-preprocessors) that inject ML/sensor data as textures, and [phase accumulators](#phase-accumulators) for jump-free speed changes.

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
| `point2D` | `vec2` | DEFAULT [x,y] | 2D position picker (0.0–1.0) |
| `image` | texture2D | — | Input texture (filters and transitions) |

All numeric parameters (float, color components, point2D axes) are MIDI/OSC-mappable and modulatable.

### Example: Float Parameter

```json
{ "NAME": "speed", "TYPE": "float", "DEFAULT": 1.0, "MIN": 0.0, "MAX": 10.0, "LABEL": "Speed" }
```

### Example: Enum Parameter

```json
{ "NAME": "mode", "TYPE": "long", "DEFAULT": 0, "VALUES": [0, 1, 2], "LABELS": ["Normal", "Mirror", "Tile"] }
```

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
};
```

### Phase Accumulators

`PHASE_TIME_0` through `PHASE_TIME_3` are smooth phase accumulators driven by user parameters. Unlike `TIME * speed` (which jumps when speed changes), phase accumulators integrate smoothly: `PHASE_TIME[i] += dt * param_value * scale`.

Declare them in the metadata:

```json
"PHASE_INPUTS": [
    { "PARAM": "rotation_speed", "INDEX": 0, "SCALE": 1.0 }
]
```

Then use in the shader: `float angle = PHASE_TIME_0 * 6.28318;` for smooth rotation that doesn't jump when the user adjusts speed.

The value integrated is the parameter's **modulated** value, its the same one the shader reads from the user-parameter buffer. Routing an audio band or LFO at `rotation_speed` therefore changes how fast the phase advances, and because integration is continuous the animation speeds up and slows down without ever jumping. A shader should read `PHASE_TIME_N` rather than the raw parameter for anything that advances over time; reading the raw parameter and multiplying by `TIME` reintroduces the jump.

#### Combining two rates

Multiplying an accumulator by a parameter like `PHASE_TIME_0 * rot_speed` reintroduces the same jump, because it scales an ever-growing phase by a live value. When a per-element rate should ride on top of a master speed, fold both into one accumulator with `MULTIPLY_BY`:

```json
"PHASE_INPUTS": [
    { "PARAM": "speed", "INDEX": 0, "SCALE": 1.0 },
    { "PARAM": "speed", "MULTIPLY_BY": "rot_speed", "INDEX": 1, "SCALE": 0.2 }
]
```

`PHASE_TIME_1` now accumulates `dt × speed × rot_speed × 0.2`, so the shader writes `float rotAngle = PHASE_TIME_1;` and both parameters stay smooth and modulatable. `MULTIPLY_BY` also takes an array when a rate depends on three parameters: `"MULTIPLY_BY": ["time_scale", "flow_speed"]`.

Two rules follow from this. Never multiply `PHASE_TIME_N` by a user parameter, and never apply a parameter that already drives an accumulator a second time in the shader body. Since the phase already contains it, applying it again makes the response quadratic in that parameter.

The first rule catches you out most often when you never wrote the multiply. Adding phase into a coordinate that something else scales later is the same thing:

```glsl
coord += PHASE_TIME_0;
float pattern = fract(coord * line_count);   // = fract(coord*n + PHASE_TIME_0*n)
```

`line_count` reads as purely spatial, but it multiplies the scroll phase, so nudging it slides the whole field. Scale the position only and add the phase afterwards, with the count inside the integral:

```glsl
float pattern = fract(coord * line_count + PHASE_TIME_1);   // MULTIPLY_BY: line_count
```

The lines then travel at the same screen speed however many of them there are, which is what the original multiply gave you, without the jump. `bars.fs`, `lines.fs` and `scanlines.fs` all shipped the broken form.

`tests/shader_param_contract_guard.rs` fails the build on all of these. It walks the whole multiplicative chain, so a parameter hiding behind a constant (`PHASE_TIME_0 * 0.5 * look_speed`) is caught, and it follows local aliases within a function, so `float t = PHASE_TIME_0;` buys you nothing. It stops at function calls, because `sin(PHASE_TIME_0) * amount` is legitimate. What it cannot see is a phase passed into a function as an argument and scaled in the callee.

#### Rates that are affine, not products

`MULTIPLY_BY` covers `speed × amount`. It does not cover `speed × (1 + k · amount)`, the shape you want when `amount` at zero should still leave the base motion running since the product form would stop the animation dead there.

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

That is exact, and continuous in both parameters. `big_bang.fs` does this.

A factor that varies across the image but not over time such as a per-cell hash stays *outside* the integral, because only the parameter needs to be inside it. `char_cycle.fs` gives every cell its own rate with `PHASE_TIME_0 + h * PHASE_TIME_1`.

The cost is one slot per affine term, and there are only four.

#### Bounding an accumulator

An accumulator grows without limit, which is correct for anything that should cycle forever such as a hue, a scroll offset, or an angle that wraps. It is wrong for anything that must stay within a range. Feeding an unbounded phase straight into a camera angle is how `dull_skull` used to orbit off behind its own backdrop and render black for a third of every cycle.

Wrap the phase in a periodic function and scale *that* by the amplitude parameter:

```glsl
float swayAngle = sin(PHASE_TIME_1) * sway_range;
```

This is not the forbidden `PHASE_TIME_N * param`: the sine is already bounded, so `sway_range` scales a value in [-1, 1] rather than one that grows forever. `sway_range` is an amplitude, so it stays a plain uniform.

#### Prefer a rate to an amplitude for anything that will be automated

Passing the guard is not the same as feeling right under an LFO. An amplitude parameter sets *where* something is; automating it moves that thing out and back at the LFO's rate, which reads as sloshing or stutter. A rate parameter can only make motion faster or slower, so no automation of it now matter how fast, or however often reversed, can relocate anything.

`liquid_light.fs`'s Agitation was built both ways. As an amplitude on the domain-warp gain it was continuous and passed every guard, but a 1 Hz triangle LFO drove per-frame change to 13.5× the parked-fader baseline. Rebuilt as a mixing *rate* on accumulator slot 1 — advancing the inner warp stages against the outer one, so the fine structure keeps reorganising — the same LFO measures 0.97×, indistinguishable from leaving the fader alone, while still spanning a 10.5× range in mixing speed.

So when a control needs more authority, reach for another rate before an amplitude. The question to ask is whether the parameter names a speed or a position; only the first survives being automated.

#### What does not belong in `PHASE_INPUTS`

Only parameters that express a *rate*. A parameter setting a static angle, scale, threshold, or count must stay a plain uniform; integrating it would ramp it to its limit and hold there. Shaders that step a simulation into a persistent buffer are already continuous by construction and need no accumulator for their step-rate coefficients.

## Binding Layout

| Binding | Content |
|---------|---------|
| `set=0, binding=0` | ISFUniforms (all shaders) |
| `set=0, binding=1` | Sampler (if shader has textures) |
| `set=0, binding=2+` | Textures (inputImage, pass buffers, imported images) |
| Last binding | UserParams (if shader has parameters) |

Fragment input: `layout(location = 0) in vec2 uv;` — normalized coordinates (0.0–1.0).

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

Varda's JSON header is ISF-compatible, so the metadata usually needs no changes at all. The work is in the GLSL body.

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
| Output range | Effectively `[0,1]` — clamped at an 8-bit target | **Unbounded** — linear-light float all the way to the tonemap |

### Don't clamp your output

Varda composites in linear-light float from the deck stage onward, so values above
1.0 are meaningful and survive to the tonemap, which rolls them off (ACES by
default). A terminal

```glsl
col = clamp(col, 0.0, 1.0);   // ← don't
```

throws that away. It flattens emissive highlights in a generator, and in a
**filter** it is worse: it destroys headroom produced by the deck upstream, so one
clamping filter anywhere in a chain acts as an HDR limiter for everything before
it.

If you need to keep negatives out of the blend math — worth doing, since negative
light is not meaningful and some blend modes will propagate it — floor without
capping:

```glsl
col = max(col, 0.0);          // ← floor only, no ceiling
```

Alpha is the exception: it is coverage, not light, and belongs in `[0, 1]`.

Two related traps when porting:

- **Don't apply your own gamma.** `col = sqrt(col)` or `pow(col, 1.0/2.2)` at the
  end of a Shadertoy port is display encoding, which Varda does at the output
  boundary. Doing it in the shader double-encodes. A few bundled shaders still do
  this and are flagged for review.
- **`IMPORTED` textures are sRGB-tagged**, so sampling them already decodes to
  linear. If a port does `pow(tex, 1.0/2.2)` on an imported atlas it is
  compensating for that decode deliberately — leave it alone.

### Steps

1. **Keep the JSON header.** `DESCRIPTION`, `CREDIT`, `CATEGORIES`, `INPUTS`, `PASSES`, `IMPORTED` all parse as-is.
2. **Add `#version 450`** as the first line after the header, and delete any existing `#version`.
3. **Add the standard prologue** — `in vec2 uv`, `out vec4 fragColor`, the `ISFUniforms` block, the sampler, your textures, and a `UserParams` block listing every non-image `INPUTS` entry **in declaration order**. Copy the layout from the [Filter example](#filter) above.
4. **Delete any `varying` declarations.** Not valid in GLSL 450 core.
5. **Replace `gl_FragColor`** with `fragColor`, and drop any terminal
   `clamp(col, 0.0, 1.0)` — see [Don't clamp your output](#dont-clamp-your-output).
6. **Rewrite sampling calls:**
   ```glsl
   texture2D(inputImage, c)      →  texture(sampler2D(inputImage, texSampler), c)
   IMG_NORM_PIXEL(inputImage, c) →  texture(sampler2D(inputImage, texSampler), c)
   IMG_THIS_PIXEL(inputImage)    →  texture(sampler2D(inputImage, texSampler), uv)
   IMG_PIXEL(inputImage, px)     →  texture(sampler2D(inputImage, texSampler), px / RENDERSIZE)
   IMG_SIZE(inputImage)          →  vec2(textureSize(sampler2D(inputImage, texSampler), 0))
   ```
7. **Fix the vertical orientation** — see below. This is the step people miss.

### The vertical flip

**ISF's `isf_FragNormCoord` has `(0,0)` at the bottom-left. Varda's `uv` has `(0,0)` at the top-left.** Substituting one for the other renders the shader upside down.

If the shader is vertically symmetric you won't notice — until you use it on something that isn't, like text or a logo. Check with an asymmetric source before you trust it.

To port ISF coordinate math unchanged, establish a flipped coordinate once at the top of `main` and use it everywhere ISF used `isf_FragNormCoord`:

```glsl
void main() {
    vec2 p = vec2(uv.x, 1.0 - uv.y);   // ISF/GL orientation
    // ... original ISF body, using p wherever it used isf_FragNormCoord
}
```

**Do not use the flipped coordinate for texture sampling.** Varda's textures are stored top-left, so `inputImage` and pass buffers are sampled with raw `uv`. Mixing the two is what produces a shader that generates correctly but samples mirrored, or vice versa.

`gl_FragCoord` needs the same treatment — it is upper-left origin in Vulkan and lower-left in OpenGL:

```glsl
vec2 fc = vec2(gl_FragCoord.x, RENDERSIZE.y - gl_FragCoord.y);
```

Around a dozen shaders in `shaders/` are ports that do exactly this — `star_nest.fs`, `apollonian_glow.fs`, `truchet_tube.fs`, and `mandelbrot_deco.fs` are good references.

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
| Filters with two or more `image` inputs | Not supported — one input image per effect. To blend two sources, use two decks in a channel with a [blend mode](04-performance.md) instead. |
| `audio` / `audioFFT` image inputs | Not bound. Use the `audio_*` scalars in `ISFUniforms` instead. |
| `.frag` / `.glsl` extensions | Only `.fs` and `.comp` are discovered |

## Multi-Pass Rendering

For feedback effects, simulations, and post-processing chains, declare multiple render passes:

```json
"PASSES": [
    { "TARGET": "feedbackBuffer", "PERSISTENT": true },
    {}
]
```

- Passes with a `TARGET` render to a named buffer (accessible as a texture in subsequent passes)
- **Persistent** buffers survive across frames — essential for feedback loops and simulations (Game of Life, reaction-diffusion)
- The final pass (empty `{}`) renders to the output
- Access pass buffers as `texture2D` samplers with the target name
- Optional `WIDTH`/`HEIGHT` expressions: `"$WIDTH/2"` for half-resolution buffers. Only `$WIDTH`, `$HEIGHT`, `$WIDTH/N` and `$WIDTH*N` with integer `N` are parsed — `$WIDTH/2.0` and arithmetic like `max($WIDTH,$HEIGHT)` are not, and fall back to full resolution. A bare integer literal (`"WIDTH": "32"`) sets a fixed size, which is how you build a reduction pyramid.

> **`RENDERSIZE` is the size of the pass you are currently rendering**, not the deck's. In a
> `"WIDTH": "1", "HEIGHT": "1"` pass, `RENDERSIZE` is `(1, 1)`. Anything that needs the deck's
> dimensions or aspect ratio — a letterbox fit, a screen-space offset — has to be computed in a
> full-size pass. `eyes_depth.fs` carries its gaze target in sensor space through a 1x1 pass and
> converts to deck space in the final pass for exactly this reason.

**Every pass buffer is double-buffered**, `PERSISTENT` or not. `PERSISTENT` controls whether the
contents mean anything across frames, not the buffering strategy — a pass reads the last value
written to its target and writes to the other texture. This exists because the bind group binds
*all* pass buffers as sampled textures on every pass, so a single-textured target would be a
colour attachment and a sampled resource at the same time, which wgpu rejects outright. Budget
two textures per declared pass when sizing large buffers.

**Reductions.** A fragment shader cannot reduce an image to one value in a single pass, and doing
it inline in the final pass repeats the whole scan for every output pixel. Use fixed-size passes
as a pyramid instead: `eyes_depth.fs` tallies the sensor image into a 32x32 buffer, reduces that
to a 1x1 gaze target, and reads one texel in the final pass — about 110k texture fetches per
frame, versus billions for the naive version.
- Optional `FLOAT: true` for 32-bit float buffers (HDR, simulation data)

### Two behaviours that will surprise you

**`PERSISTENT` passes run four times per frame.** Varda substeps persistent passes for numerical stability — 4 iterations at `TIMEDELTA / 4`, with `FRAMEINDEX` advancing once per substep. Time-based simulations integrate correctly (4 × dt/4 == dt), but anything that steps once per invocation regardless of time — cellular automata, fixed-step reaction-diffusion, `FRAMEINDEX`-gated logic — advances **four generations per frame**.

Design for it: drive state changes from `TIMEDELTA`, or rate-limit against `FRAMEINDEX` explicitly. `game_of_life.fs` does the latter. It also means a persistent multi-pass shader costs roughly 4× its apparent GPU budget, which matters when you are stacking decks.

**Any pass buffer forces nearest-neighbour filtering on every texture in the shader.** Float pass buffers aren't filterable in WebGPU, and the sampler is shared, so declaring even one `PASSES` target downgrades `inputImage` sampling from linear to nearest. If a filter looks unexpectedly blocky after you add a pass, this is why. Sample at texel centres to keep it predictable:

```glsl
vec2 texel = 1.0 / RENDERSIZE;
vec2 snapped = (floor(uv * RENDERSIZE) + 0.5) * texel;
```

## Compute Shaders

Beyond fragment shaders, Varda supports **GLSL 450 compute shaders** for work that doesn't fit the one-output-pixel-per-invocation model — particle systems, N-body simulations, cellular automata, and other GPU-native generators. Compute shaders use the **same language and compilation pipeline** as fragment shaders, with an ISF-style JSON header for metadata.

Compute shaders are **generators**: each one renders into its own output image that becomes the deck's source. There is no compute *effect* path — a compute shader does not receive an upstream input texture. If you need to process an incoming frame, use a fragment-shader filter (see [Shader Types](#shader-types)).

### Anatomy of a Compute Shader

A compute shader uses the `.comp` extension and requires `"TYPE": "compute"` plus a `"COMPUTE"` block in the header. Three things must line up:

1. The JSON `"COMPUTE".WORKGROUP_SIZE` must equal the GLSL `layout(local_size_*)` declaration.
2. The output is **always** a write-only `rgba16f` storage image at **`binding = 2`**.
3. Every `INPUTS` entry maps, in order, into the `UserParams` uniform block at `binding = 1`.

### Compute Metadata Fields

Standard ISF fields (`DESCRIPTION`, `CREDIT`, `CATEGORIES`, `INPUTS`, `PHASE_INPUTS`, `IMPORTED`, `PREPROCESSORS`) work identically. Compute adds:

| Field | Required | Description |
|-------|----------|-------------|
| `"TYPE": "compute"` | Yes | Distinguishes compute from fragment shaders |
| `"COMPUTE".WORKGROUP_SIZE` | Yes | `[x, y, z]` — must match the GLSL `layout(local_size_*)` declaration |
| `"COMPUTE".DISPATCH` | Yes | Only `"resolution"` is implemented (workgroup count derived from the output size). `"custom"` is reserved and currently behaves as a no-op — do not rely on it. |
| `"COMPUTE".NUM_PASSES` | No | Number of sequential dispatches per frame (default `1`). See [Multi-Pass Compute](#multi-pass-compute). |
| `"BUFFERS"` | No | Typed storage buffers (SSBOs). See [Storage Buffers](#storage-buffers). |

### Binding Layout

Compute bindings are fixed and assigned in this order:

| Binding | Resource | Notes |
|---------|----------|-------|
| `set=0, binding=0` | `ISFUniforms` | Same fields as fragment shaders (`TIME`, `RENDERSIZE`, audio, `PHASE_TIME_*`, etc.) |
| `set=0, binding=1` | `UserParams` | Your `INPUTS`, packed in declaration order |
| `set=0, binding=2` | Output image | `rgba16f`, `writeonly` — this is what the deck displays |
| `set=0, binding=3 …` | Storage buffers | One per `BUFFERS` entry, in declaration order |

The output format is hard-wired to `rgba16f`; declare it exactly as `rgba16f` in the layout qualifier and write with `imageStore`.

> **Changed in 0.1.12.** The output was previously `rgba8`. The whole color path now
> composites in linear-light `Rgba16Float` (see the manual's
> [Core Concepts → Signal Flow](02-concepts.md)), so compute output is float too.
> **Existing `.comp` shaders need one edit:** change `rgba8` to `rgba16f` in the
> `binding = 2` layout qualifier. Nothing else changes. The upside is that
> `imageStore` values above 1.0 are no longer clamped — additive and accumulation
> sims keep their headroom and roll off through the tonemap instead of clipping.

### Dispatch Model

In `"resolution"` mode the engine launches `ceil(RENDERSIZE / WORKGROUP_SIZE)` workgroups in X and Y (Z is always `1`):

```
dispatch_x = ceil(width  / local_size_x)
dispatch_y = ceil(height / local_size_y)
dispatch_z = 1
```

Because the count is rounded **up**, the last row/column of workgroups overruns the image. **Every kernel must bounds-check** its invocation against the work it's responsible for and early-out, or it will write out of range. For a per-pixel generator that means guarding against `RENDERSIZE`; for a buffer sim it means guarding against the element count (below).

### Worked Example 1 — Per-Pixel Generator

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

Copy the `ISFUniforms` block verbatim into every compute shader — the field order is part of the ABI.

### Storage Buffers

Storage buffers (SSBOs) give compute shaders something fragment shaders can't have: **mutable memory that persists across frames**. This is what makes simulations possible.

```json
"BUFFERS": [
    { "NAME": "particles", "TYPE": "storage", "STRUCT": "Particle", "COUNT": 65536, "STRIDE": 32, "PERSISTENT": true }
]
```

| Field | Description |
|-------|-------------|
| `NAME` | Label used for the GPU allocation (not referenced from GLSL — see below) |
| `TYPE` | `"storage"` (read-write) or `"read-only-storage"` |
| `STRUCT` | Documentation only — names the conceptual element type. The engine does **not** parse it. |
| `COUNT` | Number of elements |
| `STRIDE` | Bytes per element |
| `PERSISTENT` | `true` keeps contents across frames; `false` is zeroed before pass 0 every frame |

**Sizing.** The engine allocates exactly `COUNT × STRIDE` bytes and zero-fills it once at creation. It does *not* inspect your GLSL struct — `STRUCT` and `STRIDE` are purely for *you* to size the allocation. How you interpret those bytes in GLSL is up to you: declare a struct array or, as the bundled simulations do, a flat `vec4[]`. Just make the total match. The example above reserves `65536 × 32 = 2 MiB`, i.e. two `vec4`s (32 bytes) per particle.

**GLSL declaration.** Always `std430` layout, at the next binding after the output image:

```glsl
// First BUFFERS entry → binding 3. 32-byte stride = 2 vec4 per particle.
layout(std430, set = 0, binding = 3) buffer ParticleBuffer {
    vec4 particle_data[];   // [2*i] = position/extra, [2*i+1] = velocity/extra
};
```

Use `std430` (tightly packed) and watch the classic alignment trap: a `vec3` still consumes 16 bytes. Pack as `vec4` to keep `STRIDE` predictable.

**Lifecycle.** A `PERSISTENT: true` buffer accumulates state frame to frame — ideal for particle positions, Game-of-Life grids, or feedback. A `PERSISTENT: false` buffer is cleared to zero before pass 0 each frame — ideal for per-frame scratch space such as a spatial binning grid.

### Worked Example 2 — Buffer-Backed Simulation

A simulation updates *N* elements, not *W×H* pixels — but dispatch is still resolution-based. The idiom (taken from `shaders/black_hole_sim.comp`) is to **linearize the 2D dispatch grid into a 1D element index** and guard against the element count. Size your render resolution so that `width × height ≥ COUNT`, or some elements never get a thread.

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

The two load-bearing lines are the `idx` computation and the `if (idx >= NUM_PARTICLES) return;` guard — everything else is your simulation. To turn particle state into pixels, add a second pass that reads this buffer and writes `outputImage` (next section).

### Multi-Pass Compute

Set `"COMPUTE".NUM_PASSES` to run several dispatches per frame. The engine runs them **sequentially** — each pass completes on the GPU before the next begins — and exposes the current pass via the `PASSINDEX` uniform. Non-persistent buffers are zeroed once, before pass 0; persistent buffers carry through every pass.

```glsl
void main() {
    if (PASSINDEX == 0) {
        simulate();   // update persistent particle buffer, bin into a scratch grid
    } else {
        render();     // read buffers, imageStore() into outputImage
    }
}
```

This "simulate, then render" split is exactly how `black_hole_sim.comp` works: pass 0 advances 65536 persistent particles and bins them into a non-persistent screen grid; pass 1 reads both and ray-traces the final image.

### Limitations

- **Generators only** — no compute-effect (input-texture) path. Use a fragment filter to process upstream frames.
- **Generators write float** — output is `rgba16f`; values above 1.0 survive to the compositor and the tonemap. No clamping at the deck boundary.
- **`DISPATCH: "custom"` is not implemented** — only `"resolution"` works.

### See Also

Two reference compute shaders ship with Varda, each demonstrating a different idiom:

- `shaders/black_hole_sim.comp` — a **stateful N-body** simulation: a `PERSISTENT: true` particle buffer that leapfrog-integrates frame to frame, a non-persistent scratch grid for atomic spatial binning, two-pass simulate/render, `PHASE_INPUTS`, and audio reactivity. It puts every feature in this section to work at once.
- `shaders/cosmic_web.comp` — a **stateless, analytic** simulation: a scientifically grounded dark-matter cosmic web built from the *Zel'dovich approximation*. Pass 0 synthesises a Gaussian displacement field as plane-wave modes drawn from a CDM (BBKS) power spectrum; pass 1 displaces a grid of Lagrangian particles (`x = q + D·Ψ(q)`) and cloud-in-cell deposits them into a fixed-resolution density buffer; pass 2 tone-maps that field into a void→filament→node colormap. Because positions are recomputed each frame from a deterministic seed (no persistent state), it is fully scrubbable, and the growth factor `D` animates the collapse of structure.

Read `black_hole_sim.comp` for persistence and binning; read `cosmic_web.comp` for the multi-pass "generate → deposit → render" split and how to keep a sim deterministic and scrub-safe.

## Analyzer Preprocessors

Some effects need **structured data about the input frame** that plain GLSL can't compute — face detection bounding boxes, depth maps, segmentation masks, optical flow fields. A **preprocessor** runs an analyzer and injects its output into your shader as an additional texture binding, which you read with ordinary texture samples.

> This section covers the **authoring mechanics** — declaring preprocessors and reading their textures in GLSL. For the analyzer engine itself (how it runs, the two output paths, the full type catalogue, the depth-sensor performer controls, and the HTTP API), see [Frame Analysis & Preprocessors](14-frame-analysis.md).

This is an advanced feature for shader authors building ML integrations, sensor-driven effects, or rich data processing pipelines.

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
- **NAME**: the texture binding name your shader will use
- **TYPE**: which analyzer to run (e.g. `face_detect`, `depth_estimate`, `edge_detect`)
- **OPTIONS** (optional): JSON object passed to the analyzer for configuration (e.g. `{"resolution": "half"}`)

### How It Works

1. Varda parses `PREPROCESSORS` from your shader's ISF header
2. The engine starts the requested analyzer(s) on dedicated background threads
3. Analyzers receive downscaled input frames and produce data textures asynchronously
4. Data textures are uploaded to the GPU and bound as `texture2D` samplers alongside your other inputs
5. Your shader reads them with standard `texture()` calls

Preprocessor textures are bound **after** imported textures and **before** user params in the binding layout. They never block the render loop — if analysis is slower than the frame rate, the shader uses the most recent available result.

### Available Analyzer Types

The two analyzers you can request as preprocessors today:

| Type | Outputs | Description |
|------|---------|-------------|
| `face_detect` | `landmarks` (wireframe overlay), `face_data` (bbox/scores), `dossier_text` (character indices) | ONNX-based face detection with 478-point mesh landmarks |
| `depth_sensor` | `depth`, `mask`, `motion`, `rgb` | Live depth camera (Kinect v1). **Required** — see below |

Additional analyzer types (`depth_estimate`, `segmentation`, `optical_flow`, `edge_detect`) are planned. See [Frame Analysis & Preprocessors](14-frame-analysis.md#whats-implemented) for the authoritative implemented/planned list and the scalar outputs the same analyzers expose to modulation.

### `depth_sensor` — live depth camera

Unlike the analyzers above, `depth_sensor` reads a physical device rather than your deck's own
frame, and runs entirely on the GPU — the sensor's pixels never touch host memory. Declare one
entry per output you want:

```json
"PREPROCESSORS": [
  {"NAME": "depth",  "TYPE": "depth_sensor"},
  {"NAME": "mask",   "TYPE": "depth_sensor"},
  {"NAME": "motion", "TYPE": "depth_sensor"},
  {"NAME": "rgb",    "TYPE": "depth_sensor", "OPTIONS": {"device": 0}}
]
```

All four are at the sensor's native resolution (640×480 on Kinect v1) and are filterable, so
sample them with normalized UVs:

| `NAME` | Format | Contents |
|---|---|---|
| `depth` | `R16Float` | Distance normalized to `0..1` across the deck's near/far range. **`0.0` means invalid** — out of range, or a hole the sensor could not resolve. Hole-filled and temporally smoothed |
| `mask` | `R8Unorm` | Feathered silhouette occupancy: `1.0` on a subject, `0.0` on background |
| `motion` | `RG16Float` | Approximate screen-space velocity of the depth surface, signed, UV units per second. Use this to make things react to *movement* rather than mere presence |
| `rgb` | colour path | The sensor's colour stream. Only approximately aligned with `depth` — the IR and colour cameras are physically offset |

`OPTIONS: {"device": N}` pins a specific sensor; omit it to take the first one detected.

**This preprocessor is required.** Unlike every other preprocessor, a shader declaring
`depth_sensor` will **refuse to load** if no depth sensor is attached, with an error toast naming
the shader. A black fallback texture is a sensible answer for "depth estimation is unavailable";
it is a useless one for a shader whose entire content is a silhouette. Note the `depth` feature
is compiled out on Windows and macOS Intel, so these shaders never load there.

Runtime framing — near/far clip, smoothing, hole fill, mask feather, motion gain, and mirror — is
set per deck in the bottom bar and is MIDI/OSC-mappable at `deck/<uuid>/depth_prepro/<param>`. See
[Frame Analysis → Depth Sensor](14-frame-analysis.md#depth-sensor-performers) for the full control
reference and performer framing guidance.

See `shaders/liquid_light_depth.fs` for a worked example: an advected fluid whose flow is driven
by `mask` gradients and `motion`, rendering performers as flowing dye outlines.

### Shader Access

Preprocessor textures are accessed like any other texture. Bindings follow the standard layout — preprocessor textures appear after imported textures:

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

- Analyzers start automatically when a shader declaring them is loaded onto a deck
- Multiple shaders requesting the same analyzer type share a single instance (refcounted)
- When the last shader using an analyzer is removed, the analyzer stops and frees resources
- If an analyzer fails to initialize (missing model file, unsupported platform), the shader still loads — preprocessor textures fall back to 1×1 black. The exception is `depth_sensor`, which is *required*: if the device cannot be acquired the shader does not load at all

## Hot-Reload

Shaders in the `shaders/` directory are watched for changes. Save a `.fs` file and Varda:

1. Detects the file change
2. Recompiles GLSL → SPIR-V
3. On success: replaces the running shader, resets parameters to defaults
4. On error: keeps the old shader running, shows an error notification

No restart required. Edit shaders in any external editor and see results immediately.

## File Location

Varda loads shaders from a fixed hierarchy, lowest to highest precedence:

1. Bundled shaders (shipped inside the `.app` / AppImage / tarball)
2. `./shaders/` in the working directory
3. The workspace `.varda/shaders/`
4. The platform user shader dir (`~/.local/share/varda/shaders`, `~/Library/Application Support/Varda/Shaders`, `%APPDATA%\Varda\Shaders`)
5. Any `--shader-dir <DIR>` flags (repeatable), in the order given

On a name collision the higher-precedence directory wins, so a `--shader-dir` shader overrides a built-in of the same name. The order holds for the whole session: shaders hot-reload as you edit them, and deleting an override restores the shadowed built-in instead of dropping the shader. A `--shader-dir` that doesn't exist is skipped with a warning, not created.

Shaders are automatically discovered on startup from every directory in the hierarchy and appear in the **Library** panel under Generators, Effects, or Transitions based on their type.

---

[← Prev: Shader Library](11-shader-library.md) · [Home](README.md) · [Next: HTTP API & Headless Mode →](13-api.md)
