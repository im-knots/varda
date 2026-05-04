# Parity Gap: Varda vs. Professional VJ Software

## What Varda Has Today

### Core Rendering & Architecture
- ✅ Multi-deck rendering with ISF shaders (generators, filters, transitions)
- ✅ Per-deck opacity and blend modes (Normal, Add, Multiply, Screen, Overlay, Difference)
- ✅ 3-level effect chain hierarchy: Deck FX → Channel FX → Master FX
- ✅ Channel/Mixer architecture (Deck → Channel → Mixer → Output signal flow)
- ✅ A/B channel crossfader with snap, auto-transition (timed/beat-synced), easing curves
- ✅ Shader-based transitions between channels (ISF transition shaders: dissolve, iris, push, zoom, luma key)
- ✅ Audio reactivity (512-bin FFT, bass-driven beat detection, bass/mid/treble bands, BPM + beat phase)
- ✅ Video playback: ffmpeg CPU decode + HAP GPU-native (BC1/BC3/BC7/YCoCg/BC4, HAP Q Alpha dual-plane with YCoCg→RGB conversion shader). Playback modes: loop, ping-pong, one-shot, hold-last. Speed control, in/out points, seek.
- ✅ Image/still deck source (PNG, JPG) and solid color source
- ✅ Scaling modes: Fill, Fit, Stretch, Center (UV transform computation per mode)
- ✅ Shader hot-reload with `notify` file watcher (auto-reload on save, keep last good shader on error)
- ✅ Library panel with folder scanning (`WalkDir` recursive scan of platform-specific shader directories)
- ✅ GPU command buffer batching (deck renders collected into Vec, single `queue.submit()` per batch — ~3 submits/frame vs. 18 without batching)
- ✅ Zero-opacity culling at both deck and channel level (skip render entirely when opacity ≤ 0 or muted)
- ✅ N-channel compositing (unlimited channels, min 2; crossfader for 2-channel, per-channel opacity for 3+)

### Multi-Output & Projection Mapping
- ✅ Multi-window output system (SharedGPU + per-window wgpu surfaces)
- ✅ Create/destroy output windows at runtime via UI
- ✅ Display target selector: dropdown enumerating all connected monitors/projectors (winit `available_monitors()`)
- ✅ Five-layer signal hierarchy: Deck → Channel → Mixer → Surface → Output → Display
- ✅ Multi-window event dispatch (WindowId routing in ApplicationHandler)
- ✅ 2D Stage Editor with polygon surface model (full-screen canvas, replaces deck view when open)
- ✅ Drawing tools: Rectangle (click-drag), Polygon (click-to-place, double-click/close to finish), Circle (click-drag with CircleHint metadata)
- ✅ True circle support: `CircleHint` preserves center/radius/sides — radius DragValue, sides DragValue, interactive radius handle drag, "Convert to Polygon" to drop circle identity. Vertex drag on circle auto-converts. Circle hint center syncs on surface move.
- ✅ Vertex editing: drag individual vertices, double-click edge to insert vertex (point-to-segment projection, snaps to grid)
- ✅ Surface manipulation: Duplicate (D key), Flip Horizontal (H), Flip Vertical (V) — all work on multi-selection
- ✅ Click-to-select surface interiors (ray-casting point-in-polygon), multi-select (Shift+click toggle, marquee drag), multi-move
- ✅ Auto-tool switching: drawing tools auto-switch to Select when clicking inside existing surfaces
- ✅ Configurable grid with snap-to-grid toggle (10%, 5%, 2.5%, 1.25%)
- ✅ Polygon rendering pipeline (fan-triangulated textured polygons in output windows, bounding-box UV)
- ✅ Content mapping modes: Fill (full texture per surface) and Mapped (canvas position = UV crop)
- ✅ Surface source routing (each surface can show master, channel, or deck content)
- ✅ Surface-to-output assignment (dropdown UI, unassigned = render all as fallback)
- ✅ Per-surface corner-pin warp (DLT homography from 4 corner correspondences, perspective-correct UV interpolation in vertex shader)
- ✅ Calibration mode with test cards (8 distinct colors, 8×8 grid, center crosshair + circle, corner brackets, edge midpoints, gradient bars)

