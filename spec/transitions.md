# Transitions & Automation

## Status: PARTIALLY IMPLEMENTED

Relates to: [/spec/channel-routing.md](/spec/channel-routing.md), [/spec/deck-sources.md](/spec/deck-sources.md)

## Core Principle

**Transitions happen at the channel level, not the deck level.** Channels are fully composed multi-deck scenes. You transition between entire compositions, not individual clips.

Deck-level animation (e.g., fading a deck's opacity in/out within a channel) is handled by the **modulation engine** — not the transition system. Deck opacity is just another modulatable parameter.

## Two Modes

### 1. Crossfader Mode (Exactly 2 Channels) — ✅ IMPLEMENTED

Simple A↔B blending. The crossfader is a single value from 0.0 (full A) to 1.0 (full B).

**Manual**: Drag crossfader slider in the mixer box or use MIDI hardware fader (MIDI crossfader mapping supported). ✅

**Auto-transition triggers**: ✅
- **Timed**: Move crossfader from current position to target over 1s / 2s / 4s (buttons in mixer box)
- **Beat-synced**: 4-beat transition (button in mixer box), snaps to beat boundary
- **Snap**: Instant snap to A or B (⏮ A / B ⏭ buttons)

**Transition types**: ✅
- **Crossfade**: Simple opacity blend (A fades out, B fades in)
- **Shader-based**: ISF transition shader selected via dropdown in mixer box. Receives both channel textures + a `progress` uniform (0.0→1.0). Enables wipes, dissolves, glitch transitions, etc.

Shader-based transitions use ISF shaders that have two image inputs (`inputImage` for Channel A, a second image input for Channel B) and a `progress` float. The ISF transition shader category already supports this pattern.

### 2. Transition Builder (3+ Channels)

When more than 2 channels are active, the crossfader deactivates. Instead, the user builds **transition sequences** between channels.

A transition sequence is an ordered list of transition steps:

```
Step 1: Fade Channel A opacity 1.0 → 0.0 over 4 bars
        while Fade Channel C opacity 0.0 → 1.0 over 4 bars
Step 2: Wait 16 bars
Step 3: Dissolve (shader) Channel C → Channel B over 2 bars
Step 4: Loop to Step 1
```

Each step defines:
- **Source channel(s)**: Which channels are changing
- **Target opacity** or **transition shader**: How the change happens
- **Duration**: Time-based (seconds) or beat-based (beats/bars)
- **Trigger**: Auto (immediately after previous step), beat-synced (wait for bar boundary), manual (wait for button press)

### Transition Builder UI

A simple sequencer-style interface:
- List of steps, each with source/target channels, transition type, duration
- Add/remove/reorder steps
- Play/pause/loop controls
- Current step indicator during playback
- Beat-sync indicator (shows where in the bar we are)

## Channel Presets

Channels can be saved and loaded independently of full scenes:

- **Save channel preset**: Captures all decks, their sources, parameters, effect chains, opacities, blend modes
- **Load channel preset**: Replaces a channel's entire content with the preset
- **Channel preset library**: Browsable collection, organized by user

This lets a VJ build a library of compositions and load them onto channels during a set. "I want my plasma+trails combo on Channel B" — load the preset, crossfade to it.

## Interaction with Modulation

The modulation engine handles per-parameter animation. This includes:
- Deck opacity within a channel (LFO, audio-driven, envelope)
- Shader parameters on any deck
- Effect parameters at any level

The transition system is **separate** — it operates on channels as units, not individual parameters. But both can run simultaneously: a transition fades between channels while modulators animate parameters within each channel.

## Open Questions

- Should transition sequences be saveable/loadable separately (like channel presets)?
- Can a transition shader receive audio data for audio-reactive transitions?
- Should the transition builder support conditional steps? (e.g., "if audio level > 0.8, skip to step 3")
- How does the transition builder UI coexist with the channel column layout? Separate panel? Bottom bar tab?

