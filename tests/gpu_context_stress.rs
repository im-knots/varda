//! Reproducer for Windows crashes under parallel tests; not a gate.
//!
//! The Windows lib suite can die with `STATUS_HEAP_CORRUPTION` (0xc0000374) or
//! `STATUS_ACCESS_VIOLATION` (0xc0000005) under default test parallelism. These
//! tests do only GPU context, shader compile, or app construction from several
//! threads at once. On Windows, `Backends::all()` also brings up the Vulkan
//! loader and WGL, which is not safe to initialize concurrently.
//!
//! `#[ignore]`d; run by `.github/workflows/windows-heap-bisect.yml`.
//!
//! Knobs, all optional:
//!   `VARDA_STRESS_THREADS`  threads (default 4, matching the runner's vCPUs)
//!   `VARDA_STRESS_ITERS`    contexts built per thread (default 8)
//!   `VARDA_STRESS_BACKENDS` all | primary | dx12 | vulkan | gl (default all)
//!   `VARDA_STRESS_STAGE`    instance | adapter | device (default device)
//!
//! If `instance` alone crashes, the fault is in backend loading; if only
//! `device` crashes, it is WARP device creation.

use std::thread;

fn env_usize(key: &str, default: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn backends() -> wgpu::Backends {
    match std::env::var("VARDA_STRESS_BACKENDS")
        .unwrap_or_else(|_| "all".into())
        .to_lowercase()
        .as_str()
    {
        "primary" => wgpu::Backends::PRIMARY,
        "dx12" => wgpu::Backends::DX12,
        "vulkan" => wgpu::Backends::VULKAN,
        "gl" => wgpu::Backends::GL,
        _ => wgpu::Backends::all(),
    }
}

/// How far into GPU setup each iteration goes.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Instance,
    Adapter,
    Device,
}

fn stage() -> Stage {
    match std::env::var("VARDA_STRESS_STAGE")
        .unwrap_or_else(|_| "device".into())
        .to_lowercase()
        .as_str()
    {
        "instance" => Stage::Instance,
        "adapter" => Stage::Adapter,
        _ => Stage::Device,
    }
}

/// Run `body` on `threads` threads, `iters` times each, and report how many
/// iterations finished. A crash kills the process, so the exit code is the
/// result.
fn stress(label: &str, body: impl Fn(usize, usize) -> bool + Send + Sync + Clone + 'static) {
    let threads = env_usize("VARDA_STRESS_THREADS", 4);
    let iters = env_usize("VARDA_STRESS_ITERS", 8);
    println!("{label}: {threads} threads x {iters} iterations");

    let handles: Vec<_> = (0..threads)
        .map(|t| {
            let body = body.clone();
            thread::spawn(move || (0..iters).filter(|&i| body(t, i)).count())
        })
        .collect();

    let ok: usize = handles
        .into_iter()
        .map(|h| h.join().expect("stress thread panicked"))
        .sum();

    println!("{label}: {ok}/{} iterations succeeded", threads * iters);
    assert!(ok > 0, "{label}: every iteration failed to build a context");
}

/// The production path, `GpuContext::new_headless`, from several threads.
#[test]
#[ignore = "reproducer: builds many GPU contexts at once and may crash the process"]
fn parallel_headless_contexts() {
    stress("new_headless", |_t, _i| {
        varda::renderer::GpuContext::new_headless().is_ok()
    });
}

/// The same load with the backend set and the stage chosen, to attribute a
/// crash to one backend or one setup phase.
#[test]
#[ignore = "reproducer: builds many wgpu instances at once and may crash the process"]
fn parallel_instances() {
    let backends = backends();
    let stage = stage();
    println!(
        "backends={backends:?} stage={}",
        match stage {
            Stage::Instance => "instance",
            Stage::Adapter => "adapter",
            Stage::Device => "device",
        }
    );

    stress("instances", move |_t, _i| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            display: None,
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        });
        if stage == Stage::Instance {
            return true;
        }

        let Some(adapter) =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            }))
            .ok()
        else {
            return false;
        };
        if stage == Stage::Adapter {
            return true;
        }

        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("stress device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::default(),
            experimental_features: wgpu::ExperimentalFeatures::default(),
            trace: wgpu::Trace::default(),
        }))
        .is_ok()
    });
}

