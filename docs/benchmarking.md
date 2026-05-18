# Benchmarking

Criterion harness for the compositing pipeline and per-frame shader parameter buffer build. Ensures perf changes land with quantitative evidence.

## Quick Start

```sh
cargo bench --bench compositing      # GPU suites; needs an adapter, headless ok
cargo bench --bench shader_params    # CPU suite

./scripts/bench-smoke.sh             # --test on both, no sampling (CI/pre-commit)
```

## Before/After Comparison

```sh
cargo bench --bench compositing -- --save-baseline pre
# ... make your perf change ...
cargo bench --bench compositing -- --baseline pre
```

## GPU Suites (`benches/compositing.rs`)

| Benchmark | What it measures |
|---|---|
| `channel_composite_solid` | Solid-color decks (LoadOp::Clear, no fragment shader). Slope across deck counts isolates per-deck copy-on-composite cost. |
| `channel_composite_shader` | Same shape with `bars.fs` on every pixel. Difference vs solid at N decks ≈ N × per-deck shader execution cost. |
| `mixer_crossfade` | Two channels through the crossfader at 50%. |

A 60fps preflight panics if 8-deck solid composite at 1080p exceeds the 16.67ms frame budget. Disable with `VARDA_BENCH_SKIP_SLO=1`. After the criterion groups, a per-deck slope (decks/8 − decks/1, ÷ 7) is computed and printed.

## CPU Suite (`benches/shader_params.rs`)

| Variant | What it measures |
|---|---|
| `no_mod` | std140 byte buffer serialization only |
| `empty_mod` | Modulation engine present but no assignments — isolates per-param key construction cost |
| `active_lfo` | Full modulation path: lookup, LFO read, clamp, write |

The `empty_mod − no_mod` gap is the per-param allocation cost paid even when nothing is modulated. Multiply by params × decks × effects to estimate the per-frame floor.

## Notes

Criterion HTML reports land in `target/criterion/`.

`compositing` runs at 1920×1080 and calls `device.poll(Wait)` after each iteration so wall-clock reflects GPU work. Without a GPU adapter it prints `no GPU adapter — skipping` and exits clean. Numbers are machine-local; close other GPU work and warm the machine for stability.
