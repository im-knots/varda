# Modulation & Audio Reactivity

Any numeric parameter in Varda can be automated by one or more modulation sources. You **create** sources in the modulation panel (right sidebar) and **assign** them to parameters with the `〰` button next to any slider. Multiple sources targeting the same parameter are summed additively.

## Creating Sources

The modulation panel (right sidebar) has a row of buttons that add a new source instantly:

- **➕ LFO**
- **➕ Audio**
- **➕ ADSR**
- **➕ StepSeq**

Each new source appears as a card in the list below, named by type and index (e.g. **LFO 1**, **Audio 1**), with a live value readout in the header and an **x** button to delete it. Adjust the source's parameters directly on its card. (The **Analyzer** source is added from a deck's analyzer setup rather than this button row — see [Analyzer](#analyzer).)

Each source is automatically assigned a **color** from a fixed palette (cyan, magenta, yellow, lime, orange, pink, sky blue, coral). That color identifies the source everywhere it is used.

## Timebase

LFOs and step sequencers carry a **timebase** — the notion of time they run on. The selector sits in
the source card's header. Audio, ADSR, and Analyzer sources have no selector: an envelope follower
tracks the room, not a clock.

| Timebase | Rate is read as | Use |
|----------|-----------------|-----|
| **Free** (default) | cycles per second (Hz) | Motion that runs regardless of what the music does |
| **Beat** | cycles per **beat** | Motion locked to tempo |
| **Show** | cycles per second of show position | Motion that must land the same way every performance |

On the Beat timebase a rate of `1.0` is one cycle per beat, `0.25` is one cycle per bar in 4/4, and
`4.0` is four cycles per beat. Because rate is measured in beats, a tempo change retunes every
beat-locked source without you touching a single setting.

Beat time comes from the resolved clock (MIDI clock, OSC, or detected audio tempo — see
[Control Surfaces](06-control-surfaces.md)), and resets to zero on MIDI Start.

**If no clock source is active, a Beat-locked source freezes at its last value** and the card shows a
⚠ marker. It does not fall back to running freely: a modulator holding its last look is obvious and
fixable, whereas a silent fallback would look like everything was fine while the show drifted out of
sync.

The **Show** timebase reads the transport, the absolute position described in
[Control Surfaces](06-control-surfaces.md#transport). Its value is a pure function of that position,
so a source on the Show timebase produces the same value at 00:04:12 tonight as it did in yesterday's
rehearsal, no matter how you got there. Rewind and it rewinds with you.

Like Beat, a Show-locked source **freezes rather than free-running** when the transport is not
moving, including before it has ever been started. That is what keeps a cold start honest: if the
transport never runs, nothing on the Show timebase moves, rather than everything quietly drifting
from a position the show never reached.

## Modulation Sources

### LFO

A low-frequency oscillator that cycles through a waveform continuously.

| Setting | Range | Description |
|---------|-------|-------------|
| **Waveform** | Sine, Triangle, Sawtooth, Square, Random, Smooth Random | Shape of the cycle |
| **Frequency** | 0.01–10+ | How fast the LFO cycles, in Hz or in cycles per beat depending on the [timebase](#timebase) |
| **Amplitude** | 0.0–1.0 | How wide the sweep is (fraction of parameter range) |
| **Phase** | 0.0–1.0 | Offset in the cycle (0.5 = start halfway through) |
| **Bipolar** | on/off | Off: output 0–1 (unipolar). On: output -1 to +1 (bipolar) |

**Random** waveform produces sample-and-hold noise (a new random value each quarter-cycle, held constant until the next). **Smooth Random** interpolates between random values for organic, non-repeating motion.

**Unipolar vs. bipolar** changes where the sweep sits, not how far it travels. Unipolar sweeps upward from the slider's current position; bipolar sweeps symmetrically around it, half above and half below. At the same amplitude both cover the same distance, so switching polarity re-centres the motion without making it wider or narrower. Park the slider at the bottom for a unipolar sweep, and in the middle for a bipolar one.

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

**Audio Device**: each Audio source has a **device dropdown** to select which audio input to analyze. Different sources can use different devices. for example: one tracking the DJ mixer's bass and another tracking a microphone's treble.

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
| **Rate** | 0.01+ | Steps per second, or steps per beat on the Beat [timebase](#timebase) (MIDI-mappable) |
| **Interpolation** | None, Linear, Smooth | Blending between adjacent steps |
| **Bipolar** | on/off | Off: output 0–1. On: output -1 to +1 |

**Interpolation modes:**

- **None** — hard steps, instant value changes
- **Linear** — straight-line blend between adjacent steps
- **Smooth** — cubic smoothstep (ease in/out between steps)

Individual step values are addressable via MIDI at `mod/<idx>/step/<step_idx>`.

### Analyzer

Drives a parameter from **analysis of a deck's live input frame**. Instead of a synthetic or audio-derived signal, the source value comes from measuring the picture itself (ie. its brightness, contrast, or color balance) turning the visuals into a controller for other parameters.

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

**Optional analyzer: `face_detect`** is available in builds compiled with the `face-detection` feature. It exposes `face_x`, `face_y`, `face_size`, `face_rotation`, and `face_count`. When the feature isn't compiled in, only `brightness` appears in the picker.

Multiple modulation sources can share one running analyzer on a deck (it is reference-counted), so mapping several outputs costs only one analysis pass.

> The Analyzer source is one of two ways Varda turns a picture into data — the same engine also feeds depth/face textures to shaders. For the whole subsystem (full output tables, the depth sensor, lifecycle, and the HTTP API) see [Frame Analysis & Preprocessors](14-frame-analysis.md).

---

## Routing

### Assigning a Source to a Parameter

Every modulatable parameter slider has a small **`〰`** button beside it. To wire up modulation:

1. Click the **`〰`** button. A dropdown headed **"Assign Modulation"** opens.
2. Pick a source from the list. Each entry is labeled by type and index and shown in the source's color — for example **LFO 1**, **Audio 20-250Hz**, **ADSR 1**, **StepSeq 1**, **Analyzer brightness 1**.
3. The assignment is live immediately.

The same dropdown offers **＋ Automation lane**, which draws the parameter as a curve against show position instead — see [Automation Curves](#automation-curves).

To **remove** an assignment, open the same `〰` dropdown and click **Clear**.

#### Live Ghost Indicator

Once a parameter is modulated, a thin **vertical line in the source's color** is drawn across the slider. It marks the *effective* value (base value + combined modulation offset) and moves in real time as the modulation evolves. With several sources on one parameter, the line shows their combined effect.

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

## Automation Curves

An LFO says "keep moving." An automation curve says "be *this* at *this* moment." It is a drawn shape that sets a parameter's value as a function of show position, so the same thing happens at 00:04:12 every single run.

### Adding a Lane

Open the **`〰`** dropdown on any modulatable parameter and pick **＋ Automation lane**. That creates the curve, locks it to the **Show** timebase, and assigns it to the parameter in one step. The lane starts empty, and an empty lane does nothing at all, so the parameter keeps behaving normally until you draw the first point on it.

Curves are drawn in Arrangement mode, where each lane sits under the channel it belongs to. They do **not** appear as cards in the modulation panel: a show can easily have hundreds of them, and that panel is built for a handful of live modulators.

### One Curve, One Parameter

A curve belongs to the parameter it was drawn for, and the `〰` dropdown never lists existing curves as sources to assign elsewhere. Reuse is copy and paste between lanes instead: right-click the lane you like, **Copy curve**, then **Paste curve** at the point on the other parameter's lane where you want the shape to start. See [Reusing a shape](15-arrangement.md#reusing-a-shape).

Sharing one curve between parameters would read fine in the dropdown and then bite in the room, because editing either lane would rewrite both. Two independent copies cost a little duplication and take away that whole class of surprise.

### Curves Set the Value, They Don't Nudge It

This is the one real difference from every other source. LFOs, audio bands, and the rest are **added** to wherever you left the fader. An automation curve **replaces** it.

That is deliberate. If a curve merely nudged the fader, your arrangement would play back differently depending on where the faders happened to be when you last saved, which defeats the purpose of arranging it. A curve drawn to 40% means 40%.

Breakpoint values are always 0–100% of the parameter's range, so a curve drawn on a parameter that runs from -5 to 5 reaches -1 at 40%, and copying a curve to a different parameter keeps its shape rather than its raw number.

### Stacking a Curve With Live Modulation

You can still assign an LFO or an audio band to an automated parameter. The curve sets the value and the live sources ride on top of it:

```
value = curve_value + lfo_offset + audio_offset
```

An automated opacity ramp with a bass band stacked on it gives you a shape that is scheduled *and* still breathes with the room. That combination is the point of having both modes.

Two curves on one parameter is meaningless rather than harmful — the last one assigned wins.

### Segment Shapes

Each breakpoint chooses the shape of the segment leading to the next one:

| Shape | Behaviour |
|---|---|
| **Step** | Holds this value, then jumps at the next breakpoint. Good for switches and discrete states. |
| **Linear** | Straight line. A **tension** control bends it: negative eases in (slow start), positive eases out (fast start). |
| **Smooth** | An S-curve that leaves and arrives gently. Matches the step sequencer's smooth mode. |

### Before and After the Curve

Outside the drawn range, a curve **holds** its first and last values rather than falling to zero. A curve that collapsed at its edges would black out every automated parameter before and after the section you arranged, which is almost never what anyone wants.

### Jumping Around Is Safe

Because a curve is a pure function of position, locating to a point gives the identical result whether you played there, jumped there, or looped back to it. There is no resync, and no "wrong until it catches up" period after a jump. The same guarantee applies to timecode chases.

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

Mod-on-mod chains are limited to **4 levels deep** to prevent infinite loops. The engine evaluates sources in topological dependency order. Ie. sources with no inputs first, then those that depend on them, and so on. Chains deeper than the limit (or accidental cycles) are evaluated safely on a fallback pass rather than crashing or hanging.

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
