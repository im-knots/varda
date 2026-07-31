//! Helpers shared by the GPU-backed integration test binaries.
//!
//! Included with `mod common;` — Cargo does not treat `tests/common/` as a test
//! target of its own, so this compiles into each including binary.

use varda::renderer::context::GpuContext;

/// Open a headless GPU context, or `None` when this machine has no usable
/// adapter.
///
/// Every GPU test skips on `None`, which makes an absent adapter
/// indistinguishable from a passing suite. CI installs a software rasterizer
/// (lavapipe) and sets `VARDA_REQUIRE_GPU=1`, turning that skip into a hard
/// failure — without it a broken driver reports green having tested nothing.
///
/// # Panics
///
/// Panics if no context can be created while `VARDA_REQUIRE_GPU` is set.
pub fn headless_gpu() -> Option<GpuContext> {
    match GpuContext::new_headless() {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            assert!(
                std::env::var_os("VARDA_REQUIRE_GPU").is_none(),
                "VARDA_REQUIRE_GPU is set but no headless GPU context is available: {e:#}"
            );
            eprintln!("no GPU adapter — skipping");
            None
        }
    }
}
