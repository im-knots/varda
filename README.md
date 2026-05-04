# Varda

Open-source VJ performance tool. 

## What it does

Live visual mixing with generative shaders, video, and images. Drag sources onto channels, chain effects, modulate parameters, send output to projectors.

- **ISF shader pipeline** — community shaders work out of the box
- **Multi-channel mixer** — crossfader, per-channel blend modes, N-channel compositing
- **Effect chains** — deck → channel → master, drag-and-drop from library
- **Modulation** — LFO, audio-reactive, ADSR, step sequencer, mod-on-mod
- **MIDI control** — multi-device, learn mode, APC Mini LED feedback
- **Projection mapping** — 2D stage editor, polygon/circle surfaces, per-surface warp
- **Video** — HAP (GPU-native) + ffmpeg fallback
- **Multi-output** — multiple windows, fullscreen on any display

## Build & run

Requires Rust (stable) and a GPU with Metal (macOS) or Vulkan support.

```
cargo run
```

ISF shaders go in `shaders/generators/` and `shaders/filters/`. The app scans these on startup.

## Project structure

```
src/          — Rust source
spec/         — living design specs (how things work and why)
vision/       — north star, parity analysis
intent/       — motivations and beliefs
shaders/      — ISF shader library (generators, filters, transitions)
```


