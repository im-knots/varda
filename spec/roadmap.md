# Varda Implementation Roadmap

## Status: IMPLEMENTED (living document — updated as phases complete)

Traces from: [/intent/why.md](/intent/why.md), [/vision/parity-gap.md](/vision/parity-gap.md)

## Guiding Constraints

1. **Refactor in place** — modify existing prototype code, don't rewrite from scratch
2. **Remove dead code** — delete old paths as they're replaced; no backwards compat needed
3. **Each phase must leave the app runnable** — never break the build for more than one phase
4. **Specs before code** — each phase's spec(s) must be at AGREED status before implementation begins

---

## Phase 0: Stability Foundation

**Why**: A VJ tool that crashes mid-set is unusable. Stability is Tier 1. ([/intent/why.md](/intent/why.md), parity-gap #3)

| Task | Spec | Current State |
|---|---|---|
| Graceful shader compilation errors (catch, notify, fallback) | [error-handling.md](/spec/error-handling.md) | ✅ IMPLEMENTED — errors caught, notifications emitted |
| Notification system (non-modal toasts) | [error-handling.md](/spec/error-handling.md) | ✅ IMPLEMENTED — NotificationSystem with severity levels, auto-dismiss, overlay rendering |
| Hot-reload failure resilience (keep last good shader) | [error-handling.md](/spec/error-handling.md) | ✅ IMPLEMENTED — registry keeps last good shader, emits ShaderEvent::Error |
| Registry scan error tolerance | [error-handling.md](/spec/error-handling.md) | ✅ IMPLEMENTED — scan continues on error, logs warnings |

**Depends on**: Nothing — can start immediately.
**Delivers**: App no longer crashes from bad shaders. Foundation for all future error reporting.
**Status**: ✅ COMPLETE

---

## Phase 1: Core Architecture — Channel + Mixer — ✅ COMPLETE

**Why**: The channel/mixer routing model is the backbone of every feature that follows. Parity-gap #1, #7. ([/spec/architecture-overview.md](/spec/architecture-overview.md))

| Task | Spec | Current State |
|---|---|---|
| Create `Channel` struct (owns decks, composites them, has effect chain) | [channel-routing.md](/spec/channel-routing.md), [architecture-overview.md](/spec/architecture-overview.md) | ✅ IMPLEMENTED |
| Create `Mixer` struct (owns channels, crossfader, master FX, modulation) | [channel-routing.md](/spec/channel-routing.md), [architecture-overview.md](/spec/architecture-overview.md) | ✅ IMPLEMENTED |
| Migrate `Stage` deck compositing logic → `Channel` | [architecture-overview.md](/spec/architecture-overview.md) | ✅ IMPLEMENTED |
| Migrate `Stage` master effects + modulation → `Mixer` | [architecture-overview.md](/spec/architecture-overview.md) | ✅ IMPLEMENTED |
| Delete `src/stage/` module | [architecture-overview.md](/spec/architecture-overview.md) | ✅ DELETED |
| Update `main.rs` to use Mixer instead of Stage | — | ✅ IMPLEMENTED |
| Update scene save/load for Channel/Mixer hierarchy | [scene-management.md](/spec/scene-management.md) | Import fixed; full hierarchy save/load deferred |
| Per-channel effect chains | [channel-routing.md](/spec/channel-routing.md) | ✅ IMPLEMENTED (Channel.effects) |
| Channel opacity & blend mode | [channel-routing.md](/spec/channel-routing.md) | ✅ IMPLEMENTED (Channel.opacity, Channel.blend_mode) |

**Depends on**: Phase 0 (error handling should be in place before adding complexity).
**Delivers**: The new Deck → Channel → Mixer → Output signal flow. App still works, but decks are now grouped into channels.
**Status**: ✅ COMPLETE

---

## Phase 2: Crossfader & Basic Transitions — ✅ COMPLETE

**Why**: A/B crossfading is the core VJ workflow. Parity-gap #1, #6. ([/spec/transitions.md](/spec/transitions.md))

| Task | Spec | Current State |
|---|---|---|
| Crossfader on Mixer (0.0–1.0, blends 2 channels) | [transitions.md](/spec/transitions.md) | ✅ IMPLEMENTED |
| Timed auto-crossfade (linear/eased, N seconds) | [transitions.md](/spec/transitions.md) | ✅ IMPLEMENTED |
| Beat-synced crossfade trigger | [transitions.md](/spec/transitions.md) | ✅ IMPLEMENTED |
| Crossfader UI (slider, snap A/B, auto buttons) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |

**Depends on**: Phase 1 (channels must exist).
**Delivers**: Working A/B crossfader. VJ can prepare Channel B while Channel A is live.
**Status**: ✅ COMPLETE

---

## Phase 3: UI Overhaul — ✅ COMPLETE

**Why**: The UI must reflect the new channel-based architecture. ([/spec/ui-design.md](/spec/ui-design.md))

| Task | Spec | Current State |
|---|---|---|
| Channel column layout (side-by-side, deck grid per channel) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Main output preview panel (always visible) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED (right panel) |
| DJ-style Mixer Box (center column with faders, crossfader, transitions) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Context-sensitive bottom bar (deck detail / channel FX / master FX) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Deck preview in bottom bar (scales with panel height, 16:9) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Effect chain as horizontal columns in bottom bar | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Modulation panel in right sidebar (horizontal columns per modulator) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Shader library in right sidebar | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Dark theme color language (purple/blue/orange/green accents) | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Channel opacity/blend mode controls | [channel-routing.md](/spec/channel-routing.md) | ✅ IMPLEMENTED |
| 1920×1080 default window size | [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Channel add/remove/rename controls | [channel-routing.md](/spec/channel-routing.md) | Deferred (stretch) |
| Deck drag between channels (hot reassignment) | [channel-routing.md](/spec/channel-routing.md) | ✅ IMPLEMENTED — drag-and-drop deck thumbnails between channel columns |

**Depends on**: Phase 1 + Phase 2 (need channels + crossfader to display).
**Delivers**: Professional-looking UI that matches the channel-based workflow.
**Status**: ✅ COMPLETE (core layout)

---

## Phase 4: MIDI Control System — ✅ COMPLETE (core)

**Why**: Can't perform without hardware control. Parity-gap #2. ([/spec/midi-control.md](/spec/midi-control.md))

| Task | Spec | Current State |
|---|---|---|
| MIDI input via coremidi (macOS) | [midi-control.md](/spec/midi-control.md) | ✅ IMPLEMENTED |
| Parameter address system (hierarchical paths) | [midi-control.md](/spec/midi-control.md), [modulation.md](/spec/modulation.md) | ✅ IMPLEMENTED |
| MIDI learn mode (right-click param → move control → mapped) | [midi-control.md](/spec/midi-control.md) | ✅ IMPLEMENTED |
| Mapping store (in-memory, applies MIDI to mixer) | [midi-control.md](/spec/midi-control.md) | ✅ IMPLEMENTED |
| Crossfader MIDI mapping | [midi-control.md](/spec/midi-control.md) | ✅ IMPLEMENTED |
| MIDI learn status indicator (purple bar + cancel) | [midi-control.md](/spec/midi-control.md) | ✅ IMPLEMENTED |
| Mapping persistence (save/load to config) | [midi-control.md](/spec/midi-control.md) | Not implemented |
| LED feedback for Akai APC Mini | [midi-control.md](/spec/midi-control.md) | Not implemented |

**Depends on**: Phase 1 (parameter paths need channel/mixer hierarchy).
**Delivers**: Hardware control of Varda. VJ can map any MIDI controller to any parameter.
**Status**: ✅ COMPLETE (core — persistence and LED feedback are stretch)

---

## Phase 5: New Source Types & Resolution — ✅ COMPLETE

**Why**: Image support is simple and immediately useful. Configurable resolution is expected. Parity-gap #12. ([/spec/deck-sources.md](/spec/deck-sources.md), [/spec/resolution-and-scaling.md](/spec/resolution-and-scaling.md))

| Task | Spec | Current State |
|---|---|---|
| Image/still deck source (PNG, JPG) | [deck-sources.md](/spec/deck-sources.md) | ✅ Implemented |
| Solid color deck source | [deck-sources.md](/spec/deck-sources.md) | ✅ Implemented |
| UI for adding image/solid sources | [deck-sources.md](/spec/deck-sources.md) | ✅ Implemented |
| Scaling modes (fill, fit, stretch, center) | [resolution-and-scaling.md](/spec/resolution-and-scaling.md) | ✅ Implemented |

**Status**: ✅ COMPLETE

---

## Phase 6: Modulation Engine Expansion — ✅ COMPLETE

**Why**: Universal modulation is what makes visuals feel alive and musical. ([/spec/modulation.md](/spec/modulation.md))

| Task | Spec | Current State |
|---|---|---|
| Parameter address system (shared with MIDI — if not done in Phase 4) | [modulation.md](/spec/modulation.md) | ✅ IMPLEMENTED (Phase 4) |
| Multiple modulation sources per target (additive stacking) | [modulation.md](/spec/modulation.md) | ✅ IMPLEMENTED (existing) |
| ADSR envelope generator | [modulation.md](/spec/modulation.md) | ✅ IMPLEMENTED |
| Step sequencer source | [modulation.md](/spec/modulation.md) | ✅ IMPLEMENTED |
| Modulator-on-modulator (with depth limit of 4) | [modulation.md](/spec/modulation.md) | ✅ IMPLEMENTED |
| Modulation UI: per-parameter assignment button, source visualization | [modulation.md](/spec/modulation.md) | ✅ IMPLEMENTED (ADSR/StepSeq controls, gate buttons) |
| Modulation UI: color-coded modulators, waveform/ADSR visualization, live slider ghosting | [modulation.md](/spec/modulation.md), [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |

**Depends on**: Phase 4 (shares parameter address system), Phase 1 (hierarchy exists).
**Delivers**: Full modulation engine — LFO, audio, ADSR, step seq, mod-on-mod. Any parameter, anywhere.
**Status**: ✅ COMPLETE

---

## Phase 6b: GPU Rendering Pipeline Optimization — ✅ COMPLETE

**Why**: With 8+ decks of expensive generative shaders, framerate degraded due to CPU-GPU sync overhead from per-render `queue.submit()` calls. A VJ tool must maintain 60fps under heavy load. ([/intent/why.md](/intent/why.md) belief #2, [/vision/north-star.md](/vision/north-star.md) success criteria)

| Task | Spec | Current State |
|---|---|---|
| Command buffer batching (deck, channel, mixer render paths) | [performance-optimization.md](/spec/performance-optimization.md) | ✅ IMPLEMENTED — 18 submits → 3 per frame |
| Zero-opacity deck culling (skip invisible decks entirely) | [performance-optimization.md](/spec/performance-optimization.md) | ✅ IMPLEMENTED |
| Architecture overview updated with batching pattern | [architecture-overview.md](/spec/architecture-overview.md) | ✅ IMPLEMENTED |

**Depends on**: Phase 1 (channel/mixer architecture must exist to optimize).
**Delivers**: Handles 8+ decks of complex generative shaders without framerate collapse. GPU driver gets full workload visibility for optimal scheduling.
**Status**: ✅ COMPLETE

---

## Phase 7: Advanced Transitions — PARTIALLY COMPLETE

**Why**: Shader-based transitions and the transition builder differentiate Varda. Parity-gap #6. ([/spec/transitions.md](/spec/transitions.md))

| Task | Spec | Current State |
|---|---|---|
| Shader-based transitions (ISF transition shaders between channels) | [transitions.md](/spec/transitions.md) | ✅ IMPLEMENTED |
| Transition UI (selector dropdown in mixer box) | [transitions.md](/spec/transitions.md), [ui-design.md](/spec/ui-design.md) | ✅ IMPLEMENTED |
| Transition builder (sequenced steps for 3+ channels) | [transitions.md](/spec/transitions.md) | Not implemented |
| Transition builder UI (sequencer-style) | [transitions.md](/spec/transitions.md) | Not implemented |
| Channel presets (save/load individual channel compositions) | [transitions.md](/spec/transitions.md), [scene-management.md](/spec/scene-management.md) | Not implemented |
| Deck presets (save/load individual deck configs) | [scene-management.md](/spec/scene-management.md) | Not implemented |

**Depends on**: Phase 2 (basic crossfader), Phase 3 (UI), Phase 1 (channels).
**Delivers**: Pro transition workflow — shader wipes/dissolves, automated multi-channel sequences, preset library.
**Status**: Partially complete — shader transitions and UI done, builder/presets remaining.

---

## Phase 8: Multi-Output & Projection Mapping

**Why**: Can't use at a real venue without sending to a projector. Parity-gap #4, #5. ([/spec/multi-output.md](/spec/multi-output.md))

Uses a **three-layer abstraction**: Content (channels, master) → Surfaces (named regions in stage model) → Outputs (physical displays/projectors/LED controllers). See [multi-output.md](/spec/multi-output.md) for full design.

### Phase 8a: Multi-Window Outputs & Source Routing — Size: M

| Task | Spec | Current State |
|---|---|---|
| Refactor RenderContext into SharedGPU + per-window surface | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Create/destroy output windows at runtime via UI | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Route content sources (channel/master/deck) to output windows | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Fullscreen borderless output on target monitor | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Multi-window event dispatch (WindowId routing in ApplicationHandler) | [multi-output.md](/spec/multi-output.md) | Not implemented |

**Depends on**: Phase 1 (mixer).
**Delivers**: Multiple output windows showing different content, fullscreenable on projectors.

### Phase 8b: 2D Surface Editor — Size: M

| Task | Spec | Current State |
|---|---|---|
| 2D canvas UI for placing/naming rectangular surfaces | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Route content (channel/master/deck) to surfaces | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Live preview thumbnails on surfaces in the editor | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Surface layout saved/loaded with scene file | [multi-output.md](/spec/multi-output.md) | Not implemented |

**Depends on**: Phase 8a.
**Delivers**: Visual stage layout editor for venue pre-production and spatial content routing.

### Phase 8c: Quad Warp & Calibration — Size: M

| Task | Spec | Current State |
|---|---|---|
| Warp shader (homography-based UV remapping) | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Corner-pin calibration UI (drag corners on fullscreen output) | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Derive camera/homography matrix from corner correspondences | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Warp calibration saved with scene file | [multi-output.md](/spec/multi-output.md) | Not implemented |

**Depends on**: Phase 8a.
**Delivers**: Perspective-corrected projection output. Usable at a gig with projectors.

### Phase 8d: Multi-Surface Per Output — Size: L

| Task | Spec | Current State |
|---|---|---|
| One output window renders multiple warped surfaces | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Per-surface warp calibration within a single output | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Surface selection/editing in calibration mode | [multi-output.md](/spec/multi-output.md) | Not implemented |

**Depends on**: Phase 8b, 8c.
**Delivers**: Single projector covering multiple venue surfaces (e.g., main screen + floor).

### Phase 8e: 3D Stage Model Import — Size: XL

| Task | Spec | Current State |
|---|---|---|
| OBJ/glTF model import, named meshes → surfaces | [multi-output.md](/spec/multi-output.md) | Not implemented |
| 3D preview of mapped content on model | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Virtual camera placement matching physical projectors | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Camera-based projection rendering (render scene from projector POV) | [multi-output.md](/spec/multi-output.md) | Not implemented |

**Depends on**: Phase 8b, 8d.
**Delivers**: Full 3D spatial mapping — import venue model, map content to surfaces, calibrate projectors.

### Phase 8f: LED Direct Output — Size: M

| Task | Spec | Current State |
|---|---|---|
| LED output type (pixel-accurate crop/scale, no warp) | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Per-surface resolution and crop region config | [multi-output.md](/spec/multi-output.md) | Not implemented |
| NDI output option (stretch) | [multi-output.md](/spec/multi-output.md) | Not implemented |

**Depends on**: Phase 8a, 8b.
**Delivers**: Direct LED panel output with pixel-accurate content mapping.

### Phase 8g: Edge Blending — Size: L

| Task | Spec | Current State |
|---|---|---|
| Soft-edge blend shader for overlapping projector regions | [multi-output.md](/spec/multi-output.md) | Not implemented |
| Blend zone width/gamma configuration per edge | [multi-output.md](/spec/multi-output.md) | Not implemented |

**Depends on**: Phase 8d.
**Delivers**: Seamless multi-projector setups with smooth overlap blending.

---

## Phase 9: Settings, Monitoring & Polish

**Why**: Professional polish and performance visibility. Parity-gap #10. ([/spec/settings-and-monitoring.md](/spec/settings-and-monitoring.md))

| Task | Spec | Current State |
|---|---|---|
| Settings panel (render, audio, MIDI, OSC, shader library config) | [settings-and-monitoring.md](/spec/settings-and-monitoring.md) | Not implemented |
| FPS overlay with frame time graph | [settings-and-monitoring.md](/spec/settings-and-monitoring.md) | Not implemented |
| Per-deck render time stats | [settings-and-monitoring.md](/spec/settings-and-monitoring.md) | Not implemented |
| GPU info display | [settings-and-monitoring.md](/spec/settings-and-monitoring.md) | Not implemented |
| Webcam/capture device source | [deck-sources.md](/spec/deck-sources.md) | Not implemented |
| Config file persistence (XDG on Linux, ~/Library on macOS) | [settings-and-monitoring.md](/spec/settings-and-monitoring.md) | Not implemented |

**Depends on**: Most other phases complete.
**Delivers**: Professional polish layer — settings UI, perf stats, config persistence.

---

## Dependency Graph

```
Phase 0: Stability ──────────────────────────────────────────┐
    │                                                        │
Phase 1: Channel + Mixer ───────────────────────────┐        │
    │              │              │                  │        │
Phase 2: Crossfader │         Phase 4: MIDI     Phase 5: Sources
    │              │              │                  │
Phase 3: UI Overhaul        Phase 6: Modulation     │
    │              │              │                  │
Phase 7: Transitions         (uses param addresses) │
    │                                                │
Phase 8: Multi-Output ──────────────────────────────┘
    │
Phase 9: Polish
```

Phases 4, 5 can run in parallel with Phases 2, 3 after Phase 1 is complete.
Phase 6 depends on Phase 4 (shared parameter address system).
Phase 7 depends on Phases 2 + 3.
Phase 8 depends on Phase 1 + 3.
Phase 9 is the final polish pass.

## Traceability Summary

| Phase | Parity Gap Items | Spec Documents |
|---|---|---|
| 0 | #3 (stability) | error-handling.md |
| 1 | #1 (A/B routing), #7 (channel FX) | architecture-overview.md, channel-routing.md, scene-management.md |
| 2 | #1 (crossfader), #6 (auto-transitions) | transitions.md |
| 3 | — (enabler) | ui-design.md, channel-routing.md |
| 4 | #2 (MIDI), #8 (MIDI learn) | midi-control.md, modulation.md |
| 5 | #12 (image support) | deck-sources.md, resolution-and-scaling.md |
| 6 | — (quality of life) | modulation.md |
| 6b | — (performance, intent belief #2) | performance-optimization.md, architecture-overview.md |
| 7 | #6 (auto-transitions), #13 (presets), #14 (transition builder) | transitions.md, scene-management.md |
| 8 | #4 (multi-output), #5 (projection mapping), #9 (fullscreen) | multi-output.md |
| 9 | #10 (perf monitoring) | settings-and-monitoring.md |


