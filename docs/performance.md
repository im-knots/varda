# Performance & Automation

## Video Playback

When a deck's source is a video file, the deck detail panel (bottom bar) shows playback controls.

### Loop Modes

| Mode | Behavior |
|------|----------|
| **Loop** | Restart from in-point when reaching out-point (default) |
| **Ping-Pong** | Play forward, then reverse, repeating indefinitely |
| **One-Shot** | Play once and stop at the out-point |
| **Hold Last** | Play once and freeze on the final frame |

### Speed Control

Set the playback speed multiplier from the deck detail panel. Values below 1.0 slow down, above 1.0 speed up. Negative values play in reverse.

### In/Out Points

Define a sub-range of the clip to play:

1. Scrub to the desired start position → set **in-point**
2. Scrub to the desired end position → set **out-point**
3. Playback now loops (or plays once, depending on loop mode) within this range

Click **Clear In/Out** to reset to the full clip duration.

All video controls — play/pause, speed, loop mode, in/out points, seek position — are MIDI, OSC, and keyboard mappable.

---

## Deck Auto-Transitions

Auto-transitions let a deck play for a set duration and then transition out, revealing the deck(s) below it in the channel. This enables hands-free visual progression within a single channel.

### Configuration

Each deck has an optional auto-transition with these settings:

| Setting | Description |
|---------|-------------|
| **Play Duration** | How long the deck plays before transitioning (seconds, minutes, hours, or beats) |
| **Transition Duration** | How long the transition takes (seconds or beats) |
| **Trigger** | **Timer** — starts counting when deck becomes topmost. **ClipEnd** — starts when video reaches its out-point/end (falls back to Timer for non-video sources) |
| **Transition Shader** | Optional ISF transition shader (dissolve, iris, push, etc.). None = simple opacity fade |

### Phase Lifecycle

Each auto-transition deck moves through four phases:

```
Inactive → Playing → Transitioning → Done
                                        ↓
                              (next deck activates)
```

1. **Inactive** — waiting for its turn (not the topmost visible deck)
2. **Playing** — content is visible, countdown running. The deck detail panel shows elapsed time
3. **Transitioning** — transition shader (or opacity fade) runs from 0% to 100%, revealing the deck below
4. **Done** — deck is effectively invisible. When all decks reach Done, the sequence loops

### Workflow

1. Add multiple decks to a single channel (each with different content)
2. Enable **auto-transition** on each deck
3. Set play and transition durations
4. Optionally select a transition shader per deck
5. The channel cycles through decks automatically during performance

With **ClipEnd** trigger on video decks, each video plays to completion before transitioning — useful for pre-edited clip sequences.

---

## Transition Sequences

Transition sequences automate crossfades across channels over time. Unlike deck auto-transitions (which cycle within a channel), sequences drive the mixer's crossfader between channels.

### Step Types

| Step | Description |
|------|-------------|
| **Fade** | Crossfade from one channel to another over a duration. Supports easing curves (Linear, EaseIn, EaseOut, EaseInOut) and an optional transition shader. |
| **Wait** | Hold the current state for a duration |
| **GoTo** | Jump to a specific step index (0-based). Enables looping sequences. |

### Duration Units

All durations support: **seconds**, **minutes**, **hours**, and **beats** (resolved via the current BPM — see [Clock Synchronization](control-surfaces.md#clock-synchronization)).

### Building a Sequence

1. Open the **mixer card** in the center panel
2. Click **"+ Sequence"** to create a named sequence
3. Add steps: Fade, Wait, or GoTo
4. For Fade steps, select source/target channels, duration, easing, and optional transition shader
5. Click **Play** to start the sequence

### Simultaneous Sequences

Multiple named sequences can play at the same time. This is essential for multi-surface setups where different channel pairs need independent automation — for example, one sequence cycling the main screen (channels A↔B) while another cycles the side panels (channels C↔D).

### Easing Curves

| Easing | Formula | Use |
|--------|---------|-----|
| **Linear** | Constant speed | Default, mechanical |
| **EaseIn** | Starts slow, accelerates | Gentle starts |
| **EaseOut** | Starts fast, decelerates | Gentle landings |
| **EaseInOut** | Slow start and end | Smooth, organic |

---

## Undo / Redo

Varda maintains a 50-level undo history using scene snapshots.

| Action | Shortcut |
|--------|----------|
| **Undo** | Cmd+Z |
| **Redo** | Cmd+Shift+Z |

Both actions are MIDI and keyboard mappable via the `action/undo` and `action/redo` parameter paths.

### What's Undoable

- Adding/removing channels, decks, and effects
- Parameter changes (opacity, shader params, blend mode)
- Modulation changes (add/remove sources, assignments)
- Effect reordering (drag-and-drop)
- Deck moves between channels
- Transition shader selection

### What's NOT Undoable

- **Crossfader position** — continuous live control (too many snapshots)
- **Video playback** — temporal state (position, play/pause)
- **MIDI mappings** — device configuration, not show state
- **Outputs and surfaces** — venue configuration (stage.json, not scene.json)

Undo history is cleared on workspace load. A new action after an undo clears the redo stack (fork behavior).

---

## Presets

Save and reuse deck or channel configurations as portable JSON presets.

### Deck Presets

A deck preset captures everything about a single deck:

- Source (shader path + parameters, video path, camera name, etc.)
- Effect chain with all parameter values
- Opacity, blend mode, mute/solo, z-index
- Auto-transition configuration
- Modulation recipes (sources + assignments, using relative parameter keys)

**Save**: select a deck → click **"Save Preset"** in the deck detail panel. Name it.

**Load**: drag a deck preset from the **Library** panel into a channel. A new deck is created with all settings restored. Modulation sources are deduplicated — if an identical source already exists, it's reused.

### Channel Presets

A channel preset captures an entire channel: all decks (with their presets), the channel effect chain, opacity, and blend mode.

**Save/Load**: same workflow as deck presets, via the channel effect panel and Library panel.

### File Location

Presets are stored in `.varda/presets/decks/` and `.varda/presets/channels/` as JSON files. They appear in the Library panel for drag-and-drop loading.
