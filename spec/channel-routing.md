# Channel Routing & Crossfader Architecture

## Status: IMPLEMENTED

The channel routing architecture is fully implemented. Varda uses a Deck → Channel → Mixer → Output signal flow with per-level effect chains, A/B crossfader, and a DJ-style mixer box UI.

## Resolume's Model (Reference)

```
Deck 1 → Deck 1 FX ─┐
Deck 2 → Deck 2 FX ─┼─→ [Ch 1] ─→ Ch 1 FX ─┐
Deck 3 → Deck 3 FX ─┘                       │
                                             ├─→ [Crossfader] ─→ [Master FX] ─→ [Output]
Deck 4 → Deck 4 FX ─┐                       │
Deck 5 → Deck 5 FX ─┼─→ [Ch 2] ─→ Ch 2 FX ─┘
Deck 6 → Deck 6 FX ─┘
```

### Key Concepts

1. **Decks** — individual visual sources (shaders, video, images)
2. **Deck FX** — per-deck effect chain, applied before the deck enters the channel mix
3. **Channels (Ch 1/Ch 2/Ch 3...)** — groups of decks composited together, each with its own effect chain
4. **Channel FX** — per-channel effect chain, applied to the composited channel output
5. **Crossfader** — hardware-mappable slider that blends between Ch 1 and Ch 2
6. **Master FX** — effects applied to the final composite after crossfading
7. **Auto-Transitions** — timed crossfade or effect-based transition between channels

### Effect Chain Hierarchy (3 Levels)

Effects can be applied at every level of the routing chain:

```
Level 1: Deck FX      — transforms individual source (e.g., kaleidoscope on one shader)
Level 2: Channel FX   — transforms the mixed channel (e.g., color grade on everything in Ch 1)
Level 3: Master FX    — transforms final output (e.g., bloom, vignette on everything)
```

This gives the VJ maximum flexibility — apply a blur to one deck, a color shift to the whole channel, and a vignette to the final output, all independently.

### What This Enables

- **Smooth transitions**: Crossfade from one visual set to another without visible setup
- **Prepare-while-playing**: Build up Ch 2 while Ch 1 is live, then crossfade
- **Per-channel effects**: Different effect chains for each channel group
- **Hardware mapping**: Physical crossfader on MIDI controller maps directly

## Implemented Varda Model

```
Deck 1 → Deck 1 FX ─┐
Deck 2 → Deck 2 FX ─┼─→ [Ch 1] ─→ Ch 1 FX ─┐
Deck 3 → Deck 3 FX ─┘                       │
                                             │
Deck 4 → Deck 4 FX ─┐                       ├─→ [Crossfader/Mixer] ─→ [Master FX] ─→ [Output(s)]
Deck 5 → Deck 5 FX ─┼─→ [Ch 2] ─→ Ch 2 FX ─┤
Deck 6 → Deck 6 FX ─┘                       │
                                             │
Deck 7 → Deck 7 FX ─┐                       │
Deck 8 → Deck 8 FX ─┼─→ [Ch 3] ─→ Ch 3 FX ─┘
                     ...  (N channels)
```

### Decided

1. **N channels, default 2**: Channels are dynamic — create as many as needed. Default setup is Ch 1 and Ch 2. Naming scheme: Ch 1, Ch 2, Ch 3, etc. Names are assigned from a **monotonic counter** on the Mixer — removing a channel does not recycle its name. This prevents duplicate names and routing ambiguity. ✅ Implemented.

2. **All channels always render**: Even if the crossfader is hard-left on Ch 1, Ch 2 continues rendering. This lets the VJ prepare visuals on the non-live channel before transitioning. GPU cost is the tradeoff — accept it.

3. **Hot deck reassignment**: Decks can be reassigned to any channel on the fly during performance. Drag-and-drop deck thumbnails between channel columns. No restart, no reload. Deck properties (opacity, blend mode, solo, mute) are preserved across the move. ✅ Implemented.

4. **All auto-transition types supported**:
   - **Timed crossfade** — linear or eased fade over N seconds
   - **Beat-synced** — transition snaps to next beat/bar boundary
   - **Shader-based transitions** — ISF transition shaders (wipes, dissolves, custom effects)
   - **Triggered** — button press starts the transition, duration is configurable
   - Transition type is selectable per-crossfader

