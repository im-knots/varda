//! Spout: Windows inter-application GPU texture sharing.
//!
//! See /spec/spout-output.md. Phase 58a is the wire protocol, which is pure and
//! builds on every platform so its tests run on the development machine. The D3D
//! backend and engine integration are 58b and 58c.

/// What this machine can do with Spout's sharing primitives, measured.
///
/// Windows only, and deliberately reachable from the test suite rather than from
/// the product: it answers the question of whether CI can cover Spout at all.
#[cfg(target_os = "windows")]
pub mod capability;
pub mod protocol;
