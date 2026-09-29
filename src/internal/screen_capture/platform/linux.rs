//! Linux screen and window capture: XDG Desktop Portal plus `PipeWire` on
//! Wayland, X11 otherwise.
//!
//! The session type is detected at runtime so one binary serves both, since a
//! user can switch between Wayland and X11 sessions.
//!
//! - Wayland: clients cannot read the screen. Capture goes through
//!   `org.freedesktop.portal.ScreenCast`, which shows the compositor's picker
//!   and returns a `PipeWire` node, so Varda cannot enumerate targets; see
//!   [`wayland::enumerate`].
//! - X11: no access control and no event-driven capture, so the backend polls
//!   `GetImage` (over MIT-SHM where available) at `CaptureConfig.rate`.
//!
//! XWayland uses the Wayland path. Under XWayland `DISPLAY` is set, but X11
//! capture sees only other XWayland clients and the root window is blank.

pub mod wayland;
pub mod x11;

use crate::screen_capture::backend::{
    CaptureConfig, CaptureError, CaptureTargetInfo, PermissionState, ScreenCaptureBackend,
};

/// The display server this process uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Wayland,
    X11,
}

/// Detects the session type from the environment.
///
/// `WAYLAND_DISPLAY` decides: the compositor sets it for every Wayland client,
/// including one that also has `DISPLAY` from XWayland. `XDG_SESSION_TYPE` is
/// the fallback because some session managers set it without exporting
/// `WAYLAND_DISPLAY`.
pub fn session() -> Session {
    let wayland_display = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    if wayland_display || session_type.eq_ignore_ascii_case("wayland") {
        Session::Wayland
    } else {
        Session::X11
    }
}

pub fn backend_name() -> &'static str {
    match session() {
        Session::Wayland => wayland::BACKEND_NAME,
        Session::X11 => x11::BACKEND_NAME,
    }
}

pub fn permission_state() -> PermissionState {
    match session() {
        // The portal prompts per session and cannot be queried ahead of time.
        // `NotRequired` keeps an unresolvable permission banner out of the library
        // panel; the compositor's dialog is the real check.
        Session::Wayland | Session::X11 => PermissionState::NotRequired,
    }
}

pub fn request_permission() {}

/// Enumerates capture targets for the active session.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if the X11 display cannot be reached.
/// The Wayland branch never fails; it reports a single portal entry.
pub fn enumerate() -> Result<Vec<CaptureTargetInfo>, CaptureError> {
    match session() {
        Session::Wayland => wayland::enumerate(),
        Session::X11 => x11::enumerate(),
    }
}

/// Opens a capture session for `target`.
///
/// # Errors
///
/// Returns [`CaptureError::TargetNotFound`] if the display or window is gone,
/// [`CaptureError::PermissionDenied`] if the user dismissed the portal dialog,
/// or [`CaptureError::Backend`] for any other platform failure.
pub fn open(
    target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    match session() {
        Session::Wayland => wayland::open(target, config),
        Session::X11 => x11::open(target, config),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The env vars are process-global, so all cases share one test instead of
    /// racing in parallel tests.
    #[test]
    fn session_detection_prefers_wayland_and_treats_xwayland_as_wayland() {
        let saved = (
            std::env::var_os("WAYLAND_DISPLAY"),
            std::env::var_os("XDG_SESSION_TYPE"),
            std::env::var_os("DISPLAY"),
        );

        // SAFETY: single-threaded within this test; restored before returning.
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("XDG_SESSION_TYPE", "x11");
            std::env::set_var("DISPLAY", ":0");
            assert_eq!(session(), Session::X11);

            // XWayland sets both. Wayland must win, or capture returns a blank root
            // window.
            std::env::set_var("WAYLAND_DISPLAY", "wayland-0");
            assert_eq!(session(), Session::Wayland);

            // Session managers that set only XDG_SESSION_TYPE.
            std::env::remove_var("WAYLAND_DISPLAY");
            std::env::set_var("XDG_SESSION_TYPE", "wayland");
            assert_eq!(session(), Session::Wayland);

            // An empty WAYLAND_DISPLAY is not a Wayland session.
            std::env::set_var("XDG_SESSION_TYPE", "x11");
            std::env::set_var("WAYLAND_DISPLAY", "");
            assert_eq!(session(), Session::X11);

            for (key, value) in [
                ("WAYLAND_DISPLAY", saved.0),
                ("XDG_SESSION_TYPE", saved.1),
                ("DISPLAY", saved.2),
            ] {
                match value {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    #[test]
    fn neither_session_gates_capture_behind_a_permission_prompt() {
        // X11 has no check; Wayland's is the portal dialog, raised at `open`.
        assert!(permission_state().allows_capture());
    }
}
