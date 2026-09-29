//! Render/output configuration value types, defined in `engine::value::render`
//! and re-exported so `crate::renderer::config::…` paths resolve.

pub use crate::engine::value::render::{
    AlphaMode, CalibrationMode, EdgeBlendConfig, EdgeBlendEdge, EdgeBlendMode, ModeAvailability,
    OutputRotation, OutputSource, PresentationCapabilities, PresentationColorProfile,
    PresentationDepth, PresentationFormat, PresentationMode, PresentationPixelFormat,
    PresentationRequest, PresentationResolveError, PresentationTransfer, RecordingCodec,
    ResolvedPresentation, RtmpCodecContract, SrtCodec, StreamingCodec, TonemapMode,
};
