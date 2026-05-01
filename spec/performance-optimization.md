# Varda — Performance Optimization

## Status: IMPLEMENTED

Traces from: [/intent/why.md](/intent/why.md) belief #2 ("The GPU should do the work — CPU stays out of the render path"), [/vision/north-star.md](/vision/north-star.md) success criteria ("60fps at 1080p with 4+ active decks across multiple channels on mid-range hardware")

## Problem

With multiple decks running expensive generative shaders (e.g., 8 instances of `tas_psychedelic.fs`), framerate degraded significantly. Investigation revealed the bottleneck was not raw GPU compute but CPU-GPU synchronization overhead from excessive `queue.submit()` calls.

## Root Cause Analysis

Each `wgpu::Queue::submit()` call acts as a synchronization point between CPU and GPU. The driver must:
1. Flush any staged `write_buffer` operations
2. Submit the command buffer to the GPU command queue
3. Potentially wait for previous submissions to complete

The original rendering pipeline called `queue.submit()` once per render operation:
- **Per deck**: 1 submit for the shader render + 1 per effect
- **Per channel**: 1 submit per deck composite + 1 per channel effect
- **Per mixer**: 1 submit per master effect

With 8 decks, each with 1 effect, in a single channel: ~18 separate `submit()` calls per frame. Each one stalls the CPU while the GPU processes the previous batch.

## Solution: Command Buffer Batching

### Architecture

Collect `wgpu::CommandBuffer`s into a `Vec` and submit them in a single batch at well-defined synchronization points.

```
Before:  Deck.render() → submit()  ×8  →  Channel.composite() → submit()  ×8
After:   Deck.render() → push()    ×8  →  batch submit()  →  Channel.composite() → ...
```

### Implementation

**API change**: Render methods accept `cmd_buffers: &mut Vec<wgpu::CommandBuffer>` parameter. Instead of calling `queue.submit(iter::once(encoder.finish()))`, they call `cmd_buffers.push(encoder.finish())`.

**Synchronization points** (where batch submit occurs):
1. **Channel**: After all deck renders complete, before compositing begins (compositing reads deck output textures)
2. **Channel**: After all channel effects complete
3. **Mixer**: After all master effects complete

**Exception — multi-pass ISF shaders**: Intermediate passes within a single shader still require immediate `submit()` because they write to textures read by subsequent passes, and they share a uniform buffer updated via `queue.write_buffer()` between passes. Only the final pass pushes to the collector.

### Result

For 8 single-pass shader decks: **18 submits → 3 submits per frame** (1 batch for deck renders, 1 for channel composite, 1 for output blit). This gives the GPU driver full visibility into the workload for optimal scheduling.

## Additional Optimizations

### Zero-Opacity Deck Culling

**Location**: `src/channel/mod.rs` render loop

Decks with `opacity <= 0.0` are skipped entirely — no shader execution, no texture allocation, no command encoding. This is a free optimization with no visual impact since invisible decks contribute nothing to the composite.

### Muted Deck Skipping

Pre-existing: Decks with `muted == true` were already skipped. The opacity check extends this to cover the continuous case.

## Files Modified

| File | Change |
|---|---|
| `src/deck/mod.rs` | `render_simple_static`, `render_multi_pass_static`, `Effect::apply`, `Effect::apply_with_modulation` — accept `cmd_buffers` param, push instead of submit |
| `src/channel/mod.rs` | Creates collector `Vec`, passes through deck renders, batch submits before compositing. Zero-opacity culling added. |
| `src/mixer/mod.rs` | `apply_master_effects` uses collector pattern with batch submit |

## Open Questions

None — this optimization is complete and tested.

## Future Optimization Candidates

These were identified but not implemented. They would require spec discussion:

1. **Render resolution scaling** — per-deck or global quality setting (e.g., 50% = 960×540). Biggest potential win for GPU-bound workloads.
2. **Frame-skip for non-visible decks** — render previews every Nth frame instead of every frame.
3. **Lower-resolution preview textures** — separate small render target for UI thumbnails.
4. **Bind group caching** — reuse bind groups across frames when uniforms haven't changed.

