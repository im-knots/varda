//! Helpers shared by the GPU integration test binaries, included with
//! `mod common;`.

use varda::renderer::context::GpuContext;

/// Open a headless GPU context, or `None` without a usable adapter. See
/// [`varda::testing::headless_gpu`].
///
/// # Panics
///
/// Panics if no context can be created while `VARDA_REQUIRE_GPU` is set.
#[allow(dead_code, reason = "each test binary uses one of the two")]
pub fn headless_gpu() -> Option<GpuContext> {
    varda::testing::headless_gpu()
}

/// Open a headless GPU context on a hardware adapter, or `None` on a CPU
/// rasterizer. See [`varda::testing::hardware_gpu`].
///
/// # Panics
///
/// Panics if no context can be created while `VARDA_REQUIRE_GPU` is set.
#[allow(dead_code, reason = "each test binary uses one of the two")]
pub fn hardware_gpu() -> Option<GpuContext> {
    varda::testing::hardware_gpu()
}
