# Varda

Open-source VJ performance tool for Linux and macOS.

## What it does

Live visual mixing with generative shaders, videos, live cameras, NDI/SRT, and images. Drag sources onto channels, chain effects, modulate parameters, define surfaces, assign them to outputs, and send output to projectors, recordings, and streams.

- **ISF shader pipeline** — community shaders work out of the box
- **NDI/SRT I/O** — receive and send video streams
- **Multi-channel mixer** — crossfader, per-channel blend modes, N-channel compositing
- **Effect chains** — deck → channel → master, drag-and-drop from library
- **Modulation** — LFO, audio-reactive, ADSR, step sequencer, mod-on-mod, and audio reactivity
- **MIDI control** — multi-device, learn mode, controller profiles with LED feedback
- **Projection mapping** — 2D stage editor, polygon/circle surfaces, per-surface warp
- **Video** — HAP (GPU-native) + ffmpeg fallback
- **Multi-output** — multiple windows, fullscreen on any display, NDI, SRT, and recording

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

ISF shaders go in `shaders/`. The app scans these on startup.


## Abstractions you should know about

**Content** is a source of some visual input. It can be a shader, a video, an image, a solid color, an NDI/SRT input, or a camera feed. It is the lowest level of abstraction in the engine. Content becomes a **Deck**. Decks have their own parameters and effect chains, transition settings, and blend modes. Decks are always active in their channel. Deck opacity determiines if they are visible or not

A **Deck** is an independent render unit. It wraps a content source and renders it to a texture each frame. Decks have their own effect chains and parameters.

Decks live inside **Channels**. A channel composites its decks together using per-deck opacity, blend modes, and optional auto-transitions. Channels also have their own effect chain applied after deck compositing.

Channels are composited into the **Main Channel** by the mixer. With two channels the mixer uses a crossfader; with three or more it uses per-channel opacity and blend modes. The main channel has its own effect chain, applied to the final composite after all channels are mixed.

**Surfaces** are optional. They define polygonal regions on a 2D stage canvas. Each surface has a content source (main, a channel, or a sub-mix of channels) and a content mapping (fill or UV-mapped). Surfaces are how you map content onto physical screens, LED panels, or projection areas. When no surfaces are defined, the full output receives the main channel directly.

**Outputs** define where rendered frames are sent: a window, a fullscreen display, an NDI stream, an SRT stream, or a recording. Surfaces are assigned to outputs to complete the routing chain.

With these abstractions you can build complex routing. Two channels with their own decks and effects, each assigned to a different surface on the same output, or to different outputs entirely. The main channel on one output, a single channel isolated on another.