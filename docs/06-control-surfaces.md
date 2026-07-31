# Control Surfaces & Macros

## MIDI

### Connect a Controller

1. Plug in a MIDI controller — it appears in the **MIDI panel** (bottom section)
2. **Enable** the device with its toggle switch
3. Click **Rescan** if you hot-plug a device after launch

Varda supports multiple simultaneous MIDI controllers. Each device is identified independently, so the same CC number on different controllers maps to different parameters.

### Learn Mode

1. **Right-click** anywhere in the UI → **"Enter MIDI Learn"**
2. All mappable controls glow **purple**
3. **Click** a control to select it as the learn target (brighter purple)
4. **Move a knob or press a button** on your MIDI controller → mapping created
5. Continue mapping more controls — learn mode stays active
6. **Right-click** → **"Exit MIDI Learn"** when done

### APC Mini Auto-Mapping

The Akai APC Mini mk1 is auto-detected by name and receives LED feedback:

- **Green** — boolean parameter is on
- **Yellow** — currently selected/active deck
- **Red blink** — MIDI learn is active on this control
- Faders (CC 48–56) have no LEDs

Controller profiles are data-driven JSON files. Custom profiles can be placed in `.varda/controller-profiles/`.

### Persistence

MIDI mappings are saved to `.varda/midi.json`, keyed by device name. Mappings persist across sessions and survive device reconnection.

### Controller Profiles

A controller profile teaches Varda the physical layout of a device — its control ranges, LED capabilities, and an optional auto-map strategy. The Akai APC Mini profile is built in; you can add profiles for other controllers by dropping `.json` files into `.varda/controller-profiles/`. Files are loaded on startup and matched against connected devices by name.

A profile has four sections:

```json
{
  "profile": { "name": "Akai APC Mini mk1", "name_match": "apc mini" },
  "leds": {
    "method": "note_velocity",
    "channel": 0,
    "colors": { "off": 0, "green": 1, "green_blink": 2, "red": 3, "yellow": 5 }
  },
  "controls": [
    { "name": "grid", "type": "button", "midi_type": "note", "channel": 0, "range": [0, 63], "has_led": true },
    { "name": "faders", "type": "fader", "midi_type": "cc", "channel": 0, "range": [48, 56], "has_led": false }
  ],
  "auto_map": {
    "strategy": "channel_grid",
    "grid_control": "grid",
    "fader_control": "faders",
    "shift_control": "shift",
    "page_buttons_control": "bottom_buttons",
    "columns": 8, "rows": 8,
    "tap_hold_threshold_ms": 300,
    "tap_action": "mute", "hold_action": "solo",
    "fader_target": "channel_opacity", "last_fader_target": "crossfader",
    "led_rules": { "active": "green", "muted": "red", "zero_opacity": "red", "soloed": "yellow", "empty": "off" }
  }
}
```

| Section | Purpose |
|---------|---------|
| `profile` | Display `name` and `name_match` — a case-insensitive substring matched against the connected device's name |
| `leds` | Feedback `method` (`note_velocity`), MIDI `channel`, and a `colors` map of named states to velocity values |
| `controls` | Named control groups. Each declares `type` (`button`/`fader`), `midi_type` (`note`/`cc`), `channel`, an inclusive `range` of note/CC numbers, and `has_led` |
| `auto_map` | Optional. Maps a grid+faders layout onto channels/decks automatically (`strategy: "channel_grid"`), with tap/hold actions, fader targets, and `led_rules` that color the grid by deck state |

`auto_map` is optional — omit it to define only the device's controls and use MIDI learn for mapping. Profiles with invalid control ranges or unknown references are skipped with a warning in the log.

---

## OSC

### Input

Varda listens for OSC messages on **port 9000** (configurable via `--osc-port` or `.varda/osc.json`).

All parameters use the `/varda/` namespace with the same paths as MIDI:

