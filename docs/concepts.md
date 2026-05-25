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

A single media source that produces a texture. Each deck has its own opacity, blend mode, and effect chain.

### Channel

Composites multiple decks into a single layer. Each channel has its own effect chain applied after deck compositing. Channels are named numerically: 0, 1, 2, etc.

### Mixer

Composites channels into the final output. With 2 channels, an A/B crossfader blends between them. With 3+ channels, per-channel opacity controls the mix. The mixer owns the master effect chain.

### Surface

A named polygon region in the stage editor. Each surface pulls content from a configurable source: Master (full mix), a specific Channel, a multi-Channel sub-mix, a single Deck, or the Domemaster output. Surfaces define *what content goes where* spatially.

### Output

Renders assigned surfaces onto a display target — a monitor/projector window, NDI sender, SRT stream, HLS/DASH stream, or recording file. Each output applies per-surface warp calibration (corner-pin or mesh warp) and edge blending.

---

## Source Types

| Source | Description |
|--------|-------------|
| ISF Shader | GLSL generator with typed parameters, hot-reload on save |
| Video | ffmpeg decode with loop/ping-pong/one-shot, speed, scrub, in/out points |
| HAP Video | GPU-native codec (BC1/BC3/BC7/YCoCg), direct GPU upload |
| Image | PNG or JPG still |
| Camera | Live webcam input, shared across multiple decks |
| NDI | Network video receive via NDI SDK |
| SRT | Secure Reliable Transport stream receive |
| HLS | HTTP Live Streaming input |
| DASH | MPEG-DASH input |
| Syphon | macOS inter-app texture sharing (runtime bridge pending) |
| Solid Color | Flat RGBA color |

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
| **LFO** | 6 waveforms (sine, triangle, saw, square, random, smooth random), configurable frequency, amplitude, phase |
| **Audio Band** | Bass, mid, or treble energy from FFT analysis — drives parameters with the music |
| **ADSR Envelope** | Attack/Decay/Sustain/Release envelope, triggered manually or via MIDI |
| **Step Sequencer** | N-step pattern at configurable rate, with interpolation modes |

Multiple sources can target the same parameter (summed). Modulator-on-modulator chaining is supported up to 4 levels deep — for example, an LFO modulating the frequency of another LFO.

Parameter paths use the format `deck/<uuid>/param/<name>`, `crossfader`, `ch/<uuid>/opacity`, etc.

---

## Persistence

All state is saved to the `.varda/` directory in the workspace root:

| File | Contents |
|------|----------|
| `scene.json` | Show state — channels, decks, effects, modulation, crossfader, transition sequences |
| `stage.json` | Venue state — surface layout, outputs, warp calibration, editor preferences |
| `midi.json` | MIDI controller mappings (device-name-keyed) |
| `keymap.json` | Keyboard shortcut bindings |
| `presets/` | Saved deck and channel presets |

Save with **Cmd+S** or auto-save on clean exit. Reload everything at a different venue — the scene (your show) is separate from the stage (the venue's physical layout).
