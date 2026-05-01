# Varda — Architecture Overview

## Target Architecture

This document describes the target architecture for Varda. The existing prototype code is being refactored in-place toward this design. There are no existing users, so there are no backwards compatibility constraints. Old code paths that no longer serve the target architecture should be removed.

### Signal Flow

```
[Deck] → [Deck FX] ─┐
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┐
[Deck] → [Deck FX] ─┘                               │
                                                     ├─→ [Mixer] → [Master FX] → [Output(s)]
[Deck] → [Deck FX] ─┐                               │
[Deck] → [Deck FX] ─┼─→ [Channel] → [Channel FX] ─┘
[Deck] → [Deck FX] ─┘
```

### Subsystem Map

#### Deck (`src/deck/`)
Independent render unit. Outputs a texture at the stage render resolution.

- **Source**: ISF shader generator, video file, image, webcam, solid color (see [deck-sources.md](/spec/deck-sources.md))
- **Effect chain**: Ordered stack of ISF filter shaders
- **Parameters**: Source params + effect params, all modulatable
- **Opacity & blend mode**: Controls how this deck composites within its channel
- **Solo/mute**: Per-deck visibility control
- Sources that don't match stage resolution are scaled at the deck level (see [resolution-and-scaling.md](/spec/resolution-and-scaling.md))

#### Channel (`src/channel/`)
Groups multiple decks into a composited layer. Each channel has its own effect chain.

- **Decks**: N decks composited together using per-deck opacity and blend modes
- **Effect chain**: ISF filters applied to the composited channel output
- **Opacity & blend mode**: Controls how this channel mixes into the final output
- **Naming**: A, B, C, ... Z, A1, B1, ... (see [channel-routing.md](/spec/channel-routing.md))
- **Always renders**: Even when not visible via crossfader, so the VJ can prepare content
- Decks can be hot-reassigned between channels during performance
- **Visibility culling**: Muted decks and decks with zero opacity are skipped entirely (see [performance-optimization.md](/spec/performance-optimization.md))
- **Batched rendering**: All deck renders are collected into a single `queue.submit()` call before compositing begins (see [performance-optimization.md](/spec/performance-optimization.md))

#### Mixer (`src/mixer/`)
Composites channels into the final output. Owns the crossfader and master effects.

- **2 channels**: Crossfader active, blends between assigned pair
- **3+ channels**: Crossfader deactivates, channels mixed by per-channel opacity/blend
- **Master effect chain**: ISF filters applied to final composite
- **Transition engine**: Crossfade, beat-synced, shader-based transitions between channels (see [transitions.md](/spec/transitions.md))
- **Output routing**: Routes final output to display windows (see [multi-output.md](/spec/multi-output.md))
- **Batched master effects**: Master effect command buffers are collected and batch-submitted (see [performance-optimization.md](/spec/performance-optimization.md))

#### Rendering Core (`src/renderer/`)
Low-level GPU abstraction. Unchanged from prototype — this layer is solid.

- **wgpu** backend targeting Vulkan (Linux) and Metal (macOS)
- **UnifiedPipeline**: Single pipeline type for generators, filters, single-pass, multi-pass shaders
- **BlitPipeline**: Texture compositing with blend modes and opacity
- **RenderContext**: Surface, device, queue, resize handling
- **Command buffer batching**: Render methods accept `&mut Vec<CommandBuffer>` and push instead of submitting individually; batch submission happens at defined synchronization points (see [performance-optimization.md](/spec/performance-optimization.md))

#### ISF Shader System (`src/isf/`)
Shader loading, parsing, compilation. Unchanged from prototype.

- ISF metadata parser (JSON header → Rust structs)
- GLSL → SPIR-V compilation via `shaderc`
- SPIR-V → WGSL translation via `naga`
- Automatic uniform binding from ISF metadata
- Built-in variables: TIME, RENDERSIZE, isf_FragNormCoord, FRAMEINDEX, PASSINDEX
- Multi-pass rendering with persistent buffers (feedback effects)
- **Must handle compilation errors gracefully** — never crash (see [error-handling.md](/spec/error-handling.md))

#### Audio (`src/audio/`)
Audio input, analysis, and texture packing. Unchanged from prototype.

- Audio input via `cpal`
- FFT analysis via `rustfft`
- Audio waveform and FFT packed into textures for ISF shaders
- BPM detection, beat phase tracking
- Audio level, bass/mid/treble band extraction

#### Modulation Engine (`src/modulation/`)
Universal parameter automation. Fully implemented.

