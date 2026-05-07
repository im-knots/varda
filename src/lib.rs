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
