//! Framework-free render/output configuration value types.
//!
//! Definitions moved to `engine::value::render` (see /spec/engine-value-types.md)
//! so the engine contract layer names them directly instead of reaching into
//! `internal::renderer`. Re-exported here so existing
//! `crate::renderer::config::…` paths keep working.

pub use crate::engine::value::render::{
    AlphaMode, CalibrationMode, EdgeBlendConfig, EdgeBlendEdge, EdgeBlendMode, OutputRotation,
    OutputSource, OutputTarget, PresentationCapabilities, PresentationColorProfile,
    PresentationDepth, PresentationFormat, PresentationPixelFormat, PresentationRequest,
    PresentationResolveError, RecordingCodec, ResolvedPresentation, RtmpCodecContract, SrtCodec,
    StreamingCodec, TonemapMode,
};
