//! Framework-free render/output configuration value types.
//!
//! Definitions moved to `engine::value::render` (see /spec/engine-value-types.md)
//! so the engine contract layer names them directly instead of reaching into
//! `internal::renderer`. Re-exported here so existing
//! `crate::renderer::config::…` paths keep working.

pub use crate::engine::value::render::{
    CalibrationMode, EdgeBlendConfig, EdgeBlendEdge, EdgeBlendMode, OutputRotation, OutputSource,
    OutputTarget, RecordingCodec, SrtCodec, StreamingCodec, TonemapMode,
};