5. **Crossfader behavior depends on channel count**:
   - **Exactly 2 channels**: Crossfader is active. Blends between A and B. Hardware-mappable.
   - **3+ channels**: Crossfader **deactivates**. Each channel has its own opacity slider. Channels are mixed based on per-channel opacity and blend mode — like layers in a compositor.

6. **Full-state scenes**: Scene save/load captures everything — all channels, all decks, crossfader position, all mappings, all effect chains.

### UI Layout

The central area uses a **DJ-style split layout**: Left Channels | Mixer Box | Right Channels. With the default 2 channels this is A | Mixer | B. With more channels they distribute evenly: A,B | Mixer | C,D, and so on for N channels.

**Channel columns** contain:
- Channel name header (clickable to select channel)
- **Deck grid** — dynamically wrapping thumbnails that scale with window width, showing live deck previews with opacity slider, name, M/S/✕ buttons
- Clicking a channel header selects it for channel-level effect editing in the bottom bar
- Clicking a deck selects it for deck-level detail editing in the bottom bar

**Mixer Box** (center, between channels):
- Vertical opacity faders for all channels, side by side, with per-channel accent colors
- **2-channel mode**: Horizontal crossfader with labels derived from actual channel names (not hardcoded A/B), snap buttons, auto-transition controls, and transition shader selector. ✅ Implemented.
- **3+ channel mode**: Crossfader hides; channels mix via individual opacity and blend modes (compositor-style)
- ➕ Ch button to dynamically add new channels at runtime
- ✕ button per channel fader (visible when 3+ channels) to remove channels (minimum 2 required)
- Per-channel blend mode selectors

**Bottom bar** (resizable, context-sensitive):
- **Deck selected**: Preview (scales with bar height) | Generator params column | Effect 1 column | Effect 2 column | ... | ➕ Add Effect
- **Channel selected**: Channel effect chain as horizontal columns
- **Master selected**: Master effect chain as horizontal columns
- Each effect column contains toggle, name, remove button, and all parameters with modulation support

```
┌──────────┬─────────┬──────────┬─────────────────────┐
│ Left Chs │ 🎚 Mixer │ Right Chs│ Main Output + Sidebar│
│ A: decks │ A▮ B▮ C▮│ C: decks │ [Preview]            │
│ B: decks │ ➕ Ch   │ D: decks │ Modulation / Library  │
│          │ Blend   │          │                       │
├──────────┴─────────┴──────────┴─────────────────────┤
│ Bottom Bar: [Preview] | [Generator] | [FX1] | [FX2] | [➕ Add]  │
└──────────────────────────────────────────────────────┘
```

7. **Multiple active decks per channel**: All decks in a channel are active simultaneously and composited together (like layers), not one-at-a-time triggering. Each deck has its own opacity and blend mode within the channel. The channel output is the composite of all its decks.

This means the deck grid is **not** a clip trigger grid (Resolume-style). It's a **layer stack** view. Every deck in the channel contributes to the channel's output, controlled by per-deck opacity/blend/solo/mute.

### Resolved Questions

- **Channel resolution**: All channels share the stage render resolution (currently 1920×1080). ✅ Decided.
- **Channel solo**: Not implemented separately — deck solo/mute within channels is sufficient for now.
- **Deck grid sizing**: Dynamic — thumbnails are fixed size, grid wraps as needed.

### Resolved Questions (continued)

- **Dynamic channel creation**: Channels can be added at runtime via the ➕ Ch button in the mixer box. ✅ Implemented.
- **Dynamic channel removal**: Channels can be removed at runtime via ✕ button on each channel fader (only visible when 3+ channels; minimum 2 channels enforced). Removing a channel adjusts selected deck/channel indices. ✅ Implemented.
- **Deck grid sizing**: Dynamic — thumbnails are fixed size, grid columns scale with available window width. ✅ Implemented.
- **Hot deck reassignment**: Drag-and-drop deck thumbnails between channel columns. Deck properties (opacity, blend, solo, mute) preserved. Drop target highlights when hovering. ✅ Implemented.

### Open Questions

- How does channel routing interact with multi-output? (Channel A → projector 1, Channel B → projector 2, or is that a separate concern?)

