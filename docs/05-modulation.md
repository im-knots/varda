# Modulation & Audio Reactivity

Any numeric parameter in Varda can be automated by one or more modulation sources. You **create** sources in the modulation panel (right sidebar) and **assign** them to parameters with the `〰` button next to any slider. When several sources target the same parameter, their values are added together.

## Creating Sources

The modulation panel (right sidebar) has a row of buttons. Each one adds a new source immediately:

- **➕ LFO**
- **➕ Audio**
- **➕ ADSR**
- **➕ StepSeq**

Each new source appears as a card in the list below. The card is named by type and index (e.g. **LFO 1**, **Audio 1**) and has a live value readout in its header and an **x** button that deletes it. Adjust the source's settings on its card. The **Analyzer** source is added from a deck's analyzer setup, not from this button row (see [Analyzer](#analyzer)).

Each source gets a **color** from a fixed palette (cyan, magenta, yellow, lime, orange, pink, sky blue, coral). That color identifies the source everywhere it is used.

## Timebase

LFOs and step sequencers have a **timebase**, which sets the clock their rate is measured against. The selector is in the source card's header. Audio, ADSR, and Analyzer sources have no selector, because they follow their input instead of a clock.

| Timebase | Rate is read as | Use |
|----------|-----------------|-----|
| **Free** (default) | cycles per second (Hz) | Motion that runs regardless of what the music does |
| **Beat** | cycles per **beat** | Motion locked to tempo |
| **Show** | cycles per second of show position | Motion that must land the same way every performance |

On the Beat timebase, a rate of `1.0` is one cycle per beat, `0.25` is one cycle per bar in 4/4, and `4.0` is four cycles per beat. When the tempo changes, every beat-locked source follows it with no change to its settings.

Beat time comes from the resolved clock (MIDI clock, OSC, or detected audio tempo; see [Control Surfaces](06-control-surfaces.md)). It resets to zero on MIDI Start.

**If no clock source is active, a Beat-locked source freezes at its last value** and the card shows a ⚠ marker. It does not fall back to free-running, so a lost clock shows up as a frozen source instead of a show that silently drifts out of sync.

The **Show** timebase follows the transport position described in [Control Surfaces](06-control-surfaces.md#transport). Its value depends only on that position. A Show source gives the same value at 00:04:12 tonight as it did in yesterday's rehearsal, however you got there. When you rewind the transport, the source rewinds with it.

A Show-locked source also **freezes** while the transport is not moving, including before the transport has ever been started. If the transport never runs, nothing on the Show timebase moves.

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

The **Random** waveform produces sample-and-hold noise: a new random value each quarter-cycle, held until the next. **Smooth Random** interpolates between random values for smooth, non-repeating motion.

**Unipolar vs. bipolar** changes where the sweep sits. Unipolar sweeps upward from the slider's current position. Bipolar sweeps equally above and below it. At the same amplitude both cover the same distance, so switching polarity re-centers the motion without changing its width. For a unipolar sweep, park the slider at the bottom. For a bipolar sweep, park it in the middle.

### Audio

Drives a parameter from the energy in one frequency band of the audio input.

| Setting | Range | Description |
|---------|-------|-------------|
| **Frequency Range** | 20–20,000 Hz | Low and high bounds of the frequency band to analyze |
| **Gain** | 0.0–10.0 | Boost the signal for quiet sources |
| **Smoothing** | 0.0–0.99 | Release speed: 0 = instant response, 0.99 = slow decay |
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

- **Direct**: output tracks audio energy in real time. Attack is instant; **Smoothing** sets the release.
- **Increase**: audio energy pushes the value upward, wrapping at 1.0. Use it for ratcheting effects.
- **Decrease**: audio energy pushes the value downward, wrapping at 0.0. The inverse ratchet.

**Audio Device**: each Audio source has a **device dropdown** that selects which audio input it analyzes. Different sources can use different devices, for example one tracking the DJ mixer's bass and another tracking a microphone's treble.

### ADSR Envelope

An attack/decay/sustain/release envelope, triggered by a gate signal.

| Stage | Description |
|-------|-------------|
| **Attack** | Time to ramp from 0 to peak (≥0.001s) |
| **Decay** | Time to fall from peak to sustain level (≥0.001s) |
| **Sustain** | Level held while gate is on (0.0–1.0) |
| **Release** | Time to fall from sustain to 0 after gate off (≥0.001s) |

**Gate trigger**: click the gate button in the modulation panel, or map it to a MIDI note or button. Gate on starts Attack. Gate off starts Release.

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

- **None**: hard steps, instant value changes
- **Linear**: straight-line blend between adjacent steps
- **Smooth**: cubic smoothstep (ease in and out between steps)

Individual step values are addressable via MIDI at `mod/<idx>/step/<step_idx>`.

### Analyzer

Drives a parameter from **measurements of a deck's live input frame**, such as its brightness, contrast, or color balance. Use it to let one deck's picture control other parameters.

An analyzer runs on a background thread at its own rate and never blocks the render loop. It publishes normalized scalar outputs (0.0–1.0) that feed the modulation engine like any other source.

| Setting | Range | Description |
|---------|-------|-------------|
| **Analyzer Type** | see below | Which analyzer to run on the deck |
| **Output** | analyzer-specific | Which scalar value to read |
| **Deck** | any deck | The deck whose input frame is analyzed |
| **Smoothing** | 0.0–0.99 | Damps jitter: 0 = instant, 0.99 = heavy smoothing |

**Built-in analyzer: `brightness`** (always available, CPU-only, no ML):

| Output | Description |
|--------|-------------|
| `brightness` | Average luminance (Rec.709) |
| `contrast` | Standard deviation of luminance |
| `red` / `green` / `blue` | Average per-channel value |

**Optional analyzer: `face_detect`** is available in builds compiled with the `face-detection` feature. It exposes `face_x`, `face_y`, `face_size`, `face_rotation`, and `face_count`. In builds without the feature, only `brightness` appears in the picker.

Several modulation sources can share one running analyzer on a deck (it is reference-counted), so mapping several outputs costs one analysis pass.

> The same engine also feeds depth and face textures to shaders. For the full subsystem (complete output tables, the depth sensor, lifecycle, and the HTTP API) see [Frame Analysis & Preprocessors](14-frame-analysis.md).

---

## Routing

### Assigning a Source to a Parameter

Every modulatable parameter slider has a small **`〰`** button beside it. To assign modulation:

1. Click the **`〰`** button. A **checklist** of every source opens. Each source is labeled by type and index and shown in its own color, for example **LFO 1**, **Audio 20-250Hz**, **ADSR 1**, **StepSeq 1**, **Analyzer brightness 1**.
2. **Tick** a source (`☐` → `☑`). The assignment takes effect immediately.
3. Tick another source to stack it. The list stays open, so you can assign several sources in one visit.

The checklist is the only place that shows **every** source driving a parameter. The ghost line and the colored label on the slider use the color of the first assignment, so one source and three sources look the same on the slider. Hover the `〰` button to see the names of the active sources without opening the list.

To **remove** one source, un-tick it (`☑` → `☐`). The other sources keep driving the parameter. **Clear all**, at the bottom of the list, removes every source at once. It appears only when at least one source is assigned.

The same dropdown offers **＋ Automation lane**, which draws the parameter as a curve against show position. See [Automation Curves](#automation-curves).

#### Live Ghost Indicator

When a parameter is modulated, a thin **vertical line in the source's color** is drawn across the slider. It marks the *effective* value (base value + combined modulation offset) and moves in real time. With several sources on one parameter, the line shows their combined effect in the color of the first source. Open the `〰` checklist to see which sources are assigned.

> Assignments use the same parameter paths as MIDI and OSC (`deck/<uuid>/param/<name>`, `crossfader`, `ch/<uuid>/opacity`, `fx/<uuid>/param/<name>`, etc.; see [Parameter Paths](06-control-surfaces.md#parameter-paths)). The UI assigns each modulation at a default depth. The per-assignment **amount** (a signed scale; negative values invert) is set through the [HTTP API](13-api.md). The slider dropdown does not show it.

**Channel faders** have their own `〰`, so an LFO or a recorded curve can sweep a whole channel without changing the decks inside it. The crossfader is not a modulation target. You can still map it and drive it from macros.

#### Video Playback

A clip's **speed**, **playhead**, **play state**, and **loop mode**, and any deck's **source scaling mode**, each have their own `〰`. For example, an LFO can time-warp a clip, an audio band can gate its play state, and a drawn curve can scrub its playhead.

Speed and playhead are the two continuous parameters, and they behave differently:

- **Speed** is a multiplier from 0.1× to 4×, and it is cheap to modulate. The clip's position advances at the current speed, so modulating speed gives smooth time-warping with no seeking. Speed never goes negative, so a modulator cannot reverse a clip. Use Ping-Pong for reverse playback.
- **Playhead** modulation is an offset from where the clip would otherwise be, measured against the active loop region. On a four-bar loop, an LFO moves the playhead within those four bars. The same patch therefore behaves the same way on short and long clips.
- A **drawn curve** on the playhead sets the position directly instead of offsetting it. It reads against the whole clip, like the scrub bar and a MIDI-mapped seek: halfway up the lane is halfway through the clip. While a curve controls the playhead, loop and ping-pong transitions are suspended.

The playhead offset is measured from where the clip would otherwise be, so it behaves differently when the clip is paused and when it is playing:

- **Paused**: the clip does not advance, so the offset is measured from a fixed point. Park the playhead mid-clip and assign a bipolar LFO. The playhead swings back and forth around that point by the amplitude you set. The scrub bar's ghost line marks the center of the swing.
- **Playing**: the clip advances by its own loop, ping-pong, or one-shot rules, and the modulator offsets from the current position. The same LFO produces a wobble that moves forward through the clip.

Speed has no effect while the clip is paused, because speed scales the clip's advance and a paused clip does not advance.

Playhead modulation can be expensive. Video decoders run forward. A forward nudge decodes a few extra frames, but a backward one flushes the decoder and seeks. Gentle modulation costs nothing. A hard square wave seeks on every backward jump. All-intra formats (**HAP**, ProRes) handle this well. Long-GOP H.264 is where you will notice it. The cost depends on how hard you modulate, not on your frame rate.

**In and out points**, and **clear**, cannot be modulated. In and out points define the loop region that the playhead offset is measured against, so modulating them would move that reference every frame. You can still MIDI-map them, address them over OSC, and drive them from macros.

Other rules for playback parameters:

- **Play, loop mode, and scaling mode are set directly.** A modulator assigned to one of them sets its value instead of adding to it. `play` uses a threshold with a deadband around the middle, so a source hovering near the middle holds the current state instead of stuttering. Loop mode and scaling mode step through their options by [fader bucketing](04-performance.md#video-playback), so a continuous source switches them rapidly. Choose a source whose timing fits the music.
- **Chase overrides speed and playhead modulation.** A deck chasing the transport takes its whole timeline from the transport. Its speed and playhead assignments are ignored while it chases, and the deck panel names whichever of them you have assigned. The speed *slider* still works, because a fixed rate keeps a stable relationship to the show ("this clip runs at twice show rate"). A changing rate does not, because the chasing clip's position is computed from show position. Set **Chase** to **Never** for audio-reactive time-warping. See [Arrangement](15-arrangement.md).
- **Manual control overrides a curve.** Touching the scrub bar, speed slider, play button, loop buttons, or scaling combo takes that parameter back from the curve driving it, with no confirmation. The automation row shows an amber dot while you hold it. Click the dot to give control back to the curve. Deck and channel faders work the same way. This applies whether the gesture comes from the bottom bar, a MIDI controller, or the API. With **⏺** armed and the transport running, the gesture is recorded instead. See [Arrangement](15-arrangement.md).

### Stacking Multiple Sources

Several sources can target the same parameter. Their contributions are summed before being applied:

```
effective_offset = source_1_value × amount_1 + source_2_value × amount_2 + ...
effective_value  = clamp(base_value + effective_offset × param_range, param_min, param_max)
```

Example: an LFO plus an audio-bass source on the same brightness parameter gives a pulsing glow that also reacts to the kick drum.

### Per-Component Modulation

Colors and 2D points are modulated one channel or axis at a time. Next to a color swatch are small **r g b a** labels, and next to a point **x y**. Each label is its own parameter: it has its own `〰` menu and automation lane, and in learn mode you click it to map a MIDI fader or key to that one channel.

The paths add the channel to the parameter's path: `deck/<uuid>/color/r`, `deck/<uuid>/param/tint/g`, `deck/<uuid>/position/x`. This works for shader parameters and deck source controls alike.

---

## Automation Curves

An automation curve is a drawn shape that sets a parameter's value at each point in show position. The same value plays at 00:04:12 in every run.

### Adding a Lane

Open the **`〰`** dropdown on any modulatable parameter and pick **＋ Automation lane**. This creates the curve, locks it to the **Show** timebase, and assigns it to the parameter. The lane starts empty. An empty lane has no effect, so the parameter behaves normally until you draw the first point.

You draw curves in Arrangement mode, where each lane sits under its channel. Curves do **not** appear as cards in the modulation panel, because a show can have hundreds of them.

### Recording a Curve

You can also record a curve. Arm **⏺** in the transport strip or the top bar, then move any control while the show runs. Your movement is written into the arrangement as a curve. If the parameter has no lane, one is created. See [Recording a pass](15-arrangement.md#recording-a-pass).

### One Curve, One Parameter

A curve belongs to the parameter it was drawn for. The `〰` dropdown does not list existing curves as sources for other parameters.

To reuse a shape, copy and paste it between lanes. Right-click the lane, choose **Copy curve**, then choose **Paste curve** at the point on the other parameter's lane where the shape should start. Each paste is an independent copy, so editing one lane does not change the other. See [Reusing a shape](15-arrangement.md#reusing-a-shape).

### Curves Replace the Value

LFOs, audio bands, and the other sources are **added** to the fader's current position. An automation curve **replaces** it. A curve drawn to 40% plays back at 40% wherever the fader was left, so the arrangement plays back the same way every time.

Breakpoint values are always 0–100% of the parameter's range. On a parameter that runs from -5 to 5, 40% is -1. Copying a curve to a different parameter keeps its shape, not its raw values.

### Stacking a Curve With Live Modulation

You can still assign an LFO or an audio band to an automated parameter. The curve sets the value and the live sources are added on top:

```
value = curve_value + lfo_offset + audio_offset
```

For example, an automated opacity ramp with a bass band stacked on it follows the scheduled shape and also reacts to the music.

If two curves are assigned to one parameter, the last one assigned wins.

### Segment Shapes

Each breakpoint sets the shape of the segment leading to the next one:

| Shape | Behavior |
|---|---|
| **Step** | Holds this value, then jumps at the next breakpoint. Good for switches and discrete states. |
| **Linear** | Straight line. A **tension** control bends it: negative eases in (slow start), positive eases out (fast start). |
| **Smooth** | An S-curve that leaves and arrives gently. Matches the step sequencer's smooth mode. |

### Before and After the Curve

Outside the drawn range, a curve **holds** its first and last values. It does not drop to zero, so automated parameters keep their edge values before and after the section you arranged.

### Locating and Looping

A curve's value depends only on show position. Locating to a point gives the same result whether you played there, jumped there, or looped back to it. There is no resync period after a jump. The same applies to timecode chases.

---

## Modulator-on-Modulator

Modulation source parameters can themselves be modulated. Use this for evolving behavior that needs no manual control.

### How It Works

Each source type has these modulatable parameters:

| Source | Modulatable Parameters |
|--------|----------------------|
| **LFO** | frequency, phase, amplitude |
| **Audio** | gain, smoothing |
| **ADSR** | attack, decay, sustain, release |
| **Step Sequencer** | rate |

To route one source into another, click the **`〰`** button on the target source's parameter, as you would for any parameter. The checklist is headed **"Modulate [parameter]"** and works the same way: tick a source to attach it, un-tick it to detach only that source, or click **Clear all** to detach every source. A source is not listed against its own parameters, because a source cannot modulate itself.

### Depth Limit

Mod-on-mod chains are limited to **4 levels deep** to prevent infinite loops. The engine evaluates sources in dependency order: sources with no inputs first, then the sources that depend on them, and so on. Chains deeper than the limit, and accidental cycles, are evaluated on a fallback pass, so they do not crash or hang Varda.

### Examples

- **LFO frequency ← slow LFO**: a 0.1 Hz LFO modulates a faster LFO's frequency, creating non-repeating patterns
- **LFO amplitude ← audio bass**: bass energy controls how wide the LFO sweeps, subtle at low volume and large at high volume
- **Step sequencer rate ← audio bass**: the sequence speeds up with the kick drum

---

## Audio System

### FFT Analysis

Varda runs a 2048-point FFT on the audio input at 48 kHz, producing 1024 magnitude bins with ~23 Hz/bin resolution. A Hann window is applied before analysis.

### Beat Detection

Beats are detected with **spectral flux onset detection**:

1. Compute the transient energy increase across all frequency bins each frame
2. Compare it against an adaptive threshold (median of recent flux values)
3. Reject double-triggers within 200ms

BPM is estimated from the last 16 beat intervals. Outliers (>15% deviation from the median) are discarded, and the result is smoothed with an EMA.

### ISF Audio Uniforms

All shaders receive audio data automatically, with no setup:

| Uniform | Description |
|---------|-------------|
| `audio_level` | Overall RMS level (0.0–1.0) |
| `audio_bass` | Energy in 20–250 Hz band (0.0–1.0) |
| `audio_mid` | Energy in 250–2,000 Hz band (0.0–1.0) |
| `audio_treble` | Energy in 2,000–20,000 Hz band (0.0–1.0) |
| `audio_bpm` | Detected BPM (0.0 if unavailable) |
| `audio_beat_phase` | Phase within current beat cycle (0.0–1.0, 0.0 = on beat) |

Use these in ISF shaders for audio-reactive visuals without the modulation engine. See [ISF Authoring](12-isf-authoring.md) for shader writing details.

---

[← Prev: Performance & Automation](04-performance.md) · [Home](README.md) · [Next: Control Surfaces →](06-control-surfaces.md)
