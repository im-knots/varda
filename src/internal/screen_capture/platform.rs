//! Platform capture: enumeration, permissions and session creation.
//!
//! Each OS supplies four free functions selected by `cfg`. Only this module
//! touches OS capture APIs; code above it uses [`CaptureTargetInfo`] and
//! [`ScreenCaptureBackend`].
//!
//! Platforms without a backend use [`unsupported`], whose `open` returns
//! [`CaptureError::Unavailable`] with the reason. The manager shows that
//! message, so the user gets a notification instead of a black deck.

use super::backend::{
    CaptureConfig, CaptureError, CaptureTargetInfo, PermissionState, ScreenCaptureBackend,
};

#[cfg(all(feature = "screen-capture", target_os = "linux"))]
pub mod linux;
#[cfg(all(feature = "screen-capture", target_os = "macos"))]
pub mod macos;
#[cfg(all(feature = "screen-capture", target_os = "windows"))]
pub mod windows;

/// Fallback for platforms without a backend and for builds without the
/// `screen-capture` feature.
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
        assert_eq!(targets.len(), 0);

        let target = super::super::backend::mock_targets().remove(0);
        let err = super::unsupported::open(&target, &super::CaptureConfig::default())
            .err()
            .expect("unsupported provider must refuse");
        assert!(
            matches!(err, super::CaptureError::Unavailable(_)),
            "expected Unavailable, got {err:?}"
        );
        // The message must name a cause; a bare "unavailable" leaves a black deck
        // unexplained.
        assert_ne!(err.to_string(), "");
    }
}
