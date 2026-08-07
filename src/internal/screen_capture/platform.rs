//! Platform capture provider — enumeration, permissions, and session creation.
//!
//! Each supported OS supplies four free functions selected by `cfg`. They are
//! the only place OS capture APIs are touched; everything above this module
//! deals in [`CaptureTargetInfo`] / [`ScreenCaptureBackend`] and is portable.
//!
//! Platforms without a backend compile to [`unsupported`], whose `open` returns
//! [`CaptureError::Unavailable`] with a message naming the reason. The manager
//! surfaces that verbatim, so a missing backend degrades to a clear notification
//! rather than a silently black deck.
//!
//! See spec/screen-capture.md § Platform Support.

use super::backend::{
    CaptureConfig, CaptureError, CaptureTargetInfo, PermissionState, ScreenCaptureBackend,
};

#[cfg(all(feature = "screen-capture", target_os = "linux"))]
pub mod linux;
#[cfg(all(feature = "screen-capture", target_os = "macos"))]
pub mod macos;
#[cfg(all(feature = "screen-capture", target_os = "windows"))]
pub mod windows;

/// Fallback provider for platforms with no backend yet, and for builds with the
/// `screen-capture` feature disabled.
pub mod unsupported {
    use super::{
        CaptureConfig, CaptureError, CaptureTargetInfo, PermissionState, ScreenCaptureBackend,
    };

    pub fn backend_name() -> &'static str {
        "unsupported"
    }

    pub fn permission_state() -> PermissionState {
        PermissionState::NotRequired
    }

    pub fn request_permission() {}

    /// # Errors
    ///
    /// Never fails; reports an empty target list.
    pub fn enumerate() -> Result<Vec<CaptureTargetInfo>, CaptureError> {
        Ok(Vec::new())
    }

    /// # Errors
    ///
    /// Always returns [`CaptureError::Unavailable`].
    pub fn open(
        _target: &CaptureTargetInfo,
        _config: &CaptureConfig,
    ) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
        Err(CaptureError::Unavailable(reason().to_string()))
    }

    fn reason() -> &'static str {
        if cfg!(not(feature = "screen-capture")) {
            "built without the `screen-capture` cargo feature"
        } else {
            "no capture backend for this platform"
        }
    }
}

#[cfg(all(feature = "screen-capture", target_os = "linux"))]
pub use linux::{backend_name, enumerate, open, permission_state, request_permission};
#[cfg(all(feature = "screen-capture", target_os = "macos"))]
pub use macos::{backend_name, enumerate, open, permission_state, request_permission};
#[cfg(all(feature = "screen-capture", target_os = "windows"))]
pub use windows::{backend_name, enumerate, open, permission_state, request_permission};

#[cfg(not(all(
    feature = "screen-capture",
    any(target_os = "macos", target_os = "windows", target_os = "linux")
)))]
pub use unsupported::{backend_name, enumerate, open, permission_state, request_permission};

#[cfg(test)]
mod tests {
    #[test]
    fn unsupported_provider_reports_no_targets_and_refuses_to_open() {
        let targets = super::unsupported::enumerate().expect("enumerate never fails");
        assert!(targets.is_empty());

        let target = super::super::backend::mock_targets().remove(0);
        let err = super::unsupported::open(&target, &super::CaptureConfig::default())
            .err()
            .expect("unsupported provider must refuse");
        assert!(
            matches!(err, super::CaptureError::Unavailable(_)),
            "expected Unavailable, got {err:?}"
        );
        // The message must name a cause — a bare "unavailable" is what makes a
        // black deck unexplainable in the field.
        assert!(!err.to_string().is_empty());
    }
}