```
/varda/crossfader           0.5       → set crossfader to 0.5
/varda/deck/abc123/opacity  0.8       → set deck opacity to 0.8
/varda/deck/abc123/param/speed  0.5   → set shader parameter
/varda/action/undo          1.0       → trigger undo
```

No learn mode is needed — OSC addresses are self-describing. Discover entity UUIDs via the HTTP API (`GET /api/scene`).

### Clock Sync

```
/varda/clock/bpm   120.0    → set BPM (raw value, not normalized)
/varda/clock/beat  0.5      → set beat phase (0.0–1.0)
```

### Bidirectional Feedback

State changes from user input (MIDI, OSC, or UI interaction) are broadcast as outbound OSC messages to configured feedback targets. Engine-driven changes (modulation, auto-transitions) are not broadcast to avoid flooding.

Configure feedback targets in `.varda/osc.json`:

```json
{
  "input_port": 9000,
  "feedback_targets": ["192.168.1.100:8000"],
  "enabled": true
}
```

This enables visual feedback in TouchOSC, Lemur, and other bidirectional OSC controllers.

---

## Keyboard Shortcuts

### Learn Mode

1. **Right-click** → **"⌨ Enter Keyboard Learn"** (or click the **⌨ KB LEARN** button in the top bar)
2. Learnable controls glow **orange**
3. **Click** a control to select it (brighter orange)
4. **Press a key** → binding created, learn mode stays active
5. **Right-click** → **"⌨ Exit Keyboard Learn"** when done

MIDI learn and keyboard learn are mutually exclusive — entering one exits the other.

### Default Bindings

| Key | Action |
|-----|--------|
| Cmd+Z | Undo |
| Cmd+Shift+Z | Redo |
| Cmd+S | Save |
| L | Toggle library panel |
| S | Select tool (stage editor) |
| R | Rectangle tool |
| P | Polygon tool |
| C | Circle tool |
| D | Duplicate surface |
| H | Flip horizontal |
| V | Flip vertical |
| Delete / Backspace | Delete surface |
| Escape | Clear drawing |
| G | Combine surfaces |

### Param Toggle

When a key is bound to a parameter path:

- **Float params** — toggle between current value and 0.0
- **Bool params** — toggle true/false (mute, solo, effect bypass)

### Persistence

Keyboard bindings are saved to `.varda/keymap.json`. Delete the file to restore defaults.

---

## Clock Synchronization

Varda derives BPM and beat phase from multiple sources with automatic priority resolution:

| Priority | Source | How |
|----------|--------|-----|
| 1 (highest) | **MIDI Clock** | 24 PPQ timing ticks (0xF8) from any connected device. BPM computed from tick intervals, EMA-smoothed (α=0.3). Start (0xFA) resets beat phase; Stop (0xFC) triggers fallback. |
| 2 | **OSC Clock** | `/varda/clock/bpm` and `/varda/clock/beat` messages from network controllers |
| 3 | **Audio Detection** | Spectral flux onset detection from FFT analysis. 16-interval BPM history with outlier rejection. Range: 30–300 BPM. |
| 4 (lowest) | **Manual** | User-set BPM value. Beat phase computed from elapsed wall-clock time. |

**Stale timeout**: if the active source hasn't sent data in 2 seconds, Varda falls back to the next priority source automatically.

### Clock Preference

By default, Varda uses **Auto** mode (priority resolution). You can force a specific source:

- **Auto** — highest-priority available source wins
- **Force MIDI** — lock to a specific MIDI device
- **Force OSC** — use only OSC clock messages
- **Force Audio** — use only beat detection
- **Force Manual** — fixed BPM, no external input

Click the **BPM display** in the mixer to open the clock preference popover. All detected MIDI clock devices appear with their current BPM.

### What Uses the Clock

Beat-synced features throughout Varda consume the resolved BPM and beat phase:

