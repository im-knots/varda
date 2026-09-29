// Tests compare floats against literals they just assigned, where exact equality is correct.
#![cfg_attr(test, allow(clippy::float_cmp))]

pub mod app;
pub mod engine;
mod internal;
#[cfg(any(test, feature = "test-fixtures"))]
pub mod testing;
pub mod usecases;

// Keeps `crate::audio`, `crate::deck`, etc. paths working.
pub use internal::*;

pub use channel::BlendMode;
pub use params::ShaderParams;
pub use source::ScalingMode;
