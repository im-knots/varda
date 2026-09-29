//! Spout: Windows inter-application GPU texture sharing.
//!
//! The wire protocol is pure and builds on every platform; the D3D backend is
//! Windows only.

/// What this machine can do with Spout's sharing primitives, measured.
///
/// Windows only. Used by the test suite, not the product.
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

pub use manager::{SpoutManager, SpoutSource, provider, sink_provider};
