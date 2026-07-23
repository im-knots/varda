# Contributing to Varda

Thanks for your interest in contributing to Varda. This document covers the architecture, conventions, and workflow you need to get a change merged. For end-user documentation (install, usage, panels, shaders), see the [manual](docs/README.md).

## Getting Set Up

Varda is written in Rust. Follow [Build from Source](docs/01-getting-started.md#build-from-source) in the manual to install system dependencies and get `cargo build --release` running, then:

```bash
cargo run --release          # launch the app
cargo test --lib             # unit tests
cargo test --test ui_integration   # GPU-free integration tests
```

## Architecture

Varda is built with domain-driven design and clean architecture principles. Think Uncle Bob. The codebase separates concerns into four layers:

```
src/
  engine/        # trait contracts and shared types (no implementation)
  internal/      # domain modules (audio, camera, channel, deck, mixer, renderer, etc.)
  app/           # application layer (VardaApp: wires domain modules together, implements engine traits)
  usecases/      # delivery layer (UI panels, action handlers, HTTP API routes)
  main.rs        # thin orchestrator: parse CLI, init logger, run UI
```

The **engine layer** (`src/engine/`) defines trait contracts (`MixerCommands`, `MixerQueries`, `OutputCommands`, etc.) using only primitives and engine-defined types. No wgpu, egui, or framework types leak through.

The **internal layer** (`src/internal/`) contains domain modules that each own one concern: audio analysis, video decoding, ISF shader compilation, NDI FFI, SRT subprocess management, the modulation engine, etc. Each module is independently testable.

The **app layer** (`src/app/`, `VardaApp`) is the concrete implementation. It owns all subsystems and implements the engine traits. It can run headless without any window or UI.

The **usecases layer** (`src/usecases/`) is the only place that touches egui or HTTP routing, and owns all *main-window* presentation (blit pipeline, texture registration, `UIData` construction). It reads engine state snapshots and emits action structs. The UI never mutates engine state directly — commands flow through the app layer via the engine traits. Two documented exceptions touch `winit` (not egui) directly in `app/`: output windows (`app/outputs.rs`, `app/render.rs` — see `clean-architecture.md` Decision #10, engine-owned so they exist even in headless/API-driven setups) and the HTML-deck interactive window (`app/interactive/` — see `/spec/html-source.md` §4).

This separation means the same engine can be driven from the GUI, the HTTP API, or a test harness without changing engine code. When adding a feature, think about whether it needs updates in *all* delivery paths (UI panel, HTTP route, MIDI/OSC mapping) or just one.

External I/O (NDI, SRT, HLS/DASH, RTMP, and recording) uses a non-blocking subprocess architecture with bounded channels to keep the render thread fast. GPU work is batched into minimal command buffer submissions. The render pass culls zero opacity decks and channels so you only pay for whats live with one exception: the currently selected channel is force rendered even when off-air, so you can cue and build a composition on it and watch its preview update live without it touching the output.

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

Modulation uses a colon-separated key scheme (`deck_<uuid>:<param>`, `fx_<uuid>:<param>`) so the modulation engine can route LFOs, envelopes, and audio reactive sources to any parameter in the graph without coupling to positional indices.

When adding a new entity type or parameter, follow this scheme rather than inventing a new addressing convention. MIDI learn, OSC, modulation routing, and the HTTP API all key off of it.

## Engineering Practices

These are the practices we hold changes to. They apply whether you're fixing a bug or adding a feature.

- **Match existing layering.** New subsystems belong in `src/internal/` as an independently-testable domain module, wired together in `src/app/`, and exposed through `src/usecases/` (UI panel and/or HTTP route). Don't reach across layers — the UI and API should never mutate engine state directly, only emit actions/commands.
- **Consider every delivery path.** A new feature usually needs a UI panel, an HTTP route, and MIDI/OSC/keyboard mappability. Plan for all of them, not just the one you're testing against.
- **Test-driven, and tests stay green.** Write tests alongside new code, and update existing tests when you change the code they cover. Run `cargo test --lib` and `cargo test --test ui_integration` before opening a PR — see [`tests/`](tests/) for the full suite (some integration tests require a GPU adapter and are gated separately in CI).
- **No dead code.** Remove old code paths when you replace them; don't leave unused branches or backwards-compatibility shims around "just in case." Refactor in place rather than duplicating a module under a new name.
- **`.varda/` compatibility.** If a change alters the shape of `scene.json`, `stage.json`, or anything else persisted under `.varda/`, call it out explicitly in your PR description. Prefer backwards-compatible migrations, but don't contort the design to preserve compatibility with old files — flag the break and describe the migration path instead.
- **Use `log`, not `println!`.** Use the existing `log`/`env_logger` macros (`log::info!`, `log::warn!`, `log::error!`, etc.) so log output stays consistent and filterable; don't add ad hoc `println!`/`eprintln!` debugging output to committed code.
- **Zero warnings.** `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check` must be clean. CI enforces both (see `.github/workflows/clippy.yml` and `fmt.yml`) run them locally before pushing.
- **Performance-sensitive changes get benchmarked.** If your change touches rendering, compositing, GPU pipelines, audio processing, or another hot path, use the criterion harness in [`benches/`](benches/) — see [Benchmarking](#benchmarking) below for the suites and the before/after baseline workflow. Don't eyeball performance against a stale run; save a baseline before your change and compare after.

## Benchmarking

Criterion harness for the compositing pipeline and per-frame shader parameter buffer build. Ensures perf changes land with quantitative evidence.

### Quick Start

```sh
cargo bench --bench compositing      # GPU suites; needs an adapter, headless ok
cargo bench --bench shader_params    # CPU suite

./scripts/bench-smoke.sh             # --test on both, no sampling (CI/pre-commit)
```

### Before/After Comparison

```sh
cargo bench --bench compositing -- --save-baseline pre
# ... make your perf change ...
cargo bench --bench compositing -- --baseline pre
```

### GPU Suites (`benches/compositing.rs`)

| Benchmark | What it measures |
|---|---|
| `channel_composite_solid` | Solid-color decks (LoadOp::Clear, no fragment shader). Slope across deck counts isolates per-deck copy-on-composite cost. |
| `channel_composite_shader` | Same shape with `bars.fs` on every pixel. Difference vs solid at N decks ≈ N × per-deck shader execution cost. |
| `mixer_crossfade` | Two channels through the crossfader at 50%. |

A 60fps preflight panics if 8-deck solid composite at 1080p exceeds the 16.67ms frame budget. Disable with `VARDA_BENCH_SKIP_SLO=1`. After the criterion groups, a per-deck slope (decks/8 − decks/1, ÷ 7) is computed and printed.

### CPU Suite (`benches/shader_params.rs`)

| Variant | What it measures |
|---|---|
| `no_mod` | std140 byte buffer serialization only |
| `empty_mod` | Modulation engine present but no assignments — isolates per-param key construction cost |
| `active_lfo` | Full modulation path: lookup, LFO read, clamp, write |

The `empty_mod − no_mod` gap is the per-param allocation cost paid even when nothing is modulated. Multiply by params × decks × effects to estimate the per-frame floor.

### Notes

Criterion HTML reports land in `target/criterion/`.

`compositing` runs at 1920×1080 and calls `device.poll(Wait)` after each iteration so wall-clock reflects GPU work. Without a GPU adapter it prints `no GPU adapter — skipping` and exits clean. Numbers are machine-local; close other GPU work and warm the machine for stability.

## Pull Requests

1. Fork the repo and create a branch off `main`.
2. Keep PRs focused to one feature or fix. Its easier to review than a bundle of unrelated changes.
3. Prefix your PR title with FEAT for features, FIX for fixes, PERF for performance improvements, and DEBT for technical debt cleanup.
4. Make sure `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, and the test suites above all pass locally; CI re-runs all of them on `src/**`, `tests/**`, and `Cargo.toml`/`Cargo.lock` changes.
5. Describe what changed and why, and call out any `.varda/` compatibility impact or benchmark results if applicable.
6. A maintainer will review and merge.

## Filing Issues

Bug reports and feature requests are welcome on the [Issues page](https://github.com/im-knots/varda/issues). For bugs, include your OS, GPU, and steps to reproduce; a minimal `.varda/` workspace that reproduces the issue is even better.

## License

By contributing, you agree that your contributions will be licensed under the project's [MIT License](LICENSE).
