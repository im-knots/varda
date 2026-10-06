# Core Concepts

## The Signal Flow

Varda routes video through a hierarchy modeled on broadcast video switchers: **Sources → Decks → Channels → Mixer → Surfaces → Outputs**.

```
[Deck] → [Deck FX] ─┐
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┐
[Deck] → [Deck FX] ─┘                               │
                                                     ├─→ [Mixer] → [Master FX] → [Surfaces] → [Outputs]
[Deck] → [Deck FX] ─┐                               │
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┘
[Deck] → [Deck FX] ─┘
```

The simplest setup is two channels with a crossfader between them and one output fullscreen on a projector. Load sources into decks and crossfade between the channels. Add more only when your show needs it.

### Deck

A single media source (shader, video, image, solid color, camera, NDI stream, SRT stream, HLS/DASH stream, or RTMP stream) that produces a texture. Each deck has its own opacity, blend mode, effect chain, and auto-transition settings. Decks at zero opacity are left out of the render pass.

### Channel

Composites several decks into one layer using per-deck opacity, blend modes, and optional auto-transitions. Each channel has its own effect chain, applied after the decks are composited. Channels are numbered 0, 1, 2, and so on.

### Mixer

Composites channels into the final output. With 2 channels, an A/B crossfader blends between them. With 3+ channels, per-channel opacity and blend modes control the mix. The mixer holds the master effect chain, the show-wide look grade, the default tonemap curve, and a state-machine-driven multi-channel transition sequencer (see [Transition Sequences](04-performance.md#transition-sequences)).

### Surface

Surfaces are optional. Each surface is a named polygon region in the stage editor. It takes content from a source you choose: Master (full mix), a specific Channel, a multi-Channel sub-mix, or the Domemaster output. Use surfaces to place content on physical screens, LED panels, or projection areas. When no surfaces are defined, outputs receive the full main mix.

### Output

Renders its assigned surfaces to a target such as a monitor or projector window, NDI sender, SRT stream, HLS/DASH stream, or recording file. Each output applies per-surface warp calibration (corner-pin or mesh warp), edge blending, and optional rotation. Assigning surfaces to outputs completes the routing chain. See [Outputs](10-outputs.md).

### Triggering Decks with Opacity

Varda has no clip-launch buttons. Instead, load your sources into decks across your channels and switch decks on and off with **opacity**.

Map MIDI controller buttons to deck mute or deck opacity. Press a button and a deck's opacity goes from 0 to 1, so it is live. Press another and that deck goes to 0, so it is gone. To the audience, this looks like triggering a clip.

Zero-opacity decks are **left out of the render pass**, so they cost nothing. Only decks with non-zero opacity are rendered. When you raise a deck's opacity, its source produces frames immediately, so there is no clip load latency. Every deck stays ready, and you perform by choosing which ones are live in the mix, with full MIDI control over the routing.

---

## Source Types

| Source | Description |
|--------|-------------|
| ISF Shader | GLSL generator with typed parameters, hot-reload on save |
| Video | ffmpeg decode with loop/ping-pong/one-shot, speed, scrub, in/out points |
| HAP Video | GPU-native codec (BC1/BC3/BC7/YCoCg), direct GPU upload |
| Image | Still image: PNG, JPG, BMP, TIFF, TGA, WebP, or SVG vector art |
| Camera | Live webcam input, shared across multiple decks |
| NDI | Network video receive via NDI SDK |
| SRT | Secure Reliable Transport stream receive |
| HLS | HTTP Live Streaming input |
| DASH | MPEG-DASH input |
| RTMP | RTMP/RTMPS stream receive |
| Compute Shader | GLSL compute shader (`.comp`) for particle systems, simulations, and GPU-native generators |
| Syphon | macOS inter-app texture sharing (receive from other apps) |
| Spout | Windows inter-app texture sharing (receive from other apps) |
| Screen Capture | An OS display or a single application window, captured live (macOS) |
| Program Tap | Varda's own master program or a channel composite, one frame behind |
| HTML | Web page (HTML/CSS/JS) rendered by the embedded Servo browser engine |
| Solid Color | Flat RGBA color |

---

## Blend Modes

Each deck composites onto its channel using a blend mode. The same set is available for per-channel mixing with 3+ channels. There are 15 modes:

| Group | Modes |
|-------|-------|
| **Normal** | Normal |
| **Lighten** | Add, Screen, Color Dodge, Lighten |
| **Darken** | Multiply, Color Burn, Linear Burn, Darken |
| **Contrast** | Overlay, Soft Light, Hard Light |
| **Comparative** | Difference, Exclusion, Subtract |

---

## Effect Chains

Effects are ISF filter shaders applied at three levels:

1. **Deck FX**: applied to a single deck's output before channel compositing
2. **Channel FX**: applied to the composited channel output before mixing
3. **Master FX**: applied to the final mixer output before routing to surfaces

Drag effects to reorder them. Toggle each one on or off individually.

---

## Tonemapping & Color Grading

Varda works in **16-bit float linear light** (`Rgba16Float`) from the moment a source enters
a deck until each output encodes the picture for its destination.

Three stages shape the picture:

1. **Look LUT** grades the linear program first. It is global, so one grade reaches every
   output.
2. **Tonemap** maps the program into the range one output can show. It is **per output**,
   because a tonemap is an output transform: an SDR projector and an HDR10 recording need
   different tonemaps for the same program.
3. **Calibration LUT** corrects one display after its tonemap. It is bound to the output's
   resolved contract.

Each output's format (8-bit SDR, 10-bit SDR, HDR10, HLG, or EDR) is set per output. See
[Output Format](10-outputs.md#output-format).

### Tonemap

Maps the linear composite into the range its output can show. For an SDR output that range
is [0, 1]. For an HDR10 output it runs up to that output's configured peak. There are nine
presets:

| Preset | Character |
|--------|-----------|
| **Bypass** | No compression. Values >1.0 clamp at the output boundary |
| **ACES Filmic** (default) | Cinematic rolloff with warm highlight shift |
| **Reinhard** | Gentle curve, never reaches pure white |
| **Reinhard Extended** | Reinhard with configurable white point |
| **Hable Filmic** | Game-industry standard with nice toe and shoulder |
| **Uchimura (GT)** | Gran Turismo style, tunable shoulder |
| **Lottes (AMD)** | Fast, invertible, high contrast |
| **AgX** | Neutral, minimal hue shift |
| **PBR Neutral** | Color-accurate, minimal look modification |

Select a preset in the **🎨 Tonemap** section of the right panel, under the main output preview, or with `PUT /api/mixer/tonemap`. This sets the show-wide curve used by every output and the previews.

Any output can override it on its own card, for example to grade a projector and a master recording differently. See [Per-output tonemap](10-outputs.md#per-output-tonemap).

Only **Bypass** and **Reinhard Extended** have defined HDR forms. The other curves have shoulders fitted to an SDR target, so an HDR output uses Bypass in their place. The output card reports the substitution.

### 3D LUTs: two slots

Both slots take `.cube` and `.3dl` files, including 1D shaper LUTs for shadow precision. Put the files in `.varda/luts/` and they appear in the **🎨 Tonemap** panel. Both slots persist across sessions.

**Look LUT** is your show's grade. It runs on scene-linear light *before* the tonemap, so it reaches every output, including HDR ones. The lookup is encoded to **ACEScct** first (the log curve ACES specifies for look transforms), because linear light spends almost all its range on highlights. A `.cube` authored against ACEScct in Resolve or Nuke gives the result its author intended.

**Calibration LUT** corrects a specific display. It runs *after* the tonemap, on the display-referred signal. It is calibrated against one output transform, and after a different transform its midtones would land in the wrong place. For that reason it is **not applied to HDR outputs**, and the output card names the LUT it skipped.

If you only need one LUT, use the Look slot. Use the Calibration slot when one projector needs correcting and the rest do not.

---

## Modulation

Any numeric parameter in the hierarchy can be automated by modulation sources:

| Source | Description |
|--------|-------------|
| **LFO** | 6 waveforms (sine, triangle, saw, square, random, smooth random), configurable frequency, amplitude, phase |
| **Audio Band** | Bass, mid, or treble energy from FFT analysis for driving parameters with the music |
| **ADSR Envelope** | Attack/Decay/Sustain/Release envelope, triggered manually or via MIDI |
| **Step Sequencer** | N-step pattern at configurable rate, with interpolation modes |
| **Analyzer** | Scalar outputs derived from analysis of a deck's input frame (e.g. brightness, contrast) |

Create sources in the modulation panel and assign them to any parameter with its **〰** button. Several sources can target the same parameter; their values are summed. Modulators can modulate other modulators up to 4 levels deep, for example an LFO modulating the frequency of another LFO. See [Modulation & Audio Reactivity](06-modulation.md) for the assignment workflow.

Parameter paths use the format `deck/<uuid>/param/<name>`, `crossfader`, `ch/<uuid>/opacity`, etc.

---

## Copy and Paste

Right-click a deck, an effect card, or a channel (its header, or the empty space under its decks) for **Copy**, **Duplicate**, and **Paste**. `Cmd+C`, `Cmd+V`, and `Cmd+D` do the same thing to whatever is currently selected.

A copy is independent of the original. Renaming, remapping, or deleting one leaves the other alone.

| What travels | What does not |
|---|---|
| Every parameter value, the effect chain, and its settings | MIDI, keyboard, and OSC mappings, so one knob never silently drives two decks |
| Modulation assignments, still driven by the *same* LFO, envelope, or sequencer, so the copy moves with the original | Macro targets, for the same reason |
| Automation curves, cloned so each lane can be edited on its own | |
| Arrangement regions, but only when the copy is made in Arrangement mode | |

A deck copied in the mixer has no arrangement regions. A deck copied in Arrangement mode keeps its regions, so it plays at the same times as the original.

A paste lands directly after the item you right-clicked, or at the end when you use the container's own menu. A channel is pasted as a new channel at the end of the mixer.

Copy and paste gives the same result as a preset, without a name or a file. Use a [preset](04-performance.md#presets) when something should outlive the session.

---

## The Varda workspace

Varda uses the current working directory as a workspace. All state lives in a `.varda/` directory, which Varda creates automatically:

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

Run Varda from different directories to keep separate workspaces per show, venue, or project. Each workspace has its own scene, stage layout, and MIDI mappings.

Save with **Cmd+S**. Varda also saves automatically on a clean exit. To load your setup at another venue or share it, copy the entire `.varda/` directory. The scene (your show) is stored separately from the stage (the venue's physical layout).

---

[← Prev: Getting Started](01-getting-started.md) · [Home](README.md) · [Next: Library Panel →](03-library-panel.md)
