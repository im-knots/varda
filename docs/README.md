# Varda

Varda is a free, open-source live visual mixer and broadcast router for Linux and macOS, written in Rust. It routes video sources — shaders, video files, cameras, NDI, SRT, HLS/DASH streams — through a broadcast-style signal matrix (Deck → Channel → Mixer → Surface → Output), composites them with per-parameter modulation and ISF effect chains, and delivers the result to projectors, network streams, recordings, and the web.

Varda is built for live VJ performance, dome projection, multi-projector installations, and headless media serving — controlled via MIDI, OSC, keyboard shortcuts, or a full REST/WebSocket API.

---

- [Core Concepts](concepts.md)
  - [The Signal Flow](concepts.md#the-signal-flow) — Deck, Channel, Mixer, Surface, Output
  - [Source Types](concepts.md#source-types) — ISF shaders, video, camera, NDI, SRT, HLS, DASH, Syphon
  - [Effect Chains](concepts.md#effect-chains) — deck, channel, and master FX levels
  - [Modulation](concepts.md#modulation) — LFO, audio bands, ADSR, step sequencer
  - [Persistence](concepts.md#persistence) — scene vs stage, presets, asset handling
- [Getting Started](getting-started.md)
  - [Workspace & Content](getting-started.md#workspace--content) — project layout, supported formats
  - [Build & Run](getting-started.md#build--run)
  - [UI Layout](getting-started.md#ui-layout) — panel map
  - [Load Content](getting-started.md#load-content)
  - [Output to a Display](getting-started.md#output-to-a-display)
  - [Audio Setup](getting-started.md#audio-setup) — input device, beat detection
  - [Next Steps](getting-started.md#next-steps)
  - [CLI Flags](getting-started.md#cli-flags)
- [Performance & Automation](performance.md)
  - [Video Playback](performance.md#video-playback) — loop modes, speed, in/out points, scrub
  - [Deck Auto-Transitions](performance.md#deck-auto-transitions) — timed/clip-end triggers, transition shaders
  - [Transition Sequences](performance.md#transition-sequences) — multi-step channel automation, easing, simultaneous sequences
  - [Undo / Redo](performance.md#undo--redo) — 50-level snapshot history
  - [Presets](performance.md#presets) — save/load deck and channel configurations
- [Modulation & Audio Reactivity](modulation.md)
  - [Modulation Sources](modulation.md#modulation-sources) — LFO, Audio, ADSR, Step Sequencer
  - [Routing](modulation.md#routing) — parameter assignment, stacking, per-component color modulation
  - [Modulator-on-Modulator](modulation.md#modulator-on-modulator) — recursive chaining up to 4 levels
  - [Audio System](modulation.md#audio-system) — FFT analysis, beat detection, ISF audio uniforms
- [Control Surfaces](control-surfaces.md)
  - [MIDI](control-surfaces.md#midi) — learn mode, APC Mini, multi-device
  - [OSC](control-surfaces.md#osc) — input/output, bidirectional feedback
  - [Keyboard Shortcuts](control-surfaces.md#keyboard-shortcuts) — learn mode, default bindings, param toggle
  - [Clock Synchronization](control-surfaces.md#clock-synchronization) — MIDI/OSC/audio/manual BPM, priority resolution
  - [Parameter Paths](control-surfaces.md#parameter-paths)
- [Projection Mapping](projection.md)
  - [Basic Projection](projection.md#basic-projection) — single output, surfaces, vertex editing, corner-pin warp
  - [Advanced Projection](projection.md#advanced-projection) — multi-output edge blending, multi-channel routing, mesh warp
  - [Dome Projection](projection.md#dome-projection) — domemaster, slicer presets, 3D preview, content rotation
- [Streaming & I/O](streaming-and-io.md)
  - [NDI](streaming-and-io.md#ndi)
  - [SRT](streaming-and-io.md#srt-secure-reliable-transport)
  - [HLS & DASH](streaming-and-io.md#hls--dash)
  - [Recording](streaming-and-io.md#recording)
  - [Syphon](streaming-and-io.md#syphon-macos)
- [ISF Shader Authoring](isf-authoring.md)
  - [Shader Types](isf-authoring.md#shader-types) — generator, filter, transition
  - [Metadata Format](isf-authoring.md#metadata-format) — JSON header, input types
  - [Built-in Uniforms](isf-authoring.md#built-in-uniforms) — TIME, RENDERSIZE, audio, phase accumulators
  - [Multi-Pass Rendering](isf-authoring.md#multi-pass-rendering) — persistent buffers, feedback loops
  - [Hot-Reload](isf-authoring.md#hot-reload) — live editing workflow
- [HTTP API](api.md)
  - [Swagger UI](api.md#swagger-ui)
  - [Headless Mode](api.md#headless-mode)
  - [WebSocket](api.md#websocket)
  - [Common Patterns](api.md#common-patterns)
  - [Route Groups](api.md#route-groups)

---

## Additional Resources

- **API Reference** — interactive Swagger UI at [`http://localhost:8080/api/docs`](http://localhost:8080/api/docs) (when Varda is running)
- **ISF Shader Format** — [isf.video](https://isf.video) (external)
- **Source Code** — MIT licensed, contributions welcome
