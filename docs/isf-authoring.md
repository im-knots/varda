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

## Hot-Reload

Shaders in the `shaders/` directory are watched for changes. Save a `.fs` file and Varda:

1. Detects the file change
2. Recompiles GLSL → SPIR-V
3. On success: replaces the running shader, resets parameters to defaults
4. On error: keeps the old shader running, shows an error notification

No restart required. Edit shaders in any external editor and see results immediately.

## File Location

Place shader files in `shaders/` at the workspace root. They are automatically discovered on startup and appear in the **Library** panel under Generators, Effects, or Transitions based on their type.
