//! Helpers shared by the GPU-backed integration test binaries.
//!
//! Included with `mod common;` — Cargo does not treat `tests/common/` as a test
//! target of its own, so this compiles into each including binary.

use varda::renderer::context::GpuContext;

/// Open a headless GPU context, or `None` when this machine has no usable
/// adapter. Fails instead under `VARDA_REQUIRE_GPU`; see
/// [`varda::testing::headless_gpu`].
///
/// # Panics
///
/// Panics if no context can be created while `VARDA_REQUIRE_GPU` is set.
pub fn headless_gpu() -> Option<GpuContext> {
    varda::testing::headless_gpu()
}
