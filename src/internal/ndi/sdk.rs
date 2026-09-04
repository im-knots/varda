//! NDI SDK dynamic loader.
//!
//! Wraps `libloading::Library` + function pointers for the NDI SDK.
//! All NDI functions are accessed through the loaded SDK struct.
//! If the SDK is not installed, `NdiSdk::load()` returns `None` and
//! all NDI features gracefully degrade.

use super::ffi::{
    NDIlib_find_create_t, NDIlib_find_instance_t, NDIlib_frame_type_e, NDIlib_recv_create_v3_t,
    NDIlib_recv_instance_t, NDIlib_send_create_t, NDIlib_send_instance_t, NDIlib_source_t,
    NDIlib_video_frame_v2_t,
};
use libloading::{Library, Symbol};
use std::os::raw::{c_char, c_uint};

/// Evidence available from the dynamically loaded runtime for high-bit sending.
///
/// NDI 6 documents P216 submission through the same `send_send_video_v2`
/// symbol resolved below. The compatible v2 frame structure is compiled into
/// this adapter, so runtime generation is the remaining capability evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NdiSendCapability {
    runtime_major: Option<u32>,
    send_video_v2_resolved: bool,
}

impl NdiSendCapability {
    /// Build capability evidence from the runtime generation and sender symbol.
    pub fn from_runtime_evidence(version: Option<&str>, send_video_v2_resolved: bool) -> Self {
        Self {
            runtime_major: version.and_then(parse_runtime_major),
            send_video_v2_resolved,
        }
    }

    /// Whether the complete sender path has positively confirmed P216 support.
    pub const fn p216_confirmed(self) -> bool {
        matches!(self.runtime_major, Some(6..)) && self.send_video_v2_resolved
    }

    /// User-facing explanation for the honest UYVY fallback.
    pub fn p216_unavailable_reason(self) -> String {
        match self.runtime_major {
            None => "the NDI runtime version is unavailable, so P216 sending cannot be verified"
                .to_string(),
            Some(major) if major < 6 => {
                format!("the loaded NDI {major} runtime predates the NDI 6 high-bit contract")
            }
            Some(major) if !self.send_video_v2_resolved => {
                format!("the loaded NDI {major} runtime does not resolve NDIlib_send_send_video_v2")
            }
            Some(major) => format!("the loaded NDI {major} runtime supports P216"),
        }
    }
}

fn parse_runtime_major(version: &str) -> Option<u32> {
    version
        .split(|character: char| !character.is_ascii_digit() && character != '.')
        .filter(|token| token.contains('.'))
        .filter_map(|token| token.split('.').next())
        .find_map(|major| major.parse().ok())
}

/// Loaded NDI SDK with resolved function pointers.
pub struct NdiSdk {
    #[allow(dead_code)]
    lib: Library,
    /// Runtime identity returned by `NDIlib_version`.
    pub runtime_version: String,

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
    /// `recv_capture_v3(instance`, `video_out`, `audio_out`, `metadata_out`, `timeout_ms`) -> `frame_type`
    /// `audio_out` and `metadata_out` are opaque pointers (pass null to ignore).
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
                log::info!("Loaded NDI SDK from: {path}");
                return Some(lib);
            }
        }
        None
    }

    /// Try to load NDI from the app bundle or portable directory.
    /// macOS: <exe>/../../Frameworks/libndi.dylib (.app bundle)
    /// Windows: <`exe_dir>/Processing.NDI.Lib.x64.dll` (portable ZIP)
    fn try_load_from_bundle() -> Option<Library> {
        let exe = std::env::current_exe().ok()?;
        let exe_dir = exe.parent()?;

        #[cfg(target_os = "macos")]
        let ndi_path = exe_dir.parent()?.join("Frameworks").join("libndi.dylib");

        #[cfg(target_os = "windows")]
        let ndi_path = exe_dir.join("Processing.NDI.Lib.x64.dll");

        #[cfg(target_os = "linux")]
        let _ = exe_dir; // suppress unused on Linux
        #[cfg(target_os = "linux")]
        return None;

        #[cfg(any(target_os = "macos", target_os = "windows"))]
        {
            if !ndi_path.exists() {
                return None;
            }
            match unsafe { Library::new(&ndi_path) } {
                Ok(lib) => {
                    log::info!("Loaded NDI SDK from bundle: {}", ndi_path.display());
                    Some(lib)
                }
                Err(e) => {
                    log::warn!("Failed to load bundled NDI SDK: {e}");
                    None
                }
            }
        }
    }

    unsafe fn resolve_symbols(lib: Library) -> Option<Self> {
        macro_rules! load_fn {
            ($lib:expr, $name:expr, $ty:ty) => {{
                let sym: Symbol<$ty> = match unsafe { $lib.get($name) } {
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
                *unsafe { sym.into_raw() }
            }};
        }

        type FnInit = unsafe extern "C" fn() -> bool;
        type FnDestroy = unsafe extern "C" fn();
        type FnVersion = unsafe extern "C" fn() -> *const c_char;
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

        let version_fn = load_fn!(lib, b"NDIlib_version\0", FnVersion);
        let version_ptr = unsafe { version_fn() };
        let runtime_version = if version_ptr.is_null() {
            String::new()
        } else {
            unsafe { std::ffi::CStr::from_ptr(version_ptr) }
                .to_string_lossy()
                .into_owned()
        };

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
            runtime_version,
            lib,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{NdiSendCapability, parse_runtime_major};

    #[test]
    fn runtime_major_accepts_vendor_version_strings() {
        assert_eq!(parse_runtime_major("NDI SDK 6.3.1"), Some(6));
        assert_eq!(parse_runtime_major("6.0.0"), Some(6));
        assert_eq!(parse_runtime_major("NDI Runtime v5.6.0"), Some(5));
        assert_eq!(
            parse_runtime_major("NDI SDK APPLE 12:49:17 Apr 13 2026 6.3.2.0"),
            Some(6)
        );
        assert_eq!(parse_runtime_major("unknown"), None);
    }

    #[test]
    fn ndi_six_confirms_p216_submission_contract() {
        let capability = NdiSendCapability::from_runtime_evidence(Some("NDI SDK 6.3.1"), true);

        assert!(capability.p216_confirmed());
    }

    #[test]
    fn pre_ndi_six_runtime_reports_version_limitation() {
        let capability = NdiSendCapability::from_runtime_evidence(Some("NDI SDK 5.6.0"), true);

        assert!(!capability.p216_confirmed());
        assert!(capability.p216_unavailable_reason().contains("NDI 5"));
    }

    #[test]
    fn missing_runtime_reports_runtime_limitation() {
        let capability = NdiSendCapability::from_runtime_evidence(None, false);

        assert!(!capability.p216_confirmed());
        assert!(
            capability
                .p216_unavailable_reason()
                .contains("runtime version is unavailable")
        );
    }

    #[test]
    fn missing_sender_symbol_disables_p216() {
        let capability = NdiSendCapability::from_runtime_evidence(Some("NDI SDK 6.3.1"), false);

        assert!(!capability.p216_confirmed());
        assert!(
            capability
                .p216_unavailable_reason()
                .contains("NDIlib_send_send_video_v2")
        );
    }
}
