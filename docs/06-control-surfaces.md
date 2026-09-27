# Control Surfaces & Macros

## MIDI

### Connect a Controller

1. Plug in a MIDI controller. It appears in the **🎹 MIDI** section of the right panel.
2. **Enable** the device with its toggle switch.
3. Click **Rescan** if you plug in a device after launch.

You can use several MIDI controllers at once. Each device is identified separately, so the same CC number on two controllers can map to two different parameters.

### Learn Mode

1. **Right-click** empty space in the UI and choose **"Enter MIDI Learn"**. (Right-clicking a deck, effect or lane opens that object's menu instead.)
2. All mappable controls glow **purple**.
3. **Click** a control to make it the learn target (brighter purple).
4. **Move a knob or press a button** on your MIDI controller. The mapping is created.
5. Map more controls. Learn mode stays on.
6. **Right-click** and choose **"Exit MIDI Learn"** when done.

### APC Mini Auto-Mapping

Varda detects the Akai APC Mini mk1 by name and sends it LED feedback:

- **Green**: a boolean parameter is on
- **Yellow**: the selected or active deck
- **Red blink**: MIDI learn is active on this control
- Faders (CC 48–56) have no LEDs

Controller profiles are JSON files. Put custom profiles in `.varda/controller-profiles/`.

### Persistence

MIDI mappings are saved to `.varda/midi.json`, keyed by device name. They persist across sessions and survive reconnecting the device.

### Controller Profiles

A controller profile describes a device's physical layout: its control ranges, its LEDs, and an optional auto-map strategy. The Akai APC Mini profile is built in. To add a profile for another controller, put a `.json` file in `.varda/controller-profiles/`. Varda loads these files at startup and matches them against connected devices by name.

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
| `profile` | Display `name`, and `name_match`: a case-insensitive substring matched against the connected device's name |
| `leds` | Feedback `method` (`note_velocity`), MIDI `channel`, and a `colors` map of named states to velocity values |
| `controls` | Named control groups. Each declares `type` (`button`/`fader`), `midi_type` (`note`/`cc`), `channel`, an inclusive `range` of note/CC numbers, and `has_led` |
| `auto_map` | Optional. Maps a grid-plus-faders layout onto channels and decks (`strategy: "channel_grid"`), with tap/hold actions, fader targets, and `led_rules` that color the grid by deck state |

If you omit `auto_map`, the profile defines only the device's controls and you map them with MIDI learn. Varda skips profiles with invalid control ranges or unknown references and logs a warning.

---

## OSC

### Input

Varda listens for OSC messages on **port 9000**. Change it with `--osc-port` or in `.varda/osc.json`.

All parameters use the `/varda/` namespace with the same paths as MIDI:

```
/varda/crossfader           0.5       → set crossfader to 0.5
/varda/deck/abc123/opacity  0.8       → set deck opacity to 0.8
/varda/deck/abc123/param/speed  0.5   → set shader parameter
/varda/action/undo          1.0       → trigger undo
```

Get entity UUIDs from the HTTP API (`GET /api/scene`).

### Clock Sync

```
/varda/clock/bpm   120.0    → set BPM (raw value, not normalized)
/varda/clock/beat  0.5      → set beat phase (0.0–1.0)
```

### Bidirectional Feedback

Changes made by user input (MIDI, OSC or the UI) are sent as OSC messages to the configured feedback targets. Changes made by the engine (modulation, auto-transitions) are not sent, to avoid flooding the targets.

Set feedback targets in `.varda/osc.json`:

```json
{
  "input_port": 9000,
  "feedback_targets": ["192.168.1.100:8000"],
  "enabled": true
}
```

TouchOSC, Lemur and other bidirectional OSC controllers use this feedback to update their displays.

---

## Keyboard Shortcuts

### Learn Mode

1. **Right-click** and choose **"⌨ Enter Keyboard Learn"**, or click the **⌨ KB LEARN** button in the top bar.
2. Learnable controls glow **orange**.
3. **Click** a control to select it (brighter orange).
4. **Press a key**. The binding is created and learn mode stays on.
5. **Right-click** and choose **"⌨ Exit Keyboard Learn"** when done.

Only one learn mode can be on at a time. Entering MIDI learn exits keyboard learn, and the reverse.

### Default Bindings

| Key | Action |
|-----|--------|
| Cmd+Z | Undo |
| Cmd+Shift+Z | Redo |
| Cmd+S | Save |
| Cmd+C | Copy the selected deck or channel |
| Cmd+V | Paste what was copied |
| Cmd+D | Duplicate the selection in place |
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

The copy keys act on the selection: the deck the bottom bar is following, or its channel when no deck is selected. While an automation lane is selected, they copy that curve's breakpoints instead. They do nothing while you are typing in a field. See [Copy and Paste](02-concepts.md#copy-and-paste).

### Param Toggle

When a key is bound to a parameter path:

- **Float params** toggle between the current value and 0.0.
- **Bool params** toggle true/false (mute, solo, effect bypass).

### Persistence

Keyboard bindings are saved to `.varda/keymap.json`. Delete the file to restore the defaults.

---

## Clock Synchronization

Varda takes BPM and beat phase from several sources and picks one by priority:

| Priority | Source | How |
|----------|--------|-----|
| 1 (highest) | **MIDI Clock** | 24 PPQ timing ticks (0xF8) from any connected device. BPM is computed from tick intervals and EMA-smoothed (α=0.3). Start (0xFA) resets beat phase; Stop (0xFC) triggers fallback. |
| 2 | **OSC Clock** | `/varda/clock/bpm` and `/varda/clock/beat` messages from network controllers |
| 3 | **Audio Detection** | Spectral flux onset detection from FFT analysis. 16-interval BPM history with outlier rejection. Range: 30–300 BPM. |
| Forced only | **Manual** | A BPM you set. Beat phase is computed from elapsed wall-clock time. Auto never picks Manual. |

**Stale timeout**: if the active source sends no data for 2 seconds, Varda falls back to the next source by priority.

### Clock Preference

The default is **Auto** (priority order). You can force a source:

- **Auto**: the highest-priority available source
- **Force MIDI**: lock to a specific MIDI device
- **Force OSC**: use only OSC clock messages
- **Force Audio**: use only beat detection
- **Force Manual**: fixed BPM, no external input

Click the **BPM display** in the top bar to open the clock preference popover. It lists every detected MIDI clock device with its current BPM.

### What Uses the Clock

These features use the resolved BPM and beat phase:

- **Beat-synced crossfades**: the crossfade starts on the next beat.
- **Deck auto-transitions**: play duration is set in beats.
- **Transition sequences**: step durations are in beats.
- **ISF shaders**: the `audio_bpm` and `audio_beat_phase` uniforms.
- **LFOs and step sequencers** set to the **Beat** timebase: rate is in cycles per beat and follows the tempo. By default they run on wall-clock time. See [Modulation](05-modulation.md).

The `clock/bpm` parameter path is MIDI-mappable (0.0–1.0 → 20–300 BPM).

---

## Transport

The **transport** is the show position, shown as timecode (`HH:MM:SS:FF`). The clock sets tempo; the transport sets where in the show you are. Anything that reads the position produces the same result at the same position on every run.

The position is always shown in the top bar, beside the BPM readout. Click it to open the controls.

| Control | What it does |
|---------|--------------|
| **Position** | The current show position. Green while running, amber while held, blue while chasing timecode, grey before the show has started. |
| **Status** | Why the position is or is not moving. See below. |
| **▶ Play / ⏸ Pause** | Start or hold the position. |
| **⏮ Zero** | Jump back to 00:00:00:00. |
| **Source** | Whether the position advances internally or chases incoming timecode. |
| **Rate** | The frame rate the position is counted and displayed at: 24, 25, 29.97, 29.97 drop-frame, or 30. |

Arrangement mode has its own strip with the same controls plus **⏹ Stop** and the cue arrows. Stop holds the position. Stop again returns to 00:00:00:00. Use this to return to zero in Arrangement mode, because the back arrow steps through cue points. See [Arrangement Mode](15-arrangement.md#cue-points).

### Status

| Status | Meaning |
|--------|---------|
| **Idle** | Not started this session. Your scene renders exactly as saved. |
| **Running** | Position advancing. |
| **Stopped** | Started, then stopped. The position holds, so anything reading it holds its look. |
| **Waiting for signal** | Set to chase timecode, but none has arrived yet. |
| **Freewheeling** | Chasing, and the signal has dropped out. The position keeps moving at the last known speed for about five frames, then holds. |

On a dark stage, an idle system and one with no timecode signal look the same on the output. Check the status to tell them apart.

### Starting the Transport

The transport stays stopped at zero until you press Play. Nothing that reads the position moves before then. An unplugged cable or a wrong input setting cannot black out your output before you start: you get the scene as saved.

### Source

**Internal** advances the position on Varda's own clock. Use it for looping installations and for building a show before any timecode exists.

**Chase timecode** follows an external master. While chasing, the position is **read-only**: Play and Zero are disabled. A loop range you have set is kept but ignored, and applies again when you switch back to Internal.

### Following SMPTE

Varda reads both kinds of SMPTE timecode. Both set the position the transport chases.

| Kind | Where it arrives | What to do |
|------|------------------|------------|
| **MTC** (MIDI Timecode) | Any connected MIDI port | Nothing. Varda listens on every port. |
| **LTC** (Linear Timecode) | An audio input, as sound | Choose the input and channel under **LTC in**. |

Varda listens for LTC only on the input you choose, because it would otherwise have to open every audio device on the computer.

Set the source to **Chase timecode** to show the timecode controls:

| Control | What it does |
|---------|--------------|
| **Follow** | Which signal is used. **Auto** uses LTC if it is set up and arriving, otherwise MTC. **LTC** or a named MIDI port forces that signal and ignores the other. **Off** ignores timecode, for example in a rehearsal where the master is running and you do not want to follow it. |
| **LTC in** | The audio input carrying LTC, and which channel of it. Field rigs often send program audio on one channel and timecode on the other, so pick the right channel. |

Below these, every signal Varda can hear is listed with its own position and state, including signals that are not driving the transport. Use this list to troubleshoot:

- An input that is listed but stopped means the master is not rolling.
- An input that is not listed means a patching problem.
- If nothing is arriving, the panel says so.

Varda detects the frame rate from the signal, so a 25 fps master reads as 25 fps. The exception is `29.97` non-drop. Its signal is almost identical to `30` and uses the same labels, but the two drift 3.6 seconds an hour apart. Set **Rate** to `29.97` by hand if your master sends it.

The timecode input setup is saved with the venue in `stage.json`, alongside surfaces and outputs, not with the show. Devices are remembered by name, so they are found again on a different port. If a saved device is missing at load, the notification bar says so.

Timecode is also sent over OSC as `/varda/timecode/position` (seconds) and `/varda/timecode/string` (`HH:MM:SS:FF`). Read it over HTTP at `GET /api/state/timecode`, which reports every input and which one is in use. See [API](13-api.md).

### BPM and Timecode Readouts

The BPM and position readouts sit side by side in the top bar in both Performance and Arrangement mode. You can use both at once, for example a timecode-locked intro while beat-locked LFOs follow the DJ.

Timecode carries no tempo. Chasing timecode does not give beat-locked modulators a clock, so they still need a BPM source.

Each readout is **dimmed when nothing is reading it**:

- The BPM dims when there is no clock source, or when no modulator is set to the beat.
- The position is grey before the show has started, and colored by status after that.

Hover either readout to see how many modulators follow it. This helps when something moves unexpectedly:

- **Free-run** modulators move whenever Varda is open.
- **Beat** modulators move whenever a clock is present.
- **Transport** modulators wait for Play.

See [Timebase](05-modulation.md#timebase).

### Frame Rates and Drop-Frame

`29.97 DF` (drop-frame) is the broadcast default. It is written with a semicolon before the frames, as in `01:00:00;00`. It skips frame *numbers* (never actual frames) so the label stays in step with wall time. Plain `29.97` does not skip numbers, and its label falls about 3.6 seconds behind wall time per hour. Both count the same real frames; only the labels differ.

### What Uses the Transport

- **Show timebase**: LFOs and step sequencers set to **Show** are computed from the position alone. See [Modulation](05-modulation.md).
- **Arrangement mode**: regions and automation curves are placed against this position, and take control of their decks once it has run. See [Arrangement Mode](15-arrangement.md).

---

## Macros

A **macro** is a control you build (a **knob**, **fader** or **button**) that drives many parameters at once. For example, one knob can move two effect parameters on two different decks, or one button can switch a whole look. Each macro is also a mappable parameter, so you can map a hardware knob to it with MIDI learn and the macro drives everything wired to it.

Macros sit in the **central mixer column**, below the mixer box and the transition sequence builder. Each macro shows a small **live control** (knob, fader or button) that you play in the column. As with the sequence builder, play the control in place, and click its card (the area around the control) to open the macro's settings in the bottom bar.

### Creating a Macro

Below the macro controls in the central column are three add buttons:

- **＋ Knob**: a rotary knob (drag up/down to sweep 0–1)
- **＋ Fader**: a linear 0–1 slider. It behaves the same as a knob.
- **＋ Button**: an on/off control with three press behaviors (see [Buttons](#buttons))

New macros are named `Macro 1`, `Macro 2`, … and get an accent color from the palette shared with modulation sources. The compact widget shows a color dot, the name, an **x** delete button and the live control. Clicking **x** removes the macro immediately, as it does for a transition sequence.

**Click the card around the control** to select the macro. The bottom bar switches to the macro's detail editor, which shows a larger control, a color dot, an editable **name** field, a **kind** selector, a **🗑 Delete** button, an **x Close** button, and the target (or trigger) editor. Dragging the knob or fader, or pressing the button, plays it and does *not* open the editor.

To rename a macro, type in its name field in the detail editor. Change its kind at any time with the kind dropdown. Switching to **Button** adds the button behavior options; switching away removes them.

### Binding Targets

A **target** is a mappable parameter the macro drives. Select the macro (click its name) to open its detail editor in the bottom bar, then:

1. Open the **＋ Add target** dropdown.
2. Pick a parameter. The list is grouped and labeled by location, for example `Deck 1 · Blur · radius`, `Ch 0 · opacity`, `Master · Glow · intensity` or `Crossfader`. It also lists **modulator parameters** such as `LFO 1 · frequency`, `ADSR 2 · release` or `StepSeq 1 · rate`, so a macro can change a modulation source (for example sweep an LFO's rate) as well as deck, channel and effect parameters.
3. The target is added with defaults (`min 0.0`, `max 1.0`, `Linear` curve, not inverted) and follows the macro immediately.

Each target has its own mapping row, so one macro can push each parameter through a different range and curve:

| Control | Meaning |
|---------|---------|
| **min** / **max** | The part of the parameter's range the macro sweeps. `min 0.2, max 0.9` uses only that part. Setting **min greater than max** inverts the response. |
| **inv** | Invert the response (same as swapping min and max): the target *falls* as the macro *rises*. Use it to open one effect while closing another with one control. |
| **curve** | The response curve applied before mapping into `[min, max]`: **Linear**, **Exp** (ease-in, slow start), **Log** (ease-out, fast start), **S-Curve** (ease-in-out), or **Stepped** (quantized into discrete levels, useful for stutter or enum-like params). |
| **x** | Remove the target. |

A macro can have any number of targets. One parameter can be a target of several macros; the last one moved wins, as with two MIDI CCs mapped to one parameter.

> **Example.** To control two effect parameters on two decks with one knob: add a Knob macro, add target `Deck A · FX1 · scale`, then add target `Deck B · FX2 · warp` and tick **inv** on the second. Turning the knob now opens the first effect and closes the second.

#### Macros and Modulation

A macro sets a parameter's **base** value. The modulation engine adds its offset **on top** every frame. A parameter can be driven by a macro and modulated at the same time.

A macro **cannot** target another macro, to prevent loops. Target the underlying parameters instead.

### Modulating a Macro

A **Knob** or **Fader** macro can be driven by a **modulation source** (LFO, ADSR, audio, step sequencer). The modulator then sweeps all of the macro's targets.

In the macro's detail editor (bottom bar), the **Mod** section lists each assigned source on its own row (color dot, name, and an **x** to remove that source). The **＋ Modulate** dropdown below adds more:

1. Pick a source from **＋ Modulate**. It appears as a new row in the source's color. Assign several to stack them; their offsets add up.
2. The value label reads, for example, `value 0.50 → 0.73`. The first number is your manual setting (the **base**). The second is the live **effective** value sent to the targets. The control also shows a colored **ghost** marker at the effective value (a ghost pointer on a knob, a ghost line on a fader). The base pointer stays where you set it.
3. Click a row's **x** to remove that source. The others keep driving the macro.

Modulation is added to the base. Turn the knob to move the center of the sweep, as with a modulated effect parameter. Each target still applies its own min, max, curve and invert to the modulated value, so one LFO can open one effect while closing another.

- Only **Knob** and **Fader** macros can be modulated. **Button** macros are on/off and cannot.
- Create modulators in the **Modulation** panel (see [Modulation](05-modulation.md)). Any source there can be assigned to a macro.
- Macro modulation assignments are saved **per scene** (in `scene.json`) and can be undone.

> Tip: you can also modulate a macro's target parameters directly from their deck or effect panels. Modulating the macro moves all its targets together; modulating a target moves only that one.

### Buttons

A Button macro has three behaviors:

| Behavior | Press | Release |
|----------|-------|---------|
| **Momentary** | drive all targets to their **max** | drive all targets back to **min** |
| **Toggle** | each press switches targets between **max** and **min** | (ignored) |
| **Trigger** | fire one-shot **actions** once, on the press | (ignored) |

Set the behavior in the macro's detail editor (bottom bar). Momentary and Toggle buttons use the same **target** list as knobs and faders. A **Trigger** button shows an **On press** editor instead:

- **Undo / Redo / Save** checkboxes run that app action on press.
- **＋ Add param** adds an action that writes a fixed value (`1.0`) to a path on press, for example `deck/<uuid>/trigger` to set a deck to full opacity. Remove one with **x**.

> Trigger buttons fire once and hold no state. Use them to map a pad to Undo, a reset snapshot, or a deck slam.

### Mapping a Macro to MIDI / OSC / Keyboard

A macro's address is `macro/<uuid>/value`, so MIDI, OSC and the keyboard can control it with no extra setup:

- **MIDI**: enter **MIDI Learn** (right-click, *Enter MIDI Learn*), click a macro's live control in the central column (it glows purple like any other control), and move a hardware control. A button macro maps to a pad: note-on sends `1.0`, note-off sends `0.0`. See [MIDI](#midi).
- **OSC**: send `/varda/macro/<uuid>/value <0..1>`. Get the UUID from `GET /api/scene/macros` or `GET /api/state`.
- **Keyboard**: keyboard learn can bind a key to a macro (useful for buttons) through the same value path.


---

## Parameter Paths

MIDI, OSC and keyboard shortcuts all use the same parameter paths:

| Path | Description |
|------|-------------|
| `crossfader` | Mixer crossfader (0.0–1.0) |
| `clock/bpm` | Manual BPM (mapped 0.0–1.0 → 20–300 BPM for MIDI) |
| `deck/<uuid>/opacity` | Deck opacity |
| `deck/<uuid>/mute` | Deck mute toggle |
| `deck/<uuid>/solo` | Deck solo toggle |
| `deck/<uuid>/trigger` | Set deck opacity to 1.0 |
| `deck/<uuid>/param/<name>` | Shader parameter |
| `deck/<uuid>/video/play` | Set video play state (playing when > 0.5) |
| `deck/<uuid>/video/speed` | Video playback speed (0.0–1.0 → 0.1×–4.0×) |
| `deck/<uuid>/video/position` | Seek position (0.0–1.0 → start–end of clip); `video/seek` also works |
| `deck/<uuid>/video/in_point` | Loop in-point (0.0–1.0 → start–end of clip) |
| `deck/<uuid>/video/out_point` | Loop out-point (0.0–1.0 → start–end of clip) |
| `deck/<uuid>/video/clear` | Clear in/out points (trigger, > 0.5) |
| `deck/<uuid>/video/loop_mode` | Loop mode, fader range split into steps (Loop / Ping-Pong / One Shot / Hold Last) |
| `deck/<uuid>/scaling_mode` | Source scaling, fader range split into steps (Fill / Fit / Stretch / Center) |
| `deck/<uuid>/transparent` | Transparent background toggle (> 0.5) |
| `deck/<uuid>/html/reload` | Reload an HTML deck's page (> 0.5) |
| `deck/<uuid>/html/interactive` | Open or close the interactive window on an HTML deck (> 0.5) |
| `deck/<uuid>/capture/rate` | Screen-capture rate (0.0–1.0 → 1–120 fps) |
| `deck/<uuid>/capture/crop_x` | Screen-capture crop origin X (0.0–1.0) |
| `deck/<uuid>/capture/crop_y` | Screen-capture crop origin Y (0.0–1.0) |
| `deck/<uuid>/capture/crop_w` | Screen-capture crop width (0.0–1.0) |
| `deck/<uuid>/capture/crop_h` | Screen-capture crop height (0.0–1.0) |
| `deck/<uuid>/capture/cursor` | Include the mouse pointer (toggle, > 0.5) |
| `deck/<uuid>/capture/exclude_varda` | Leave Varda's own windows out of a display capture (toggle, > 0.5) |
| `ch/<uuid>/opacity` | Channel opacity |
| `effect/<effect_uuid>/param/<name>` | Effect parameter, on a deck, a channel or the master chain. The longer `deck/<uuid>/effect/...`, `ch/<uuid>/effect/...` and `master/effect/...` forms also work |
| `mod/<mod_uuid>/frequency` | LFO frequency |
| `mod/<mod_uuid>/amplitude` | LFO amplitude |
| `mod/<mod_uuid>/step/<n>` | Step-sequencer step value (step index is the step's position in the source) |
| `macro/<uuid>/value` | Macro control (0.0–1.0); drives all the macro's targets. See [Macros](#macros) |
| `action/undo` | Trigger undo |
| `action/redo` | Trigger redo |
| `action/save` | Trigger save |
| `action/record` | Arm or disarm automation recording (> 0.5). See [Recording a pass](15-arrangement.md#recording-a-pass) |
| `cue/<uuid>/fire` | Take the show to that cue (> 0.5). See [Cue pads](15-arrangement.md#cue-pads-in-performance-mode) |

---

[← Prev: Modulation & Audio Reactivity](05-modulation.md) · [Home](README.md) · [Next: Outputs →](07-outputs.md)