### Modulation & Control
- ✅ Full modulation engine: LFO (6 waveforms), audio band (bass/mid/treble), ADSR envelope, step sequencer
- ✅ Universal modulation: any numeric parameter modulatable (generators, deck FX, channel FX, master FX)
- ✅ Modulator-on-modulator chaining (depth-limited to 4)
- ✅ MIDI input with learn mode (right-click param → move control → mapped), multi-device (N simultaneous controllers, device-aware keys)
- ✅ APC Mini mk1 controller profile with LED feedback (green/red/yellow, solid/blink, diff-only sends)
- ✅ MIDI UI panel (device list, enable/disable, rescan, mappings table, clear all, remove individual mappings)
- ✅ OSC input (port 9000, handles deck opacity/solo/mute/params) and output (configurable target)
- ✅ Scene save/load (JSON — saves decks, effects, opacity, blend modes, mute/solo)

### UI
- ✅ Three-column layout: Library (left sidebar, L toggle) | Stage/Decks (center) | Output/Control (right sidebar)
- ✅ Library panel: generators, effects, images, video, solid color — drag-and-drop to channels/effect chains
- ✅ Deferred DnD pattern: drop zone rects stored per-frame, resolved after all panels render (egui tooltip-layer workaround)
- ✅ Effect chain reordering: ⠿ grip-only drag handle, thin drop zones between effects
- ✅ DJ-style mixer box: channel opacity faders, crossfader, auto-transition buttons with easing, transition shader selector
- ✅ Context-sensitive bottom bar: deck detail (preview + generator params + effect chain), channel effects, master effects
- ✅ Live deck preview thumbnails in channel grid (egui textures re-registered every frame)
- ✅ Main output preview (always visible in right panel)
- ✅ Dark theme only (VJ venue aesthetic — purple/blue/orange/green accent colors)
- ✅ Notification system (non-modal toasts: severity levels, auto-dismiss, overlay rendering)
- ✅ 1920×1080 default window size and render resolution
- ✅ Deck drag between channels (hot reassignment via drag-and-drop thumbnails)
- ✅ Keyboard shortcuts: L (library toggle), stage editor tools (S/R/P/C/Esc), surface manipulation (D/H/V/Delete)

## What's Not Persisted

The following state is runtime-only and lost on exit:

- ❌ **Surface layout** — `SurfaceManager` has `Serialize`/`Deserialize` derives but `SceneConfig` has no field for surfaces
- ❌ **Output window configuration** — display targets, surface assignments not saved
- ❌ **Warp calibration corners** — computed per-session, not serialized to scene file
- ❌ **MIDI mappings** — `MidiMappingStore` is in-memory only, no disk persistence
- ❌ **Stage editor state** — grid size, snap, tool selection reset each session

This is the single biggest gap for real-world use. A VJ who sets up surfaces, assigns outputs, calibrates warp corners, and maps MIDI controllers will lose all of that work when they close the app.

## What's Missing

### Tier 1 — All Cleared ✅
1. ~~A/B Channel routing with crossfader~~ ✅
2. ~~MIDI controller support~~ ✅
3. ~~Stability / crash resilience~~ ✅
4. ~~Multi-output~~ ✅
5. ~~Projection mapping~~ ✅
6. ~~Auto-transitions~~ ✅
7. ~~Per-channel effect chains~~ ✅

### Tier 2 — Expected by Professionals
| # | Feature | Status | Notes |
|---|---------|--------|-------|
| 8 | ~~MIDI learn / mapping UI~~ | ✅ | Working, but mappings not persisted to disk |
| 9 | ~~Fullscreen output~~ | ✅ | Display target selector with monitor enumeration |
| 10 | Performance monitoring | ❌ Missing | Spec exists (`settings-and-monitoring.md`), no implementation |
| 11 | Undo/redo | ❌ Missing | No implementation anywhere |
| 12 | ~~Image/still support~~ | ✅ | PNG, JPG, solid color |
| 13 | State persistence | ❌ Missing | Surfaces, outputs, warp, MIDI mappings all lost on exit |
| 14 | Channel/deck presets | ❌ Missing | Save/load individual compositions |
| 15 | Transition builder | ❌ Missing | Sequenced transitions for 3+ channels and installations |

