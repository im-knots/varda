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
            // On macOS, verify the dylib matches the current process architecture
            // before calling dlopen. Loading an arm64 dylib from an x86_64 process
            // (or vice versa) can deadlock in Rosetta's translation layer.
            if !dylib_matches_current_arch(candidate) {
                log::info!(
                    "Skipping ONNX Runtime at {} (architecture mismatch)",
                    candidate.display()
                );
                continue;
            }
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

/// Check whether a Mach-O dylib contains a slice for the current process
/// architecture. Returns `true` on non-macOS or if the file cannot be read
/// (let dlopen decide). On macOS, reads the Mach-O magic to determine if
/// it's a fat binary (always ok) or a thin binary whose CPU type matches.
fn dylib_matches_current_arch(path: &std::path::Path) -> bool {
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        return true;
    }

    #[cfg(target_os = "macos")]
    {
        use std::io::Read;
        let mut f = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(_) => return true, // let dlopen report the error
        };
        let mut magic = [0u8; 4];
        if f.read_exact(&mut magic).is_err() {
            return true;
        }
        let magic_u32 = u32::from_be_bytes(magic);
        // FAT_MAGIC / FAT_CIGAM — universal binary, contains multiple arches
        if magic_u32 == 0xCAFE_BABE || magic_u32 == 0xBEBA_FECA {
            return true;
        }
        // Thin Mach-O: read CPU type (bytes 4..8)
        let mut cputype_bytes = [0u8; 4];
        if f.read_exact(&mut cputype_bytes).is_err() {
            return true;
        }
        // Determine endianness from magic
        let is_le = magic_u32 == 0xCEFA_EDFE || magic_u32 == 0xCFFA_EDFE;
        let cputype = if is_le {
            u32::from_le_bytes(cputype_bytes)
        } else {
            u32::from_be_bytes(cputype_bytes)
        };
        // CPU_TYPE_X86_64 = 0x01000007, CPU_TYPE_ARM64 = 0x0100000C
        let expected = if cfg!(target_arch = "x86_64") {
            0x0100_0007u32
        } else if cfg!(target_arch = "aarch64") {
            0x0100_000Cu32
        } else {
            return true; // unknown arch, let dlopen decide
        };
        cputype == expected
    }
}
