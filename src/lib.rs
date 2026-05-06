pub mod app;
pub mod audio;
pub mod camera;
pub mod channel;
pub mod clock;
pub mod deck;
pub mod engine;
pub mod isf;
pub mod midi;
pub mod mixer;
pub mod modulation;
pub mod notifications;
pub mod osc;
pub mod params;
pub mod persistence;
pub mod registry;
pub mod renderer;
pub mod scene;
pub mod surface;
pub mod usecases;
pub mod video;

// Re-export commonly used types at crate root for convenience
pub use channel::BlendMode;
pub use deck::ScalingMode;
pub use params::ShaderParams;
