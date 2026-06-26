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

Composites channels into the final output. With 2 channels, an A/B crossfader blends between them. With 3+ channels, per-channel opacity controls the mix. The mixer owns the master effect chain, tonemapping, and optional 3D LUT color grading.

### Surface

A named polygon region in the stage editor. Each surface pulls content from a configurable source: Master (full mix), a specific Channel, a multi-Channel sub-mix, or the Domemaster output. Surfaces define *what content goes where* spatially.

### Output

Renders assigned surfaces onto a display target — a monitor/projector window, NDI sender, SRT stream, HLS/DASH stream, or recording file. Each output applies per-surface warp calibration (corner-pin or mesh warp), edge blending, and optional rotation. See [Outputs](07-outputs.md).

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
| RTMP | RTMP/RTMPS stream receive |
| Compute Shader | GLSL compute shader (`.comp`) — particle systems, simulations, GPU-native generators |
| Syphon | macOS inter-app texture sharing (receive from other apps) |
| Solid Color | Flat RGBA color |

---

## Blend Modes

Each deck composites onto its channel using a blend mode (and the same set is available for per-channel mixing with 3+ channels). Varda implements 15 modes:

| Group | Modes |
|-------|-------|
| **Normal** | Normal |
| **Lighten** | Add, Screen, Color Dodge, Lighten |
| **Darken** | Multiply, Color Burn, Linear Burn, Darken |
| **Contrast** | Overlay, Soft Light, Hard Light |
| **Comparative** | Difference, Exclusion, Subtract |

Compositing runs in linear-light HDR, so additive and screen modes can push values above 1.0 — the tonemap stage (below) brings them back into displayable range.

---

## Effect Chains

Effects are ISF filter shaders applied at three levels:

1. **Deck FX** — applied to a single deck's output before channel compositing
2. **Channel FX** — applied to the composited channel output before mixing
3. **Master FX** — applied to the final mixer output before routing to surfaces

Effects can be reordered via drag-and-drop and toggled on/off individually.

---

## Tonemapping & Color Grading

The compositing pipeline operates in **linear-light HDR** (`Rgba16Float`). Before frames reach outputs, two optional color transforms are applied in order:

### Tonemap

Compresses HDR values into displayable [0, 1] range. Nine algorithmic presets are available:

| Preset | Character |
|--------|-----------|
| **Bypass** | No compression — values >1.0 clamp at the output boundary |
| **ACES Filmic** (default) | Cinematic rolloff with warm highlight shift |
| **Reinhard** | Gentle curve, never reaches pure white |
| **Reinhard Extended** | Reinhard with configurable white point |
| **Hable Filmic** | Game-industry standard with nice toe and shoulder |
| **Uchimura (GT)** | Gran Turismo style, tunable shoulder |
| **Lottes (AMD)** | Fast, invertible, high contrast |
| **AgX** | Neutral, minimal hue shift |
| **PBR Neutral** | Color-accurate, minimal look modification |

Select via the **TM:** label in the top bar or `PUT /api/mixer/tonemap`.

### 3D LUT

An optional 3D Look-Up Table applied after tonemapping for color grading, gamut transforms, or creative looks. Supports industry-standard `.cube` and `.3dl` files (including 1D shaper LUTs for shadow precision).

Place LUT files in `.varda/luts/` — they appear in the tonemap popover for one-click selection. The active LUT persists across sessions.

LUTs are the universal mechanism for importing color transforms from DaVinci Resolve, Photoshop, or any color grading tool. A single `.cube` file can encode tonemapping + color grading + gamut mapping in one pass.

---

## Modulation

Any numeric parameter in the hierarchy can be automated by modulation sources:

| Source | Description |
|--------|-------------|
| **LFO** | 6 waveforms (sine, triangle, saw, square, random, smooth random), configurable frequency, amplitude, phase |
| **Audio Band** | Bass, mid, or treble energy from FFT analysis — drives parameters with the music |
| **ADSR Envelope** | Attack/Decay/Sustain/Release envelope, triggered manually or via MIDI |
| **Step Sequencer** | N-step pattern at configurable rate, with interpolation modes |
| **Analyzer** | Scalar outputs derived from analysis of a deck's input frame (e.g. brightness, contrast) |

Sources are created in the modulation panel and assigned to any parameter with its **〰** button. Multiple sources can target the same parameter (summed). Modulator-on-modulator chaining is supported up to 4 levels deep — for example, an LFO modulating the frequency of another LFO. See [Modulation & Audio Reactivity](05-modulation.md) for the assignment workflow.

Parameter paths use the format `deck/<uuid>/param/<name>`, `crossfader`, `ch/<uuid>/opacity`, etc.

---

## Persistence

All state is saved to the `.varda/` directory in the workspace root:

| File | Contents |
|------|----------|
| `scene.json` | Show state — channels, decks, effects, modulation, crossfader, tonemap mode, active LUT, transition sequences |
| `stage.json` | Venue state — surface layout, outputs, warp calibration, editor preferences |
| `midi.json` | MIDI controller mappings (device-name-keyed) |
| `keymap.json` | Keyboard shortcut bindings |
| `osc.json` | OSC input port and feedback targets |
| `presets/` | Saved deck and channel presets |
| `luts/` | 3D LUT files (.cube, .3dl) for color grading |
| `controller-profiles/` | Custom MIDI controller profiles (JSON) |

Save with **Cmd+S** or auto-save on clean exit. Reload everything at a different venue — the scene (your show) is separate from the stage (the venue's physical layout).

---

[← Prev: Getting Started](01-getting-started.md) · [Home](README.md) · [Next: Library Panel →](03-library-panel.md)