- **Beat-synced crossfades** — crossfade triggered on the next beat boundary
- **Deck auto-transitions** — play duration specified in beats
- **Transition sequences** — step durations in beats
- **Step sequencer** — rate synchronized to BPM
- **ISF shaders** — `audio_bpm` and `audio_beat_phase` uniforms

The `clock/bpm` parameter path is MIDI-mappable (0.0–1.0 → 20–300 BPM).

---

## Macros

A **macro** is a performance control you build yourself — a **knob**, **fader**, or **button** — that drives many parameters at once. Turn one knob and two effect parameters on two different decks move together; press one button and a whole look snaps into place. Each macro is also a mappable parameter in its own right, so a single hardware knob mapped by MIDI learn drives the macro, and the macro drives everything wired to it.

Macros live in the **central mixer column**, stacked directly below the mixer box and the transition sequence builder — the same place you reach for the crossfader. Each macro shows as a small **live control** (knob, fader, or button) you play with right there in the column. This works just like the sequence builder: **play the control in place, and click anywhere around it** (its card) to open the macro's full configuration in the bottom bar.

### Creating a Macro

Below the macro controls in the central column are three centered add buttons:

- **＋ Knob** — a rotary knob (drag up/down to sweep 0–1)
- **＋ Fader** — a linear 0–1 slider (identical behavior to a knob; pick whichever matches your mental model)
- **＋ Button** — an on/off control with three press behaviors (see [Buttons](#buttons))

Each new macro is named `Macro 1`, `Macro 2`, … with an accent color from the shared palette (the same colors used by modulation sources). Its compact widget shows a color dot, the name, an **x** delete button, and the interactive live control. Clicking **x** removes the macro immediately (like deleting a transition sequence).

The central column — and every channel column — scrolls vertically when its stack of decks, sequences, and macros is taller than the panel, so adding many macros never pushes content off-screen.

**Click the card around the control** to select it — the bottom bar switches to that macro's detail editor, showing a larger control, a color dot, an editable **name** field, a **kind** selector, a **🗑 Delete** button, an **x Close** button, and the target (or trigger) editor. This mirrors how clicking a deck, channel, or sequence fills the bottom bar. (Dragging the knob/fader or pressing the button plays it live and does *not* open the editor.)

Rename a macro by typing in its name field in the detail editor — the new name is committed when the field loses focus. Change a macro's kind at any time with the kind dropdown; switching to **Button** adds button behavior options, switching away removes them.

### Binding Targets

A **target** is any mappable parameter the macro should drive. Select the macro (click its name) to open its detail editor in the bottom bar, then:

1. Open the **＋ Add target** dropdown in the detail editor.
2. Pick a parameter from the list. It is grouped and labeled by location — for example `Deck 1 · Blur · radius`, `Ch 0 · opacity`, `Master · Glow · intensity`, `Crossfader`, or a **modulator parameter** such as `LFO 1 · frequency`, `ADSR 2 · release`, or `StepSeq 1 · rate`. Driving a modulator param lets a macro reshape a modulation source (e.g. sweep an LFO's rate) as well as deck/channel/effect params.
3. The target is added instantly with defaults (`min 0.0`, `max 1.0`, `Linear` curve, not inverted) and starts following the macro immediately.

Every target has its own mapping row so a single macro can push each parameter through a different range and shape:

| Control | Meaning |
|---------|---------|
| **min** / **max** | The slice of the parameter's full range the macro sweeps. `min 0.2, max 0.9` uses only that portion. Setting **min greater than max** inverts the response. |
| **inv** | Invert the response (equivalent to swapping min/max) — the target *falls* as the macro *rises*. Use it to open one effect while closing another from one gesture. |
| **curve** | The response shape applied before mapping into `[min, max]`: **Linear**, **Exp** (ease-in, slow start), **Log** (ease-out, fast start), **S-Curve** (ease-in-out), or **Stepped** (quantize into discrete levels — great for stutter/enum-like params). |
| **x** | Remove the target. |

There is no limit on targets per macro, and the same parameter can be a target of more than one macro (last gesture wins, exactly like two MIDI CCs mapped to one parameter).

> **The motivating example.** To control two effect parameters on two decks with one knob: add a Knob macro, add target `Deck A · FX1 · scale`, then add target `Deck B · FX2 · warp` and tick **inv** on the second. Now one knob turn opens the first effect while closing the second.

#### Macros and modulation compose

A macro sets a parameter's **base** value; the modulation engine adds its offset **on top** every frame. So a parameter can be both macro-driven and modulated at once — the macro sets where the value sits, modulation animates around it. They never conflict.

A macro **cannot** target another macro (loop prevention). Target the underlying parameters directly instead.

### Modulating a Macro

Beyond driving targets *by hand*, a **Knob** or **Fader** macro can itself be driven by a **modulation source** (LFO, ADSR, audio, step sequencer). Assign one and the modulator sweeps the whole macro — and through it every mapped target — automatically. This is the "one gesture animates everything" move: bind an LFO to a macro that drives five parameters, and all five breathe together.

In the macro's detail editor (bottom bar), the **Mod** section lists each assigned source on its own row (color dot + name + an **x** to remove just that source) with an **＋ Modulate** dropdown below to add more:

1. Pick a source from **＋ Modulate** to assign it — it appears as a new row in the source's color. Assign several to stack them (their offsets sum).
2. The value label then reads `value 0.50 → 0.73` — the first number is your manual set point (the **base**), the second is the live **effective** value being fanned out. The **control itself** also shows a colored **ghost** marker at the effective value (the knob's ghost pointer / the fader's ghost line), so a modulated macro visibly tracks its source — just like a modulated param slider. The base pointer stays where you set it.
3. Click a row's **x** to remove that one source (the others keep driving the macro).

Modulation rides *on top* of the base: turn the knob to move where the sweep is centered, exactly like a modulated effect parameter. Each mapped target still applies its own min/max/curve/invert to the modulated value, so one LFO can open one effect while closing another.

- Only **Knob/Fader** macros can be modulated; **Button** macros cannot (they're discrete).
- Modulators are created in the **Modulation** panel (see [Modulation](05-modulation.md)); any source there is assignable to a macro.
- Macro modulation assignments are **per-scene** (saved in `scene.json`) and **undoable**.

> Tip: you can *also* modulate a macro's individual target parameters directly from their own deck/effect panels. Modulating the macro animates all targets together; modulating a target animates just that one.

### Buttons

A Button macro has a behavior selector with three modes:

| Behavior | Press | Release |
|----------|-------|---------|
| **Momentary** | drive all targets to their **max** | drive all targets back to **min** |
| **Toggle** | latch on/off — each press flips targets between **max** and **min** | (ignored) |
| **Trigger** | fire one-shot **actions** once, on the press | (ignored) |

The behavior selector lives in the macro's detail editor (bottom bar). Momentary and Toggle buttons use the same **target** list as knobs and faders. A **Trigger** button instead shows an **On press** editor:

- **Undo / Redo / Save** checkboxes — fire the corresponding global app action on press.
- **＋ Add param** — add a parameter action that writes a fixed value (`1.0`) to a path on press, e.g. `deck/<uuid>/trigger` to snap a deck to full opacity. Remove one with **x**.

> Trigger buttons are fire-and-forget — perfect for mapping a pad to Undo, a "reset" snapshot, or a deck slam.

### Mapping a Macro to MIDI / OSC / Keyboard

Because a macro is addressable as `macro/<uuid>/value`, it inherits Varda's whole control plane with no extra setup:

- **MIDI** — enter **MIDI Learn** (right-click → *Enter MIDI Learn*), then click a macro's live control in the central column (it glows purple like any other control) and move a hardware control. A button macro maps naturally to a pad: note-on drives `1.0`, note-off drives `0.0`. See [MIDI](#midi).
- **OSC** — send `/varda/macro/<uuid>/value <0..1>`. Discover the UUID via `GET /api/scene/macros` or `GET /api/state`.
- **Keyboard** — keyboard learn can bind a key to a macro (especially buttons) via the same value path.

### Macro Persistence

Macros are **per-scene** and saved in `scene.json` alongside channels, effects, and modulation. Loading a scene restores its macros, values, targets, and button configuration. Older scenes saved before macros existed load normally with no macros.

MIDI mappings to `macro/<uuid>/value` are stored in `.varda/midi.json` like any other mapping. As with deck/effect mappings, a mapping saved against one scene's macro resolves to nothing (logged, no crash) if a different scene is loaded, since macro UUIDs are per-scene.

### Macro Undo / Redo

Macro **configuration** changes — add, remove, rename, change kind, edit a target, change button behavior or triggers — are undoable. **Turning a macro** (its live value) is performance input and is deliberately *not* undoable, consistent with the crossfader, channel opacity, and MIDI live control.

### Macro HTTP API

Every macro operation available in the UI is exposed over the [HTTP API](13-api.md) under the **Macros** tag. Read state via `GET /api/state/macros` or `GET /api/scene/macros`.

#### Create a macro

```sh
curl -X POST http://localhost:8080/api/macros \
  -H "Content-Type: application/json" \
  -d '{"kind": "Knob"}'
```

Returns the new macro's `uuid`. `kind` is `Knob`, `Fader`, or `Button`.

#### Add a target

```sh
curl -X POST http://localhost:8080/api/macros/<uuid>/targets \
  -H "Content-Type: application/json" \
  -d '{"path": "deck/<deck_uuid>/effect/<fx_uuid>/param/scale"}'
```

`path` is any mappable parameter-router path, including **modulator params** such as `mod/<mod_uuid>/frequency`, `mod/<mod_uuid>/rate`, or `mod/<mod_uuid>/step/3` — so the API can bind a macro to a modulator exactly like the UI target picker. (Only `macro/*` paths are rejected, to prevent loops.)

#### Shape a target (range / curve / invert)

Targets are addressed by zero-based index in the order they were added:

```sh
curl -X PUT http://localhost:8080/api/macros/<uuid>/targets/0 \
  -H "Content-Type: application/json" \
  -d '{"min": 0.2, "max": 0.9, "curve": "SCurve", "invert": true}'
```

`curve` is `"Linear"`, `"Exponential"`, `"Logarithmic"`, `"SCurve"`, or `{"Stepped": 4}`.

#### Drive a macro (live)

```sh
curl -X PUT http://localhost:8080/api/macros/<uuid>/value \
  -H "Content-Type: application/json" \
  -d '{"value": 0.75}'
```

This is equivalent to the shared parameter route `PUT /api/params` with `{"path": "macro/<uuid>/value", "value": 0.75}` — both fan out to every target.

#### Modulate a macro's value

Drive a Knob/Fader macro from a modulation source (the modulator's offset rides on top of the manual set point and re-fans to every target each frame):

```sh
# Assign a modulation source to the macro's value
curl -X PUT http://localhost:8080/api/macros/<uuid>/modulation \
  -H "Content-Type: application/json" \
  -d '{"source_id": "<mod_uuid>", "amount": 0.5}'

# Clear ALL modulation on the macro
curl -X DELETE http://localhost:8080/api/macros/<uuid>/modulation

# Clear only one source (leave any others intact)
curl -X DELETE http://localhost:8080/api/macros/<uuid>/modulation/<mod_uuid>
```

#### Configure a Button macro

```sh
# Set behavior: Momentary | Toggle | Trigger
curl -X PUT http://localhost:8080/api/macros/<uuid>/button/behavior \
  -H "Content-Type: application/json" -d '{"behavior": "Trigger"}'

# Set trigger actions (fired on press)
curl -X PUT http://localhost:8080/api/macros/<uuid>/button/triggers \
  -H "Content-Type: application/json" \
  -d '{"triggers": [{"Global": "Save"}, {"Param": {"path": "crossfader", "value": 0.0}}]}'
```

#### Other macro operations

| Operation | Request |
|-----------|---------|
| Rename | `PUT /api/macros/<uuid>/name` `{"name": "Sweep"}` |
| Change kind | `PUT /api/macros/<uuid>/kind` `{"kind": "Fader"}` |
| Remove target | `DELETE /api/macros/<uuid>/targets/<idx>` |
| Delete macro | `DELETE /api/macros/<uuid>` |

---

## Parameter Paths

MIDI, OSC, and keyboard shortcuts all use the same parameter path format:

| Path | Description |
|------|-------------|
| `crossfader` | Mixer crossfader (0.0–1.0) |
| `clock/bpm` | Manual BPM (mapped 0.0–1.0 → 20–300 BPM for MIDI) |
| `deck/<uuid>/opacity` | Deck opacity |
| `deck/<uuid>/mute` | Deck mute toggle |
| `deck/<uuid>/solo` | Deck solo toggle |
| `deck/<uuid>/trigger` | Set deck opacity to 1.0 |
| `deck/<uuid>/param/<name>` | Shader parameter |
| `deck/<uuid>/effect/<effect_uuid>/param/<name>` | Deck effect parameter |
| `deck/<uuid>/video/play` | Set video play state (playing when > 0.5) |
| `deck/<uuid>/video/speed` | Video playback speed (0.0–1.0 → 0.1×–4.0×) |
| `deck/<uuid>/video/seek` | Seek position (0.0–1.0 → start–end of clip) |
| `deck/<uuid>/video/in_point` | Loop in-point (0.0–1.0 → start–end of clip) |
| `deck/<uuid>/video/out_point` | Loop out-point (0.0–1.0 → start–end of clip) |
| `deck/<uuid>/video/clear` | Clear in/out points (trigger, > 0.5) |
| `deck/<uuid>/video/loop_mode` | Loop mode, fader-bucketed (Loop / Ping-Pong / One Shot / Hold Last) |
| `deck/<uuid>/scaling_mode` | Source scaling, fader-bucketed (Fill / Fit / Stretch / Center) |
| `ch/<uuid>/opacity` | Channel opacity |
| `ch/<uuid>/effect/<effect_uuid>/param/<name>` | Channel effect parameter |
| `master/effect/<effect_uuid>/param/<name>` | Master effect parameter |
| `mod/<mod_uuid>/frequency` | LFO frequency |
| `mod/<mod_uuid>/amplitude` | LFO amplitude |
| `mod/<mod_uuid>/step/<n>` | Step-sequencer step value (step index is positional within the source) |
| `macro/<uuid>/value` | Macro control (0.0–1.0); fans out to all the macro's targets — see [Macros](#macros) |
| `action/undo` | Trigger undo |
| `action/redo` | Trigger redo |
| `action/save` | Trigger save |

Entity UUIDs are stable 8-character hex strings that persist across moves, reorders, and scene save/restore. Decks, channels, effects, and modulation sources are all addressed by UUID — never by positional index — so a saved mapping keeps targeting the same entity after a chain or rack is reordered. LED feedback reads state back through the same UUID paths.

The `video/*` and `scaling_mode` paths resolve only on the matching source type (video controls on video decks; `scaling_mode` on image, video, camera, and external-source decks). **Discrete (enum) controls** — `loop_mode` and `scaling_mode` — use **fader bucketing**: the 0.0–1.0 range is split into equal segments so a fader or knob sweeps through the options. **Seek and in/out points** scale the 0.0–1.0 value against the clip's duration, so one mapping works for clips of any length.

---

[← Prev: Modulation & Audio Reactivity](05-modulation.md) · [Home](README.md) · [Next: Outputs →](07-outputs.md)
