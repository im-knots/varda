# Scene Management

## Status: AGREED (basic scene save/load implemented; channel/deck presets not yet implemented)

Relates to: [/spec/channel-routing.md](/spec/channel-routing.md), [/spec/transitions.md](/spec/transitions.md)

## Hierarchy

```
Scene (the performance)
├── Channel A
│   ├── Deck 1 (source + params)
│   ├── Deck 2 (source + params)
│   ├── Deck 1 FX chain
│   ├── Deck 2 FX chain
│   └── Channel A FX chain
├── Channel B
│   ├── Deck 3 (source + params)
│   ├── Deck 4 (source + params)
│   └── Channel B FX chain
├── Crossfader / Transition builder config
├── Master FX chain
├── Modulation engine state (all sources + assignments)
├── MIDI mappings
└── Output configuration (resolution, multi-output routing)
```

## One Scene Per Gig

A scene is a **project file**. You build it before the show, load it at the venue, perform within it, save when done.

There is no scene switching or scene bank during performance. All live manipulation happens within the scene — swapping decks between channels, triggering transitions, tweaking parameters, loading channel presets.

## Saveable Units

Three levels of save/load, from smallest to largest:

### 1. Deck Preset
Save/load an individual deck configuration:
- Source type + source reference (shader path, video path, image path)
- All source parameters (shader uniforms, video playback settings, scaling mode)
- Deck effect chain (ordered list of effects with their parameters)

### 2. Channel Preset
Save/load an entire channel composition:
- All decks in the channel (each with their full deck preset data)
- Per-deck opacity and blend mode within the channel
- Channel effect chain

### 3. Scene (Full Project)
Save/load the entire performance state:
- All channels with all their contents
- Crossfader position and transition configuration
- Transition builder sequences (if any)
- Master effect chain
- Modulation engine (all sources and all parameter assignments)
- MIDI mappings
- Output configuration (resolution, multi-output routing, projection mapping warp)

## File Format

JSON for human readability and version control friendliness. Structure mirrors the hierarchy above.

Asset references (shader paths, video paths, image paths) are stored as relative paths where possible, with a configurable asset search path for portability between machines.

## Scene Workflow

1. **Create**: Start with an empty scene or a template
2. **Build**: Add channels, populate with decks, set up effects, configure transitions
3. **Test**: Preview and tweak at home/studio
4. **Save**: Save scene file
5. **Load at venue**: Open scene file, audio input auto-connects, MIDI devices auto-map
6. **Perform**: Manipulate live within the scene
7. **Save after**: Optionally save any changes made during performance

## Open Questions

- Should scenes support an "asset pack" export that bundles all referenced files (shaders, videos, images) into a portable archive?
- Should there be scene templates (e.g., "2-channel starter", "installation mode")?
- How to handle missing assets on load? (shader path doesn't exist on this machine)