- **Sources**: LFO, audio band, ADSR envelope, step sequencer (see [modulation.md](/spec/modulation.md))
- **Targets**: Any numeric parameter anywhere in the hierarchy via address path
- **Stacking**: Multiple sources per target, summed with per-source amount
- **Modulator-on-modulator**: Source parameters are themselves modulatable (max depth limit)
- Global engine, evaluated every frame before rendering

#### Control (`src/osc/`, `src/midi/`)
External hardware/software control. MIDI fully implemented, OSC from prototype.

- **OSC**: Receive and send, configurable ports (see [settings-and-monitoring.md](/spec/settings-and-monitoring.md))
- **MIDI**: Learn mode (right-click any parameter), mapping store, crossfader mapping, via coremidi on macOS (see [midi-control.md](/spec/midi-control.md))
- Both map to the same parameter address system as modulation

#### Shader Registry (`src/registry/`)
Shader library discovery and hot-reload. Unchanged from prototype.

- Library folder scanning (configurable paths)
- ISF metadata indexing and categorization (generator, filter, transition)
- File watcher for hot-reload via `notify`
- Graceful error handling for invalid shaders

#### Scene (`src/scene/`)
Project file save/load. Expanding to support new hierarchy.

- **Scene**: Full project state — all channels, decks, effects, modulation, MIDI mappings, output config
- **Channel preset**: Save/load individual channel compositions
- **Deck preset**: Save/load individual deck configurations
- JSON format (see [scene-management.md](/spec/scene-management.md))

#### UI (`src/ui/`)
Immediate-mode UI via egui. Resolume-inspired layout fully implemented.

- Fixed layout with resizable panels, 1920×1080 default (see [ui-design.md](/spec/ui-design.md))
- **Central area**: Channel A column | DJ-style Mixer Box | Channel B column
- **Mixer Box**: Vertical opacity faders (purple A / blue B), horizontal crossfader, snap buttons, auto-transition buttons (timed/beat-synced), transition shader selector, blend mode selectors
- **Right sidebar**: Main output preview (always visible, clickable for master FX editing), modulation panel (horizontal columns per modulator), shader library browser
- **Bottom bar** (resizable, context-sensitive): Deck detail (preview + generator params + effect chain columns), channel effects, or master effects — depending on selection
- Deck preview in bottom bar scales with panel height, maintains 16:9 aspect ratio
- Dark theme with accent colors: purple (Channel A), blue (Channel B), orange (modulation), green (audio)
- Notification system for errors (see [error-handling.md](/spec/error-handling.md))
- Settings panel (see [settings-and-monitoring.md](/spec/settings-and-monitoring.md))

#### Video (`src/video/`)
Video file playback. Expanding for full playback controls.

- Decode via ffmpeg-next
- Loop modes, speed control, beat-sync, scrub/seek, in/out points (see [deck-sources.md](/spec/deck-sources.md))

### Prototype → Target Migration (Completed)

The prototype code has been refactored in-place. All key structural changes are done:

| Prototype | Target | Status |
|---|---|---|
| `Stage` composites decks directly | `Channel` composites decks, `Mixer` composites channels | ✅ Done — `src/stage/` deleted |
| `Stage.master_effects` | `Mixer.master_effects` | ✅ Done |
| `Stage.modulation` | Global `ModulationEngine` on Mixer | ✅ Done |
| Flat deck list | Decks grouped by channel | ✅ Done |
| No channel FX | Per-channel effect chain | ✅ Done |
| No crossfader | Crossfader on Mixer | ✅ Done |
| No MIDI | Full MIDI system (coremidi) | ✅ Done |
| No image source | Image as deck source | ✅ Done |
| Fixed 1080p | User-configurable resolution | Settings — deferred |

Old code paths have been removed during refactoring. No dead code.

## Tech Stack

| Layer | Technology |
|---|---|
| Language | Rust (2021 edition) |
| Graphics API | wgpu 27.0 (Vulkan/Metal) |
| Shader Format | ISF/GLSL → SPIR-V → WGSL |
| Shader Compiler | shaderc 0.10 |
| Shader Translator | naga 27.0 |
| Windowing | winit 0.30 |
| UI | egui 0.33 + egui-wgpu + egui-winit |
| Audio | cpal 0.17, rustfft 6.4 |
| Video | ffmpeg-next 8.0 |
| MIDI | coremidi (macOS) |
| OSC | rosc 0.11 |
| File Watching | notify 8.2 |
| Serialization | serde + serde_json |

