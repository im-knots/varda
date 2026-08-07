//! Linux screen/window capture — XDG Desktop Portal + `PipeWire` on Wayland,
//! X11 otherwise.
//!
//! Unlike macOS and Windows, "Linux" is two unrelated capture stacks with
//! opposite security models, so the provider dispatches on the session type at
//! runtime rather than at compile time. One binary has to serve both: distro
//! packages are built once, and a user can log out of Wayland into an X11
//! session without changing the executable.
//!
//! - **Wayland.** The compositor never lets a client read the screen. Capture
//!   goes through `org.freedesktop.portal.ScreenCast`, which shows the
//!   compositor's own picker and hands back a `PipeWire` node. Varda therefore
//!   cannot enumerate targets: see [`wayland::enumerate`] for what the library
//!   panel shows instead.
//! - **X11.** No access control at all, and no event-driven capture API, so the
//!   backend polls `GetImage` (over MIT-SHM where available) at
//!   `CaptureConfig.rate`.
//!
//! XWayland deliberately resolves to the Wayland branch. A Varda running under
//! XWayland has `DISPLAY` set and X11 would appear to work, but it would only
//! ever see other XWayland clients and the root window would come back blank —
//! a plausible-looking black capture is worse than the portal dialog.
//!
//! See spec/screen-capture.md § Platform Support.

pub mod wayland;
pub mod x11;

use crate::screen_capture::backend::{
    CaptureConfig, CaptureError, CaptureTargetInfo, PermissionState, ScreenCaptureBackend,
};

/// Which display server this process is talking to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Session {
    Wayland,
    X11,
}

/// Detect the session type from the environment.
///
/// `WAYLAND_DISPLAY` is the load-bearing signal — it is set by the compositor
/// for any client that can speak Wayland, including one that also has `DISPLAY`
/// from XWayland. `XDG_SESSION_TYPE` is consulted as a fallback because a few
/// session managers set it without exporting `WAYLAND_DISPLAY`.
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
        // The portal prompts per session and the answer is not queryable ahead
        // of time, so there is no state for the UI to render. `NotRequired`
        // keeps the library panel free of a permission banner that could never
        // resolve; the compositor's dialog is the real gate.
        Session::Wayland | Session::X11 => PermissionState::NotRequired,
    }
}

pub fn request_permission() {}

/// Enumerate capturable targets for the active session.
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

/// Open a capture session for `target`.
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

    /// The env vars this reads are process-global, so the cases share one test
    /// rather than racing each other under the test harness's thread pool.
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

            // XWayland: both are set. Wayland must win, or the capture silently
            // returns a blank root window.
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
        // X11 has no gate; Wayland's gate is the portal dialog, which is raised
        // at `open` and cannot be queried in advance.
        assert!(permission_state().allows_capture());
    }
}
