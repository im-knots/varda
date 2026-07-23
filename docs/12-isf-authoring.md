# ISF Shader Authoring

Varda uses [ISF (Interactive Shader Format)](https://isf.video) for all generators, filters, and transitions. Shaders are GLSL 450 (Vulkan) with a JSON metadata header that declares parameters, inputs, and passes.

## Shader Types

| Type | Detection | Purpose |
|------|-----------|---------|
| **Generator** | No `image` type inputs | Creates visuals from scratch (patterns, fractals, color fields) |
| **Filter** | Has at least one `image` input | Processes an input image (blur, color grade, distort) |
| **Transition** | Has `Transition` category + image inputs | Blends two images via a `progress` parameter (dissolve, wipe, push) |

Varda classifies shaders automatically from their metadata — no manual type annotation needed.

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
- Optional `WIDTH`/`HEIGHT` expressions: `"$WIDTH/2"` for half-resolution buffers
- Optional `FLOAT: true` for 32-bit float buffers (HDR, simulation data)

## Compute Shaders

Beyond fragment shaders, Varda supports **GLSL 450 compute shaders** for work that doesn't fit the one-output-pixel-per-invocation model — particle systems, N-body simulations, cellular automata, and other GPU-native generators. Compute shaders use the **same language and compilation pipeline** as fragment shaders, with an ISF-style JSON header for metadata.

Compute shaders are **generators**: each one renders into its own output image that becomes the deck's source. There is no compute *effect* path — a compute shader does not receive an upstream input texture. If you need to process an incoming frame, use a fragment-shader filter (see [Shader Types](#shader-types)).

### Anatomy of a Compute Shader

A compute shader uses the `.comp` extension and requires `"TYPE": "compute"` plus a `"COMPUTE"` block in the header. Three things must line up:

1. The JSON `"COMPUTE".WORKGROUP_SIZE` must equal the GLSL `layout(local_size_*)` declaration.
2. The output is **always** a write-only `rgba8` storage image at **`binding = 2`**.
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
| `set=0, binding=2` | Output image | `rgba8`, `writeonly` — this is what the deck displays |
| `set=0, binding=3 …` | Storage buffers | One per `BUFFERS` entry, in declaration order |

The output format is hard-wired to `rgba8`; declare it exactly as `rgba8` in the layout qualifier and write with `imageStore`.

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

// Binding 2: the output image (always rgba8, writeonly).
layout(set = 0, binding = 2, rgba8) uniform writeonly image2D outputImage;

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
layout(set = 0, binding = 2, rgba8) uniform writeonly image2D outputImage;

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
- **Output is `rgba8`** — no HDR/float compute output yet.
- **`DISPATCH: "custom"` is not implemented** — only `"resolution"` works.

### See Also

Two reference compute shaders ship with Varda, each demonstrating a different idiom:

- `shaders/black_hole_sim.comp` — a **stateful N-body** simulation: a `PERSISTENT: true` particle buffer that leapfrog-integrates frame to frame, a non-persistent scratch grid for atomic spatial binning, two-pass simulate/render, `PHASE_INPUTS`, and audio reactivity. It puts every feature in this section to work at once.
- `shaders/cosmic_web.comp` — a **stateless, analytic** simulation: a scientifically grounded dark-matter cosmic web built from the *Zel'dovich approximation*. Pass 0 synthesises a Gaussian displacement field as plane-wave modes drawn from a CDM (BBKS) power spectrum; pass 1 displaces a grid of Lagrangian particles (`x = q + D·Ψ(q)`) and cloud-in-cell deposits them into a fixed-resolution density buffer; pass 2 tone-maps that field into a void→filament→node colormap. Because positions are recomputed each frame from a deterministic seed (no persistent state), it is fully scrubbable, and the growth factor `D` animates the collapse of structure.

Read `black_hole_sim.comp` for persistence and binning; read `cosmic_web.comp` for the multi-pass "generate → deposit → render" split and how to keep a sim deterministic and scrub-safe.

## Analyzer Preprocessors (Advanced)

Some effects need **structured data about the input frame** that plain GLSL can't compute — face detection bounding boxes, depth maps, segmentation masks, optical flow fields. Varda's analyzer/preprocessor system bridges this gap: CPU-side analysis (often ML-powered via ONNX Runtime) produces data textures that are automatically injected into your shader as additional texture bindings.

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

| Type | Outputs | Description |
|------|---------|-------------|
| `face_detect` | `landmarks` (wireframe overlay), `face_data` (bbox/scores), `dossier_text` (character indices) | ONNX-based face detection with 468-point mesh landmarks |

Additional analyzer types (`depth_estimate`, `segmentation`, `optical_flow`, `edge_detect`) are planned.

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
- If an analyzer fails to initialize (missing model file, unsupported platform), the shader still loads — preprocessor textures fall back to 1×1 black

### Use Cases

- **ML-powered effects**: face detection overlays, body segmentation masks, depth-aware fog
- **Sensor integration**: external data sources encoded as textures (hardware sensors, network data)
- **Rich data processing**: any CPU-side computation too complex for fragment shaders (physics simulations, pathfinding, text layout)


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
