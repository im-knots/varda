# MIDI Control System

## Status: IMPLEMENTED (core + multi-device + APC Mini mk1 LED feedback + UI panel)

## Goal

Pure MIDI-learn mapping: any parameter in the UI can be mapped to any MIDI control. Supports N simultaneous MIDI devices with per-device mapping. Controller profiles provide LED feedback for controllers that support it.

## Priority Controller

**Akai APC Mini mk1** — compact, affordable, popular with VJs. 8×8 button grid + 9 faders.

## Architecture

### Multi-Device Support — IMPLEMENTED

- `MidiDeviceManager` handles N simultaneous MIDI devices (input + output)
- Each device gets a unique `DeviceId` (u32) assigned at scan time
- `MidiKey` includes `DeviceId` so the same CC# on different controllers maps independently
- Devices can be enabled/disabled individually from the UI
- Hot-plug supported via "Rescan" button
- Controller profiles auto-detected by device name (e.g., "APC MINI" → `ApcMini` profile)

### MIDI Mapping System — IMPLEMENTED

1. **Learn Mode**: Right-click anywhere in UI → "Enter MIDI Learn" → all mappable params glow purple → click a param to select it → move a MIDI control → mapping created → stays in learn mode to map more → right-click → "Exit MIDI Learn"
2. **Mapping Store**: In-memory map of `MidiKey → Parameter Path` (device-aware keys)
3. **Parameter Paths**: Hierarchical addressing for any controllable value
   - `crossfader` → mixer crossfader position
   - `ch/<n>/opacity` → channel opacity
   - `ch/<n>/deck/<m>/opacity` → deck opacity
   - `ch/<n>/deck/<m>/mute` → deck mute toggle
   - `ch/<n>/deck/<m>/solo` → deck solo toggle
   - `ch/<n>/deck/<m>/trigger` → deck trigger (sets opacity to 1.0)
   - `ch/<n>/deck/<m>/param/<name>` → generator param (float)
   - `ch/<n>/deck/<m>/effect/<k>/param/<name>` → deck effect param
   - `ch/<n>/effect/<k>/param/<name>` → channel effect param
   - `master/effect/<k>/param/<name>` → master effect param
   - `mod/<idx>/frequency` → LFO frequency (0.01–10 Hz)
   - `mod/<idx>/amplitude` → LFO amplitude (0–1)
   - `mod/<idx>/phase` → LFO phase offset (0–1)
   - `mod/<idx>/smoothing` → Audio band smoothing (0–0.99)
   - `mod/<idx>/attack` → ADSR attack time (0.001–5s)
   - `mod/<idx>/decay` → ADSR decay time (0.001–5s)
   - `mod/<idx>/sustain` → ADSR sustain level (0–1)
   - `mod/<idx>/release` → ADSR release time (0.001–5s)
   - `mod/<idx>/rate` → StepSequencer rate (0.1–20 Hz)
   - `mod/<idx>/step/<step_idx>` → StepSequencer step value (0–1)

### MIDI Output — IMPLEMENTED

- Uses midir for cross-platform MIDI I/O (CoreMIDI on macOS, ALSA/JACK on Linux, WinMM on Windows)
- `MidiDeviceManager` handles output per device (send by `DeviceId`)
- Source/destination pairing by name matching
- Sends Note On messages to control button LEDs

### Controller Profiles — IMPLEMENTED

Controller-specific knowledge lives in profile modules (`src/midi/apc_mini.rs`).
Profiles are purely for LED feedback — no hardcoded mappings. All control mapping is via MIDI learn.
`ApcMiniManager` tracks N APC Mini devices, each with independent LED state.

### MIDI UI Panel — IMPLEMENTED

Bottom panel section showing:
- Connected devices with enable/disable toggles and profile badges
- Rescan button for hot-plug
- Mappings table: device name, control key, parameter path, delete button
- Clear All mappings button

#### Akai APC Mini mk1 Protocol

**Physical layout:**
- 8×8 grid: Notes 0–63 (row 0 = bottom, row 7 = top; col 0 = left, col 7 = right)
- 8 side buttons (right column): Notes 82–89
- 8 bottom buttons: Notes 64–71 (▲ ▼ ◀ ▶ Volume Pan Send Device)
- Shift: Note 98
- 9 faders: CC 48–56 (faders 1–8 + master fader 9)

**LED control** — send Note On (0x90) to the device:
- Velocity 0 = off
- Velocity 1 = green (solid)
- Velocity 2 = green (blink)
- Velocity 3 = red (solid)
- Velocity 4 = red (blink)
- Velocity 5 = yellow (solid)
- Velocity 6 = yellow (blink)

**LED feedback rules** (driven by mapped parameter state):
- **Boolean/toggle params** (mute, solo, effect on/off): green = on, off = off
- **Active state** (currently selected deck): yellow
- **MIDI learn active on this control**: red blink
- **Mapped but idle**: green dim (vel 1)
- Faders have no LEDs — no feedback needed

### LED State Machine

Each frame, the LED manager:
1. Iterates all MIDI mappings that are Note-type keys
2. Reads the current value of the mapped parameter from the mixer
3. Determines the correct LED color based on parameter type and value
4. Compares to last-sent LED state — only sends if changed (avoids MIDI flood)

## Open Questions

- MIDI clock sync for BPM-locked transitions?
- Mapping persistence (save/load to JSON config file)?
- Multiple controller profiles active simultaneously?