// ── Shader compilation ──────────────────────────────────────────────

/// Serializes pipeline creation when `VARDA_STRESS_LOCK_COMPILE` is set, for
/// A/B comparison.
static COMPILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An empty value counts as unset: CI fills these from a matrix, and on Windows
/// an empty variable is still present.
fn compile_lock_enabled() -> bool {
    std::env::var("VARDA_STRESS_LOCK_COMPILE").is_ok_and(|v| !v.is_empty())
}

/// A trivial pipeline with `tag` in the source so no compile hits a cache.
fn build_pipeline(device: &wgpu::Device, tag: usize) -> wgpu::RenderPipeline {
    let source = format!(
        r"
@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> @builtin(position) vec4<f32> {{
    let x = f32(i32(i) - 1) * {tag}.0 / {tag}.0;
    let y = f32(i32(i & 1u) * 2 - 1);
    return vec4<f32>(x, y, 0.0, 1.0);
}}

@fragment
fn fs_main() -> @location(0) vec4<f32> {{
    return vec4<f32>({tag}.0 / 255.0, 0.0, 0.0, 1.0);
}}
"
    );

    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("stress shader"),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    });

    // The lock, if any, covers only this call. On DX12 naga emits HLSL here and
    // wgpu-hal calls `d3dcompiler_47.dll`.
    let _guard = compile_lock_enabled().then(|| {
        COMPILE_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("stress pipeline"),
        layout: None,
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: wgpu::PipelineCompilationOptions::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Which DX12 shader compiler to force, from `VARDA_STRESS_COMPILER`.
///
/// `None` uses `GpuContext::new_headless`, which is `Dx12Compiler::Auto`:
/// `dxcompiler.dll`, falling back to FXC. `WGPU_DX12_COMPILER` has no effect,
/// because `new_headless` uses `BackendOptions::default`.
///
/// Explicit settings don't fall back: wgpu errors if the compiler won't load,
/// so a pass always means the named compiler was tested.
fn compiler_override() -> Option<wgpu::Dx12Compiler> {
    match std::env::var("VARDA_STRESS_COMPILER")
        .ok()
        .filter(|v| !v.is_empty())?
        .as_str()
    {
        "fxc" => Some(wgpu::Dx12Compiler::Fxc),
        "dxc" => Some(wgpu::Dx12Compiler::default_dynamic_dxc()),
        other => panic!("VARDA_STRESS_COMPILER must be fxc or dxc, got {other:?}"),
    }
}

/// A DX12 device using an explicitly chosen shader compiler.
///
/// Returns the instance too, because dropping it unloads the compiler library.
fn device_with_compiler(
    compiler: wgpu::Dx12Compiler,
) -> Option<(wgpu::Instance, wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::default(),
        backend_options: wgpu::BackendOptions {
            dx12: wgpu::Dx12BackendOptions {
                shader_compiler: compiler,
                ..Default::default()
            },
            ..Default::default()
        },
        display: None,
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
    });

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }))
    .ok()?;

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("stress device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: wgpu::MemoryHints::default(),
        experimental_features: wgpu::ExperimentalFeatures::default(),
        trace: wgpu::Trace::default(),
    }))
    .ok()?;

    Some((instance, device, queue))
}

