//! A deliberate reproducer for the Windows parallel-test crash, not a gate.
//!
//! The Windows lib suite dies within the first few seconds under the default
//! test parallelism, with `STATUS_HEAP_CORRUPTION` (0xc0000374) on one run and
//! `STATUS_ACCESS_VIOLATION` (0xc0000005) on another. Two rounds of bisecting
//! by module put the fault in neither half: splitting the suite into
//! `internal::renderer` and everything-else crashed *both* cells. So the fault
//! is not one bad test, it is something both halves do a great deal of.
//!
//! What both halves do is build GPU contexts. Roughly sixty test sites call
//! `GpuContext::new_headless`, spread across nearly every module, and each one
//! constructs its own `wgpu::Instance` with `Backends::all()`, enumerates
//! adapters, and creates a device. On the runner the adapter is WARP, a
//! software D3D12 implementation. `Backends::all()` on Windows also brings up
//! the Vulkan loader and a WGL context, and WGL in particular has never been
//! safe to initialise concurrently.
//!
//! These tests strip everything else away and do only that, from several
//! threads at once. They are `#[ignore]`d: they exist to be run deliberately by
//! `.github/workflows/windows-heap-bisect.yml`, and a reproducer that crashes
//! the process has no business running in the normal suite.
//!
//! Knobs, all optional:
//!   `VARDA_STRESS_THREADS`  threads (default 4, matching the runner's vCPUs)
//!   `VARDA_STRESS_ITERS`    contexts built per thread (default 8)
//!   `VARDA_STRESS_BACKENDS` all | primary | dx12 | vulkan | gl (default all)
//!   `VARDA_STRESS_STAGE`    instance | adapter | device (default device)
//!
//! `STAGE` is the sharp end. If `instance` alone crashes, the fault is in
//! backend loading and never reaches wgpu's device code; if only `device`
//! crashes, instance creation is innocent and WARP device churn is the cause.
//!
//! See spec/roadmap.md, DEBT: Windows Heap Corruption Under Parallel Tests.

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
/// iterations reached the end. A crash takes the whole process down and never
/// gets here, which is the point: the result is the exit code, not an assertion.
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

/// The production path, exactly as ~60 test sites call it, from several threads.
///
/// This is the shape the suite actually has. If it crashes, the crash is
/// explained and the fix is a shared context rather than one per test.
#[test]
#[ignore = "reproducer: builds many GPU contexts at once and may crash the process"]
fn parallel_headless_contexts() {
    stress("new_headless", |_t, _i| {
        varda::renderer::GpuContext::new_headless().is_ok()
    });
}

/// The same load, but with the backend set and the stage under our control, so
/// a crash can be attributed to one backend's initialisation or to one phase of
/// setup rather than to "GPU stuff".
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

// ── Round 4: shader compilation ─────────────────────────────────────

/// Serialises pipeline creation when `VARDA_STRESS_LOCK_COMPILE` is set, so a
/// crash and its absence can be compared with nothing else changed.
static COMPILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// An empty value counts as unset. CI fills these from a matrix, and on Windows
/// an empty variable is still present in the environment.
fn compile_lock_enabled() -> bool {
    std::env::var("VARDA_STRESS_LOCK_COMPILE").is_ok_and(|v| !v.is_empty())
}

/// A trivial pipeline, with `tag` woven into the source so no two iterations
/// compile identical text and nothing can be served from a cache.
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

    // The lock, if any, covers exactly this call. On DX12 this is where naga
    // emits HLSL and wgpu-hal calls into `d3dcompiler_47.dll`.
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
/// `None` means take the production path through `GpuContext::new_headless`,
/// which is `Dx12Compiler::Auto`: try `dxcompiler.dll`, fall back to FXC. Note
/// that `WGPU_DX12_COMPILER` cannot be used for this. It is only read by
/// `BackendOptions::from_env_or_default`, and `new_headless` calls
/// `BackendOptions::default`, so Varda ignores that variable entirely.
///
/// Neither explicit setting falls back: wgpu returns an instance error if the
/// named compiler will not load. That is what we want here, because a silent
/// fall back to FXC would report a green cell having tested nothing.
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
/// The instance is returned alongside the device because dropping it would take
/// the loaded compiler library with it.
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

/// Build a context **and compile a shader on it**, on several threads at once.
///
/// This is the difference between Round 3, which was green on every cell, and
/// what the real suite does. Round 3 built contexts and dropped them; it never
/// compiled anything. On DX12 `create_render_pipeline` is where wgpu-hal calls
/// `D3DCompile` in `d3dcompiler_47.dll`, and wgpu-hal holds no lock around it:
/// every `Instance` loads the library separately, but Windows reference counts
/// modules, so all of them are calling into one copy with one set of globals.
///
/// Set `VARDA_STRESS_LOCK_COMPILE=1` to serialise just the pipeline call. If
/// that turns a crashing run green, the compiler is named and the shape of the
/// fix is known.
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
