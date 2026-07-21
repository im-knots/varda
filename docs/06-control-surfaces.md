# Control Surfaces — MIDI, OSC & Keyboard

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
| `action/undo` | Trigger undo |
| `action/redo` | Trigger redo |
| `action/save` | Trigger save |

Entity UUIDs are stable 8-character hex strings that persist across moves, reorders, and scene save/restore. Decks, channels, effects, and modulation sources are all addressed by UUID — never by positional index — so a saved mapping keeps targeting the same entity after a chain or rack is reordered. LED feedback reads state back through the same UUID paths.

The `video/*` and `scaling_mode` paths resolve only on the matching source type (video controls on video decks; `scaling_mode` on image, video, camera, and external-source decks). **Discrete (enum) controls** — `loop_mode` and `scaling_mode` — use **fader bucketing**: the 0.0–1.0 range is split into equal segments so a fader or knob sweeps through the options. **Seek and in/out points** scale the 0.0–1.0 value against the clip's duration, so one mapping works for clips of any length.

---

[← Prev: Modulation & Audio Reactivity](05-modulation.md) · [Home](README.md) · [Next: Outputs →](07-outputs.md)
