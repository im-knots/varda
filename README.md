# Varda VJ

Open-source visual performance tool with broadcast-style routing for VJs and installation artists.

![Varda](docs/img/screenshot.png)


Varda applies broadcast video workflows to live visuals. Sources (video, cameras, generative shaders, streams, images) flow through a routing graph of decks, channels, and surfaces to reach outputs (projectors, streams, recordings). Instead of a clip-launch grid, you control what's live by adjusting opacity, blend modes, crossfaders, mute/solo, and effect chains. Zero-opacity decks and channels are automatically culled from the render pass, the same way a broadcast switcher only processes sources that are live on a bus.

- **Routing matrix**: Sources > Decks > Channels > Mixer > Surfaces > Outputs. Any source to any output, split, branch, or sub-mix at every junction
- **Sources**: video (HAP GPU-native + ffmpeg), cameras, ISF shaders (generators/filters), NDI, SRT, HLS, DASH, RTMP/RTMPS, images
- **Mixing**: N-channel compositing, A/B crossfader, per-deck opacity, 6 blend modes
- **Transitions**: ISF shader transitions between channels, deck auto-transitions (timer/clip-end triggers), multi-channel transition sequencer with beat-synced or timed triggers (seconds, minutes, hours). Allowing for quick automated live transitions or long running automated installations. 
- **Effect chains**: 3-level hierarchy (deck > channel > master), drag-and-drop from library, reorderable
- **Modulation**: LFO, audio-reactive, ADSR, step sequencer, mod-on-mod chaining on any parameter
- **Audio**: 2048-bin FFT, beat detection, bass/mid/treble bands, BPM with beat phase
- **Control**: MIDI, OSC, and HTTP API co-equal consumers of the same engine
- **Projection mapping**: 2D stage editor, polygon/circle surfaces, per-surface corner-pin warp, calibration cards, edge blending (Auto with precise polygon overlap detection, Manual per-edge)
- **Multi-output**: multiple windows, fullscreen on any display, headless outputs with surface assignments
- **Network I/O**: NDI, SRT, HLS, LL-HLS, DASH, and RTMP/RTMPS send/receive
- **Recording**: H.264, h.265, AV1, ProRes 422, HAP Q per-output
- **Presets**: save/load deck and channel presets with modulation recipes
- **Persistence**: full scene/venue/MIDI state saved and restored across sessions

Experimental: 
- **Dome projection**: fisheye to equirectangular (360°) and cubemap (3D) rendering with configurable lens correction and chromatic aberration.
- **Surface overlap zones**: manual and auto-detect modes for precise edge blending.

## Install

