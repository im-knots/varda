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
/// The `D3D11On12` bridge Spout's textures cross. Windows only.
#[cfg(target_os = "windows")]
pub mod d3d;
pub mod manager;
pub mod protocol;
/// Spout's sender registry in named shared memory. Windows only, and needs no
/// GPU, so CI covers it.
#[cfg(target_os = "windows")]
pub mod sharedmem;

pub use manager::{SpoutManager, SpoutSource};
