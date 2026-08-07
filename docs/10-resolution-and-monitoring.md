# Resolution, Settings & Monitoring

This page covers the global controls that live in the **top bar**: render resolution, per-deck scaling, and the live performance metrics.

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

**Landscape**

- 1280×720 (720p)
- 1920×1080 (1080p) — default
- 2560×1440 (1440p)
- 3840×2160 (4K)

**Vertical & square**

- 1080×1920 (9:16) — Instagram Reels, TikTok, YouTube Shorts, Stories and Facebook Reels all take this exact size, so it is the one vertical master to record if you only record one
- 2160×3840 (9:16 at 4K)
- 1080×1350 (4:5) — Instagram feed posts, which take up more of the scroll than square
- 1080×1080 (1:1) — square

### Custom Resolution

Choose **Custom…** to enter freeform width × height for LED walls, vertical strips, or unusual aspect ratios. There is no aspect-ratio lock and **no artificial maximum**. The render size is bounded only by your GPU's maximum texture size (commonly 8192² or 16384²), so capable hardware can render at 8K and beyond.

Changes take effect immediately: the engine resizes every render texture and confirms with a toast (e.g. "📐 Resolution changed to 3840×2160"). Scenes saved without these fields default to 1920×1080.

### What outputs do when you change resolution

Everything you send out conforms to the resolution you set, not to the shape it happened to have when you created it.

**Recordings, NDI, Syphon and streams** are resized to match. Anything already running through ffmpeg such as a recording or a stream, is **stopped**, and a toast names which ones. The encoder's frame size is fixed when it starts, so it can't follow you mid-take; stopping is safer than writing frames it will misread, and safer than restarting, which would reopen the file and wipe the take you already have. Start it again and it comes back at the new size. NDI and Syphon keep publishing across the change without interruption.

**Output windows** letterbox. The window is whatever size you or the OS made it, so a 9:16 project on a 16:9 projector is centred with black bars rather than stretched. The projector calibration card still covers the full output — it has to, for alignment to mean anything — and surfaces you place on the stage carry their own shape, so neither is letterboxed. A brand-new output window opens at the master's aspect ratio, so a vertical or square project doesn't start life in a 16:9 window you have to drag into shape. Once you have resized it, that size is saved with the stage and used from then on.

**The dome is the one exception.** A domemaster is square by definition, so it has its own size setting (1K/2K/4K) in the Stage Editor's dome toolbar rather than following the master. See [Projection Mapping](08-projection.md).

### What outputs do with frame rate

Everything runs at the **Target FPS** you set in the top bar — there are no per-output frame-rate settings and nothing runs on its own clock. Recordings and streams declare that rate to their encoder, and NDI advertises it to receivers, so a Varda running at 60 shows up as 60 in OBS or Studio Monitor rather than claiming some fixed number.

**Uncapped** is the one case where a real number has to be invented, because a file or a stream has to state a frame rate and "as fast as possible" isn't one. Outputs declare 60 in that mode. If you are recording or streaming and you care about exact timing, set an explicit target rather than leaving it uncapped.

When the renderer misses a frame, recordings repeat the previous one to keep the file's running time honest, rather than producing a file that plays back faster than the session really ran.

## Per-Deck Scaling

Every deck renders to a texture at the render resolution, regardless of its source's native resolution. ISF shaders are resolution-independent (they receive `RENDERSIZE` and render directly at the deck size). Video and image sources are scaled once on the GPU using the deck's **scaling mode**:

| Mode | Behavior |
|------|----------|
| **Fill** (default) | Scale to fill the deck, cropping edges if the aspect ratio differs |
| **Fit** | Scale to fit inside the deck, letterbox/pillarbox if the aspect ratio differs |
| **Stretch** | Stretch to exactly match deck dimensions (distorts mismatched aspect ratios) |
| **Center** | No scaling — center at native size, black borders if smaller, crop if larger |

Because scaling happens once on load rather than every frame, the compositing pipeline and all effect chains operate at a single consistent resolution.

**SVG is the exception, and deliberately so.** Vector art has no native pixel size, so instead of being decoded once and scaled, it is *redrawn* to fit the deck — and redrawn again whenever you change the render resolution. A logo that looks crisp while you build the set at 720p is re-rendered at 4K when you switch the master up for the show, rather than being magnified. The drawing's own proportions are preserved, so the scaling mode above applies to an SVG exactly as it does to a photograph.

## Performance Monitoring

All metrics are displayed inline in the top bar, each with a clickable drill-down popover. Reading left to right:

```
[Undo] [Redo] [Save] | [📐 Resolution] | [CPU%] [RAM] | [GPU Load%] | [FPS] | [BPM/Clock]
```

This order follows a causal chain of: *what you set → what it costs → what's producing it → how fast → the music.*

### FPS

Real-time frame-rate counter, color-coded: green (>55), yellow (30–55), red (<30). Click the **⏱ Render Pipeline** popover for per-channel stats (average FPS, active deck count, render time in ms). Per-deck FPS is tracked with an exponential moving average over a 60-frame rolling window.

### GPU Load

Render load as a percentage of the frame budget: `(total_render_ms / 16.67ms) × 100%`, color-coded green (<50%), yellow (50–80%), red (>80%). The **🖥 GPU Details** popover shows device name, backend (Metal/Vulkan), driver info, device type (discrete/integrated), and render-load ms.

### CPU / RAM

CPU percentage and RAM usage (used/total), both color-coded. These are sampled once per second (not per frame) to avoid measurement overhead.

---

[← Prev: Streaming, Recording & Network I/O](09-streaming-and-io.md) · [Home](README.md) · [Next: Shader Library →](11-shader-library.md)
