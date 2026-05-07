# Varda

Open-source visual performance instrument with broadcast-style routing for VJs and installation artists. Linux and macOS.

![Varda](img/screenshot.png)

## What it does

Varda applies broadcast video workflows to live visuals. Sources (video, cameras, generative shaders, NDI streams, SRT feeds, images) flow through a routing graph of decks, channels, and surfaces to reach outputs (projectors, streams, recordings). Instead of a clip-launch grid, you control what's live by adjusting opacity, blend modes, crossfaders, mute/solo, and effect chains. Zero-opacity decks and channels are automatically culled from the render pass, the same way a broadcast switcher only processes sources that are live on a bus.

- **Routing matrix**: Sources > Decks > Channels > Mixer > Surfaces > Outputs. Any source to any output, split, branch, or sub-mix at every junction
- **Sources**: video (HAP GPU-native + ffmpeg), cameras, ISF shaders (generators/filters), NDI, SRT, images, solid color
- **Mixing**: N-channel compositing, A/B crossfader, per-deck opacity, 6 blend modes
- **Transitions**: ISF shader transitions between channels, deck auto-transitions (timer/clip-end triggers), multi-channel transition sequencer
- **Effect chains**: 3-level hierarchy (deck > channel > master), drag-and-drop from library, reorderable
- **Modulation**: LFO, audio-reactive, ADSR, step sequencer, mod-on-mod chaining on any parameter
- **Audio**: 512-bin FFT, beat detection, bass/mid/treble bands, BPM with beat phase
- **Control**: MIDI (multi-device, learn mode, controller profiles, LED feedback), OSC in/out
- **Projection mapping**: 2D stage editor, polygon/circle surfaces, per-surface corner-pin warp, calibration cards
- **Multi-output**: multiple windows, fullscreen on any display, headless outputs with surface assignments
- **Network I/O**: NDI send/receive, SRT stream/receive, source library with drag-to-channel
- **Recording**: H.264, ProRes 422, HAP Q per-output
- **Persistence**: full scene/venue/MIDI state saved and restored across sessions

## Build & run

Requires Rust (stable) and a GPU with Metal (macOS) or Vulkan support.


### (Optional) Install ffmpeg with SRT support for SRT I/O
#### macOS
```bash
brew tap homebrew-ffmpeg/ffmpeg
brew install homebrew-ffmpeg/ffmpeg/ffmpeg --with-srt
```

### (Optional) Install NDI support for NDI I/O
#### macOS
```bash
brew install --cask libndi
```


### Run

```
cargo run
```

Varda treats the current working directory as a workspace. All state lives in a `.varda/` directory created automatically:

```
your-show/
  .varda/
    scene.json       # channels, decks, effects, modulation, crossfader, transition sequences
    stage.json       # surface layout, outputs, warp calibration
    midi.json        # MIDI controller mappings
  shaders/           # ISF shaders (scanned on startup)
```

Run `cargo run` from different directories to maintain separate workspaces per show, venue, or project. Each workspace has its own scene, stage layout, and MIDI mappings.


## Abstractions you should know about

The simplest setup is two channels with a crossfader between them, output going fullscreen to a projector. Load sources into decks, crossfade between channels. You only add complexity when you need it.

The full routing graph is: **Sources → Decks → Channels → Mixer → Surfaces → Outputs**.

A **Deck** wraps a source (shader, video, image, solid color, camera, NDI stream, or SRT stream) with its own effect chain, deck specific transition settings, and parameters. Decks at zero opacity are culled from the render pass.

**Channels** composite their decks together using per-deck opacity, blend modes, and optional auto-transitions. Channels have their own effect chain applied after compositing.

The **Mixer** composites channels together. With two channels you get an A/B crossfader. With three or more, per-channel opacity and blend modes. The mixer has its own master effect chain applied to the final composite. The mixer also has a state machine like multi channel transistion sequence builder. 

**Surfaces** are optional. They define polygonal regions on a 2D stage canvas, each with its own content source (main, a channel, or a sub-mix of channels). Surfaces are how you map content onto physical screens, LED panels, or projection areas. When no surfaces are defined, outputs receive the full main mix directly.

**Outputs** are where rendered frames go: a window, a fullscreen display, an NDI stream, an SRT stream, or a recording. Surfaces are assigned to outputs to complete the routing chain.

At every junction you can branch, split, or re-route. Two channels feeding different surfaces on the same output. The main mix on one output, a single channel isolated on another. A sub-mix of specific channels to an NDI stream while the master goes to projection. You only use the complexity you need for your use case. Be it simple A/B crossfading between two decks, or a multi-channel mixing console with dedicated FX and transitions, or a complex multi-screen setup with individual content routing. 