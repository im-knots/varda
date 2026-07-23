# Core Concepts

## The Signal Flow

Varda routes video through a five-layer hierarchy inspired by broadcast video switchers: **Sources → Decks → Channels → Mixer → Surfaces → Outputs**.

```
[Deck] → [Deck FX] ─┐
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┐
[Deck] → [Deck FX] ─┘                               │
                                                     ├─→ [Mixer] → [Master FX] → [Surfaces] → [Outputs]
[Deck] → [Deck FX] ─┐                               │
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┘
[Deck] → [Deck FX] ─┘
```

The simplest setup is two channels with a crossfader between them, output going fullscreen to a projector: load sources into decks, crossfade between channels. Add complexity — more channels, surfaces, sub-mixes — only as your use case needs it.

### Deck

A single media source (shader, video, image, solid color, camera, NDI stream, SRT stream, HLS/DASH stream, or RTMP stream) that produces a texture. Each deck has its own opacity, blend mode, effect chain, and auto-transition settings. Decks at zero opacity are culled from the render pass entirely — they cost nothing to render.

### Channel

Composites multiple decks into a single layer using per-deck opacity, blend modes, and optional auto-transitions. Each channel has its own effect chain applied after deck compositing. Channels are named numerically: 0, 1, 2, etc.

### Mixer

Composites channels into the final output. With 2 channels, an A/B crossfader blends between them. With 3+ channels, per-channel opacity and blend modes control the mix. The mixer owns the master effect chain, tonemapping, optional 3D LUT color grading, and a state-machine-driven multi-channel transition sequencer (see [Transition Sequences](04-performance.md#transition-sequences)).

### Surface

Surfaces are optional. Each is a named polygon region in the stage editor that pulls content from a configurable source: Master (full mix), a specific Channel, a multi-Channel sub-mix, or the Domemaster output. Surfaces define *what content goes where* spatially — they're how you map content onto physical screens, LED panels, or projection areas. When no surfaces are defined, outputs receive the full main mix directly.

### Output

Renders assigned surfaces onto a display target — a monitor/projector window, NDI sender, SRT stream, HLS/DASH stream, or recording file. Each output applies per-surface warp calibration (corner-pin or mesh warp), edge blending, and optional rotation. Surfaces are assigned to outputs to complete the routing chain. See [Outputs](07-outputs.md).

### Routing Flexibility

At every junction you can branch, split, or re-route. Two channels feeding different surfaces on the same output. The main mix on one output, a single channel isolated on another. A sub-mix of specific channels to an NDI stream while the master goes to projection. You only use the complexity you need: simple A/B crossfading between two decks, a multi-channel mixing console with dedicated FX and transitions, or a complex multi-screen setup with individual content routing.

### Clip Triggering Without Clip Triggering

Most VJ software uses a clip-launch paradigm: press a button, a clip starts playing; press another, the previous one stops. Varda doesn't have clip triggers, but it doesn't need them, because the broadcast mixer model gives you the same result with more control.

Load your sources into decks across your channels. What makes a deck live is its **opacity**. Map your MIDI controller buttons to deck mute or deck opacity. Press a button, a deck's opacity goes from 0 to 1, it's live. Press another, that deck goes to 0, it's gone. From the audience's perspective, you just triggered a clip. Under the hood, zero-opacity decks are **culled from the render pass entirely**, so they cost nothing. Only decks with non-zero opacity are actually rendered. When you bring a deck's opacity up, its source starts producing frames immediately. You're not "stopping and starting clips," you're mixing a live signal path where the GPU only pays for what's visible.

This is how broadcast video works. A switcher doesn't start and stop cameras. Every camera is always hot, and the director cuts between them by routing signals onto buses. Varda applies the same idea: your decks are always ready, and you perform by controlling which signals are live in the mix. The result is instant transitions (no clip load latency), full MIDI control over the routing, and a mental model that scales.

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
| HTML | Web page (HTML/CSS/JS) rendered by the embedded Servo browser engine |
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

## The Varda workspace
Varda treats the current working directory as a workspace. All state lives in a `.varda/` directory created automatically:

```
your-show/
  .varda/
    scene.json            # channels, decks, effects, modulation, crossfader, tonemap, LUT, transition sequences
    stage.json            # surface layout, outputs, warp calibration
    midi.json             # MIDI controller mappings that differ from the auto-mapped defaults
    keymap.json           # keyboard shortcut bindings
    osc.json              # OSC input port and feedback targets
    presets/
      decks/              # saved deck presets (JSON)
      channels/           # saved channel presets (JSON)
    shaders/              # ISF shaders
    luts/                 # 3D LUT files (.cube, .3dl) for color grading
    controller-profiles/  # MIDI controller profiles (JSON)
    recordings/           # recording output files
    streams/              # HLS/DASH output files
```

Run Varda from different directories to maintain separate workspaces per show, venue, or project. Each workspace has its own scene, stage layout, and MIDI mappings.

Save with **Cmd+S** or auto-save on clean exit. Reload everything at a different venue — the scene (your show) is separate from the stage (the venue's physical layout).

---

[← Prev: Getting Started](01-getting-started.md) · [Home](README.md) · [Next: Library Panel →](03-library-panel.md)
