# Resolution, Settings & Monitoring

Varda has **no settings menu**. Every configurable aspect of the app is exposed inline in the main UI — the same "see it, click it" philosophy as MIDI/keyboard learn. This page covers the global controls that live in the **top bar**: render resolution, per-deck scaling, and the live performance metrics.

## Where Settings Live

| Setting | Where | How |
|---------|-------|-----|
| Render resolution | Top bar (📐 W×H) | Click to pick a preset or enter a custom size |
| Audio input device | Audio modulator → device dropdown | Selected per modulator; capture is automatic (a device runs only while referenced) |
| MIDI devices | Library → MIDI | Enable/disable, rescan |
| MIDI mappings | Right-click → MIDI learn | Visual mapping (purple glow) |
| Keyboard shortcuts | Right-click → Keyboard learn | Visual mapping (orange glow) |
| Clock source | Top bar → BPM display | Auto priority + manual override |
| OSC port / feedback | `.varda/osc.json` or `--osc-port` | Config file (see [Control Surfaces](06-control-surfaces.md#osc)) |
| Shader library | `shaders/` directory | Filesystem convention, hot-reloaded |

## Render Resolution

The **render resolution** is the master size at which all decks, channels, and the mixer composite. It is a scene-level setting saved in `scene.json` (`render_width` / `render_height`).

Set it from the 📐 control in the top bar:

### Presets

- 1280×720 (720p)
- 1920×1080 (1080p) — default
- 2560×1440 (1440p)
- 3840×2160 (4K)

### Custom Resolution

Choose **Custom…** to enter freeform width × height for LED walls, vertical strips, or unusual aspect ratios — there is no aspect-ratio lock and **no artificial maximum**. The render size is bounded only by your GPU's maximum texture size (commonly 8192² or 16384²), so capable hardware can render at 8K and beyond.

Changes take effect immediately: the engine resizes every render texture and confirms with a toast (e.g. "📐 Resolution changed to 3840×2160"). Scenes saved without these fields default to 1920×1080. The stage/output system independently fits the master render to each output's actual display resolution.

## Per-Deck Scaling

Every deck renders to a texture at the render resolution, regardless of its source's native resolution. ISF shaders are resolution-independent (they receive `RENDERSIZE` and render directly at the deck size). Video and image sources are scaled once on the GPU using the deck's **scaling mode**:

| Mode | Behavior |
|------|----------|
| **Fill** (default) | Scale to fill the deck, cropping edges if the aspect ratio differs |
| **Fit** | Scale to fit inside the deck, letterbox/pillarbox if the aspect ratio differs |
| **Stretch** | Stretch to exactly match deck dimensions (distorts mismatched aspect ratios) |
| **Center** | No scaling — center at native size, black borders if smaller, crop if larger |

Because scaling happens once on load rather than every frame, the compositing pipeline and all effect chains operate at a single consistent resolution.

Scaling mode is **MIDI/OSC/keyboard-mappable and modulatable** via `deck/<uuid>/scaling_mode` — a fader sweeps through Fill → Fit → Stretch → Center ([fader bucketing](06-control-surfaces.md#parameter-paths)). The selected mode is **saved in the scene** and restored on reload (older scenes without the field default to Fill).

## Performance Monitoring

All metrics are displayed inline in the top bar, each with a clickable drill-down popover. Reading left to right:

```
[Undo] [Redo] [Save] | [📐 Resolution] | [CPU%] [RAM] | [GPU Load%] | [FPS] | [BPM/Clock]
```

This order follows a causal chain — *what you set → what it costs → what's producing it → how fast → the music.*

### FPS

Real-time frame-rate counter, color-coded: green (>55), yellow (30–55), red (<30). Click the **⏱ Render Pipeline** popover for per-channel stats (average FPS, active deck count, render time in ms). Per-deck FPS is tracked with an exponential moving average over a 60-frame rolling window.

### GPU Load

Render load as a percentage of the frame budget: `(total_render_ms / 16.67ms) × 100%`, color-coded green (<50%), yellow (50–80%), red (>80%). The **🖥 GPU Details** popover shows device name, backend (Metal/Vulkan), driver info, device type (discrete/integrated), and render-load ms.

### CPU / RAM

CPU percentage and RAM usage (used/total), both color-coded. These are sampled once per second (not per frame) to avoid measurement overhead.

---

[← Prev: Streaming, Recording & Network I/O](09-streaming-and-io.md) · [Home](README.md) · [Next: Shader Library →](11-shader-library.md)
