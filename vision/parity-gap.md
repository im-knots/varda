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
- ✅ N-channel compositing (3+ channels with correct alpha blending)

### Multi-Output & Projection Mapping
- ✅ Multi-window output system (SharedGPU + per-window surfaces)
- ✅ Create/destroy output windows at runtime
- ✅ Display target selector: dropdown listing all connected monitors/projectors (auto-detected via winit)
- ✅ Five-layer signal hierarchy: Deck → Channel → Mixer → Surface → Output → Display
- ✅ Multi-window event dispatch (WindowId routing)
- ✅ 2D Stage Editor with polygon surface model
- ✅ Drawing tools: Rectangle, Polygon (click-to-place), Circle/N-gon (configurable sides 3–128)
- ✅ Vertex editing: drag individual vertices, double-click edge to insert vertex
- ✅ Surface manipulation: Duplicate (D), Flip Horizontal (H), Flip Vertical (V)
- ✅ Click-to-select surface interiors (ray-casting point-in-polygon)
- ✅ Auto-tool switching: drawing tools auto-switch to Select when clicking inside existing surfaces
- ✅ Configurable grid with snap-to-grid toggle (10%, 5%, 2.5%, 1.25%)
- ✅ Polygon rendering pipeline (fan-triangulated textured polygons in output windows)
- ✅ Content mapping modes: Fill (full texture per surface) and Mapped (spatial UV crop)
- ✅ Surface source routing (each surface can show master, channel, or deck content)

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
- ✅ Stage editor replaces deck view when open (full central panel canvas)

## Critical Missing Features (Blocks Real Use)

### Tier 1 — Cannot Gig Without These
1. ~~**A/B Channel routing with crossfader**~~ ✅ IMPLEMENTED
2. ~~**MIDI controller support**~~ ✅ IMPLEMENTED
3. ~~**Stability / crash resilience**~~ ✅ IMPLEMENTED
4. ~~**Multi-output**~~ ✅ IMPLEMENTED — multi-window outputs, content routing, fullscreen on projectors
5. ~~**Projection mapping**~~ ✅ IMPLEMENTED — 2D surface editor, surface-to-output assignment, per-surface corner-pin warp with DLT homography, calibration UI with draggable corners. Perspective-correct GPU rendering.
6. ~~**Auto-transitions**~~ ✅ IMPLEMENTED
7. ~~**Per-channel effect chains**~~ ✅ IMPLEMENTED

### Tier 2 — Expected by Professionals
8. ~~**MIDI learn / mapping UI**~~ ✅ IMPLEMENTED
9. ~~**Fullscreen output**~~ ✅ IMPLEMENTED — display target selector with monitor enumeration
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
21. **Advanced projection mapping** — 3D model import (Phase 8e), edge blending (Phase 8g)
22. **Recording** — capture output to video file
23. **Plugin API** — let users extend Varda with custom sources/effects
24. **MIDI mapping persistence** — save/load mappings to config file
25. **LED feedback** — controller LED state (e.g., Akai APC Mini)

## Competitive Comparison

### vs. Resolume Arena/Avenue (~$400–$800)
**Where Varda matches or exceeds:**
- ISF shader pipeline (same format Resolume uses)
- Modulation engine (LFO, ADSR, step seq, audio bands, mod-on-mod — Resolume has this but Varda's is comparable)
- Channel/mixer architecture with crossfader
- Multi-window output with content routing
- Polygon surface editor for stage design

**Where Resolume is ahead:**
- Edge blending for multi-projector overlap
- DXV/HAP codec support for high-perf video
- Massive built-in effects/source library
- MIDI/OSC mapping persistence and LED feedback
- Undo/redo, copy/paste, preset library
- Recording to video file
- Syphon/Spout/NDI inter-app sharing
- Years of stability and edge-case hardening

### vs. TouchDesigner (free for non-commercial / ~$600+)
TouchDesigner is a different paradigm (node-based programming environment) — not directly comparable as a performance tool. Varda targets the "load-and-play" VJ workflow, not the "build-your-own-tool" approach. TouchDesigner is more powerful but has a much steeper learning curve.

### vs. VDMX (~$200)
macOS-only like Varda currently. VDMX has deeper plugin ecosystem and Quartz Composer integration. Varda's GPU-native wgpu pipeline and ISF shader support give it a more modern rendering foundation.

### vs. MadMapper (~$400)
MadMapper specializes in projection mapping. Its surface/warp/calibration tools are best-in-class. Varda's Phase 8c/8d will need to match MadMapper's corner-pin workflow to be competitive for mapping use cases.

## The Honest Assessment

Varda has crossed the threshold from "tech demo" into **"usable VJ tool for basic gigs."** The rendering core, channel/mixer architecture, crossfader, modulation engine, MIDI control, multi-window output, fullscreen projection, and 2D stage editor are all working. A VJ can now: load shaders and media, mix between channels with effects, modulate parameters with LFOs and audio, control via MIDI, design a stage layout with polygon surfaces, and send content to a projector in fullscreen.

**All Tier 1 blockers are cleared.** Varda can now drive a projector with corner-pin calibration aligned to physical surfaces. What remains is professional polish: performance monitoring, preset management, recording, and the long tail of quality-of-life features that separate "it works" from "I trust it for a paid show."

**Biggest competitive gap**: Resolume's years of edge-case hardening, massive content library, and polish. Varda's advantage is its modern GPU pipeline (wgpu), open shader format (ISF), and the potential for a more hackable/extensible architecture.

