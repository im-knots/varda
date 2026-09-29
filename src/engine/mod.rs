//! Engine contract: `EngineCommand`, the `EngineState` snapshot types, and
//! the value types in [`value`].
//!
//! No implementation and no GPU types. Consumers (UI, HTTP API, CLI) send
//! commands and read snapshots; the implementation lives in `src/app/`.

pub mod types;
pub mod value;

pub use types::*;

/// Result of an `EngineCommand`, sent back on the envelope's optional
/// `oneshot::Sender`.
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

/// Typed, in-process result of executing a command through the GUI drain.
///
/// The GUI needs same-frame typed data to finish a mutation (register a
/// preview texture by UUID). Bus consumers get [`CommandResult`] instead.
#[derive(Debug, Clone)]
pub enum CommandOutcome {
    /// The wire result, with no extra GUI data.
    Plain(CommandResult),
    /// Decks were created; the GUI registers a preview texture for each UUID.
    DecksCreated { uuids: Vec<String> },
}

/// Error codes for command failures.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
pub enum ErrorCode {
    NotFound,
    InvalidInput,
    InternalError,
    Unavailable,
}

/// A command plus an optional reply channel. The UI sends `None`; the HTTP API
/// sends `Some(tx)`.
pub type CommandEnvelope = (
    EngineCommand,
    Option<tokio::sync::oneshot::Sender<CommandResult>>,
);

