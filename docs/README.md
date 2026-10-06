# Varda

Varda is a free, open-source live visual mixer and broadcast router for Linux, macOS, and Windows, written in Rust. It routes video sources such as shaders, video files, cameras, NDI, SRT, HLS/DASH streams, screen and window captures, and HTML/web pages through a broadcast-style signal matrix (Deck → Channel → Mixer → Surface → Output), composites them with per-parameter modulation and ISF effect chains, and delivers the result to projectors, network streams, recordings, and the web.

Varda is built for live VJ performance, dome projection, multi-projector installations, and headless media serving. You control it with MIDI, OSC, keyboard shortcuts, or a full REST/WebSocket API.

You can work on a scene in two modes. Both edit the same scene. **Performance mode** is the mixer: channels, decks and faders, all by hand. **[Arrangement mode](05-arrangement.md)** lays the same scene out against show time. Regions set when a deck is up, curves automate any parameter, cue points mark moments to return to, and the transport can follow SMPTE timecode. Use either mode alone or both together. When both are running, your hand wins: touch an automated control and it follows you until you re-arm it.

This manual covers everything you need to get started.

---

## Manual

### Part I: Getting Started

- **1. [Getting Started](01-getting-started.md)**
  - [Install](01-getting-started.md#install) (macOS DMG, Linux Flatpak and AppImage, Windows ZIP)
  - [Workspace & Content](01-getting-started.md#workspace--content) (project layout, supported formats)
  - [Build from Source](01-getting-started.md#build-from-source)
  - [UI Layout](01-getting-started.md#ui-layout) (panel map)
  - [Load Content](01-getting-started.md#load-content)
  - [Output to a Display](01-getting-started.md#output-to-a-display)
  - [Audio Reactivity](01-getting-started.md#audio-reactivity) (input device, beat detection)
  - [Next Steps](01-getting-started.md#next-steps)
  - [CLI Flags](01-getting-started.md#cli-flags)
- **2. [Core Concepts](02-concepts.md)**
  - [The Signal Flow](02-concepts.md#the-signal-flow) (Deck, Channel, Mixer, Surface, Output, routing flexibility, clip-launch behavior from the mixer)
  - [Source Types](02-concepts.md#source-types) (ISF shaders, video, camera, NDI, SRT, HLS, DASH, RTMP, compute, Syphon, HTML)
  - [Blend Modes](02-concepts.md#blend-modes) (15 compositing modes)
  - [Effect Chains](02-concepts.md#effect-chains) (deck, channel, and master FX levels)
  - [Modulation](02-concepts.md#modulation) (LFO, audio bands, ADSR, step sequencer, analyzer)
  - [Copy and Paste](02-concepts.md#copy-and-paste) (right-click or Cmd+C/V/D on decks, channels, effects)
  - [Persistence](02-concepts.md#the-varda-workspace) (scene vs stage, presets, asset handling)

### Part II: Performing

- **3. [Library Panel](03-library-panel.md)** (content browser)
  - [Sections](03-library-panel.md#sections) (generators, effects, images, video, cameras, screen capture, taps, streams, HTML, presets)
  - [Drag-and-Drop](03-library-panel.md#drag-and-drop) (drop onto a channel to create a deck)
  - [Stream Sources](03-library-panel.md#stream-sources) (NDI/SRT/HLS/DASH/RTMP grouping, status indicators)
  - [HTML Sources](03-library-panel.md#html-sources) (add web page URLs, drag-to-channel)
  - [Cameras](03-library-panel.md#cameras) (rescan, resolution selector)
  - [Screen Capture](03-library-panel.md#screen-capture) (displays and windows, rescan, permission prompt)
  - [Taps](03-library-panel.md#taps) (Varda's own master program and channels as sources)
- **4. [Performance & Automation](04-performance.md)**
  - [Video Playback](04-performance.md#video-playback) (loop modes, speed, scrub, HAP codecs, ping-pong cache)
  - [Deck Auto-Transitions](04-performance.md#deck-auto-transitions) (timed/clip-end triggers, transition shaders)
  - [Transition Sequences](04-performance.md#transition-sequences) (multi-step channel automation, easing, simultaneous sequences)
  - [Undo / Redo](04-performance.md#undo--redo) (50-level snapshot history)
  - [Presets](04-performance.md#presets) (save/load deck and channel configurations)
- **5. [Arrangement Mode](05-arrangement.md)** (the mixer laid out against show time)
  - [What Changes and What Doesn't](05-arrangement.md#shared-panels) (only the central area changes)
  - [Anatomy](05-arrangement.md#anatomy) (transport strip, ruler, groups, lanes, navigation)
  - [Cue Points](05-arrangement.md#cue-points) (mark a moment, step through cues with the arrows, fire a cue from a pad in Performance mode)
  - [Reordering Decks](05-arrangement.md#reordering-decks) (drag a lane header, shared with the mixer's order)
  - [Regions](05-arrangement.md#regions) (draw, move, resize, fades, frame snapping)
  - [Automation Lanes](05-arrangement.md#automation-lanes) (breakpoints, curve shapes, copy/paste)
  - [Reusing a Shape](05-arrangement.md#reusing-a-shape) (one curve per parameter, copied between lanes)
  - [Authority and Override](05-arrangement.md#authority-and-override) (grabbing a control back, re-arm)
  - [Chasing a clip to the show](05-arrangement.md#video-chase) (lock a video deck to the transport)
  - [Idle Behavior](05-arrangement.md#idle-behavior) (what plays before the show starts)
  - [Undo, Saving, and Load](05-arrangement.md#undo-saving-and-load) (scene version 7, memory)
  - [Sleeping Clips](05-arrangement.md#sleeping-clips) (why a distant clip stops decoding, and its effects)
- **6. [Modulation & Audio Reactivity](06-modulation.md)**
  - [Creating Sources](06-modulation.md#creating-sources) (the ➕ buttons, source colors)
  - [Modulation Sources](06-modulation.md#modulation-sources) (LFO, Audio, ADSR, Step Sequencer, Analyzer)
  - [Routing](06-modulation.md#routing) (the 〰 assign button, live ghost indicator, stacking)
  - [Modulator-on-Modulator](06-modulation.md#modulator-on-modulator) (recursive chaining up to 4 levels)
  - [Audio System](06-modulation.md#audio-system) (FFT analysis, beat detection, ISF audio uniforms)
- **7. [Control Surfaces & Macros](07-control-surfaces.md)**
  - [MIDI](07-control-surfaces.md#midi) (learn mode, APC Mini, multi-device)
  - [OSC](07-control-surfaces.md#osc) (input/output, bidirectional feedback)
  - [Keyboard Shortcuts](07-control-surfaces.md#keyboard-shortcuts) (learn mode, default bindings, param toggle)
  - [Clock Synchronization](07-control-surfaces.md#clock-synchronization) (MIDI/OSC/audio/manual BPM, priority resolution)
  - [Parameter Paths](07-control-surfaces.md#parameter-paths)
  - [Macros](07-control-surfaces.md#macros) (one knob/fader/button drives many parameters)
    - [Creating a Macro](07-control-surfaces.md#creating-a-macro) (knob, fader, button cards)
    - [Binding Targets](07-control-surfaces.md#binding-targets) (per-target range, curve, invert)
    - [Buttons](07-control-surfaces.md#buttons) (momentary, toggle, trigger actions)
    - [Mapping to MIDI / OSC / Keyboard](07-control-surfaces.md#mapping-a-macro-to-midi--osc--keyboard)
    - [Mapping a Macro (MIDI, OSC, HTTP)](07-control-surfaces.md#mapping-a-macro-to-midi--osc--keyboard)

### Part III: Content

- **8. [Shader Library](08-shader-library.md)** (catalog of bundled generators, filters, transitions, and compute shaders)
- **9. [Fractal Explorer](09-fractal-explorer.md)** (flying through 3D fractals)
  - [How It's Built](09-fractal-explorer.md#how-its-built) (one shader and one preprocessor: a visual engine inside Varda)
  - [Flying](09-fractal-explorer.md#flying) (throttle, distance-scaled speed, autopilot, saved locations, tours)
  - [Building the Fractal](09-fractal-explorer.md#building-the-fractal) (six formula slots, alternate and combine hybrids)
  - [Look](09-fractal-explorer.md#look) (lighting, palette, atmosphere, lens, grade)
  - [Quality and Speed](09-fractal-explorer.md#quality-and-speed) (detail, geometry band, diagnostics)

### Part IV: Output & Display

- **10. [Outputs](10-outputs.md)**
  - [Creating an Output](10-outputs.md#creating-an-output) (output types, choosing a monitor)
  - [Output Format](10-outputs.md#output-format) (8-bit, 10-bit, HDR10, HLG, EDR)
  - [Rotation](10-outputs.md#rotation)
  - [Surface Sources](10-outputs.md#surface-sources) (Master, Channel, Channels sub-mix, Deck)
  - [Recording](10-outputs.md#recording)
- **11. [Projection Mapping](11-projection.md)**
  - [Basic Projection](11-projection.md#basic-projection) (drawing tools, surfaces, corner-pin warp, combine/multi-contour)
  - [Advanced Projection](11-projection.md#advanced-projection) (multi-output edge blending (auto/manual), multi-channel routing, mesh warp)
  - [Dome Projection](11-projection.md#dome-projection) 🧪 (domemaster, slicer presets, 3D preview navigation, content rotation)
  - [Surface Auto-Detection](11-projection.md#surface-auto-detection) 🧪 (file import and live camera detection)
- **12. [Streaming, Recording & Network I/O](12-streaming-and-io.md)**
  - [NDI](12-streaming-and-io.md#ndi)
  - [SRT](12-streaming-and-io.md#srt-secure-reliable-transport)
  - [HLS & DASH](12-streaming-and-io.md#hls--dash)
  - [Recording](12-streaming-and-io.md#recording)
  - [Stream Input Reliability](12-streaming-and-io.md#stream-input-reliability) (dedup, stall detection, reconnect)
  - [HTML / Web Content](12-streaming-and-io.md#html--web-content)
  - [Syphon](12-streaming-and-io.md#syphon-macos) (send and receive, shared-memory in both directions, color handling, framework install)
  - [Spout](12-streaming-and-io.md#spout-windows) (send and receive, color handling, 8-bit and 10-bit)
  - [Screen & Window Capture](12-streaming-and-io.md#screen--window-capture) (displays and windows as decks, permissions, crop and rate, capturing Varda itself)
  - [Program Tap](12-streaming-and-io.md#program-tap) (Varda's own output as a source, one frame behind, feedback loops)
- **13. [Resolution, Settings & Monitoring](13-resolution-and-monitoring.md)**
  - [Render Resolution](13-resolution-and-monitoring.md#render-resolution) (presets, custom sizes)
  - [Per-Deck Scaling](13-resolution-and-monitoring.md#per-deck-scaling) (fill, fit, stretch, center)
  - [Performance Monitoring](13-resolution-and-monitoring.md#performance-monitoring) (FPS, GPU, CPU/RAM)

### Part V: Reference

- **14. [Shader Authoring](14-isf-authoring.md)**
  - [Shader Types](14-isf-authoring.md#shader-types) (generator, filter, transition)
  - [Metadata Format](14-isf-authoring.md#metadata-format) (JSON header, input types)
  - [Built-in Uniforms](14-isf-authoring.md#built-in-uniforms) (TIME, RENDERSIZE, audio, phase accumulators)
  - [Porting an ISF Shader](14-isf-authoring.md#porting-an-isf-shader) (dialect differences, the vertical flip, what isn't supported)
  - [Multi-Pass Rendering](14-isf-authoring.md#multi-pass-rendering) (persistent buffers, feedback loops, substepping)
  - [Compute Shaders](14-isf-authoring.md#compute-shaders) (`.comp` shaders, storage buffers, dispatch)
  - [Hot-Reload](14-isf-authoring.md#hot-reload) (live editing workflow)
  - [File Location](14-isf-authoring.md#file-location) (shader directory precedence hierarchy, `--shader-dir`)
- **15. [HTTP API & Headless Mode](15-api.md)**
  - [Swagger UI](15-api.md#swagger-ui)
  - [Headless Mode](15-api.md#headless-mode)
  - [WebSocket](15-api.md#websocket)
  - [Common Patterns](15-api.md#common-patterns)
  - [Route Reference](15-api.md#route-reference)
- **16. [Analyzers & Preprocessors](16-analyzers-and-preprocessors.md)** (frame analysis for modulation, and preprocessors that feed shaders)
  - [What's Implemented](16-analyzers-and-preprocessors.md#whats-implemented) (brightness, face_detect, depth_sensor; planned types)
  - [Analysis as Modulation](16-analyzers-and-preprocessors.md#analysis-as-modulation-performers) (brightness/face scalars drive any parameter)
  - [Depth Sensor](16-analyzers-and-preprocessors.md#depth-sensor-performers) (near/far framing, per-deck controls)
  - [Preprocessors](16-analyzers-and-preprocessors.md#preprocessors) (analyzer, GPU and host-inline kinds)
  - [What a Host-Inline Preprocessor Can Do](16-analyzers-and-preprocessors.md#what-a-host-inline-preprocessor-can-do) (inputs, outputs, saved state, notifications, build inputs)
  - [Worked Example: the Fractal Explorer](16-analyzers-and-preprocessors.md#worked-example-the-fractal-explorer) (a visual engine from one shader and one preprocessor)
  - [Analyzer HTTP API](16-analyzers-and-preprocessors.md#analyzer-http-api)

---

## Additional Resources

- **API Reference**: interactive Swagger UI at [`http://localhost:8080/api/docs`](http://localhost:8080/api/docs) (when Varda is running)
- **ISF Shader Format**: [isf.video](https://isf.video) (external)
- **Contributing**: architecture overview, entity/address scheme, engineering conventions, and the benchmarking harness are in [`CONTRIBUTING.md`](../CONTRIBUTING.md)
