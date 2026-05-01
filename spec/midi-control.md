# MIDI Control System

## Status: IMPLEMENTED (core — persistence and LED feedback are stretch goals)

## Goal

Resolume-style MIDI mapping: any parameter in the UI can be mapped to any MIDI control. Support presets for common controllers so button LEDs and feedback work correctly.

## Priority Controller

**Akai APC Mini** — compact, affordable, popular with VJs. Grid of buttons + faders.

## Architecture

### MIDI Mapping System

1. **Learn Mode**: Click a parameter in the UI, move a MIDI control, mapping is created
2. **Mapping Store**: Persistent map of `MIDI CC/Note → Parameter Path`
3. **Parameter Paths**: Hierarchical addressing for any controllable value
   - `channel/a/deck/0/opacity`
   - `channel/a/deck/0/effect/1/param/blur_amount`
   - `crossfader/position`
   - `master/effect/0/param/intensity`
   - `channel/b/deck/2/generator/param/speed`

### Controller Presets

Pre-built mappings for common controllers:
- **Akai APC Mini**: Grid → deck triggers, faders → channel/deck opacity, buttons → solo/mute
- **Novation Launchpad**: Grid → scene triggers, side buttons → channel select
- **Generic**: Just CC mapping, no LED feedback

### LED Feedback

For controllers with LEDs (APC Mini, Launchpad):
- Button LEDs reflect state (solo/mute on/off, active deck highlighted)
- Velocity-based LED colors where supported
- Requires knowing controller's LED protocol (varies per device)

### MIDI Crate Options

- `midir` — pure Rust, cross-platform MIDI I/O
- `wmidi` — MIDI message parsing

## Open Questions

- How to handle MIDI clock sync (for BPM-locked transitions)?
- Should MIDI mapping be per-scene or global?
- How to handle multiple MIDI devices simultaneously?