/// Build a context and compile a shader on it, on several threads at once.
///
/// On DX12, `create_render_pipeline` calls `D3DCompile` in
/// `d3dcompiler_47.dll` with no lock. Each `Instance` loads the library, but
/// Windows reference-counts modules, so all share one copy and its globals.
///
/// Set `VARDA_STRESS_LOCK_COMPILE=1` to serialize only the pipeline call.
#[test]
#[ignore = "reproducer: compiles many shaders at once and may crash the process"]
fn parallel_pipelines() {
    let locked = compile_lock_enabled();
    println!("compile lock: {}", if locked { "on" } else { "off" });

    let forced = compiler_override();
    println!(
        "compiler: {}",
        match &forced {
            None => "auto (production path)",
            Some(wgpu::Dx12Compiler::Fxc) => "fxc",
            Some(_) => "dxc",
        }
    );

    stress("pipelines", move |t, i| {
        let tag = t * 97 + i + 1;
        match forced.clone() {
            None => {
                let Ok(gpu) = varda::renderer::GpuContext::new_headless() else {
                    return false;
                };
                let _pipeline = build_pipeline(&gpu.device, tag);
                gpu.device.poll(wgpu::PollType::Poll).is_ok()
            }
            Some(compiler) => {
                let Some((_instance, device, _queue)) = device_with_compiler(compiler) else {
                    return false;
                };
                let _pipeline = build_pipeline(&device, tag);
                device.poll(wgpu::PollType::Poll).is_ok()
            }
        }
    });
}

// ── Whole-app construction ──────────────────────────────────────────

/// Build a full headless `VardaApp` on several threads at once, as the
/// `headless_app()` fixture does.
///
/// `VARDA_STRESS_APP_STAGE` picks how much of startup to run:
///
///   `gpu`       GPU context only (control).
///   `audio`     `AudioManager::new`. Enumerates devices: cpal into WASAPI on
///               Windows, which means COM.
///   `camera`    `CameraManager::new`. Not feature gated. nokhwa on Windows is
///               Media Foundation: COM and an `MFStartup` refcount.
///   `screencap` `ScreenCaptureManager::new`. Windows Graphics Capture, which
///               calls `CoIncrementMTAUsage` to pin the process into the MTA.
///   `midi`      `MidiDeviceManager::new`, which enumerates MIDI ports under a
///               process-wide lock.
///   `midi_in`   midir's input enumeration directly, without that lock.
///   `midi_out`  midir's output enumeration directly, without that lock.
///   `app`       the whole `VardaApp::new`.
///
/// Three subsystems enter a COM apartment during startup without coordinating,
/// and mixed apartment models can cause this kind of crash.
#[test]
#[ignore = "reproducer: builds whole apps at once and may crash the process"]
fn parallel_app_construction() {
    let stage = std::env::var("VARDA_STRESS_APP_STAGE").unwrap_or_else(|_| "app".into());
    println!("app stage: {stage}");

    stress("app", move |_t, _i| match stage.as_str() {
        "gpu" => varda::renderer::GpuContext::new_headless().is_ok(),
        "audio" => {
            let mgr = varda::audio::AudioManager::new();
            // Read from it so construction can't be optimized out.
            let _ = mgr.devices().len();
            true
        }
        "camera" => {
            let _mgr = varda::camera::CameraManager::new();
            true
        }
        "screencap" => {
            let _mgr = varda::screen_capture::ScreenCaptureManager::new();
            true
        }
        "midi" => varda::midi::MidiDeviceManager::new().is_ok(),

        // These call midir directly, bypassing the lock in
        // `MidiDeviceManager::scan_devices`, to show which half of enumeration
        // faults. CI runners usually have no MIDI inputs but one output (GS
        // Wavetable Synth).
        "midi_in" => {
            let Ok(client) = midir::MidiInput::new("varda stress in") else {
                return false;
            };
            for port in &client.ports() {
                let _ = client.port_name(port);
            }
            true
        }
        "midi_out" => {
            let Ok(client) = midir::MidiOutput::new("varda stress out") else {
                return false;
            };
            for port in &client.ports() {
                let _ = client.port_name(port);
            }
            true
        }
        _ => {
            let Ok(gpu) = varda::renderer::GpuContext::new_headless() else {
                return false;
            };
            let config = varda::testing::headless_config();
            varda::app::VardaApp::new(gpu, &config).is_ok()
        }
    });
}
