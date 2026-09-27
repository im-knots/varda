# Contributing to Varda

Thanks for your interest in contributing to Varda. This document covers the architecture, conventions, and workflow you need to get a change merged. For end-user documentation (install, usage, panels, shaders), see the [manual](docs/README.md).

First, join #varda on the [Libera.Chat](https://web.libera.chat/#varda) IRC Network and say hello! Open an issue in this repo with your bug/suggestion/request. Or see below on how to open your own PR. 

## AI Usage Policy

Listen its basically just me at this point. I honestly dont care as long as the quality of output is "good" and you can explain in your own words in your PR descriptions: what you are trying to do, why, and how you did it. Also you know... the linux foundation's guidence of dont use AI tools with ToS that say "we own everything you make :)". Good engineering gets praise and merged. Bad engineering gets your PR closed. 

## Getting Set Up

Varda is written in Rust. Follow [Build from Source](docs/01-getting-started.md#build-from-source) in the manual to install system dependencies and get `cargo build --release` running, then:

```bash
cargo run --release          # launch the app
cargo test --lib             # unit tests
cargo test --test ui_integration   # GPU-free integration tests
```

## Architecture

The code has four layers:

```
src/
  engine/        # contracts: EngineCommand, EngineState snapshots, engine::value types
  internal/      # domain modules (audio, camera, channel, deck, mixer, renderer, etc.)
  app/           # VardaApp: owns the domain modules, dispatches commands, runs the frame
  usecases/      # UI panels, action handlers, HTTP API routes
  main.rs        # parses the CLI, starts the logger, runs the UI
```

- **`engine/`** defines what every consumer speaks: the `EngineCommand` vocabulary, the `EngineState` snapshots, and plain value types in `engine::value`. It uses no wgpu, egui or framework types.
- **`internal/`** holds domain modules, one concern each (audio analysis, video decoding, ISF compilation, NDI, the modulation engine). Each is tested on its own. The modules are ordered in tiers, and a module may only use modules below it. `tests/domain_dependency_guard.rs` holds the order. If a lower module needs something from a higher one, move the shared type down (often into `engine::value`) or pass the behavior in. Do not route it through `EngineCommand`, which is for consumers, not for domains. See `/spec/domain-dependencies.md`.
- **`app/`** is the engine. `VardaApp` owns every subsystem, dispatches every `EngineCommand`, and runs the frame (`begin_frame`, `render_frame`). Work on one domain goes on that domain (`Mixer`, `SurfaceManager`, `MacroBank`); `app/` holds work that spans several. It runs without a window.
- **`usecases/`** is the only layer that uses egui or HTTP routing. It reads snapshots and sends commands, and never mutates engine state. Two places in `app/` use `winit` directly: output windows (`app/outputs.rs`) and the HTML interactive window (`app/interactive/`). `tests/egui_layer_boundary_guard.rs` rejects egui types in `internal/` and `app/`.

The GUI, the HTTP API and the tests all drive the same engine. A new feature usually needs a UI panel, an HTTP route and MIDI/OSC mapping.

Network and file outputs (NDI, SRT, HLS/DASH, RTMP, recording) run in subprocesses or threads fed by bounded channels, so the render thread never waits on them. The renderer skips decks and channels at zero opacity, except the selected channel, which always renders so its preview works while it is off air.

### Addresses

Channels, decks, effects, surfaces and outputs get an 8-character hex UUID (such as `a3f1b20c`) when created. The UUID survives moves, reorders and save/restore, so MIDI mappings and modulation keep working after you rearrange things.

Parameters are addressed by a path starting at the UUID:

```
crossfader                              # mixer crossfader position

deck/<uuid>/opacity                     # deck opacity
deck/<uuid>/mute                        # deck mute toggle
deck/<uuid>/solo                        # deck solo toggle
deck/<uuid>/trigger                     # deck trigger (set opacity to 1)
deck/<uuid>/param/<name>                # generator shader param
deck/<uuid>/at/play_duration            # auto-transition play duration
deck/<uuid>/at/trans_duration           # auto-transition transition duration

ch/<uuid>/opacity                       # channel opacity

effect/<uuid>/param/<name>              # effect param, on any chain (deck, channel, master)

mod/<uuid>/<param_name>                 # modulation source param (frequency, amplitude, etc.)
mod/<uuid>/step/<step_idx>              # step sequencer step value
macro/<uuid>/value                      # macro control

surface/<uuid>/source                   # surface content: master, ch/<uuid>, chs/<uuid>,<uuid>, deck/<uuid>, domemaster
output/<uuid>/start | stop | active     # start, stop or toggle an output
output/<uuid>/calibration | rotation    # calibration card and rotation
output/<uuid>/surface/<surface_uuid>    # whether a surface is shown on the output
output/<uuid>/<name>                    # one of the output's sink settings
```

MIDI learn, OSC, modulation and the HTTP API all use these paths. Build them with `engine::value::param::ParamAddress` (`ParamAddress::deck_param`, `ParamAddress::effect_param`, ...), not `format!`. New entities and parameters use the same scheme.

### Adding a Deck Source

Every deck source (shader, clip, camera, NDI feed, web page) is a provider. A new one needs:

1. **A provider module next to its backend** in `src/internal/` (for example `internal/camera/provider.rs` or `internal/solid_color.rs`), implementing two traits from `crate::source`:
   - `DeckSourceProvider`, once per type: `id`, `label`, `icon`, the controls every deck of the type has (`params`), what the library panel offers (`library`), and `create` (or `loader` to build off the render thread). Optional: `tick`, `prepare`, `release`, `restore`.
   - `DeckSourceInstance`, once per deck: `render`, `config`, and `param`/`set_param`/`trigger`. Optional: `control`, `upload`, resizing.
2. **One line in `src/app/sources.rs`** registering it.

The library panel, the deck controls, the deck API routes, save and restore, undo, MIDI learn and modulation then work through the traits. `tests/deck_source_guard.rs` fails if a source type id appears anywhere else in `src/`.

- **Never rename the id.** It is the `type` tag in every `scene.json` (`{"type": "Video", "path": ...}`). Use CamelCase. A scene with an unknown type keeps it as a placeholder deck.
- **Declare controls as `ControlSpec`s** (`float`, `toggle`, `choice`, `color`, `text`, `action`, ...). `routed("my_source/thing")` makes a control addressable as `deck/<uuid>/my_source/thing`; `modulatable()` lets an LFO drive it. Numeric controls take `0.0` to `1.0`. Widget hints (`Transport`, `Orbit`, `CropRect`) group controls into richer widgets; a new hint is a GUI change.
- **Shared devices go through `Services`.** Register a device manager that other features use (cameras, depth sensors, NDI) in `source_services` in `app/sources.rs`, and reach it with `env.services.get_mut::<MyManager>()`.
- **Test the provider directly** by building a `SourceEnv` with an empty `Services` and `ShaderRegistry` and calling `create`, as `tests/render_correctness.rs` does. Get the GPU from `varda::testing::headless_gpu()` and an engine from `varda::testing::headless_app()`: they skip when there is no adapter, fail under `VARDA_REQUIRE_GPU=1` (as CI runs), and fail when the engine does not build.

### Adding an Output Type

Every output (window, display, recording, stream, NDI sender, Syphon or Spout server) is a sink provider from `crate::output`. A new one is a module next to its backend plus one line in `output_sinks` in `src/app/sources.rs`.

- `OutputSinkProvider`, once per type: `id`, `label`, `icon`, `params`, `default_config`, `availability`, an optional `library` (the display type lists monitors), and `create`.
- `OutputSinkInstance`, once per output: `config`, `frame_path` (`Present` for a window, `Gpu` for a shared texture, `Converted` for NDI, `Readback` for ffmpeg), `configure` to resolve a presentation request, and the per-frame hooks for its path (`acquire`/`present`, `encode`, `publish`, `deliver`). Recordings and streams also implement `startable`, `start` and `stop`.

`Output` in `internal/output/compose.rs` composes surfaces, edge blend and rotation, so a sink only gets the finished texture. A sink that needs a window asks through `window_request` and gets it in `attach_window`. The id is the `type` tag in `stage.json` (snake_case, never renamed). Settings are `ControlSpec`s, addressable at `output/<uuid>/<name>`. `tests/deck_source_guard.rs` checks sink ids like source ids.

## Engineering Practices

- **Keep the layers.** New subsystems go in `src/internal/`, get wired in `src/app/`, and are exposed through `src/usecases/`. The UI and API only send commands.
- **Cover every delivery path.** Plan for the UI panel, the HTTP route and MIDI/OSC/keyboard mapping.
- **Write tests with the code**, and update tests when you change what they cover. Run `cargo test --lib` and `cargo test --test ui_integration` before opening a PR. Some integration tests need a GPU and run separately in CI; see [`tests/`](tests/).
- **Delete replaced code.** No unused branches or compatibility shims. Change a module in place instead of copying it under a new name.
- **Flag `.varda/` changes.** If a change alters `scene.json`, `stage.json` or anything else under `.varda/`, say so in the PR and describe the migration. Prefer migrating old files, but not at the cost of a worse design.
- **Log with `log`** (`log::info!`, `log::warn!`, `log::error!`), not `println!`.
- **No warnings.** `cargo clippy --all-targets -- -D warnings` (pedantic included, see [Lints](#lints)) and `cargo fmt --all -- --check` must pass. CI runs both.
- **Benchmark hot paths.** Changes to rendering, compositing, GPU pipelines or audio need a before/after run; see [Benchmarking](#benchmarking).

## Lints

Lint settings live in `[lints.clippy]` in `Cargo.toml`, so `cargo clippy` locally matches CI for the library, binary, tests, benches and examples. Do not put lint settings in CI flags or crate-root `#![allow]`s.

`clippy::pedantic` is on at `warn`, and CI runs with `-D warnings`, so a pedantic finding fails the PR:

```sh
cargo clippy --all-targets -- -D warnings
```

Clippy only checks code compiled for your OS. Code behind `#[cfg(target_os = "...")]` for another OS (the NDI loader, the CLI installer, shader search paths, Syphon restore) is skipped locally, so `clippy.yml` runs on Linux, macOS and Windows. If CI reports a lint you cannot reproduce, check for platform gating first. The Windows job uses the release feature set (`--no-default-features --features face-detection,html,screen-capture`) because `depth` has no vcpkg port.

Five pedantic lints are allowed crate-wide, each with a comment in `Cargo.toml`:

| Lint | Why it's off |
|---|---|
| `cast_precision_loss`, `cast_possible_truncation`, `cast_sign_loss` | Pixel, sample and vertex math converts between integers and floats everywhere, and `try_from` would add error handling to the render loop. |
| `must_use_candidate` | Fires on hundreds of snapshot accessors and catches no bugs. |
| `too_many_lines` | About 130 functions are over 100 lines. Splitting them is tracked in [`/spec/roadmap.md`](spec/roadmap.md) § DEBT: Function Decomposition. |

Add to this list only when a lint is wrong for the whole codebase. `clippy::float_cmp` is also allowed in tests (see `src/lib.rs`), where assertions compare against literals the test just set.

Anywhere else, put a narrow `#[allow(clippy::lint_name)]` on the item with a one-line reason. Accepted cases:

- `many_single_char_names` and `similar_names` in geometry and DSP math, where `x`, `y`, `u`, `v`, `t` and `minx`/`miny` are the clearest names.
- `float_cmp` in tests, as above.

Otherwise fix the finding. Answer `missing_errors_doc` and `missing_panics_doc` with `# Errors` and `# Panics` sections that name the real failure conditions.

## Benchmarking

Benchmarks use criterion and live in [`benches/`](benches/), one file per hot path (compositing, shader parameters, modulation, audio, outputs, snapshots and others). Each file's header says what it measures.

```sh
cargo bench --bench compositing      # GPU; needs an adapter, headless is fine
cargo bench --bench shader_params    # CPU
./scripts/bench-smoke.sh             # runs both once without sampling (CI, pre-commit)
```

Compare before and after a change in the same session:

```sh
cargo bench --bench compositing -- --save-baseline pre
# make the change
cargo bench --bench compositing -- --baseline pre
```

`compositing` waits for the GPU after each iteration, so its times include GPU work. GPU benches skip themselves without an adapter. `compositing` also fails if 8 solid decks at 1080p exceed a 60 fps frame; set `VARDA_BENCH_SKIP_SLO=1` to skip that check. Reports go to `target/criterion/`. Close other GPU work while measuring.

## Pull Requests

1. Fork the repo and create a branch off `main`.
2. Keep PRs focused to one feature or fix. Its easier to review than a bundle of unrelated changes.
3. Prefix your PR title with FEAT for features, FIX for fixes, PERF for performance improvements, and DEBT for technical debt cleanup.
4. Make sure `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings` (pedantic included — see [Lints](#lints)), and the test suites above all pass locally; CI re-runs all of them on `src/**`, `tests/**`, `benches/**`, `examples/**`, and `Cargo.toml`/`Cargo.lock` changes.
5. Describe what changed and why, and call out any `.varda/` compatibility impact or benchmark results if applicable.
6. A maintainer will review and merge.

## Filing Issues

Bug reports and feature requests are welcome on the [Issues page](https://github.com/im-knots/varda/issues). For bugs, include your OS, GPU, and steps to reproduce; a minimal `.varda/` workspace that reproduces the issue is even better.

## License

By contributing, you agree that your contributions will be licensed under the project's [MIT License](LICENSE).
