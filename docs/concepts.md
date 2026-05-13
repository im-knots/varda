# Core Concepts

## The Signal Flow

Varda routes video through a five-layer hierarchy inspired by broadcast video switchers:

```
[Deck] → [Deck FX] ─┐
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┐
[Deck] → [Deck FX] ─┘                               │
                                                     ├─→ [Mixer] → [Master FX] → [Surfaces] → [Outputs]
[Deck] → [Deck FX] ─┐                               │
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┘
[Deck] → [Deck FX] ─┘
```

### Deck

A single media source that produces a GPU texture each frame. Every deck has:

- **Opacity** (0.0–1.0) — zero-opacity decks are culled from the render pass entirely
- **Blend mode** — determines how the deck composites onto the layer below:

| Mode | Effect |
|------|--------|
| **Normal** | Standard alpha compositing — back layer shows through transparent areas |
| **Add** | Brightens — adds pixel values (black = no change, white = double bright). Use for glows, light effects |
| **Multiply** | Darkens — multiplies pixel values (white = no change, black = full black). Use for shadows, vignettes |
| **Screen** | Brightens — inverse multiply (like projecting two slides onto the same screen). Use for additive light |
| **Overlay** | Contrast — multiplies darks, screens lights. Use for punch and texture overlays |
| **Difference** | Inverts — absolute difference of pixel values (identical inputs = black). Use for psychedelic effects |
- **Z-index** — controls draw order within the channel (lower draws first)
- **Mute / Solo** — muted decks are skipped; when any deck is soloed, only soloed decks render
- **Effect chain** — ISF filters applied to this deck's output before channel compositing

### Channel

Composites its decks into a single layer. Decks are drawn in z-index order, each blended onto the composite using its blend mode and opacity. After deck compositing, the channel's own effect chain is applied.

Channels are named A, B, C, ... Z, A1, B1, etc. Every channel always renders — even when hidden by the crossfader — so the VJ can prepare content before transitioning.

Each channel also carries its own **opacity** and **blend mode**, which control how it mixes into the final output at the mixer level.

### Mixer

Composites channels into the master mix. The mixing behavior depends on channel count:

- **2 channels** — an A/B **crossfader** (0.0 = full A, 1.0 = full B) controls the blend. When a **transition shader** is active, it replaces the opacity crossfade with a GPU shader effect (dissolve, iris, push, etc.)
- **3+ channels** — per-channel **opacity** and **blend mode** control the mix. The crossfader is not used.

The mixer also owns:

- **Master effect chain** — ISF filters applied to the final composite before it reaches surfaces
- **Auto-crossfade** — timed crossfade with configurable easing
- **Beat-synced crossfade** — crossfade triggered on the next detected beat
- **Transition sequences** — programmable multi-step channel-to-channel automation

### Surface

A named polygon region drawn in the stage editor. Each surface has a **content source** that determines what it displays:

| Source | Content |
|--------|---------|
| **Master** | The full mixer output (post-master FX) |
| **Channel** | A single channel's composited output |
| **Channels** | A sub-mix of selected channels (composited with their individual opacities and blend modes; master FX are *not* applied) |
| **Deck** | A single deck's raw texture output |
| **Domemaster** | The dome fisheye projection |

Each surface also has a **content mapping** mode:

- **Fill** — the entire source texture is scaled to fill the surface. Each surface gets an independent full copy.
- **Mapped** — the surface's position on the canvas determines which region of the source it displays. The canvas *is* the content space — a surface at position (0.2, 0.3) with size (0.1, 0.1) shows UVs from (0.2, 0.3) to (0.3, 0.4). Multiple surfaces with the same source in Mapped mode each show their slice of one continuous image.

Surfaces also define their **output type**: Projection (content is warped to match projector geometry) or LED Direct (pixel-accurate crop/scale, no perspective warp).

### Output

Renders assigned surfaces onto a display target. Each output applies per-surface warp calibration (corner-pin or mesh warp) and edge blending. Output targets include:

| Target | Description |
|--------|-------------|
| **Windowed** | Floating desktop window |
| **Display** | Fullscreen/borderless on a specific monitor or projector |
| **NDI Send** | Network video output via NDI |
| **SRT Stream** | Network video output via SRT |
| **HLS Stream** | HTTP Live Streaming output (standard or LL-HLS) |
| **DASH Stream** | MPEG-DASH streaming output |
| **Recording** | File recording (H.264, H.265, AV1, ProRes, HAP, HAP Alpha, HAP Q) |
| **Syphon** | macOS inter-app texture sharing |