### Tier 3 — Competitive Differentiation
| # | Feature | Status | Notes |
|---|---------|--------|-------|
| 16 | Camera input | ✅ Done | Shared CameraManager, dedicated capture thread, SIMD YUV→RGBA (yuvutils-rs), macOS + Linux, 1 cam → N decks, library DnD, manual rescan |
| 17 | Keyboard shortcuts (performance) | ❌ Partial | Stage editor shortcuts only; no spacebar/arrow/number key deck control |
| 18 | Shader editor | ❌ Missing | No built-in ISF editor |
| 19 | Shadertoy import | ❌ Missing | |
| 20 | NDI input/output | ❌ Missing | |
| 21 | Syphon / Spout | ❌ Missing | |
| 22 | MIDI clock sync | ❌ Missing | Audio-based BPM detection works; no MIDI clock input |
| 23 | Edge blending | ❌ Missing | Multi-projector overlap feathering |
| 24 | 3D model import | ❌ Missing | OBJ/glTF → surfaces |
| 25 | LED Direct output | ❌ Partial | `SurfaceOutputType::LEDDirect` enum exists, no actual pipeline |
| 26 | Recording | ❌ Missing | Capture output to video file |
| 27 | Plugin API | ❌ Missing | |

## Competitive Comparison

### vs. Resolume Arena/Avenue (~$400–$800)

**Where Varda matches or exceeds:**
- ISF shader pipeline (same format Resolume uses)
- Modulation engine (LFO, ADSR, step seq, audio bands, mod-on-mod)
- Channel/mixer architecture with crossfader and shader transitions
- Multi-window output with surface-based content routing
- Polygon surface editor with true circles, multi-select, vertex editing
- Drag-and-drop workflow (library → channels, effects → chains, deck reordering)
- GPU-native HAP video (including Q Alpha dual-plane — matches Resolume's DXV pipeline)
- Per-surface corner-pin warp with calibration cards

**Where Resolume is ahead:**
- **State persistence** — Resolume saves everything: compositions, mappings, output configs, warp corners. Varda loses surfaces/outputs/warp/MIDI on exit.
- **Edge blending** for multi-projector overlap
- **Massive built-in content** — hundreds of effects/generators vs. Varda's ISF community library
- **Undo/redo** throughout
- **Recording** to video file
- **Syphon/Spout/NDI** inter-app video sharing
- **DXV codec** (Resolume's proprietary GPU codec; Varda has HAP but not DXV)
- **Performance monitoring** (FPS, GPU stats)
- **Years of stability** and edge-case hardening

### vs. TouchDesigner (free for non-commercial / ~$600+)
Different paradigm (node-based programming). Varda targets the "load-and-play" VJ workflow; TouchDesigner targets "build-your-own-tool." More powerful but much steeper learning curve. Not directly comparable for live performance.

### vs. VDMX (~$200)
macOS-only like Varda currently. VDMX has deeper plugin ecosystem and Quartz Composer integration. Varda's wgpu pipeline and ISF shader support are a more modern rendering foundation. VDMX has better state management and persistence.

### vs. MadMapper (~$400)
Specializes in projection mapping. Surface/warp/calibration tools are best-in-class with advanced features (edge blending, mesh warping, pixel mapping). Varda's polygon surface editor with per-surface corner-pin warp handles basic mapping workflows but lacks MadMapper's depth.

## The Honest Assessment

Varda has crossed from "tech demo" into **"usable VJ tool for basic gigs."** The rendering core, channel/mixer architecture, modulation engine, MIDI control with LED feedback, multi-window output, projection mapping with per-surface warp, and drag-and-drop library workflow are all functional. A VJ can: browse and drag content from the library, mix between channels with crossfader and shader transitions, modulate parameters with LFOs/audio/ADSR/step sequencers, control via MIDI with visual feedback, design a stage layout with polygons and true circles, calibrate projector alignment with corner-pin warp, and send content to displays in fullscreen.

**All Tier 1 blockers are cleared.**

**The critical gap is persistence.** Varda can drive a projector show, but the VJ has to rebuild their surface layout, output config, warp calibration, and MIDI mappings every time they launch the app. This is the difference between "it works in a demo" and "I can use this at a gig." Fixing scene persistence for surfaces, outputs, warp, and MIDI mappings is the highest-impact next step.

**Secondary gaps:** No undo/redo, no performance monitoring, no recording. These are expected by professionals but don't block basic use.

**Biggest competitive advantage:** Modern GPU pipeline (wgpu), open shader format (ISF), HAP video with GPU-native decode, and a clean architecture that's still small enough to move fast.
