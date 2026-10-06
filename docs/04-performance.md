# Performance & Automation

## Video Playback

When a deck's source is a video file, the deck detail panel (bottom bar) shows playback controls.

### Loop Modes

| Mode | Behavior |
|------|----------|
| **Loop** 🔁 | Restart from the in-point on reaching the out-point (default) |
| **Ping-Pong** 🔄 | Play forward, then in reverse, repeating |
| **One Shot** 1️⃣ | Play once and stop at the out-point |
| **Hold Last** ⏹ | Play once and freeze on the final frame |

Ping-Pong plays its reverse leg from frames cached during the forward pass. See [Ping-Pong & Reverse Cache](#ping-pong--reverse-cache).

### Speed Control

The **speed** slider in the deck detail panel runs from **0.1× to 4.0×**. Below 1.0 slows the clip down; above 1.0 speeds it up. The slider has no reverse (negative) speeds, although the playback engine supports them.

### Scrub / Seek

The **position slider** in the Playback section shows the current time on the left and the clip duration on the right. **Click or drag** it to scrub.

You can map seeking to MIDI, OSC or a key with `deck/<uuid>/video/seek`. The 0.0–1.0 value maps to the clip's full duration, so one mapping works for clips of any length, for example to scrub from a fader or an LFO.

### In/Out Points

To play only part of a clip:

1. Scrub to the start position and set the **in-point**.
2. Scrub to the end position and set the **out-point**.
3. Playback now loops, or plays once, within this range, depending on the loop mode.

Click **Clear In/Out** to go back to the full clip.

Play/pause, speed, seek, loop mode, in/out points and clear can all be mapped to MIDI, OSC and keys, and driven by a macro. To bind one, enter learn mode and click the control in the deck detail panel. See [Parameter Paths](07-control-surfaces.md#parameter-paths).

Loop mode uses **fader bucketing**: sweeping a fader or knob steps through Loop → Ping-Pong → One Shot → Hold Last.

Transport chase (Auto / Always / Never, offset, delay) is set on the clip. See [Arrangement Mode](05-arrangement.md#video-chase).

Play, speed, seek and loop mode are also **modulation targets**. For example, an LFO can time-warp a clip and an audio band can gate it. In and out points are not modulation targets, because the playhead offset is measured against them. See [Video Playback](06-modulation.md#video-playback) for how each target behaves and the decode cost of modulating the playhead.

A modulated playhead also moves a **paused** clip. Pause stops the clip advancing on its own, but a modulator still scrubs it, with the swing centered on where you parked the playhead. A paused clip with a playhead assignment keeps decoding, so it has the same seek cost as a playing clip.

### HAP Hardware Codecs

HAP clips decode straight to GPU-native compressed textures with no CPU color conversion. They are the most efficient choice for high-resolution playback, reverse and scrubbing. All HAP variants play:

| Variant | Encoding |
|---------|----------|
| **HAP** | BC1 (RGB) |
| **HAP Alpha** | BC3 (RGBA) |
| **HAP Q** | YCoCg (BC3), higher quality |
| **HAP Q Alpha** | Dual-plane: YCoCg color + BC4 alpha |

### Ping-Pong & Reverse Cache

In Ping-Pong mode, Varda caches decoded frames during forward playback and replays them in reverse. The cache holds up to **2 GB**, about 13 s of 1080p at 60 fps. If the forward pass is longer, the reverse leg is cut to what fits, and Varda shows this notice **once**:

> Deck '<name>': reverse playback truncated (cache full). Transcode to HAP for full-length reverse.

Transcode the clip to a **HAP** codec to remove the limit. HAP frames decode fast enough to play in reverse directly, without the cache.

---

## Deck Auto-Transitions

An auto-transition plays a deck for a set time and then transitions it out, revealing the deck or decks below it in the channel. Use it to step through visuals in one channel without touching the controls.

### Configuration

Each deck has an optional auto-transition with these settings:

| Setting | Description |
|---------|-------------|
| **Play Duration** | How long the deck plays before transitioning (seconds, minutes, hours or beats) |
| **Transition Duration** | How long the transition takes (seconds or beats) |
| **Trigger** | **Timer**: starts counting when the deck becomes topmost. **ClipEnd**: starts when the video reaches its out-point or end (non-video sources use Timer). |
| **Transition Shader** | Optional ISF transition shader (dissolve, iris, push, etc.). None gives a plain opacity fade. |

### Phase Lifecycle

Each auto-transition deck moves through four phases:

```
Inactive → Playing → Transitioning → Done
                                        ↓
                              (next deck activates)
```

1. **Inactive**: waiting for its turn (not the topmost visible deck)
2. **Playing**: content is visible and the countdown runs. The deck detail panel shows elapsed time.
3. **Transitioning**: the transition shader (or opacity fade) runs from 0% to 100%, revealing the deck below.
4. **Done**: the deck is invisible. When every deck reaches Done, the sequence loops.

### Workflow

1. Add several decks with different content to one channel.
2. Enable **auto-transition** on each deck.
3. Set play and transition durations.
4. Optionally select a transition shader for each deck.
5. During the show, the channel cycles through the decks on its own.

With the **ClipEnd** trigger on video decks, each video plays to the end before transitioning. Use it for pre-edited clip sequences.

While the arrangement drives a deck, that deck's auto-transition is suspended. See [Arrangement Mode](05-arrangement.md#performance-sequencers-while-the-arrangement-runs).

---

## Transition Sequences

Transition sequences automate crossfades between channels over time. Deck auto-transitions cycle decks within one channel; sequences drive the mixer's crossfader between channels.

### Step Types

| Step | Description |
|------|-------------|
| **Fade** | Crossfade from one channel to another over a duration. Supports easing curves (Linear, EaseIn, EaseOut, EaseInOut) and an optional transition shader. |
| **Wait** | Hold the current state for a duration |
| **GoTo** | Jump to a step index (0-based). Use it to loop a sequence. |

### Duration Units

Durations can be in **seconds**, **minutes**, **hours** or **beats**. Beats use the current BPM (see [Clock Synchronization](07-control-surfaces.md#clock-synchronization)).

### Building a Sequence

1. Open the **mixer card** in the center panel.
2. Click **"+ Sequence"** to create a named sequence.
3. Add steps: Fade, Wait or GoTo.
4. For Fade steps, select the source and target channels, duration, easing and optional transition shader.
5. Click **Play** to start the sequence.

### Simultaneous Sequences

Several named sequences can play at once. Use this in multi-surface setups where different channel pairs need their own automation. For example, one sequence cycles the main screen (channels A↔B) while another cycles the side panels (channels C↔D).

Sequences cannot run while an arrangement holds the mixer, because a Fade step controls a pair of channels. While the arrangement is in control, Varda refuses to start a free-running sequence and stops any that is running.

### Easing Curves

| Easing | Formula | Use |
|--------|---------|-----|
| **Linear** | Constant speed | Default, mechanical |
| **EaseIn** | Starts slow, accelerates | Gentle starts |
| **EaseOut** | Starts fast, decelerates | Gentle landings |
| **EaseInOut** | Slow start and end | Smooth, organic |

---

## Undo / Redo

Varda keeps a 50-level undo history. One timeline covers both mixer and scene edits and stage-editor and warp edits, so Cmd+Z undoes your most recent action of either kind.

| Action | Shortcut |
|--------|----------|
| **Undo** | Cmd+Z |
| **Redo** | Cmd+Shift+Z |

You can map both to MIDI and keys with the `action/undo` and `action/redo` parameter paths.

### What's Undoable

- Adding and removing channels, decks and effects
- Parameter changes (opacity, shader params, blend mode)
- Modulation changes (adding and removing sources and assignments)
- Effect reordering (drag-and-drop)
- Moving decks between channels
- Transition shader selection
- **Surface geometry**: add, remove or duplicate a surface, move vertices or edges, the move/rotate/scale gizmo, flip H/V, bezier edge edits, circle radius and sides
- **Warp**: corner-pin and mesh point drags, subdivide, convert to bezier, bezier cage edits, bind-to-shape, reset warp
- **Masking & layers**: make and punch holes, combine surfaces, stacking order (front/back/up/down)
- **Assignments & dome**: assigning a surface to an output, dome mode, preset and geometry

A continuous drag (a vertex, a warp point, a bezier handle or the gizmo) is **one** undo step. Cmd+Z returns the shape to where it was before the drag started.

### What's NOT Undoable

- **Crossfader position**: a continuous live control that would create too many snapshots
- **Video playback**: position and play/pause
- **MIDI mappings**: device configuration, not show state
- **Output windows**: creating, removing, moving or resizing a projector output window. Undo does not tear down or recreate an output feed during a set. Assigning surfaces *onto* an existing output can be undone.

Loading a workspace clears the undo history. A new action after an undo clears the redo stack.

---

## Presets

Presets save deck or channel setups as portable JSON files you can reuse.

### Deck Presets

A deck preset stores everything about one deck:

- Source (shader path and parameters, video path, camera name, etc.)
- Effect chain with all parameter values
- Opacity, blend mode, mute/solo, z-index
- Auto-transition settings
- Modulation recipes (sources and assignments, using relative parameter keys)

**Save**: select a deck, click **"Save Preset"** in the deck detail panel, and name it.

**Load**: drag a deck preset from the **Library** panel into a channel. Varda creates a new deck with all settings restored. If an identical modulation source already exists, the deck reuses it instead of adding a duplicate.

### Channel Presets

A channel preset stores a whole channel: all its decks (with their presets), the channel effect chain, opacity and blend mode.

**Save/Load**: the same as deck presets, from the channel effect panel and the Library panel.

### File Location

Presets are JSON files in `.varda/presets/decks/` and `.varda/presets/channels/`. They appear in the Library panel for drag-and-drop loading.

---

## Finding a Look: Random and Mutate

Three buttons under the parameter list in the params column of the deck detail panel help you explore a generative shader's parameters:

- **Reset** returns every parameter to the shader's declared defaults.
- **Random** picks a new value for every parameter from its declared range. Use it to jump to a completely different look.
- **Mutate** nudges every parameter by a fraction of its range, set by the **by** drag beside it (0.10 by default). Use it to make small changes to a look that is nearly right.

Random and Mutate are each a single undo step, so one Ctrl/Cmd+Z restores the previous look however many parameters changed. A typical loop: mutate, look, **Save Preset** if it is good, undo if it is not.

If the shader [groups its parameters](14-isf-authoring.md), a second row has a scope selector. Leave it on **Everything** to change the whole image, or pick one group to change only that group. For example, change the formula while keeping the lighting and grade.

Random and Mutate do not change:

- colors, because random colors usually look muddy
- parameters whose shader declares no minimum and maximum, because there is no range to pick from

They do change enum menus and checkboxes.

Over HTTP: `POST /api/decks/{uuid}/params/randomize` and `/params/mutate`. Each call takes an optional `group`, an optional `seed` and, for mutate, an `amount`. The same seed always gives the same values, so you can reproduce a look from its seed without saving a preset.

---

[← Prev: Library Panel](03-library-panel.md) · [Home](README.md) · [Next: Arrangement Mode →](05-arrangement.md)