/// Cross-thread command envelope for message-passing consumers.
///
/// Each variant mirrors a trait method. The HTTP API and CLI send these over
/// `mpsc::Sender<EngineCommand>`; the engine drains them once per frame.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum EngineCommand {
    // ── Mixer ──────────────────────────────────────────────────
    SetCrossfader(f32),
    SetTonemapMode(crate::engine::value::render::TonemapMode),
    LoadLut {
        filename: String,
    },
    UnloadLut,
    /// Load a scene-referred look LUT, applied to the linear program before
    /// any output transform so it reaches every output. [`EngineCommand::LoadLut`]
    /// loads the display-referred calibration LUT.
    LoadLookLut {
        filename: String,
    },
    UnloadLookLut,
    AutoCrossfade {
        target: f32,
        duration_secs: f32,
        easing: CrossfadeEasing,
    },
    BeatCrossfade {
        target: f32,
        beats: f32,
    },
    /// Add a deck whose source `source` describes: `{"type": "<id>", ...}`,
    /// where the type is one of `EngineState::sources`. Returns the new deck's
    /// UUID. Background loads report progress in `EngineState::deck_loads`.
    AddDeck {
        channel_uuid: String,
        source: crate::engine::value::source::SourceConfig,
    },
    /// Swap a deck's source for another, keeping the deck's identity, effects,
    /// opacity and modulation (repoint a tap, switch a camera).
    ReplaceDeckSource {
        deck_uuid: String,
        source: crate::engine::value::source::SourceConfig,
    },
    /// Write one of a deck's source controls, by the name its source type
    /// declares. Numeric controls take normalized values.
    SetSourceParam {
        deck_uuid: String,
        name: String,
        value: crate::engine::value::provider::ControlValue,
    },
    /// Fire one of a deck's source actions (reload a page, clear in/out).
    TriggerSourceAction {
        deck_uuid: String,
        action: String,
    },
    RemoveDeck {
        deck_uuid: String,
    },
    MoveDeck {
        deck_uuid: String,
        dst_channel_uuid: String,
    },
    /// Reposition a deck within its channel. `from_idx`/`to_idx` are positions.
    ReorderDeck {
        channel_uuid: String,
        from_idx: usize,
        to_idx: usize,
    },
    SetDeckOpacity {
        deck_uuid: String,
        opacity: f32,
    },
    SetDeckBlendMode {
        deck_uuid: String,
        mode: BlendMode,
    },
    SetDeckSolo {
        deck_uuid: String,
        solo: bool,
    },
    SetDeckMute {
        deck_uuid: String,
        mute: bool,
    },
    SetDeckRenderFps {
        deck_uuid: String,
        render_fps: DeckRenderFps,
    },
    SetDeckTransparent {
        deck_uuid: String,
        transparent: bool,
    },
    SetChannelOpacity {
        channel_uuid: String,
        opacity: f32,
    },
    SetChannelBlendMode {
        channel_uuid: String,
        mode: BlendMode,
    },
    AddChannel,
    RemoveChannel {
        channel_uuid: String,
    },
    AddEffect {
        target: EffectTarget,
        shader_name: String,
    },
    RemoveEffect {
        effect_uuid: String,
    },
    ToggleEffect {
        effect_uuid: String,
    },
    /// Reposition an effect within the chain `target` names.
    MoveEffect {
        target: EffectTarget,
        from_idx: usize,
        to_idx: usize,
    },

    // ── Clipboard ───────────────────
    /// Copy an object's config to the clipboard. Not undoable.
    ///
    /// `include_arrangement` also copies a deck's regions; the UI sets it when
    /// copying on the timeline.
    Copy {
        source: ClipboardSource,
        #[serde(default)]
        include_arrangement: bool,
    },
    /// Paste the clipboard with fresh UUIDs throughout.
    Paste {
        target: PasteTarget,
    },
    /// Paste beside the original in one step, leaving the clipboard untouched.
    Duplicate {
        source: ClipboardSource,
    },
    SetTransition {
        shader_name: Option<String>,
    },
    SetParam {
        path: String,
        value: ParamValue,
    },
    /// Toggle a parameter between its extremes by path (crossfader 0↔1,
    /// mute/solo). See `param_router::toggle_param_by_path`.
    ToggleParam {
        path: String,
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
    /// Create an automation envelope and assign it to `target` in `Absolute`
    /// mode ("Add automation lane").
    AddAutomationLane {
        target: String,
        /// Timebase the curve is drawn against. Arrangement-authored lanes use
        /// `Transport`.
        timebase: crate::timebase::Timebase,
    },
    /// Replace an envelope's breakpoints. The engine sorts them.
    SetEnvelopeBreakpoints {
        uuid: String,
        breakpoints: Vec<crate::modulation::Breakpoint>,
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
    ClearModulationSource {
        target: String,
        source_id: String,
    },

    // ── Deck Auto-Transitions ──────────────────────────────────
    SetAutoTransitionEnabled {
        deck_uuid: String,
        enabled: bool,
    },
    SetAutoTransitionTrigger {
        deck_uuid: String,
        clip_end: bool,
    },
    SetAutoTransitionPlayDuration {
        deck_uuid: String,
        value: f64,
        unit: crate::channel::DurationUnit,
    },
    SetAutoTransitionDuration {
        deck_uuid: String,
        value: f64,
        unit: crate::channel::DurationUnit,
    },
    SetAutoTransitionShader {
        deck_uuid: String,
        shader_name: Option<String>,
    },
    ToggleAutoTransitionPlayDurationUnit {
        deck_uuid: String,
    },
    ToggleAutoTransitionDurationUnit {
        deck_uuid: String,
    },
    SetAutoTransitionPlayDurationValue {
        deck_uuid: String,
        value: f64,
    },
    SetAutoTransitionDurationValue {
        deck_uuid: String,
        value: f64,
    },

    // ── HTML interactive window ────────────────────────────────
    /// Open the interactive window for an HTML deck.
    OpenHtmlInteractive {
        deck_uuid: String,
    },
    /// Close the interactive HTML window (if any).
    CloseHtmlInteractive,

    // ── Transition Sequences ───────────────────────────────────
    CreateSequence,
    DeleteSequence {
        sequence_uuid: String,
    },
    PlaySequence {
        sequence_uuid: String,
    },
    StopSequence {
        sequence_uuid: String,
    },
    ToggleSequence {
        sequence_uuid: String,
    },
    // `step_idx` is a position within the sequence.
    AddFadeStep {
        sequence_uuid: String,
        from_channel_uuid: String,
        to_channel_uuid: String,
    },
    AddWaitStep {
        sequence_uuid: String,
    },
    AddGoToStep {
        sequence_uuid: String,
        step_index: usize,
    },
    RemoveStep {
        sequence_uuid: String,
        step_idx: usize,
    },
    SetStepDuration {
        sequence_uuid: String,
        step_idx: usize,
        value: f64,
        unit: crate::channel::DurationUnit,
    },
    SetStepEasing {
        sequence_uuid: String,
        step_idx: usize,
        easing: String,
    },
    SetStepTransitionShader {
        sequence_uuid: String,
        step_idx: usize,
        shader_name: Option<String>,
    },
    MoveStep {
        sequence_uuid: String,
        from: usize,
        to: usize,
    },
    SetStepDurationUnit {
        sequence_uuid: String,
        step_idx: usize,
        unit: crate::channel::DurationUnit,
    },
    SetStepFromCh {
        sequence_uuid: String,
        step_idx: usize,
        channel_uuid: String,
    },
    SetStepToCh {
        sequence_uuid: String,
        step_idx: usize,
        channel_uuid: String,
    },
    SetGoToTarget {
        sequence_uuid: String,
        step_idx: usize,
        target: usize,
    },
    ToggleStepDurationUnit {
        sequence_uuid: String,
        step_idx: usize,
    },
    SetStepDurationValue {
        sequence_uuid: String,
        step_idx: usize,
        value: f64,
    },
    SetStepTargetAmount {
        sequence_uuid: String,
        step_idx: usize,
        amount: f32,
    },

    // ── Source Library ─────────────────────────────────────────
    /// Save an entry a user filled in (a stream URL) to a source type's
    /// library. `entry.type` names the source type.
    AddSourceLibraryEntry {
        entry: crate::engine::value::source::SourceConfig,
    },
    RemoveSourceLibraryEntry {
        entry: crate::engine::value::source::SourceConfig,
    },
    /// Run a library action a source type offers: `rescan`, or one a library
    /// notice names (granting screen-recording access).
    SourceLibraryAction {
        source_type: String,
        action: String,
    },

    // ── Output ─────────────────────────────────────────────────
    /// Create an output delivering through `sink`: `{"type": "windowed"}`,
    /// `{"type": "recording", "path": ...}`, any registered sink type.
    /// Returns the new output's UUID.
    CreateOutput {
        sink: crate::engine::value::provider::ProviderConfig,
    },
    CloseOutput {
        output_uuid: String,
    },
    /// Point an output at another sink, keeping its surfaces, warp, edge
    /// blend and presentation. A running output is stopped first.
    SetOutputTarget {
        output_uuid: String,
        sink: crate::engine::value::provider::ProviderConfig,
    },
    /// Write one of an output's sink settings, by the name its type declares.
    SetSinkParam {
        output_uuid: String,
        name: String,
        value: crate::engine::value::provider::ControlValue,
    },
    /// Run a library action an output type offers (`rescan`). Returns the
    /// type's entries.
    SinkLibraryAction {
        sink_type: String,
        action: String,
    },
    /// Show or hide a surface on an output, assigning it on first show
    /// (`output/<uuid>/surface/<surface_uuid>`).
    SetSurfaceAssignmentEnabled {
        output_uuid: String,
        surface_uuid: String,
        enabled: bool,
    },
    /// Write text to an address that takes it: `surface/<uuid>/source`, or an
    /// output's text setting at `output/<uuid>/<route>`.
    SetPathText {
        path: String,
        value: String,
    },
    /// Choose what an output shows with no surfaces assigned, or `None` for
    /// its sink's default.
    SetOutputUnassigned {
        output_uuid: String,
        unassigned: Option<crate::engine::value::render::Unassigned>,
    },
    StartOutput {
        output_uuid: String,
    },
    StopOutput {
        output_uuid: String,
    },
    /// Set the calibration display mode for an output (Off / Projector / Surfaces).
    SetCalibrationMode {
        output_uuid: String,
        mode: crate::engine::value::render::CalibrationMode,
    },
    /// Move one corner-pin corner of a surface's warp (per-surface).
    SetWarpCorner {
        surface_uuid: String,
        corner_idx: usize,
        position: [f32; 2],
    },
    /// Clear a surface's warp.
    ResetWarp {
        surface_uuid: String,
    },
    /// Set the warp grid resolution for a surface, converting its warp to a
    /// `cols` × `rows` mesh, keeping the current deformation. Dimensions ≥2.
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
    /// Bind or unbind a surface's warp from its shape. Binding re-derives the
    /// warp from the outline; unbinding keeps it for manual editing.
    SetWarpBound {
        surface_uuid: String,
        bound: bool,
    },
    /// Convert a surface's warp into a bezier patch grid, seeded from the
    /// current warp.
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
        output_uuid: String,
        config: crate::engine::value::render::EdgeBlendConfig,
    },
    SetEdgeBlendMode {
        output_uuid: String,
        mode: crate::engine::value::render::EdgeBlendMode,
    },
    SetOutputRotation {
        output_uuid: String,
        rotation: crate::engine::value::render::OutputRotation,
    },
    SetOutputPresentation {
        output_uuid: String,
        request: crate::engine::value::render::PresentationRequest,
    },
    /// Override the tonemap curve for one output. `None` inherits the mixer's
    /// curve.
    SetOutputTonemap {
        output_uuid: String,
        tonemap: Option<crate::engine::value::render::TonemapMode>,
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
    /// Change a surface's stacking order.
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
    /// Add a cut-out hole to a surface from a closed path.
    AddSurfaceHole {
        uuid: String,
        hole: SurfacePath,
    },
    /// Remove the hole at `hole_index` from a surface.
    RemoveSurfaceHole {
        uuid: String,
        hole_index: usize,
    },
    /// Turn a surface into a hole in the topmost other surface under its
    /// centroid, then remove it. One command, so it applies atomically.
    PunchSurfaceHole {
        source_uuid: String,
    },
    AssignSurfaceToOutput {
        output_uuid: String,
        surface_uuid: String,
    },
    UnassignSurfaceFromOutput {
        output_uuid: String,
        surface_uuid: String,
    },

    // ── Surface Auto-Detection ──────────────────────────────────
    /// Detect contours from a raster image and create surfaces from them.
    DetectFromImage {
        image_data: Vec<u8>,
        params: crate::engine::value::detect::DetectionParams,
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
        contours: Vec<crate::engine::value::detect::DetectedContour>,
    },
    /// Detect contours in a stage-plan file (image, SVG, DXF) and create
    /// surfaces from them.
    ImportSurfacesFromFile {
        path: std::path::PathBuf,
    },
    /// Generate per-projector dome surfaces with warp meshes from a dome setup.
    /// Removes existing "Dome P*" surfaces, computes meshes, creates new ones.
    GenerateDomeSlices {
        setup: crate::engine::value::dome::DomeSetup,
    },
    /// Detect contours from a camera snapshot.
    DetectFromCamera {
        camera_id: CameraId,
        params: crate::engine::value::detect::DetectionParams,
    },

    // ── Transport ──────────────────────────────────────────────
    // Absolute show position.
    /// Start the show position advancing. Rejected while chasing timecode.
    TransportPlay,
    /// Hold the show position. Readers freeze, so the current look stays.
    TransportStop,
    /// Jump to an absolute position in seconds. Rejected while chasing timecode.
    TransportLocate {
        position: f64,
    },
    /// Choose whether position advances locally or chases incoming timecode.
    SetTransportSource {
        source: crate::transport::TransportSource,
    },
    /// Set or clear the range internal playback wraps within.
    SetTransportLoop {
        region: Option<crate::transport::LoopRegion>,
    },
    /// Frame rate positions are displayed and quantized at.
    SetTimecodeRate {
        rate: crate::transport::TimecodeRate,
    },
    /// Which incoming timecode signal the transport follows.
    SetTimecodePreference {
        preference: crate::timecode::TimecodePreference,
    },
    /// Name the audio input carrying LTC, or stop listening for it.
    SetLtcInput {
        input: Option<crate::timecode::LtcInput>,
    },
    /// Keep live parameter writes as automation curves while the transport
    /// runs. Arming from a stop also starts the transport.
    SetRecordArmed {
        armed: bool,
    },
    /// Locate to the cue before the playhead, or to zero when there is none.
    TransportPrevCue,
    /// Locate to the cue after the playhead, or stay put when there is none.
    TransportNextCue,
    /// Locate to one named cue, leaving the transport running or stopped as it
    /// was. Sent by the Performance-mode cue bank.
    TriggerCue {
        uuid: String,
    },

    // ── Arrangement ────────────────────────────────────────────
    // Deck activity positioned against transport time.
    /// Give a deck a row in the arrangement. Idempotent: one lane per deck.
    AddLane {
        deck_uuid: String,
    },
    /// Drop a row and the envelopes it owned, returning the deck to
    /// Performance mode.
    RemoveLane {
        deck_uuid: String,
    },
    /// Add a visibility span, creating the lane if the deck has none.
    AddRegion {
        deck_uuid: String,
        region: crate::arrangement::RegionConfig,
    },
    /// Replace a span in place, for a move, a resize, or a fade drag.
    UpdateRegion {
        deck_uuid: String,
        index: usize,
        region: crate::arrangement::RegionConfig,
    },
    RemoveRegion {
        deck_uuid: String,
        index: usize,
    },
    /// Fold a lane's automation rows. Saved with the scene.
    SetLaneCollapsed {
        deck_uuid: String,
        collapsed: bool,
    },
    /// What renders before the transport reaches the arranged range.
    SetIdleBehaviour {
        idle: crate::arrangement::IdleBehaviour,
    },
    /// Return one overridden parameter to the arrangement, ramping over
    /// `seconds`.
    RearmParam {
        param_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<f64>,
    },
    /// Return every overridden parameter at once.
    RearmAll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<f64>,
    },
    /// Mark an instant worth returning to. Returns the cue's UUID.
    AddCue {
        at: f64,
        /// Empty names the cue by the cue count.
        #[serde(default)]
        name: String,
    },
    /// Move or rename a cue. Absent fields are left alone.
    UpdateCue {
        uuid: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        at: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    RemoveCue {
        uuid: String,
    },

    // ── Modulation Updates ─────────────────────────────────────
    /// Choose which timebase a modulation source follows.
    UpdateModulationTimebase {
        uuid: String,
        timebase: crate::timebase::Timebase,
    },
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

    // ── Macros ─────────────────────────────────────────────────
    AddMacro {
        kind: crate::macros::MacroKind,
    },
    RemoveMacro {
        uuid: String,
    },
    RenameMacro {
        uuid: String,
        name: String,
    },
    SetMacroKind {
        uuid: String,
        kind: crate::macros::MacroKind,
    },
    /// Live macro turn, fanned out to all targets. Not undoable.
    SetMacroValue {
        uuid: String,
        value: f32,
    },
    AddMacroTarget {
        uuid: String,
        path: String,
    },
    RemoveMacroTarget {
        uuid: String,
        target_idx: usize,
    },
    UpdateMacroTarget {
        uuid: String,
        target_idx: usize,
        min: f32,
        max: f32,
        curve: crate::macros::MacroCurve,
        invert: bool,
    },
    SetMacroButtonBehavior {
        uuid: String,
        behavior: crate::macros::ButtonBehavior,
    },
    SetMacroTriggers {
        uuid: String,
        actions: Vec<crate::macros::TriggerAction>,
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
    RescanMidi,
    RescanAudio,
    ToggleAudioSource {
        source_id: u32,
        enabled: bool,
    },
    SetMidiDeviceEnabled {
        device_id: crate::engine::value::midi::DeviceId,
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

    // ── Parameters ───────────────────────────────────────────────
    SetGeneratorParam {
        deck_uuid: String,
        name: String,
        value: ParamValue,
    },
    /// Set a parameter on any effect — deck, channel, or master chain. Effect
    /// UUIDs are globally unique, so one variant covers all three scopes.
    SetEffectParam {
        effect_uuid: String,
        name: String,
        value: ParamValue,
    },
    ResetGeneratorParamsToDefaults {
        deck_uuid: String,
    },
    /// Draw the deck's generator params afresh from their declared ranges.
    /// `group` scopes to one inspector section; `None` covers all.
    RandomizeGeneratorParams {
        deck_uuid: String,
        group: Option<String>,
        seed: u64,
    },
    /// Nudge the deck's generator params by `amount` of their declared ranges.
    MutateGeneratorParams {
        deck_uuid: String,
        group: Option<String>,
        amount: f32,
        seed: u64,
    },

    // ── Resolution ─────────────────────────────────────────────
    SetRenderResolution {
        width: u32,
        height: u32,
    },

    /// Set the domemaster output size. Square, and independent of the render
    /// resolution.
    SetDomemasterResolution {
        resolution: crate::engine::value::dome::DomemasterResolution,
    },
    /// Set the projector arrangement the domemaster is rendered for.
    SetDomePreset {
        preset: crate::engine::value::dome::DomePreset,
    },
    /// Set the dome the domemaster projects onto, content rotation included.
    SetDomeGeometry {
        geometry: crate::engine::value::dome::DomeGeometry,
    },
    /// Replace the stage editor preferences the engine persists for the GUI.
    /// Not undoable.
    SetEditorPrefs {
        prefs: crate::engine::value::editor::EditorPrefs,
    },

    // ── Frame pacing ─────────────────────────────────────────
    SetTargetFps {
        fps: u32,
    },

    // ── Performance profiling ──────────────────────────────────
    /// Profile the GPU for the next N frames. Inserts `device.poll(Wait)`
    /// between stages to time each category, and logs every frame.
    StartPerfProfile {
        frames: u32,
    },

    // ── Presets ────────────────────────────────────────────────
    /// Load a named deck preset as a new deck on a channel. Presets are
    /// addressed by name because rescans reorder the library.
    LoadDeckPreset {
        channel_uuid: String,
        preset_name: String,
    },
    /// Load a named channel preset. Fills `target_channel_uuid` if it is given
    /// and empty; otherwise appends a new channel.
    LoadChannelPreset {
        target_channel_uuid: Option<String>,
        preset_name: String,
    },
    /// Save a deck's current config as a named deck preset (writes to disk).
    SaveDeckPreset {
        deck_uuid: String,
        name: String,
    },
    /// Save a channel's current config as a named channel preset (writes to disk).
    SaveChannelPreset {
        channel_uuid: String,
        name: String,
    },

    // ── Learn modes and notifications ──────────────────────────
    /// Enter or leave MIDI learn. Entering it leaves keyboard learn.
    MidiLearnToggle,
    /// Choose the parameter path the next MIDI control is bound to.
    MidiLearnSelect {
        path: String,
    },
    /// Enter or leave keyboard learn. Entering it leaves MIDI learn.
    KeyboardLearnToggle,
    /// Choose what the next key combination is bound to.
    KeyboardLearnSelect {
        target: crate::engine::value::keymap::KeyTarget,
    },
    /// Bind `combo` to the selected keyboard learn target.
    KeyboardLearnBind {
        combo: crate::engine::value::keymap::KeyCombo,
    },
    /// Dismiss a notification. Ids are stable; positions shift as others expire.
    DismissNotification {
        id: u64,
    },
    /// Show an informational notification.
    NotifyInfo {
        message: String,
    },

    // ── Consumer views ──────────────────────────────────────────
    /// Force-render these channels for off-air preview even at zero opacity.
    /// Replaces the current set; not persisted.
    SetPreviewChannels {
        channel_uuids: Vec<String>,
    },
    /// Open a camera for surface detection, releasing the one held before.
    AcquireDetectionCamera {
        camera_id: CameraId,
    },
    /// Release the camera held for surface detection, if any.
    ReleaseDetectionCamera,

    // ── Persistence ────────────────────────────────────────────
    SaveWorkspace,
    LoadWorkspace,

    // ── History ─────────────────────────────────────────────────
    Undo,
    Redo,

    // ── System ──────────────────────────────────────────────────
    Shutdown,
}
