# Persistence

## Status: AGREED

Relates to: [/spec/scene-management.md](/spec/scene-management.md), [/vision/parity-gap.md](/vision/parity-gap.md)

## Design Philosophy

Varda is lightweight. Run the binary in a directory and you're off. No registry, no internal config database, no `~/.config/varda/`. The working directory **is** the project.

## Workspace Layout

```
my-show/
├── shaders/          # ISF/GLSL shaders (scanned by library)
├── media/            # Videos, images (user-organized)
└── .varda/           # Auto-created by Varda on first save
    ├── scene.json    # Decks, channels, effects, mixer state, surfaces, outputs, warp
    ├── midi.json     # MIDI controller mappings (hardware-dependent, separate from scene)
    └── stage.json    # Stage editor preferences (grid size, snap, tool state)
```

### Why Multiple Files

- **scene.json** is the show. Copy it to another machine with the same shaders/media and it works.
- **midi.json** is hardware-specific. You might share a show file without your personal MIDI setup, or use different controllers at different venues.
- **stage.json** is editor preferences, not content. Lightweight, changes often during setup.

### Why `.varda/` (Hidden Directory)

- Keeps the workspace root clean — only user content (`shaders/`, `media/`) is visible
- Follows Unix convention for tool metadata (`.git/`, `.cargo/`, `.vscode/`)
- Room to grow (logs, cache, thumbnails) without cluttering the root

## What Gets Persisted

### scene.json — The Show

Everything needed to reconstruct the performance state:

| State | Source | Notes |
|-------|--------|-------|
| Channels (count, names) | `Mixer` | |
| Decks per channel | `Channel` | Source type, shader path, video path, image path |
| Deck parameters | `Deck` | All ISF uniform values, opacity, blend mode, mute, solo |
| Deck effect chains | `Deck` | Ordered effects with params, enabled state |
| Channel effect chains | `Channel` | Same as deck effects |
| Master effect chain | `Mixer` | Same as deck effects |
| Crossfader position | `Mixer` | Float 0..1 |
| Active transition | `Mixer` | Transition shader name |
| Surface layout | `SurfaceManager` | Already has Serialize/Deserialize derives |
| Surface assignments per output | `OutputWindow` | Which surfaces each output renders |
| Warp calibration corners | `SurfaceAssignment` | Already has Serialize/Deserialize derives |
| Output window config | `OutputWindow` | Name, target display name (not monitor index — indices change) |
| Modulation sources | `ModulationEngine` | LFO/ADSR/step seq configs |
| Modulation assignments | `ModulationEngine` | Source → parameter path mappings |

Asset references (shader paths, video paths, image paths) are stored as **relative paths** when they fall within the workspace directory, **absolute paths** otherwise. This keeps shows portable when the workspace is copied to another machine.

### midi.json — Controller Mappings

| State | Source | Notes |
|-------|--------|-------|
| MIDI mappings | `MidiMappingStore` | MidiKey → parameter path (e.g., `ch/0/deck/1/param/speed`) |

Keyed by MIDI device name + channel + CC/note number, not by OS device index (indices change between sessions). On load, mappings are matched to currently connected devices by name.

### stage.json — Editor Preferences

| State | Source | Notes |
|-------|--------|-------|
| Grid size | `App` | Float (0.05 default) |
| Snap enabled | `App` | Bool |
| Library panel open | `App` | Bool |
| Stage editor open | `App` | Bool |

## Save Triggers

1. **Manual save**: `Ctrl+S` (Linux) / `Cmd+S` (macOS) — saves all three files
2. **Save on exit**: Auto-save when the app is closed cleanly (window close, quit)
3. **No auto-save during runtime** — avoids any disk I/O hitches during live performance

Save is synchronous and fast (small JSON files, <1ms). No background thread needed.

## Load Behavior

On startup, Varda checks for `.varda/` in the current working directory:

- **`.varda/scene.json` exists**: Load and reconstruct the full scene (channels, decks, surfaces, outputs, warp, modulation). Missing assets (shader/video/image files that no longer exist) produce a warning notification but don't prevent loading — affected decks show black.
- **`.varda/midi.json` exists**: Load MIDI mappings. Mappings for devices not currently connected are kept in memory (they'll activate if the device is plugged in later).
- **`.varda/stage.json` exists**: Load editor preferences.
- **No `.varda/` directory**: Start fresh. Directory is created on first save.

## What Is NOT Persisted

- Runtime-only state: audio levels, BPM detection, beat phase
- Camera sessions (cameras are re-detected on startup, user re-drags to decks)
- Output window OS handles (windows are re-created from config on load)
- MIDI learn mode state (always starts with learn mode off)
- Notification history
- egui widget state (scroll positions, collapsed headers)

## Open Questions

- Should output display targets be matched by monitor name, position, or resolution on load? Monitor names are most stable but can be ambiguous with identical displays.
- Should we support a `--workspace <path>` CLI flag to override CWD? Useful for desktop shortcuts.
- Should `Ctrl+Shift+S` do "save as" (copy workspace to new directory)?
