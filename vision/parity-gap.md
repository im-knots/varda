# Parity Gap: Varda vs. Professional VJ Software

## What Varda Has Today

### Core Rendering & Architecture
- ✅ Multi-deck rendering with ISF shaders (generators, filters, transitions)
- ✅ Per-deck opacity and blend modes (Normal, Add, Multiply, Screen, Overlay, Difference)
- ✅ 3-level effect chain hierarchy: Deck FX → Channel FX → Master FX
- ✅ Channel/Mixer architecture (Deck → Channel → Mixer → Output signal flow)
- ✅ A/B channel crossfader with snap, auto-transition (timed/beat-synced), easing
- ✅ Shader-based transitions between channels (ISF transition shaders)
- ✅ Audio reactivity (FFT, beat detection, bass/mid/treble bands)
- ✅ Video playback (ffmpeg-next)
- ✅ Image/still deck source (PNG, JPG) and solid color source
- ✅ Scaling modes (Fill, Fit, Stretch, Center)
- ✅ Shader hot-reload with file watching
- ✅ Shader browser with library folder scanning and categorization
- ✅ GPU command buffer batching (18 submits → 3 per frame with 8 decks)
- ✅ Zero-opacity deck culling (invisible decks skip rendering entirely)

### Modulation & Control
- ✅ Full modulation engine: LFO, audio band, ADSR envelope, step sequencer
- ✅ Universal modulation: any numeric parameter modulatable (generators, deck FX, channel FX, master FX)
- ✅ Modulator-on-modulator chaining (depth-limited)
- ✅ MIDI input with learn mode (click-to-map any parameter)
- ✅ MIDI crossfader mapping
- ✅ OSC input/output
- ✅ Scene save/load (JSON)

### UI
- ✅ Resolume-inspired layout: channel columns with deck grids, central mixer box, right sidebar, bottom bar
- ✅ DJ-style mixer box: channel opacity faders, crossfader, auto-transition buttons, transition selector, blend mode selectors
- ✅ Context-sensitive bottom bar: deck detail (preview + generator params + effect chain columns), channel effects, master effects
- ✅ Deck preview in bottom bar scales with panel height
- ✅ Modulation and shader library in right sidebar with horizontal column layout for modulators
- ✅ Main output preview (always visible in right panel)
- ✅ Dark theme with accent colors: purple (Channel A), blue (Channel B), orange (modulation), green (audio)
- ✅ Live deck preview thumbnails in channel grid
- ✅ Notification system (non-modal toasts for errors/info)
- ✅ Resizable bottom panel
- ✅ 1920×1080 default window size

## Critical Missing Features (Blocks Real Use)

### Tier 1 — Cannot Gig Without These
1. ~~**A/B Channel routing with crossfader**~~ ✅ IMPLEMENTED
2. ~~**MIDI controller support**~~ ✅ IMPLEMENTED
3. ~~**Stability / crash resilience**~~ ✅ IMPLEMENTED
4. **Multi-output** — need to send to projector while keeping UI on laptop → DESIGNED ([spec/multi-output.md](/spec/multi-output.md)), three-layer architecture (Content → Surfaces → Outputs)
5. **Projection mapping** — full spatial mapping system: 2D surface editor, camera-based projection, LED direct output, corner-pin calibration → DESIGNED ([spec/multi-output.md](/spec/multi-output.md))
6. ~~**Auto-transitions**~~ ✅ IMPLEMENTED
7. ~~**Per-channel effect chains**~~ ✅ IMPLEMENTED

### Tier 2 — Expected by Professionals
8. ~~**MIDI learn / mapping UI**~~ ✅ IMPLEMENTED
9. **Fullscreen output** — borderless fullscreen on target display → covered by Phase 8a in multi-output spec
10. **Performance monitoring** — FPS counter, GPU usage, frame timing
11. **Undo/redo** — for parameter changes, deck additions, etc.
12. ~~**Image/still support**~~ ✅ IMPLEMENTED
13. **Channel presets** — save/load individual channel compositions
14. **Transition builder** — sequenced transitions for 3+ channel setups and installations

### Tier 3 — Stretch Goals & Competitive Differentiation
15. **Keyboard shortcuts** — spacebar, arrow keys, number keys for deck control
16. **Shader editor** — built-in ISF editor with syntax highlighting
17. **Shadertoy import** — auto-convert Shadertoy shaders to ISF
18. **NDI input/output** — inter-app video sharing
19. **Syphon / Spout** — inter-app video sharing (macOS / Windows)
20. **MIDI clock sync** — lock to external tempo
21. **Advanced projection mapping** — multi-surface (Phase 8d), 3D model import (Phase 8e), edge blending (Phase 8g)
22. **Recording** — capture output to video file
23. **Plugin API** — let users extend Varda with custom sources/effects
24. **MIDI mapping persistence** — save/load mappings to config file
25. **LED feedback** — controller LED state (e.g., Akai APC Mini)

## The Honest Assessment

Varda has evolved from a tech demo into a **functional VJ tool with a professional workflow foundation**. The rendering core, channel/mixer architecture, crossfader system, modulation engine, MIDI control, and UI layout are all working. The GPU rendering pipeline has been optimized with command buffer batching and visibility culling, handling 8+ decks of complex generative shaders without framerate collapse. A VJ could load shaders, mix between two channels, apply effects at every level, modulate parameters with LFOs and audio, and control it via MIDI.

The remaining gap is in **venue output and polish**: multi-output to projectors, fullscreen display windows, projection mapping, performance monitoring, and preset management. These are the features that separate "works on my laptop" from "works at a gig."

