# Modulation Engine

## Status: IMPLEMENTED

Relates to: [/spec/channel-routing.md](/spec/channel-routing.md), [/spec/transitions.md](/spec/transitions.md)

## Core Principle

**Any numeric parameter anywhere in the hierarchy is modulatable.** The modulation engine is a global system that applies modulation sources to parameter targets every frame.

## Parameter Addressing

Every modulatable parameter has a hierarchical address:

```
channel/a/deck/0/opacity
channel/a/deck/0/param/speed
channel/a/deck/0/effect/1/param/blur_amount
channel/a/opacity
channel/a/effect/0/param/intensity
master/effect/0/param/bloom_strength
crossfader/position
video/deck/2/playback_speed
modulation/source/3/frequency          ← modulator-on-modulator
```

## Modulation Sources

### LFO (IMPLEMENTED)
- **Waveforms**: Sine, square, triangle, saw, random (sample-and-hold)
- **Parameters**: Frequency (Hz, 0.01–10.0, logarithmic), phase offset (0.0–1.0), amplitude (0.0–1.0, scales the output range), bipolar/unipolar toggle
- **Amplitude** controls how much of the parameter range the LFO sweeps. At 1.0 (default) the LFO uses its full range; at 0.5 the sweep is halved. This lets the performer dial in subtle modulation without changing the modulation assignment amount.
- All LFO parameters (frequency, phase, amplitude) are themselves modulatable (modulator-on-modulator)

### Audio Band (IMPLEMENTED)
- **Bands**: Bass, mid, treble, full level
- **Parameters**: Smoothing factor, gain/sensitivity
- Outputs 0.0–1.0 based on audio energy in the band

### Envelope Generator (ADSR) — IMPLEMENTED
- **Stages**: Attack, Decay, Sustain level, Release
- **Trigger**: Manual gate button in UI (MIDI trigger support via Phase 4 infrastructure)
- **Parameters**: Attack time, decay time, sustain level, release time
- **Implementation**: State machine in `ModulationSource::ADSR` with `gate_on()`/`gate_off()` methods
- Useful for: one-shot parameter sweeps, rhythmic hits, reactive bursts

### Step Sequencer — IMPLEMENTED
- **Steps**: N steps (default 8), each with a value (0.0–1.0)
- **Rate**: Free-running Hz
- **Interpolation**: None (hard steps), linear, smooth (cubic smoothstep)
- **Parameters**: Rate, interpolation mode, bipolar toggle, individual step values
- **Implementation**: `ModulationSource::StepSequencer` with vertical slider UI per step
- Useful for: rhythmic parameter patterns, coordinated visual sequences

## Modulation Routing

### Multiple Sources Per Target
A single parameter can have multiple modulation sources stacked:

```
deck/0/param/speed:
  ├── LFO (sine, 0.5Hz, amount: 0.3)
  ├── Audio Bass (amount: 0.5)
  └── Step Sequencer (8 steps, beat-synced, amount: 0.2)
```

Sources are **summed** (additive), then clamped to the parameter's valid range. Each source has its own **amount** (0.0–1.0) controlling how much it contributes.

### Modulator-on-Modulator
Modulation source parameters are themselves addressable, so you can modulate them:

```
LFO_1.frequency:
  └── LFO_2 (sine, 0.1Hz, amount: 0.5)   ← LFO 2 slowly sweeps LFO 1's speed

LFO_1.amplitude:
  └── Audio Bass (amount: 0.3)            ← bass energy controls how wide the sweep is

Step_Seq_1.rate:
  └── Audio Bass (amount: 0.3)             ← bass energy changes sequencer speed
```

This enables complex evolving behaviors without math expressions. An LFO modulating another LFO's frequency creates patterns that never exactly repeat.

### Depth Limit
To prevent infinite loops: modulator-on-modulator chains are limited to a max depth (e.g., 4 levels). The engine evaluates sources in dependency order — sources with no modulation inputs are evaluated first, then sources that depend on those, etc.

## Evaluation Order

Each frame:
1. Read audio data (bass, mid, treble, level, BPM, beat phase)
2. Evaluate modulation sources in dependency order (leaves first)
3. For each parameter target, sum all assigned source contributions
4. Clamp result to parameter's valid range
5. Apply to engine state before rendering

## UI

### Modulation Panel (right sidebar)
- Located in the right sidebar, below the main output preview
- Add buttons for each source type: ➕ LFO, ➕ Audio, ➕ ADSR, ➕ StepSeq
- Each modulator displayed as a horizontal column within a scrollable area
- Per-source parameter controls (frequency, waveform, phase, amplitude for LFO; attack/decay/sustain/release + gate for ADSR; step values + rate + interpolation for StepSeq; band + smoothing for Audio)
- Matches the "effect chain column" aesthetic used in the bottom bar

### Modulator Color Coding — IMPLEMENTED
Each modulation source is assigned a unique color from a fixed palette:

| Index | Color | RGB |
|---|---|---|
| 0 | Cyan | (0, 220, 220) |
| 1 | Magenta | (220, 60, 220) |
| 2 | Yellow | (220, 200, 40) |
| 3 | Lime | (100, 220, 60) |
| 4 | Orange | (240, 140, 40) |
| 5 | Pink | (240, 100, 140) |
| 6 | Sky Blue | (80, 160, 240) |
| 7 | Coral | (240, 120, 80) |
| 8+ | Wraps around palette |

- The modulator card header uses its assigned color
- Parameter labels/sliders that are modulated by a source are highlighted with that source's color
- At a glance, the performer can see which modulator drives which parameters

### Waveform / Envelope Visualization — IMPLEMENTED
- **LFO cards** display a small real-time waveform preview showing the current waveform shape (sine, square, triangle, saw, random)
- **ADSR cards** display an envelope shape visualization showing the attack/decay/sustain/release curve
- **StepSequencer cards** already show step sliders which serve as the visualization
- **Audio cards** show the current audio level as a progress bar
- Visualizations are drawn with `egui::Painter` using the modulator's assigned color
- A moving playhead or highlight indicates the current position in the waveform cycle

### Live Modulation on Parameter Sliders — IMPLEMENTED
- When a parameter is modulated, the slider shows the **base value** (user-set) as normal
- A **ghost indicator** (colored line/overlay) shows the **effective modulated value** in real-time
- The ghost uses the modulator's assigned color, so the performer can instantly see which modulator is driving each parameter
- Multiple modulations on one parameter show the combined effect
- The modulation indicator animates smoothly each frame

### Per-Parameter Assignment
- Dropdown combo box next to every numeric parameter slider in the UI
- Select a modulation source from the dropdown to assign it — source names are color-coded to match their modulator color
- Works for all parameter types: generator params, deck effect params, channel effect params, master effect params
- Remove assignment via the same dropdown
- All parameters support MIDI learn (right-click → move controller → mapped)

## Stretch Goals

- **Math expressions**: User-defined formulas combining multiple inputs (adds expression parser + editor UI)
- **Full signal routing matrix**: Visual patchbay for connecting any source to any target
- **MIDI CC as modulation source**: Map a MIDI knob directly as a modulation source (distinct from MIDI parameter mapping — this feeds into the modulation engine for stacking/chaining)

## Open Questions

- Should modulation assignments be per-scene or global?
- How to visualize modulator-on-modulator chains without overwhelming the UI?
- Should there be preset modulation "macros"? (e.g., "pulse on beat" = ADSR triggered by beat detection)