Download the latest release from the [Releases page](https://github.com/im-knots/varda/releases).

### macOS (Universal DMG)

1. Download `Varda-macOS-universal.dmg`
2. Open the DMG and drag **Varda.app** to `/Applications`
3. Before first launch, open Terminal and run:
   ```bash
   xattr -cr /Applications/Varda.app
   ```
   This removes the macOS quarantine flag. Varda is not yet signed with an Apple Developer certificate, so Gatekeeper will block it without this step.
4. Launch Varda — on first run it will prompt for your password to install the `varda` CLI command to `/usr/local/bin/`

### Linux (AppImage)

1. Download `Varda-x86_64.AppImage`
2. Make it executable and run:
   ```bash
   chmod +x Varda-x86_64.AppImage
   ./Varda-x86_64.AppImage
   ```
   On first launch, a `varda` symlink is created in `~/.local/bin/` so you can run `varda` from any terminal.

Both releases bundle FFmpeg and NDI, no extra dependencies needed.

---

## Build from source

Requires [Rust](https://rustup.rs/) (stable) and a GPU with Metal (macOS) or Vulkan (Linux) support.

### Ubuntu / Debian

```bash
sudo apt install build-essential cmake pkg-config libvulkan-dev libavcodec-dev libavformat-dev libavutil-dev libswscale-dev libswresample-dev libavdevice-dev libsrt-gnutls-dev libasound2-dev libv4l-dev libshaderc-dev libwayland-dev libxkbcommon-dev libx11-dev libxrandr-dev libxi-dev libgtk-3-dev
```

```bash
cargo build --release
./target/release/varda
```

#### Optional: NDI
NDI is proprietary and not available via apt. To enable NDI send/receive:
```bash
wget https://downloads.ndi.tv/SDK/NDI_SDK_Linux/Install_NDI_SDK_v6_Linux.tar.gz
tar -xzf Install_NDI_SDK_v6_Linux.tar.gz
sudo ./Install_NDI_SDK_v6_Linux.sh
sudo cp -P NDI\ SDK\ for\ Linux/lib/x86_64-linux-gnu/* /usr/local/lib/
sudo ldconfig
```
Without it, NDI features are silently disabled 

### macOS

```bash
# FFmpeg (with SRT support)
brew tap homebrew-ffmpeg/ffmpeg
brew install homebrew-ffmpeg/ffmpeg/ffmpeg --with-srt

# Optional: NDI
brew install --cask libndi
```

```bash
cargo build --release
./target/release/varda
```

### Run from source

```
cargo run
```

## The Varda workspace
Varda treats the current working directory as a workspace. All state lives in a `.varda/` directory created automatically:

```
your-show/
  .varda/
    scene.json            # channels, decks, effects, modulation, crossfader, transition sequences
    stage.json            # surface layout, outputs, warp calibration
    midi.json             # MIDI controller mappings that differ from the auto-mapped defaults
    keymap.json           # keyboard shortcut bindings
    presets/
      decks/              # saved deck presets (JSON)
      channels/           # saved channel presets (JSON)
    shaders/              # ISF shaders 
    controller-profiles/  # MIDI controller profiles (JSON)
    recordings/           # recording output files
    streams/              # HLS/DASH output files
```

Run Varda from different directories to maintain separate workspaces per show, venue, or project. Each workspace has its own scene, stage layout, and MIDI mappings.

## CLI flags

```
varda [OPTIONS]

    --headless                Run without main UI window (API-only control)
    --port <PORT>             HTTP API port [default: 8080]
    --fps <FPS>               Target render FPS in headless mode [default: 60]
    --workspace <DIR>         Workspace root directory [default: cwd]
    --scene <PATH>            Scene file to load
    --stage <PATH>            Stage file to load
    --osc-port <PORT>         OSC input port (overrides osc.json)
    --osc-out <HOST:PORT>     OSC feedback target (repeatable)
    --no-osc                  Disable OSC
    --no-ndi                  Disable NDI
    --no-syphon               Disable Syphon (macOS)
```

Headless mode runs the full engine without a UI window — controlled entirely via the HTTP API. Outputs defined in `stage.json` auto-start on launch. Graceful shutdown on Ctrl-C or `POST /api/shutdown`.

```bash
# Headless on custom port with 30fps render
varda --headless --port 9090 --fps 30

# Separate workspace per venue
varda --workspace /shows/festival-2026

# Disable subsystems you don't need
varda --no-ndi --no-syphon --osc-port 7000
```

## HTTP API

The GUI and HTTP API are co-equal consumers of the same engine. The API runs on port 8080 (configurable with `--port`) alongside the GUI, or standalone in headless mode (`--headless`). Interactive docs at `/api/docs`, OpenAPI spec at `/api/openapi.json`. WebSocket at `/api/ws` streams state via JSON Patch (RFC 6902) deltas.


## Abstractions you should know about

The simplest setup is two channels with a crossfader between them, output going fullscreen to a projector. Load sources into decks, crossfade between channels. From there you can add complexity as needed for your use case.

The full routing graph is: **Sources → Decks → Channels → Mixer → Surfaces → Outputs**.

A **Deck** wraps a source (shader, video, image, solid color, camera, NDI stream, SRT stream, HLS/DASH stream, or RTMP stream) with its own effect chain, deck specific transition settings, and parameters. Decks at zero opacity are culled from the render pass.

**Channels** composite their decks together using per-deck opacity, blend modes, and optional auto-transitions. Channels have their own effect chain applied after compositing.

The **Mixer** composites channels together. With two channels you get an A/B crossfader. With three or more, per-channel opacity and blend modes. The mixer has its own master effect chain applied to the final composite. The mixer also has a state machine like multi channel transistion sequence builder.

**Surfaces** are optional. They define polygonal regions on a 2D stage canvas, each with its own content source (main, a channel, or a sub-mix of channels). Surfaces are how you map content onto physical screens, LED panels, or projection areas. When no surfaces are defined, outputs receive the full main mix directly.

**Outputs** are where rendered frames go: a window, a fullscreen display, an NDI stream, an SRT stream, an RTMP/RTMPS stream, or a recording. Surfaces are assigned to outputs to complete the routing chain.

At every junction you can branch, split, or re-route. Two channels feeding different surfaces on the same output. The main mix on one output, a single channel isolated on another. A sub-mix of specific channels to an NDI stream while the master goes to projection. You only use the complexity you need for your use case. Be it simple A/B crossfading between two decks, or a multi-channel mixing console with dedicated FX and transitions, or a complex multi-screen setup with individual content routing.

### Clip triggering without clip triggering

Most VJ software uses a clip-launch paradigm: press a button, a clip starts playing; press another, the previous one stops. Varda doesn't have clip triggers, but it doesn't need them, because the broadcast mixer model gives you the same result with more control.

Load your sources into decks across your channels. What makes a deck live is its **opacity**. Map your MIDI controller buttons to deck mute or deck opacity. Press a button, a deck's opacity goes from 0 to 1, it's live. Press another, that deck goes to 0, it's gone. From the audience's perspective, you just triggered a clip. Under the hood, zero-opacity decks are **culled from the render pass entirely**, so they cost nothing. Only decks with non-zero opacity are actually rendered. When you bring a deck's opacity up, its source starts producing frames immediately. You're not "stopping and starting clips," you're mixing a live signal path where the GPU only pays for what's visible.

This is how broadcast video works. A switcher doesn't start and stop cameras. Every camera is always hot, and the director cuts between them by routing signals onto buses. Varda applies the same idea: your decks are always ready, and you perform by controlling which signals are live in the mix. The result is instant transitions (no clip load latency), full MIDI control over the routing, and a mental model that scales.


## Architecture

Varda is built with domain-driven design and clean architecture principles. The codebase separates concerns into four layers:

```
src/
  engine/        # trait contracts and shared types (no implementation)
  internal/      # domain modules (audio, camera, channel, deck, mixer, renderer, etc.)
  app/           # application layer (VardaApp: wires domain modules together, implements engine traits)
  usecases/      # delivery layer (UI panels, action handlers)
  main.rs        # thin orchestrator: parse CLI, init logger, run UI
```

The **engine layer** defines trait contracts (`MixerCommands`, `MixerQueries`, `OutputCommands`, etc.) using only primitives and engine-defined types. No wgpu, egui, or framework types leak through.

The **internal layer** contains domain modules that each own one concern: audio analysis, video decoding, ISF shader compilation, NDI FFI, SRT subprocess management, modulation engine, etc. Each module is independently testable.

The **app layer** (`VardaApp`) is the concrete implementation. It owns all subsystems and implements the engine traits. It can run headless without any window or UI.

The **usecases layer** is the only place that touches egui or windowed rendering. It reads engine state snapshots and emits action structs. The UI never mutates engine state directly.

This means you can drive the same engine from the GUI, the HTTP API, or a test harness without changing engine code.

External I/O (NDI, SRT, HLS/DASH, RTMP, and recording) uses non-blocking subprocess architecture with bounded channels to keep the render thread fast. GPU work is batched into minimal command buffer submissions. The render pass culls zero-opacity decks and channels so you only pay for what's live.

### Entity Identity & Address Scheme

Every mutable entity in the signal graph (channels, decks, effects, surfaces, and outputs) is assigned a stable 8-character hex UUID on creation (e.g. `a3f1b20c`). UUIDs persist across moves, reorders, and scene save/restore. This means MIDI mappings, modulation assignments, and scene references never break when you rearrange your setup. Outputs (windowed, recording, NDI, SRT) carry their own UUIDs so surface assignments and saved window positions survive reconfiguration.

Parameters are addressed with a slash-delimited path rooted at the entity UUID:

```
crossfader                              # mixer crossfader position

deck/<uuid>/opacity                     # deck opacity
deck/<uuid>/mute                        # deck mute toggle
deck/<uuid>/solo                        # deck solo toggle
deck/<uuid>/trigger                     # deck trigger (set opacity to 1)
deck/<uuid>/param/<name>                # generator shader param
deck/<uuid>/effect/<index>/param/<name> # deck effect chain param
deck/<uuid>/at/play_duration            # auto-transition play duration
deck/<uuid>/at/trans_duration           # auto-transition transition duration

ch/<uuid>/opacity                       # channel opacity
ch/<uuid>/effect/<index>/param/<name>   # channel effect chain param

master/effect/<index>/param/<name>      # master effect chain param

mod/<index>/<param_name>                # modulation source param (frequency, amplitude, etc.)
mod/<index>/step/<step_idx>             # step sequencer step value

surface/<uuid>/source                   # surface content source (Master, Channel, Channels, Deck)
output/<uuid>/surface/<surface_uuid>    # output ↔ surface assignment with warp calibration
```

Modulation uses a colon-separated key scheme (`deck_<uuid>:<param>`, `fx_<uuid>:<param>`) so the modulation engine can route LFOs, envelopes, and audio-reactive sources to any parameter in the graph without coupling to positional indices.

## Benchmarking

Criterion harness for the compositing pipeline and per-frame shader parameter buffer build so perf changes land with quantitative evidence.

**GPU suites** (`benches/compositing.rs`):
- `channel_composite_solid` — solid-color decks measuring per-deck copy-on-composite slope
- `channel_composite_shader` — same shape with a fragment shader on every pixel (difference vs solid ≈ per-deck shader cost)
- `mixer_crossfade` — two channels through the crossfader at 50%
- 60fps preflight at 8-deck 1080p (panics if frame budget exceeded)
- Per-deck slope reporter (decks/8 − decks/1, ÷ 7)

**CPU suite** (`benches/shader_params.rs`):
- `no_mod` — std140 buffer serialization only
- `empty_mod` — modulation engine present but no assignments (isolates per-param key allocation cost)
- `active_lfo` — full modulation path: lookup, LFO read, clamp, write

**Run benchmarks:**

```bash
# Full criterion run with HTML reports
cargo bench --bench compositing
cargo bench --bench shader_params

# Headless smoke test (compile + execute, no statistics)
./scripts/bench-smoke.sh
```

**Compare before/after a perf change:**

```bash
# Save baseline before your change
cargo bench --bench compositing -- --save-baseline pre

# Make your change, then compare
cargo bench --bench compositing -- --baseline pre
```

## License
[MIT](LICENSE)