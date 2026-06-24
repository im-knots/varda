# Modulation & Audio Reactivity

Any numeric parameter in Varda can be automated by one or more modulation sources. You **create** sources in the modulation panel (right sidebar) and **assign** them to parameters with the `〰` button next to any slider. Multiple sources targeting the same parameter are summed additively.

## Creating Sources

The modulation panel (right sidebar) has a row of buttons that add a new source instantly:

- **➕ LFO**
- **➕ Audio**
- **➕ ADSR**
- **➕ StepSeq**

Each new source appears as a card in the list below, named by type and index (e.g. **LFO 1**, **Audio 1**), with a live value readout in the header and an **x** button to delete it. Adjust the source's parameters directly on its card. (The **Analyzer** source is added from a deck's analyzer setup rather than this button row — see [Analyzer](#analyzer).)

Each source is automatically assigned a **color** from a fixed palette (cyan, magenta, yellow, lime, orange, pink, sky blue, coral). That color identifies the source everywhere it is used — including the ghost indicator on modulated sliders.

## Modulation Sources

### LFO

A low-frequency oscillator that cycles through a waveform continuously.

| Setting | Range | Description |
|---------|-------|-------------|
| **Waveform** | Sine, Triangle, Sawtooth, Square, Random, Smooth Random | Shape of the cycle |
| **Frequency** | 0.01–10+ Hz | How fast the LFO cycles |
| **Amplitude** | 0.0–1.0 | How wide the sweep is (fraction of parameter range) |
| **Phase** | 0.0–1.0 | Offset in the cycle (0.5 = start halfway through) |
| **Bipolar** | on/off | Off: output 0–1 (unipolar). On: output -1 to +1 (bipolar) |

**Random** waveform produces sample-and-hold noise — a new random value each quarter-cycle, held constant until the next. **Smooth Random** interpolates between random values for organic, non-repeating motion.

### Audio

Drives a parameter from frequency-band energy in the audio input. Connects visuals directly to the music.

| Setting | Range | Description |
|---------|-------|-------------|
| **Frequency Range** | 20–20,000 Hz | Low and high bounds of the frequency band to analyze |
| **Gain** | 0.0–10.0 | Boost the signal for quiet sources |
| **Smoothing** | 0.0–0.99 | Release speed — 0 = instant response, 0.99 = slow decay |
| **Noise Gate** | 0.0–1.0 | Signals below this threshold are muted (default: 0.1) |
| **Mode** | Direct, Increase, Decrease | How energy maps to output (see below) |

**Presets** for quick setup:

| Preset | Frequency Range | Use |
|--------|----------------|-----|
| **Low (Bass)** | 20–250 Hz | Kick drums, bass lines |
| **Mid** | 250–2,000 Hz | Vocals, snare, guitar |
| **High (Treble)** | 2,000–20,000 Hz | Cymbals, hi-hats, presence |
| **Full** | 20–20,000 Hz | Overall energy level |

**Modes:**

- **Direct** — output tracks audio energy in real-time. Instant attack, smoothing controls release.
- **Increase** — audio energy accumulates the value upward (wraps at 1.0). Creates ratcheting effects.
- **Decrease** — audio energy accumulates the value downward (wraps at 0.0). Inverse ratchet.

**Audio Device**: each Audio source has a **device dropdown** to select which audio input to analyze. Different sources can use different devices — for example, one tracking the DJ mixer's bass and another tracking a microphone's treble.

### ADSR Envelope

A classic attack/decay/sustain/release envelope, triggered by a gate signal.

| Stage | Description |
|-------|-------------|
| **Attack** | Time to ramp from 0 to peak (≥0.001s) |
| **Decay** | Time to fall from peak to sustain level (≥0.001s) |
| **Sustain** | Level held while gate is on (0.0–1.0) |
| **Release** | Time to fall from sustain to 0 after gate off (≥0.001s) |

**Gate trigger**: click the gate button in the modulation panel, or map it to a MIDI note/button. Gate on starts Attack; gate off starts Release.

```
Level
1.0 ─────┐
         │╲
         │  ╲───── Sustain
         │        ╲
0.0 ─────┘         ╲────
     Attack Decay   Release
```

### Step Sequencer

An N-step pattern that cycles at a configurable rate.

| Setting | Range | Description |
|---------|-------|-------------|
| **Steps** | 2+ values | Each step is a value from 0.0 to 1.0 |
| **Rate** | 0.01+ Hz | Steps per second (MIDI-mappable) |
| **Interpolation** | None, Linear, Smooth | Blending between adjacent steps |
| **Bipolar** | on/off | Off: output 0–1. On: output -1 to +1 |

**Interpolation modes:**

- **None** — hard steps, instant value changes
- **Linear** — straight-line blend between adjacent steps
- **Smooth** — cubic smoothstep (ease in/out between steps)

Individual step values are addressable via MIDI at `mod/<idx>/step/<step_idx>`.

### Analyzer

Drives a parameter from **analysis of a deck's live input frame**. Instead of a synthetic or audio-derived signal, the source value comes from measuring the picture itself — its brightness, contrast, or color balance — turning the visuals into a controller for other parameters.

An analyzer runs on a background thread at its own cadence (it never blocks the render loop) and publishes normalized scalar outputs (0.0–1.0) that feed the modulation engine like any other source.

| Setting | Range | Description |
|---------|-------|-------------|
| **Analyzer Type** | see below | Which analyzer to run on the deck |
| **Output** | analyzer-specific | Which scalar value to read |
| **Deck** | any deck | The deck whose input frame is analyzed |
| **Smoothing** | 0.0–0.99 | Damps jitter — 0 = instant, 0.99 = heavy smoothing |

**Built-in analyzer: `brightness`** (always available, CPU-only, no ML):

| Output | Description |
|--------|-------------|
| `brightness` | Average luminance (Rec.709) |
| `contrast` | Standard deviation of luminance |
| `red` / `green` / `blue` | Average per-channel value |

**Optional analyzer: `face_detect`** — available in builds compiled with the `face-detection` feature. It uses an ONNX model to expose face position and size outputs. When the feature isn't compiled in, only `brightness` appears in the picker.

Multiple modulation sources can share one running analyzer on a deck (it is reference-counted), so mapping several outputs costs only one analysis pass.

---

## Routing

### Assigning a Source to a Parameter

Every modulatable parameter slider — deck/generator parameters, effect parameters, and even another source's parameters — has a small **`〰`** button beside it. To wire up modulation:

1. Click the **`〰`** button. A dropdown headed **"Assign Modulation"** opens.
2. Pick a source from the list. Each entry is labeled by type and index and shown in the source's color — for example **LFO 1**, **Audio 20-250Hz**, **ADSR 1**, **StepSeq 1**, **Analyzer brightness 1**.
3. The assignment is live immediately.

To **remove** an assignment, open the same `〰` dropdown and click **Clear**.

#### Live Ghost Indicator

Once a parameter is modulated, a thin **vertical line in the source's color** is drawn across the slider. It marks the *effective* value (base value + combined modulation offset) and moves in real time as the modulation evolves — so you can see exactly where a parameter is being driven without watching the number. With several sources on one parameter, the line shows their combined effect.

> Behind the scenes, assignments map to the same parameter paths as MIDI/OSC (`deck/<uuid>/param/<name>`, `crossfader`, `ch/<uuid>/opacity`, `fx/<uuid>/param/<name>`, etc. — see [Parameter Paths](06-control-surfaces.md#parameter-paths)). The UI assigns each modulation at a sensible default depth; fine-grained per-assignment **amount** (a signed scale where negative inverts) is exposed through the [HTTP API](13-api.md) rather than the slider dropdown.

Deck **video playback** (play, speed, seek, in/out points, loop mode) and **source scaling mode** are modulatable too, since they share the same parameter router. An LFO can scrub `seek`, an audio band can gate `play`, and discrete targets (`loop_mode`, `scaling_mode`) step through their options via fader bucketing. As with `mute`/`solo`, choose musically sensible sources for these.

### Stacking Multiple Sources

Multiple sources can target the same parameter. Their contributions are summed before being applied:

```
effective_offset = source_1_value × amount_1 + source_2_value × amount_2 + ...
effective_value  = clamp(base_value + effective_offset × param_range, param_min, param_max)
```

Example: an LFO plus an audio-bass source on the same brightness parameter produces a pulsing glow that also reacts to the kick drum.

### Per-Component Modulation

Color parameters (vec4) support per-component modulation — assign a source to just the red, green, blue, or alpha channel independently.

---

## Modulator-on-Modulator

Modulation source parameters are themselves modulatable. This enables complex, evolving behaviors without manual control.

### How It Works

Each source type exposes modulatable parameters:

| Source | Modulatable Parameters |
|--------|----------------------|
| **LFO** | frequency, phase, amplitude |
| **Audio** | gain, smoothing |
| **ADSR** | attack, decay, sustain, release |
| **Step Sequencer** | rate |

To wire one source into another, use the **`〰`** button on the target source's parameter (the same gesture as parameter assignment). The dropdown is headed **"Modulate [parameter]"**; pick a source with the **+ [source name]** entry, or click **x Remove** (red) to detach it.

### Depth Limit

Mod-on-mod chains are limited to **4 levels deep** to prevent infinite loops. The engine evaluates sources in topological dependency order — sources with no inputs first, then those that depend on them, and so on. Chains deeper than the limit (or accidental cycles) are evaluated safely on a fallback pass rather than crashing or hanging.

### Examples

- **LFO frequency ← slow LFO**: A 0.1 Hz LFO modulates a faster LFO's frequency, creating non-repeating patterns
- **LFO amplitude ← audio bass**: Bass energy controls how wide the LFO sweeps — subtle at low volume, dramatic at high
- **Step sequencer rate ← audio bass**: The sequence speeds up with the kick drum

---

## Audio System

### FFT Analysis

Varda runs a 2048-point FFT on the audio input at 48 kHz, producing 1024 magnitude bins with ~23 Hz/bin resolution. A Hann window is applied before analysis.

### Beat Detection

Beats are detected via **spectral flux onset detection**:

1. Compute the transient energy increase across all frequency bins each frame
2. Compare against an adaptive threshold (median of recent flux values)
3. Reject double-triggers within 200ms

BPM is estimated from the last 16 beat intervals, with outlier rejection (>15% deviation from median discarded) and EMA smoothing.

### ISF Audio Uniforms

All shaders receive audio data automatically — no setup required:

| Uniform | Description |
|---------|-------------|
| `audio_level` | Overall RMS level (0.0–1.0) |
| `audio_bass` | Energy in 20–250 Hz band (0.0–1.0) |
| `audio_mid` | Energy in 250–2,000 Hz band (0.0–1.0) |
| `audio_treble` | Energy in 2,000–20,000 Hz band (0.0–1.0) |
| `audio_bpm` | Detected BPM (0.0 if unavailable) |
| `audio_beat_phase` | Phase within current beat cycle (0.0–1.0, 0.0 = on beat) |

Use these directly in ISF shaders for audio-reactive visuals without needing the modulation engine. See [ISF Authoring](12-isf-authoring.md) for shader writing details.

---

[← Prev: Performance & Automation](04-performance.md) · [Home](README.md) · [Next: Control Surfaces →](06-control-surfaces.md)
