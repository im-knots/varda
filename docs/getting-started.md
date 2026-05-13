# Getting Started

## Prerequisites

- **Rust toolchain** — install via [rustup.rs](https://rustup.rs)
- **GPU** — Vulkan-capable (Linux) or Metal-capable (macOS)
- **ffmpeg** — required for video playback, recording, and streaming (`brew install ffmpeg` on macOS, or your package manager on Linux). Build with `--enable-libsrt` for SRT streaming support.

## Workspace & Content

Varda treats the current directory (or `--workspace` path) as the workspace root. Create a project folder and put your content in it:

```
my-show/
  shaders/       ← ISF shader files (.fs) — auto-discovered, hot-reloaded on save
  media/         ← videos and images (loaded via Library panel)
  .varda/        ← created automatically (scene, stage, presets, mappings)
```

Shaders in `shaders/` appear automatically in the Library panel under **Generators**, **Effects**, or **Transitions** based on their type. Videos and images are loaded through the Library panel's file browser.

**Supported formats:**

| Type | Formats |
|------|---------|
| **Shaders** | `.fs` (ISF GLSL 450) |
| **Video** | Any ffmpeg-supported container/codec — MP4, MOV, MKV, AVI, WebM (H.264, H.265, ProRes, VP9, etc.) |
| **HAP Video** | MOV with HAP, HAP Alpha, HAP Q, HAP Q Alpha, HAP R — GPU-native decode, no CPU overhead |
| **Images** | PNG, JPG/JPEG |

## Build & Run

```sh
cargo run --release
```

## UI Layout

![Varda UI](../img/screenshot.png)

- **Library** (left, toggle with **L**) — content browser: shaders, video, images, cameras, NDI, SRT, HLS/DASH sources, and presets
- **Center** — channel/deck grid with mixer crossfader. Toggle to **Stage Editor** for surface drawing.
- **Right** — main output preview (always visible), output list, modulation panel, MIDI device list
- **Bottom** (resizable) — context-sensitive: shows the selected deck's parameters, effect chains, or sequence editor

All panel dividers are draggable to resize. Left and right panels can be collapsed.

## Load Content

1. Open the **Library** panel (press **L** to toggle)
2. Browse the **Generators** section for ISF shaders
3. **Drag** a shader from the library into a channel column — a new deck is created

The shader renders immediately in the deck's preview thumbnail and the main output.

To load **video or images**, use the Video or Image sections in the Library — a file browser opens to select files from your workspace.

## Add a Second Channel

1. Drag another shader into the second channel column
2. Use the **crossfader** in the mixer box to blend between Channel A and Channel B
3. Click the **auto-transition** button for timed or beat-synced crossfades
4. Select a **transition shader** (dissolve, iris, push, etc.) from the dropdown

## Apply Effects

1. In the library, switch to the **Effects** section
2. Drag an effect onto a deck, channel, or the master output
3. Select the deck or effect to see its parameters in the **bottom bar**
4. Adjust parameters with sliders — all are MIDI/OSC-mappable and modulatable

## Output to a Display

1. In the right panel, click **"+ Output"** to create a new output window
2. A floating window appears — this is your output
3. In the output settings, select a **display target** (enumerate monitors from the dropdown)
4. Click **Fullscreen** to send the output to the selected projector or display

## Audio Reactivity

Varda analyzes audio input for beat detection and frequency-band modulation. To get started:

1. In the **modulation panel** (right sidebar), add an **Audio** modulation source
2. In the source's **device dropdown**, select the audio input receiving your music feed (line-in, USB interface, etc.)
3. Choose a **frequency preset** — Low (bass), Mid, or High (treble) — or set a custom Hz range
4. Assign the source to any parameter (opacity, shader param, etc.) — it now reacts to the music

Beat detection activates automatically from the audio input — BPM appears in the mixer for beat-synced transitions and auto-crossfades.

ISF shaders also receive audio data directly via built-in uniforms (`audio_bass`, `audio_mid`, `audio_treble`, `audio_bpm`, `audio_beat_phase`) — no modulation setup needed. See [Modulation & Audio Reactivity](modulation.md) for the full guide.

## Next Steps

Once you have content playing on a display, explore deeper capabilities:

- **[Performance & Automation](performance.md)** — video playback controls, deck auto-transitions, transition sequences, undo/redo, presets
- **[Modulation & Audio Reactivity](modulation.md)** — LFO, audio bands, ADSR, step sequencer, mod-on-mod chaining
- **[Control Surfaces](control-surfaces.md)** — MIDI learn, OSC, keyboard shortcuts, clock sync
- **[Projection Mapping](projection.md)** — surfaces, corner-pin warp, multi-projector edge blending, dome projection
- **[Streaming & I/O](streaming-and-io.md)** — NDI, SRT, HLS/DASH, recording
- **[ISF Shader Authoring](isf-authoring.md)** — write your own generators, filters, and transitions
- **[HTTP API](api.md)** — REST/WebSocket control, headless mode

## Save Your Work

Press **Cmd+S** or click the **💾 Save** button to save the current state. Varda persists:

- `scene.json` — your show (channels, decks, effects, modulation)
- `stage.json` — the venue (surfaces, outputs, warp calibration)
- `midi.json` — controller mappings
- `keymap.json` — keyboard shortcuts

## CLI Flags

```
varda [OPTIONS]

    --headless              Run without UI window (API-only control)
    --port <PORT>           HTTP API port (default: 8080)
    --fps <FPS>             Target render FPS in headless mode (default: 60)
    --workspace <DIR>       Workspace root directory (default: current directory)
    --scene <PATH>          Scene file to load (default: .varda/scene.json)
    --stage <PATH>          Stage file to load (default: .varda/stage.json)
    --osc-port <PORT>       OSC input port (overrides osc.json config)
    --osc-out <HOST:PORT>   OSC feedback target (repeatable)
    --no-osc                Disable OSC input
    --no-ndi                Disable NDI discovery and sending
    --no-syphon             Disable Syphon (macOS only)
```

CLI flags override persisted config for that session without modifying the saved files.