When no surfaces are assigned, the output receives the full master mix directly. See [Projection Mapping](projection.md) for surface and multi-output workflows.

---

## Source Types

| Source | Description |
|--------|-------------|
| **ISF Shader** | GLSL 450 generator with typed parameters. Hot-reloads on save. Place `.fs` files in `shaders/`. See [ISF Authoring](isf-authoring.md) |
| **Video** | Any ffmpeg-supported format (H.264, H.265, ProRes, VP9, etc. in MP4/MOV/MKV/AVI/WebM). Loop, ping-pong, one-shot, speed, in/out points. See [Video Playback](performance.md#video-playback) |
| **HAP Video** | GPU-native codec — HAP (BC1), HAP Alpha (BC3), HAP Q (YCoCg), HAP Q Alpha, HAP R (BC7). Direct GPU upload, zero CPU decode overhead. MOV container |
| **Image** | PNG or JPG/JPEG still |
| **Camera** | Live webcam input (macOS AVFoundation, Linux V4L2). One capture thread per camera, shared GPU texture across multiple decks. Scaling modes: Fill, Fit, Stretch, Center |
| **NDI** | Network video receive via NDI SDK. Auto-discovered on the local network |
| **SRT** | Secure Reliable Transport stream receive. Listener or caller mode |
| **HLS** | HTTP Live Streaming input (standard and LL-HLS) |
| **DASH** | MPEG-DASH streaming input |
| **Syphon** | macOS inter-app texture sharing |
| **Solid Color** | Flat RGBA color fill — zero GPU overhead. Useful as base layers, test surfaces, or color overlays |

---

## Effect Chains

Effects are ISF filter shaders applied at three levels:

1. **Deck FX** — applied to a single deck's output before channel compositing
2. **Channel FX** — applied to the composited channel output before mixing
3. **Master FX** — applied to the final mixer output before routing to surfaces

Effects can be reordered via drag-and-drop and toggled on/off individually.

---

## Modulation

Any numeric parameter in the hierarchy can be automated by modulation sources:

| Source | Description |
|--------|-------------|
| **LFO** | 5 waveforms (sine, triangle, sawtooth, square, random), configurable frequency, amplitude, phase |
| **Audio** | Bass, mid, or treble energy from FFT analysis — drives parameters with the music |
| **ADSR Envelope** | Attack/Decay/Sustain/Release envelope, triggered manually or via MIDI |
| **Step Sequencer** | N-step pattern at configurable rate, with interpolation modes (none, linear, smooth) |

Multiple sources can target the same parameter (summed additively). Modulator-on-modulator chaining is supported up to 4 levels deep — for example, an LFO modulating the frequency of another LFO.

Parameter paths use the format `deck/<uuid>/param/<name>`, `crossfader`, `ch/<uuid>/opacity`, etc.

For detailed coverage of all source types, routing, mod-on-mod chaining, and audio reactivity, see the [Modulation Guide](modulation.md).

---

## Persistence

All state is saved to the `.varda/` directory in the workspace root, split into two categories:

**Scene** (your show — portable between venues):

| File | Contents |
|------|----------|
| `scene.json` | Channels, decks, effects, modulation, crossfader, transition sequences, render resolution |
| `midi.json` | MIDI controller mappings (device-name-keyed) |
| `keymap.json` | Keyboard shortcut bindings |
| `osc.json` | OSC input port and feedback targets |
| `presets/decks/` | Saved deck presets |
| `presets/channels/` | Saved channel presets |

**Stage** (the venue — stays with the installation):

| File | Contents |
|------|----------|
| `stage.json` | Surface layout, outputs, warp calibration, dome config, editor preferences |

This split means you can load your show at a different venue by copying `scene.json` (and presets) while keeping the venue's projection mapping in its own `stage.json`.

Save with **Cmd+S** or the save button. Varda also saves on clean exit. Asset paths (shaders, videos, images) are stored as relative paths from the workspace root. On restore, missing assets are skipped with a warning notification — the rest of the scene loads normally.
