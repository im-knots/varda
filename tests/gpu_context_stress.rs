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
