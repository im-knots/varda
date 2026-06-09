use clap::Parser;

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("naga", log::LevelFilter::Error)
        .filter_module("egui_wgpu", log::LevelFilter::Error)
        .filter_module("ort", log::LevelFilter::Error)
        .init();

    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("PANIC: {}", info);
        default_hook(info);
    }));

    // Initialize ONNX Runtime via load-dynamic before any ort usage.
    // Resolve libonnxruntime.dylib from the app bundle's Frameworks dir,
    // falling back to the executable's directory.
    init_ort_runtime();

    let config = varda::app::AppConfig::parse();

    // First-launch CLI install (non-blocking, non-fatal)
    varda::cli_install::ensure_cli_installed();

    log::info!("Varda VJ Software - Starting up...");
    if config.headless {
        log::info!("Headless mode enabled (API port {})", config.api_port);
    }

    varda::usecases::ui::runner::UIRunner::new(config).run()
}

/// Try to locate and load the ONNX Runtime dynamic library.
///
/// Search order:
/// 1. `<exe_dir>/../Frameworks/libonnxruntime.dylib` (macOS .app bundle)
/// 2. `<exe_dir>/libonnxruntime.{dylib,so,dll}` (dev / non-bundled)
/// 3. Fall through — ort will try `ORT_DYLIB_PATH` env or system search paths.
///
/// Failure is non-fatal: face detection simply won't be available.
fn init_ort_runtime() {
    let exe_dir = match std::env::current_exe().and_then(|p| {
        p.parent()
            .map(|d| d.to_path_buf())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no parent"))
    }) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("Could not resolve executable directory for ort dylib: {e}");
            return;
        }
    };

    // macOS .app bundle: Contents/MacOS/../Frameworks/
    let candidates = [
        exe_dir.join("../Frameworks/libonnxruntime.dylib"),
        exe_dir.join("libonnxruntime.dylib"),
        exe_dir.join("libonnxruntime.so"),
        exe_dir.join("onnxruntime.dll"),
    ];

    for candidate in &candidates {
        if candidate.exists() {
            match ort::init_from(candidate) {
                Ok(builder) => {
                    builder.commit();
                    log::info!("ONNX Runtime loaded from {}", candidate.display());
                    return;
                }
                Err(e) => {
                    log::warn!(
                        "Failed to load ONNX Runtime from {}: {e}",
                        candidate.display()
                    );
                }
            }
        }
    }

    log::info!(
        "ONNX Runtime dylib not found in app bundle; \
         face detection will be unavailable unless ORT_DYLIB_PATH is set"
    );
}
