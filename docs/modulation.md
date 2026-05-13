# Modulation & Audio Reactivity

Any numeric parameter in Varda can be automated by one or more modulation sources. Sources are created in the **modulation panel** (right sidebar) and assigned to parameters. Multiple sources targeting the same parameter are summed additively.

## Modulation Sources

### LFO

A low-frequency oscillator that cycles through a waveform continuously.

| Setting | Range | Description |
|---------|-------|-------------|
| **Waveform** | Sine, Triangle, Sawtooth, Square, Random | Shape of the cycle |
| **Frequency** | 0.01–10+ Hz | How fast the LFO cycles |
| **Amplitude** | 0.0–1.0 | How wide the sweep is (fraction of parameter range) |
| **Phase** | 0.0–1.0 | Offset in the cycle (0.5 = start halfway through) |
| **Bipolar** | on/off | Off: output 0–1 (unipolar). On: output -1 to +1 (bipolar) |

**Random** waveform produces sample-and-hold noise — a new random value each quarter-cycle, held constant until the next.

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

---

## Routing

### Assigning Sources to Parameters

Modulation assignments use the same parameter paths as MIDI/OSC (see [Parameter Paths](control-surfaces.md#parameter-paths)):

```
deck/<uuid>/param/<name>          → shader parameter
deck/<uuid>/opacity               → deck opacity
ch/<uuid>/opacity                 → channel opacity
fx/<uuid>/param/<name>            → effect parameter
crossfader                        → mixer crossfader
```

Each assignment has an **amount** (-1.0 to 1.0). Positive amounts modulate in the normal direction; negative amounts invert the modulation. The amount scales the source's output as a fraction of the target parameter's valid range.

### Stacking Multiple Sources

Multiple sources can target the same parameter. Their contributions are summed:

```
effective_offset = source_1_value × amount_1 + source_2_value × amount_2 + ...
effective_value  = clamp(base_value + effective_offset × param_range, param_min, param_max)
```

Example: an LFO (amount 0.3) + audio bass (amount 0.5) on the same brightness parameter produces a pulsing glow that also reacts to the kick drum.

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

Assign any source to another source's parameter using the `mod:<uuid>:<param>` path format.

### Depth Limit

Mod-on-mod chains are limited to **4 levels deep** to prevent infinite loops. The engine evaluates sources in topological dependency order — sources with no inputs first, then those that depend on them, and so on.

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

Use these directly in ISF shaders for audio-reactive visuals without needing the modulation engine. See [ISF Authoring](isf-authoring.md) for shader writing details.
