//! NDI SDK dynamic loader.
//!
//! Wraps `libloading::Library` + function pointers for the NDI SDK.
//! All NDI functions are accessed through the loaded SDK struct.
//! If the SDK is not installed, `NdiSdk::load()` returns `None` and
//! all NDI features gracefully degrade.

use super::ffi::*;
use libloading::{Library, Symbol};
use std::os::raw::c_uint;

/// Loaded NDI SDK with resolved function pointers.
pub struct NdiSdk {
    #[allow(dead_code)]
    lib: Library,

    // Core lifecycle
    pub initialize: unsafe extern "C" fn() -> bool,
    pub destroy: unsafe extern "C" fn(),

    // Find (discovery)
    pub find_create_v2: unsafe extern "C" fn(*const NDIlib_find_create_t) -> NDIlib_find_instance_t,
    pub find_destroy: unsafe extern "C" fn(NDIlib_find_instance_t),
    pub find_wait_for_sources: unsafe extern "C" fn(NDIlib_find_instance_t, c_uint) -> bool,
    pub find_get_current_sources:
        unsafe extern "C" fn(NDIlib_find_instance_t, *mut c_uint) -> *const NDIlib_source_t,

    // Receive
    pub recv_create_v3:
        unsafe extern "C" fn(*const NDIlib_recv_create_v3_t) -> NDIlib_recv_instance_t,
    pub recv_destroy: unsafe extern "C" fn(NDIlib_recv_instance_t),
    /// recv_capture_v3(instance, video_out, audio_out, metadata_out, timeout_ms) -> frame_type
    /// audio_out and metadata_out are opaque pointers (pass null to ignore).
    pub recv_capture_v3: unsafe extern "C" fn(
        NDIlib_recv_instance_t,
        *mut NDIlib_video_frame_v2_t,
        *mut std::ffi::c_void,
        *mut std::ffi::c_void,
        c_uint,
    ) -> NDIlib_frame_type_e,
    pub recv_free_video_v2:
        unsafe extern "C" fn(NDIlib_recv_instance_t, *const NDIlib_video_frame_v2_t),

    // Send
    pub send_create: unsafe extern "C" fn(*const NDIlib_send_create_t) -> NDIlib_send_instance_t,
    pub send_destroy: unsafe extern "C" fn(NDIlib_send_instance_t),
    pub send_send_video_v2:
        unsafe extern "C" fn(NDIlib_send_instance_t, *const NDIlib_video_frame_v2_t),
}

impl NdiSdk {
    /// Try to load the NDI SDK from known platform paths.
    /// Returns `None` if the SDK is not installed.
    pub fn load() -> Option<Self> {
        let lib = Self::try_load_library()?;
        unsafe { Self::resolve_symbols(lib) }
    }

    fn try_load_library() -> Option<Library> {
        // Check app bundle Frameworks directory first (bundled NDI)
        if let Some(lib) = Self::try_load_from_bundle() {
            return Some(lib);
        }

        let paths: &[&str] = if cfg!(target_os = "macos") {
            &[
                "/Library/NDI SDK for Apple/lib/macOS/libndi.dylib",
                "/usr/local/lib/libndi.dylib",
            ]
        } else if cfg!(target_os = "linux") {
            &[
                "libndi.so",
                "/usr/lib/libndi.so",
                "/usr/local/lib/libndi.so",
                "/usr/lib/x86_64-linux-gnu/libndi.so",
            ]
        } else if cfg!(target_os = "windows") {
            &["Processing.NDI.Lib.x64.dll"]
        } else {
            &[]
        };

        for path in paths {
            if let Ok(lib) = unsafe { Library::new(*path) } {
                log::info!("Loaded NDI SDK from: {}", path);
                return Some(lib);
            }
        }
        None
    }

