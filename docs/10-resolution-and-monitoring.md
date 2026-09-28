# Resolution, Settings & Monitoring

This page covers render resolution and per-deck scaling, which you set from the **top bar**, and the performance metrics at the foot of the **right panel**.

## Settings Locations

| Setting | Where | How |
|---------|-------|-----|
| Render resolution | Top bar (📐 W×H) | Click to pick a preset or enter a custom size |
| Audio input device | Audio modulator → device dropdown | Selected per modulator; capture is automatic (a device runs only while referenced) |
| MIDI devices | Right panel → 🎹 MIDI | Enable/disable, rescan |
| MIDI mappings | Right-click → MIDI learn | Visual mapping (purple glow) |
| Keyboard shortcuts | Right-click → Keyboard learn | Visual mapping (orange glow) |
| Clock source | Top bar → BPM display | Auto priority + manual override |
| Transport | Top bar → position readout | Play/stop, return to zero, source, timecode rate |
| Tonemap curve and 3D LUT | Right panel → 🎨 Tonemap | Pick a curve; LUTs are read from `.varda/luts/` |
| OSC port / feedback | `.varda/osc.json` or `--osc-port` | Config file (see [Control Surfaces](06-control-surfaces.md#osc)) |
| Shader library | `shaders/` directory | Filesystem convention, hot-reloaded |

## Render Resolution

The **render resolution** is the master size at which all decks, channels, and the mixer composite. It is a scene-level setting saved in `scene.json` (`render_width` / `render_height`).

Set it from the 📐 control in the top bar.

### Presets

**Landscape**

- 1280×720 (720p)
- 1920×1080 (1080p), the default
- 2560×1440 (1440p)
- 3840×2160 (4K)

**Vertical & square**

- 1080×1920 (9:16): the size Instagram Reels, TikTok, YouTube Shorts, Stories, and Facebook Reels all use. Use this one if you record only one vertical master.
- 2160×3840 (9:16 at 4K)
- 1080×1350 (4:5): Instagram feed posts, which take more screen height in the feed than square posts
- 1080×1080 (1:1): square

### Custom Resolution

Choose **Custom…** to enter any width × height, for LED walls, vertical strips, or unusual aspect ratios. There is no aspect-ratio lock and **no fixed maximum**. The limit is your GPU's maximum texture size (commonly 8192² or 16384²), so capable hardware can render at 8K and above.

Changes take effect immediately. The engine resizes every render texture and shows a toast (e.g. "📐 Resolution changed to 3840×2160"). Scenes saved without these fields default to 1920×1080.

### Outputs and resolution changes

All outputs follow the current render resolution.

**Recordings, NDI, Syphon and streams** are resized to match. A recording or stream running through ffmpeg is **stopped**, and a toast names each one stopped. The encoder's frame size is fixed when it starts, so it cannot change mid-take. Varda stops it instead of restarting it, because a restart would reopen the file and overwrite the take. Start it again to continue at the new size. NDI and Syphon keep publishing through the change without interruption.

**Output windows** letterbox. The window keeps the size you or the OS gave it, so a 9:16 project on a 16:9 projector is centered with black bars. The projector calibration card still covers the full output, so alignment is correct. Surfaces you place on the stage have their own shape and are not letterboxed. A new output window opens at the master's aspect ratio. Once you resize it, that size is saved with the stage and used from then on.

**The dome** does not follow the master resolution. A domemaster is always square, so it has its own size setting (1K/2K/4K) in the Stage Editor's dome toolbar. See [Projection Mapping](08-projection.md).

### Outputs and frame rate

All outputs run at the **Target FPS** you set in the top bar. There are no per-output frame-rate settings. Recordings and streams declare that rate to their encoder, and NDI advertises it to receivers, so Varda running at 60 shows as 60 in OBS or Studio Monitor.

In **Uncapped** mode, outputs declare 60, because files and streams need a stated frame rate. If you are recording or streaming and need exact timing, set an explicit target.

When the renderer misses a frame, recordings repeat the previous frame. This keeps the file's running time equal to the session's, so it does not play back faster than it ran.

## Per-Deck Scaling

Every deck renders to a texture at the render resolution, whatever its source's native resolution. ISF shaders are resolution-independent: they receive `RENDERSIZE` and render directly at the deck size. Video and image sources are scaled once on the GPU using the deck's **scaling mode**:

| Mode | Behavior |
|------|----------|
| **Fill** (default) | Scale to fill the deck, cropping edges if the aspect ratio differs |
| **Fit** | Scale to fit inside the deck, letterbox/pillarbox if the aspect ratio differs |
| **Stretch** | Stretch to exactly match deck dimensions (distorts mismatched aspect ratios) |
| **Center** | No scaling. Center at native size, black borders if smaller, crop if larger |

Scaling happens once on load, so the compositing pipeline and all effect chains run at one resolution.

**SVG** images are redrawn at the deck size instead of scaled. They are redrawn again whenever you change the render resolution, so a logo stays sharp when you switch from 720p to 4K. The drawing keeps its proportions, and the scaling mode applies to an SVG the same way it applies to a photograph.

## Performance Monitoring

All five readouts sit together at the **foot of the right panel**, left to right:

```
[FPS] [🖥 GPU Load%] [CPU%] [RAM] [decks + VRAM estimate]
```
When you collapse the right panel, the first four (FPS, GPU, CPU and RAM) appear stacked down the narrow rail. Hover any of them for the full label.

Click FPS or GPU Load to open a detail popover.

### FPS

Real-time frame-rate counter, color-coded: green (>55), yellow (30–55), red (<30). Click the **⏱ Render Pipeline** popover for per-channel stats (average FPS, active deck count, render time in ms). Per-deck FPS is an exponential moving average over a 60-frame rolling window.

### GPU Load

Render load as a percentage of the frame budget: `(total_render_ms / 16.67ms) × 100%`, color-coded green (<50%), yellow (50–80%), red (>80%). The **🖥 GPU Details** popover shows device name, backend (Metal/Vulkan), driver info, device type (discrete/integrated), and render-load ms.

### CPU / RAM

CPU percentage and RAM usage (used/total), both color-coded. They are sampled once per second, not per frame, to keep measurement overhead low.

### Decks & VRAM

The deck count across every channel, followed by an estimate of the GPU memory their color targets use at the current render resolution. Video decoders and camera buffers use additional memory that is not included. Use it to judge whether a scene is getting heavy, not as an exact total.

This readout matters most in [Arrangement Mode](15-arrangement.md), where every deck in the show holds its targets for the whole show, whether or not a region covers it. A deck the arrangement has put to sleep stops decoding but keeps its memory, so the number does not change when a deck sleeps. Hover for the channel count and the target size.

---

[← Prev: Streaming, Recording & Network I/O](09-streaming-and-io.md) · [Home](README.md) · [Next: Shader Library →](11-shader-library.md)
