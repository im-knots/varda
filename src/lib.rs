// Test assertions compare floats against the exact literals the test itself just
// assigned (`assert_eq!(snap.speed, 2.0)`), where exact equality is the correct
// assertion and an epsilon would weaken it. `float_cmp` stays live in non-test
// code, where an exact float comparison usually is a bug.
#![cfg_attr(test, allow(clippy::float_cmp))]

pub mod app;
pub mod engine;
mod internal;
pub mod usecases;

// Re-export all internal domain modules at crate root so existing
// crate::audio, crate::deck, etc. paths continue to work unchanged.
pub use internal::*;

// Re-export commonly used types at crate root for convenience
pub use channel::BlendMode;
pub use deck::ScalingMode;
pub use params::ShaderParams;
