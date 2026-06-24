# Varda

Varda is a free, open-source live visual mixer and broadcast router for Linux, macOS, and Windows, written in Rust. It routes video sources — shaders, video files, cameras, NDI, SRT, HLS/DASH streams — through a broadcast-style signal matrix (Deck → Channel → Mixer → Surface → Output), composites them with per-parameter modulation and ISF effect chains, and delivers the result to projectors, network streams, recordings, and the web.

Varda is built for live VJ performance, dome projection, multi-projector installations, and headless media serving — controlled via MIDI, OSC, keyboard shortcuts, or a full REST/WebSocket API.

---

## Manual

### Part I — Getting Started

- **1. [Getting Started](01-getting-started.md)**
  - [Install](01-getting-started.md#install) — macOS DMG, Linux tarball, Windows ZIP
  - [Workspace & Content](01-getting-started.md#workspace--content) — project layout, supported formats
  - [Build from Source](01-getting-started.md#build-from-source)
  - [UI Layout](01-getting-started.md#ui-layout) — panel map
  - [Load Content](01-getting-started.md#load-content)
  - [Output to a Display](01-getting-started.md#output-to-a-display)
  - [Audio Reactivity](01-getting-started.md#audio-reactivity) — input device, beat detection
  - [Next Steps](01-getting-started.md#next-steps)
  - [CLI Flags](01-getting-started.md#cli-flags)
- **2. [Core Concepts](02-concepts.md)**
  - [The Signal Flow](02-concepts.md#the-signal-flow) — Deck, Channel, Mixer, Surface, Output
  - [Source Types](02-concepts.md#source-types) — ISF shaders, video, camera, NDI, SRT, HLS, DASH, RTMP, compute, Syphon
  - [Blend Modes](02-concepts.md#blend-modes) — 15 compositing modes
  - [Effect Chains](02-concepts.md#effect-chains) — deck, channel, and master FX levels
  - [Modulation](02-concepts.md#modulation) — LFO, audio bands, ADSR, step sequencer, analyzer
  - [Persistence](02-concepts.md#persistence) — scene vs stage, presets, asset handling

### Part II — Performing

- **3. [Library Panel](03-library-panel.md)** — content browser
  - [Sections](03-library-panel.md#sections) — generators, effects, images, video, cameras, streams, presets
  - [Drag-and-Drop](03-library-panel.md#drag-and-drop) — drop onto a channel to create a deck
  - [Stream Sources](03-library-panel.md#stream-sources) — NDI/SRT/HLS/DASH/RTMP grouping, status indicators
  - [Cameras](03-library-panel.md#cameras) — rescan, resolution selector
- **4. [Performance & Automation](04-performance.md)**
  - [Video Playback](04-performance.md#video-playback) — loop modes, speed, scrub, HAP codecs, ping-pong cache
  - [Deck Auto-Transitions](04-performance.md#deck-auto-transitions) — timed/clip-end triggers, transition shaders
  - [Transition Sequences](04-performance.md#transition-sequences) — multi-step channel automation, easing, simultaneous sequences
  - [Undo / Redo](04-performance.md#undo--redo) — 50-level snapshot history
  - [Presets](04-performance.md#presets) — save/load deck and channel configurations
- **5. [Modulation & Audio Reactivity](05-modulation.md)**
  - [Creating Sources](05-modulation.md#creating-sources) — the ➕ buttons, source colors
  - [Modulation Sources](05-modulation.md#modulation-sources) — LFO, Audio, ADSR, Step Sequencer, Analyzer
  - [Routing](05-modulation.md#routing) — the 〰 assign button, live ghost indicator, stacking
  - [Modulator-on-Modulator](05-modulation.md#modulator-on-modulator) — recursive chaining up to 4 levels
  - [Audio System](05-modulation.md#audio-system) — FFT analysis, beat detection, ISF audio uniforms
- **6. [Control Surfaces](06-control-surfaces.md)**
  - [MIDI](06-control-surfaces.md#midi) — learn mode, APC Mini, multi-device
  - [OSC](06-control-surfaces.md#osc) — input/output, bidirectional feedback
  - [Keyboard Shortcuts](06-control-surfaces.md#keyboard-shortcuts) — learn mode, default bindings, param toggle
  - [Clock Synchronization](06-control-surfaces.md#clock-synchronization) — MIDI/OSC/audio/manual BPM, priority resolution
  - [Parameter Paths](06-control-surfaces.md#parameter-paths)

### Part III — Output & Display

- **7. [Outputs](07-outputs.md)**
  - [Creating an Output](07-outputs.md#creating-an-output) — windowed, recording, stream
  - [Output Targets](07-outputs.md#output-targets) — display selection, hot-plug
  - [Output Rotation](07-outputs.md#output-rotation) — 0/90/180/270 for portrait
  - [Source Routing](07-outputs.md#source-routing) — Master / Channel / Channels sub-mix / Deck
  - [Recording Outputs](07-outputs.md#recording-outputs)
- **8. [Projection Mapping](08-projection.md)**
  - [Basic Projection](08-projection.md#basic-projection) — drawing tools, surfaces, corner-pin warp, combine/multi-contour
  - [Advanced Projection](08-projection.md#advanced-projection) — multi-output edge blending (auto/manual), multi-channel routing, mesh warp
  - [Dome Projection](08-projection.md#dome-projection) 🧪 — domemaster, slicer presets, 3D preview navigation, content rotation
  - [Surface Auto-Detection](08-projection.md#surface-auto-detection) 🧪 — file import and live camera detection
- **9. [Streaming, Recording & Network I/O](09-streaming-and-io.md)**
  - [NDI](09-streaming-and-io.md#ndi)
  - [SRT](09-streaming-and-io.md#srt-secure-reliable-transport)
  - [HLS & DASH](09-streaming-and-io.md#hls--dash)
  - [Recording](09-streaming-and-io.md#recording)
  - [Stream Input Reliability](09-streaming-and-io.md#stream-input-reliability) — dedup, stall detection, reconnect
  - [Syphon](09-streaming-and-io.md#syphon-macos)
- **10. [Resolution, Settings & Monitoring](10-resolution-and-monitoring.md)**
  - [Render Resolution](10-resolution-and-monitoring.md#render-resolution) — presets, custom sizes
  - [Per-Deck Scaling](10-resolution-and-monitoring.md#per-deck-scaling) — fill, fit, stretch, center
  - [Performance Monitoring](10-resolution-and-monitoring.md#performance-monitoring) — FPS, GPU, CPU/RAM

### Part IV — Reference

- **11. [Shader Library](11-shader-library.md)** — catalog of bundled generators, filters, transitions, and compute shaders
- **12. [ISF Shader Authoring](12-isf-authoring.md)**
  - [Shader Types](12-isf-authoring.md#shader-types) — generator, filter, transition
  - [Metadata Format](12-isf-authoring.md#metadata-format) — JSON header, input types
  - [Built-in Uniforms](12-isf-authoring.md#built-in-uniforms) — TIME, RENDERSIZE, audio, phase accumulators
  - [Multi-Pass Rendering](12-isf-authoring.md#multi-pass-rendering) — persistent buffers, feedback loops
  - [Compute Shaders](12-isf-authoring.md#compute-shaders) — `.comp` shaders, storage buffers, dispatch
  - [Hot-Reload](12-isf-authoring.md#hot-reload) — live editing workflow
- **13. [HTTP API & Headless Mode](13-api.md)**
  - [Swagger UI](13-api.md#swagger-ui)
  - [Headless Mode](13-api.md#headless-mode)
  - [WebSocket](13-api.md#websocket)
  - [Common Patterns](13-api.md#common-patterns)
  - [Route Groups](13-api.md#route-groups)
- **14. [Benchmarking](14-benchmarking.md)** — criterion harness, GPU/CPU suites, before/after comparison

---

## Additional Resources

- **API Reference** — interactive Swagger UI at [`http://localhost:8080/api/docs`](http://localhost:8080/api/docs) (when Varda is running)
- **ISF Shader Format** — [isf.video](https://isf.video) (external)
- **Source Code** — MIT licensed, contributions welcome