    /// Try to load NDI from the app bundle's Frameworks directory.
    /// Path: <exe>/../Frameworks/libndi.dylib (macOS .app bundle)
    fn try_load_from_bundle() -> Option<Library> {
        let exe = std::env::current_exe().ok()?;
        let frameworks = exe.parent()?.parent()?.join("Frameworks");
        let ndi_path = if cfg!(target_os = "macos") {
            frameworks.join("libndi.dylib")
        } else {
            return None;
        };
        if !ndi_path.exists() {
            return None;
        }
        match unsafe { Library::new(&ndi_path) } {
            Ok(lib) => {
                log::info!("Loaded NDI SDK from bundle: {}", ndi_path.display());
                Some(lib)
            }
            Err(e) => {
                log::warn!("Failed to load bundled NDI SDK: {}", e);
                None
            }
        }
    }

    unsafe fn resolve_symbols(lib: Library) -> Option<Self> {
        macro_rules! load_fn {
            ($lib:expr, $name:expr, $ty:ty) => {{
                let sym: Symbol<$ty> = match $lib.get($name) {
                    Ok(s) => s,
                    Err(e) => {
                        log::warn!(
                            "NDI SDK missing symbol {}: {}",
                            String::from_utf8_lossy($name),
                            e
                        );
                        return None;
                    }
                };
                *sym.into_raw()
            }};
        }

        type FnInit = unsafe extern "C" fn() -> bool;
        type FnDestroy = unsafe extern "C" fn();
        type FnFindCreate =
            unsafe extern "C" fn(*const NDIlib_find_create_t) -> NDIlib_find_instance_t;
        type FnFindDestroy = unsafe extern "C" fn(NDIlib_find_instance_t);
        type FnFindWait = unsafe extern "C" fn(NDIlib_find_instance_t, c_uint) -> bool;
        type FnFindSources =
            unsafe extern "C" fn(NDIlib_find_instance_t, *mut c_uint) -> *const NDIlib_source_t;
        type FnRecvCreate =
            unsafe extern "C" fn(*const NDIlib_recv_create_v3_t) -> NDIlib_recv_instance_t;
        type FnRecvDestroy = unsafe extern "C" fn(NDIlib_recv_instance_t);
        type FnRecvCapture = unsafe extern "C" fn(
            NDIlib_recv_instance_t,
            *mut NDIlib_video_frame_v2_t,
            *mut std::ffi::c_void,
            *mut std::ffi::c_void,
            c_uint,
        ) -> NDIlib_frame_type_e;
        type FnRecvFree =
            unsafe extern "C" fn(NDIlib_recv_instance_t, *const NDIlib_video_frame_v2_t);
        type FnSendCreate =
            unsafe extern "C" fn(*const NDIlib_send_create_t) -> NDIlib_send_instance_t;
        type FnSendDestroy = unsafe extern "C" fn(NDIlib_send_instance_t);
        type FnSendVideo =
            unsafe extern "C" fn(NDIlib_send_instance_t, *const NDIlib_video_frame_v2_t);

        Some(Self {
            initialize: load_fn!(lib, b"NDIlib_initialize\0", FnInit),
            destroy: load_fn!(lib, b"NDIlib_destroy\0", FnDestroy),
            find_create_v2: load_fn!(lib, b"NDIlib_find_create_v2\0", FnFindCreate),
            find_destroy: load_fn!(lib, b"NDIlib_find_destroy\0", FnFindDestroy),
            find_wait_for_sources: load_fn!(lib, b"NDIlib_find_wait_for_sources\0", FnFindWait),
            find_get_current_sources: load_fn!(
                lib,
                b"NDIlib_find_get_current_sources\0",
                FnFindSources
            ),
            recv_create_v3: load_fn!(lib, b"NDIlib_recv_create_v3\0", FnRecvCreate),
            recv_destroy: load_fn!(lib, b"NDIlib_recv_destroy\0", FnRecvDestroy),
            recv_capture_v3: load_fn!(lib, b"NDIlib_recv_capture_v3\0", FnRecvCapture),
            recv_free_video_v2: load_fn!(lib, b"NDIlib_recv_free_video_v2\0", FnRecvFree),
            send_create: load_fn!(lib, b"NDIlib_send_create\0", FnSendCreate),
            send_destroy: load_fn!(lib, b"NDIlib_send_destroy\0", FnSendDestroy),
            send_send_video_v2: load_fn!(lib, b"NDIlib_send_send_video_v2\0", FnSendVideo),
            lib,
        })
    }
}
