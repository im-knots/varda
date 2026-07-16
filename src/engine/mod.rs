//! Engine layer — domain contracts (traits + types).
//!
//! This module defines the public API for the Varda engine.
//! NO implementation, NO GPU types. Pure contracts.
//!
//! Consumers (UI, HTTP API, CLI) program against these traits.
//! The concrete implementation lives in `src/app/`.

pub mod traits;
pub mod types;

pub use traits::*;
pub use types::*;

/// Result of processing an `EngineCommand`. Sent back to the caller
/// via the optional `oneshot::Sender` in the command envelope.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub enum CommandResult {
    /// Command succeeded with no additional data.
    Ok,
    /// Command succeeded and created an entity with the given UUID.
    OkWithId { uuid: String },
    /// Command succeeded with additional data payload.
    OkWithData { data: serde_json::Value },
    /// Command failed.
    Err { code: ErrorCode, message: String },
}

/// Error codes for command failures.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    InternalError,
    Unavailable,
}

/// A command envelope: the command itself plus an optional reply channel.
/// UI consumers send `None` (fire-and-forget). HTTP API sends `Some(tx)`.
pub type CommandEnvelope = (
    EngineCommand,
    Option<tokio::sync::oneshot::Sender<CommandResult>>,
);

/// Cross-thread command envelope for message-passing consumers.
///
/// Each variant mirrors a trait method 1:1. Cross-thread consumers
/// (HTTP API, CLI) send these via `mpsc::Sender<EngineCommand>`.
/// The engine processes them once per frame.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum EngineCommand {
    // ── Mixer ──────────────────────────────────────────────────
    SetCrossfader(f32),
    SetTonemapMode(crate::renderer::tonemap::TonemapMode),
    LoadLut {
        filename: String,
    },
    UnloadLut,
    AutoCrossfade {
        target: f32,
        duration_secs: f32,
        easing: CrossfadeEasing,
    },
    BeatCrossfade {
        target: f32,
        beats: f32,
    },
    AddDeck {
        channel_idx: usize,
        shader_name: String,
    },
    AddImageDeck {
        channel_idx: usize,
        path: std::path::PathBuf,
    },
    AddVideoDeck {
        channel_idx: usize,
        path: std::path::PathBuf,
    },
    AddSolidColorDeck {
        channel_idx: usize,
        color: [f32; 4],
    },
    AddCameraDeck {
        channel_idx: usize,
        camera_id: CameraId,
    },
    RemoveDeck {
        channel_idx: usize,
        deck_idx: usize,
    },
    MoveDeck {
        src_ch: usize,
        src_deck: usize,
        dst_ch: usize,
    },
    ReorderDeck {
        ch: usize,
        from_idx: usize,
        to_idx: usize,
    },
    SetDeckOpacity {
        channel_idx: usize,
        deck_idx: usize,
        opacity: f32,
    },
    SetDeckBlendMode {
        channel_idx: usize,
        deck_idx: usize,
        mode: BlendMode,
    },
    SetDeckSolo {
        channel_idx: usize,
        deck_idx: usize,
        solo: bool,
    },
    SetDeckMute {
        channel_idx: usize,
        deck_idx: usize,
        mute: bool,
    },
    SetDeckRenderFps {
        channel_idx: usize,
        deck_idx: usize,
        render_fps: DeckRenderFps,
    },
    SetDeckScalingMode {
        channel_idx: usize,
        deck_idx: usize,
        mode: ScalingMode,
    },
    SetDeckTransparent {
        channel_idx: usize,
        deck_idx: usize,
        transparent: bool,
    },
    SetChannelOpacity {
        channel_idx: usize,
        opacity: f32,
    },
    SetChannelBlendMode {
        channel_idx: usize,
        mode: BlendMode,
    },
    AddChannel,
    RemoveChannel {
        channel_idx: usize,
    },
    AddEffect {
        target: EffectTarget,
        shader_name: String,
    },
    RemoveEffect {
        target: EffectTarget,
        effect_idx: usize,
    },
    ToggleEffect {
        target: EffectTarget,
        effect_idx: usize,
    },
    MoveEffect {
        target: EffectTarget,
        from_idx: usize,
        to_idx: usize,
    },
    SetTransition {
        shader_name: Option<String>,
    },
    SetParam {
        path: String,
        value: ParamValue,
    },

    // ── Audio ──────────────────────────────────────────────────
    OpenAudioSource {
        source_id: AudioSourceId,
    },
    CloseAudioSource {
        source_id: AudioSourceId,
    },
    ScanAudioDevices,

    // ── Modulation ─────────────────────────────────────────────
    AddLfo {
        waveform: LFOWaveform,
        frequency: f32,
    },
    AddAudioBand {
        preset: AudioBandPreset,
        source_id: Option<AudioSourceId>,
    },
    AddAdsr {
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
    },
    AddStepSequencer {
        num_steps: usize,
        rate: f32,
    },
    RemoveModulationSource {
        uuid: String,
    },
    AssignModulation {
        target: String,
        source_id: String,
        amount: f32,
    },
    ClearModulation {
        target: String,
    },

    // ── Video Playback ────────────────────────────────────────────
    VideoTogglePlay {
        channel_idx: usize,
        deck_idx: usize,
    },
    VideoSeek {
        channel_idx: usize,
        deck_idx: usize,
        position_secs: f64,
    },
    VideoSetSpeed {
        channel_idx: usize,
        deck_idx: usize,
        speed: f64,
    },
    VideoSetLoopMode {
        channel_idx: usize,
        deck_idx: usize,
        mode: crate::video::LoopMode,
    },
    VideoSetInPoint {
        channel_idx: usize,
        deck_idx: usize,
        secs: f64,
    },
    VideoSetOutPoint {
        channel_idx: usize,
        deck_idx: usize,
        secs: f64,
    },
    VideoClearInOutPoints {
        channel_idx: usize,
        deck_idx: usize,
    },

    // ── Deck Auto-Transitions ──────────────────────────────────
    SetAutoTransitionEnabled {
        channel_idx: usize,
        deck_idx: usize,
        enabled: bool,
    },
    SetAutoTransitionTrigger {
        channel_idx: usize,
        deck_idx: usize,
        clip_end: bool,
    },
    SetAutoTransitionPlayDuration {
        channel_idx: usize,
        deck_idx: usize,
        value: f64,
        unit: crate::channel::DurationUnit,
    },
    SetAutoTransitionDuration {
        channel_idx: usize,
        deck_idx: usize,
        value: f64,
        unit: crate::channel::DurationUnit,
    },
    SetAutoTransitionShader {
        channel_idx: usize,
        deck_idx: usize,
        shader_name: Option<String>,
    },
    ToggleAutoTransitionPlayDurationUnit {
        channel_idx: usize,
        deck_idx: usize,
    },
    ToggleAutoTransitionDurationUnit {
        channel_idx: usize,
        deck_idx: usize,
    },
    SetAutoTransitionPlayDurationValue {
        channel_idx: usize,
        deck_idx: usize,
        value: f64,
    },
    SetAutoTransitionDurationValue {
        channel_idx: usize,
        deck_idx: usize,
        value: f64,
    },

    // ── External I/O Deck Sources ──────────────────────────────
    AddNdiDeck {
        channel_idx: usize,
        source_name: String,
    },
    AddSyphonDeck {
        channel_idx: usize,
        server_name: String,
    },
    AddSrtDeck {
        channel_idx: usize,
        url: String,
        mode: crate::stream::SrtMode,
    },
    AddHlsDeck {
        channel_idx: usize,
        url: String,
    },
    AddDashDeck {
        channel_idx: usize,
        url: String,
    },
    AddRtmpDeck {
        channel_idx: usize,
        url: String,
        mode: crate::stream::RtmpMode,
    },
    AddHtmlDeck {
        channel_idx: usize,
        url: String,
    },
    ReloadHtmlDeck {
        channel_idx: usize,
        deck_idx: usize,
    },
    /// Open the interactive window for the HTML deck at `(channel_idx, deck_idx)`.
    OpenHtmlInteractive {
        channel_idx: usize,
        deck_idx: usize,
    },
    /// Close the interactive HTML window (if any).
    CloseHtmlInteractive,

    // ── Transition Sequences ───────────────────────────────────
    CreateSequence,
    DeleteSequence {
        idx: usize,
    },
    PlaySequence {
        idx: usize,
    },
    StopSequence {
        idx: usize,
    },
    ToggleSequence {
        idx: usize,
    },
    AddFadeStep {
        seq_idx: usize,
        from_ch: usize,
        to_ch: usize,
    },
    AddWaitStep {
        seq_idx: usize,
    },
    AddGoToStep {
        seq_idx: usize,
        step_index: usize,
    },
    RemoveStep {
        seq_idx: usize,
        step_idx: usize,
    },
    SetStepDuration {
        seq_idx: usize,
        step_idx: usize,
        value: f64,
        unit: crate::channel::DurationUnit,
    },
    SetStepEasing {
        seq_idx: usize,
        step_idx: usize,
        easing: String,
    },
    SetStepTransitionShader {
        seq_idx: usize,
        step_idx: usize,
        shader_name: Option<String>,
    },
    MoveStep {
        seq_idx: usize,
        from: usize,
        to: usize,
    },
    SetStepDurationUnit {
        seq_idx: usize,
        step_idx: usize,
        unit: crate::channel::DurationUnit,
    },
    SetStepFromCh {
        seq_idx: usize,
        step_idx: usize,
        ch: usize,
    },
    SetStepToCh {
        seq_idx: usize,
        step_idx: usize,
        ch: usize,
    },
    SetGoToTarget {
        seq_idx: usize,
        step_idx: usize,
        target: usize,
    },
    ToggleStepDurationUnit {
        seq_idx: usize,
        step_idx: usize,
    },
    SetStepDurationValue {
        seq_idx: usize,
        step_idx: usize,
        value: f64,
    },
    SetStepTargetAmount {
        seq_idx: usize,
        step_idx: usize,
        amount: f32,
    },

    // ── Stream Library ─────────────────────────────────────────
    AddStreamLibraryEntry {
        url: String,
        mode: crate::stream::SrtMode,
    },
    RemoveStreamLibraryEntry {
        url: String,
    },
    AddHlsLibraryEntry {
        url: String,
    },
    RemoveHlsLibraryEntry {
        url: String,
    },
    AddDashLibraryEntry {
        url: String,
    },
    RemoveDashLibraryEntry {
        url: String,
    },
    AddRtmpLibraryEntry {
        url: String,
        mode: crate::stream::RtmpMode,
    },
    RemoveRtmpLibraryEntry {
        url: String,
    },

    // ── Output ─────────────────────────────────────────────────
    CreateOutput,
    CreateHeadlessOutput {
        target: crate::renderer::context::OutputTarget,
    },
    CloseOutput {
        idx: usize,
    },
    SetOutputDisplay {
        idx: usize,
        monitor_name: String,
    },
    SetOutputTarget {
        idx: usize,
        target: crate::renderer::context::OutputTarget,
    },
    StartOutput {
        idx: usize,
    },
    StopOutput {
        idx: usize,
    },
    /// Set the calibration display mode for an output (Off / Projector / Surfaces).
    SetCalibrationMode {
        idx: usize,
        mode: crate::renderer::context::CalibrationMode,
    },
    /// Move one corner-pin corner of a surface's warp (per-surface).
    SetWarpCorner {
        surface_uuid: String,
        corner_idx: usize,
        position: [f32; 2],
    },
    /// Clear a surface's warp (back to no-warp / native position).
    ResetWarp {
        surface_uuid: String,
    },
    /// Set the warp grid resolution for a surface, converting its warp to a
    /// `cols` × `rows` mesh (preserving the current deformation). Dimensions ≥2.
    SetWarpSubdivisions {
        surface_uuid: String,
        cols: u32,
        rows: u32,
    },
    /// Move a single mesh grid point (row-major) of a surface's mesh warp.
    /// No-op if the surface's warp is not currently a mesh.
    SetWarpMeshPoint {
        surface_uuid: String,
        row: usize,
        col: usize,
        position: [f32; 2],
    },
    /// Bind or unbind a surface's warp from its shape (auto-warp). Binding
    /// re-derives the warp from the outline; unbinding materialises it for
    /// manual fine-tuning.
    SetWarpBound {
        surface_uuid: String,
        bound: bool,
    },
    /// Convert a surface's warp into a smooth bezier patch grid (8i.6), seeding
    /// the control cage from the current warp so the shape is preserved.
    ConvertWarpToBezier {
        surface_uuid: String,
    },
    /// Move a bezier-warp control anchor (row-major grid coords).
    MoveWarpAnchor {
        surface_uuid: String,
        row: usize,
        col: usize,
        position: [f32; 2],
    },
    /// Move a bezier-warp tangent handle. `horizontal` selects a horizontal edge
    /// (`(r,c)→(r,c+1)`) vs a vertical edge (`(r,c)→(r+1,c)`); `which` is 0/1.
    MoveWarpHandle {
        surface_uuid: String,
        horizontal: bool,
        row: usize,
        col: usize,
        which: usize,
        position: [f32; 2],
    },
    /// Set the bezier-warp control-cage resolution (anchor `cols` × `rows`).
    SetBezierCageSubdivisions {
        surface_uuid: String,
        cols: u32,
        rows: u32,
    },
    SetEdgeBlend {
        output_idx: usize,
        config: crate::renderer::edge_blend::EdgeBlendConfig,
    },
    SetEdgeBlendMode {
        output_idx: usize,
        mode: crate::renderer::edge_blend::EdgeBlendMode,
    },
    SetOutputRotation {
        idx: usize,
        rotation: crate::renderer::context::OutputRotation,
    },

    // ── Surfaces ────────────────────────────────────────────────
    AddSurface {
        name: String,
        source: OutputSource,
    },
    AddPolygonSurface {
        name: String,
        vertices: Vec<[f32; 2]>,
        source: OutputSource,
    },
    AddCircleSurface {
        name: String,
        center: [f32; 2],
        radius: f32,
        sides: u32,
        aspect_ratio: f32,
        source: OutputSource,
    },
    RemoveSurface {
        uuid: String,
    },
    /// Change a surface's global stacking order (8i.12).
    ReorderSurface {
        uuid: String,
        op: SurfaceReorderOp,
    },
    SetSurfaceSource {
        uuid: String,
        source: OutputSource,
    },
    SetSurfaceOutputType {
        uuid: String,
        output_type: SurfaceOutputType,
    },
    SetSurfaceContentMapping {
        uuid: String,
        mapping: ContentMapping,
    },
    RenameSurface {
        uuid: String,
        name: String,
    },
    UpdateSurfaceVertices {
        uuid: String,
        vertices: Vec<[f32; 2]>,
    },
    DuplicateSurface {
        uuid: String,
    },
    FlipSurfaceHorizontal {
        uuid: String,
    },
    FlipSurfaceVertical {
        uuid: String,
    },
    InsertSurfaceVertex {
        uuid: String,
        after_vert_idx: usize,
        position: [f32; 2],
    },
    SetCircleRadius {
        uuid: String,
        radius: f32,
    },
    SetCircleSides {
        uuid: String,
        sides: u32,
    },
    ConvertSurfaceToPolygon {
        uuid: String,
    },
    CombineSurfaces {
        uuids: Vec<String>,
    },
    MoveSurface {
        uuid: String,
        dx: f32,
        dy: f32,
    },
    RotateSurface {
        uuid: String,
        /// Rotation in radians (clockwise in canvas space, y-down).
        angle: f32,
        /// Pivot the rotation is applied around, in normalized canvas coords.
        pivot: [f32; 2],
    },
    ScaleSurface {
        uuid: String,
        sx: f32,
        sy: f32,
        /// Pivot the scale is applied around, in normalized canvas coords.
        pivot: [f32; 2],
    },
    UpdateSurfaceContourVertices {
        uuid: String,
        contour: usize,
        vertices: Vec<[f32; 2]>,
    },
    /// Convert a curve-path edge to a cubic bezier (`to_cubic`) or back to a
    /// straight line. Lazily builds a path from the polygon if absent.
    ConvertSurfaceEdge {
        uuid: String,
        edge_idx: usize,
        to_cubic: bool,
    },
    /// Move a curve-path anchor to `pos` (normalized coords).
    MovePathAnchor {
        uuid: String,
        anchor_idx: usize,
        pos: [f32; 2],
    },
    /// Move a cubic control handle of a curve-path segment to `pos`.
    MovePathHandle {
        uuid: String,
        segment_idx: usize,
        handle: CubicHandle,
        pos: [f32; 2],
    },
    /// Add a subtractive cut-out hole (8i.7) to a surface from a closed path.
    AddSurfaceHole {
        uuid: String,
        hole: SurfacePath,
    },
    /// Remove the hole at `hole_index` from a surface.
    RemoveSurfaceHole {
        uuid: String,
        hole_index: usize,
    },
    /// "Make Hole" (8i.7): convert an existing surface into a cut-out hole in the
    /// topmost other surface under its centroid, then remove the source surface.
    /// Atomic (single command — no half-punched state).
    PunchSurfaceHole {
        source_uuid: String,
    },
    AssignSurfaceToOutput {
        output_uuid: String,
        surface_uuid: String,
    },
    UnassignSurfaceFromOutput {
        output_uuid: String,
        assignment_idx: usize,
    },
    AssignSurfaceToOutputByIdx {
        output_idx: usize,
        surface_uuid: String,
    },
    UnassignSurfaceFromOutputByIdx {
        output_idx: usize,
        assignment_idx: usize,
    },

    // ── Surface Auto-Detection ──────────────────────────────────
    /// Detect contours from a raster image and create surfaces from them.
    DetectFromImage {
        image_data: Vec<u8>,
        params: crate::surface::detect::DetectionParams,
    },
    /// Detect contours from an SVG file.
    DetectFromSvg {
        svg_data: Vec<u8>,
    },
    /// Detect contours from a DXF file.
    DetectFromDxf {
        dxf_data: Vec<u8>,
    },
    /// Confirm detected contours: create surfaces from them.
    ConfirmDetectedContours {
        contours: Vec<crate::surface::detect::DetectedContour>,
    },
    /// Detect contours from a camera snapshot.
    DetectFromCamera {
        camera_id: CameraId,
        params: crate::surface::detect::DetectionParams,
    },

    // ── Modulation Updates ─────────────────────────────────────
    UpdateLfoFrequency {
        uuid: String,
        frequency: f32,
    },
    UpdateLfoWaveform {
        uuid: String,
        waveform: LFOWaveform,
    },
    UpdateLfoPhase {
        uuid: String,
        phase: f32,
    },
    UpdateLfoAmplitude {
        uuid: String,
        amplitude: f32,
    },
    UpdateLfoBipolar {
        uuid: String,
        bipolar: bool,
    },
    UpdateAudioSmoothing {
        uuid: String,
        smoothing: f32,
    },
    UpdateAudioFreqRange {
        uuid: String,
        freq_low: f32,
        freq_high: f32,
    },
    UpdateAudioFreqLow {
        uuid: String,
        freq_low: f32,
    },
    UpdateAudioFreqHigh {
        uuid: String,
        freq_high: f32,
    },
    UpdateAudioGain {
        uuid: String,
        gain: f32,
    },
    UpdateAudioPreset {
        uuid: String,
        preset: AudioBandPreset,
    },
    UpdateAudioMode {
        uuid: String,
        mode: crate::modulation::AudioReactMode,
    },
    UpdateAudioSource {
        uuid: String,
        source_id: Option<AudioSourceId>,
    },
    UpdateAudioNoiseGate {
        uuid: String,
        noise_gate: f32,
    },
    UpdateAdsrAttack {
        uuid: String,
        attack: f32,
    },
    UpdateAdsrDecay {
        uuid: String,
        decay: f32,
    },
    UpdateAdsrSustain {
        uuid: String,
        sustain: f32,
    },
    UpdateAdsrRelease {
        uuid: String,
        release: f32,
    },
    TriggerAdsr {
        uuid: String,
    },
    ReleaseAdsr {
        uuid: String,
    },
    UpdateStepSeqSteps {
        uuid: String,
        steps: Vec<f32>,
    },
    UpdateStepSeqRate {
        uuid: String,
        rate: f32,
    },
    UpdateStepSeqInterpolation {
        uuid: String,
        interpolation: crate::modulation::StepInterpolation,
    },
    UpdateStepSeqBipolar {
        uuid: String,
        bipolar: bool,
    },
    SetStepSeqCount {
        uuid: String,
        count: usize,
    },
    UpdateStepSeqValue {
        uuid: String,
        step_idx: usize,
        value: f32,
    },
    AssignModOnMod {
        target_source_id: String,
        param_name: String,
        modulator_id: String,
        amount: f32,
    },
    RemoveModOnMod {
        target_source_id: String,
        param_name: String,
    },

    // ── Analyzers ──────────────────────────────────────────────────
    RequestAnalyzer {
        deck_id: String,
        analyzer_type: String,
        options: serde_json::Value,
    },
    ReleaseAnalyzer {
        deck_id: String,
        analyzer_type: String,
    },
    AddAnalyzerModSource {
        deck_id: String,
        analyzer_type: String,
        output_name: String,
    },
    UpdateAnalyzerSmoothing {
        uuid: String,
        smoothing: f32,
    },

    // ── Device Scanning ────────────────────────────────────────
    RescanNdi,
    RescanSyphon,
    RescanCameras,
    RescanMidi,
    RescanAudio,
    ToggleAudioSource {
        source_id: u32,
        enabled: bool,
    },
    SetMidiDeviceEnabled {
        device_id: crate::midi::DeviceId,
        enabled: bool,
    },

    // ── MIDI Mappings ──────────────────────────────────────────
    ClearMidiMappings,
    RemoveMidiMapping {
        key: crate::midi::MidiKey,
    },

    // ── Clock ──────────────────────────────────────────────────
    SetClockPreference {
        preference: crate::clock::ClockPreference,
    },
    SetManualBpm {
        bpm: f32,
    },

    // ── Parameters (index-based) ─────────────────────────────────
    SetGeneratorParam {
        channel_idx: usize,
        deck_idx: usize,
        name: String,
        value: ParamValue,
    },
    SetEffectParam {
        channel_idx: usize,
        deck_idx: usize,
        effect_idx: usize,
        name: String,
        value: ParamValue,
    },
    SetChannelEffectParam {
        channel_idx: usize,
        effect_idx: usize,
        name: String,
        value: ParamValue,
    },
    SetMasterEffectParam {
        effect_idx: usize,
        name: String,
        value: ParamValue,
    },
    ResetGeneratorParamsToDefaults {
        channel_idx: usize,
        deck_idx: usize,
    },

    // ── Resolution ─────────────────────────────────────────────
    SetRenderResolution {
        width: u32,
        height: u32,
    },

    // ── Frame pacing ─────────────────────────────────────────
    SetTargetFps {
        fps: u32,
    },

    // ── Performance profiling ──────────────────────────────────
    /// Start GPU performance profiling for the next N frames.
    /// Inserts device.poll(Wait) between GPU stages to measure actual
    /// GPU execution time per category. Logs every frame.
    StartPerfProfile {
        frames: u32,
    },

    // ── Persistence ────────────────────────────────────────────
    SaveWorkspace,
    LoadWorkspace,

    // ── History ─────────────────────────────────────────────────
    Undo,
    Redo,

    // ── System ──────────────────────────────────────────────────
    Shutdown,
}
