//! Engine layer — domain contracts (traits + types).
//!
//! This module defines the public API for the Varda engine.
//! NO implementation, NO GPU types. Pure contracts.
//!
//! Consumers (UI, HTTP API, CLI) program against these traits.
//! The concrete implementation lives in `src/app/`.

pub mod traits;
pub mod types;
pub mod value;

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

/// Typed, in-process result of executing a command through the GUI drain.
///
/// Distinct from [`CommandResult`] (the serializable HTTP/WS wire type): the
/// windowed consumer needs same-frame, strongly-typed data to complete a
/// mutation (register a preview texture by UUID, refresh state after undo).
/// The bus consumers never see this — they get `CommandResult` over the
/// oneshot reply. See [`/spec/ui-engine-boundary.md`] Decision #9.
#[derive(Debug, Clone)]
pub enum CommandOutcome {
    /// No GUI side-channel data; carries the wire result verbatim.
    Plain(CommandResult),
    /// One or more decks were created. The GUI registers a preview texture for
    /// each UUID. Mirrors `OkWithId` for the single-deck case.
    DecksCreated { uuids: Vec<String> },
    /// Undo/redo restored engine state. `structural_changed` tells the GUI to
    /// re-register all preview textures; `dome_layout` carries the UI-local
    /// dome flags to sync back into layout state.
    HistoryRestored {
        structural_changed: bool,
        dome_layout: DomeLayoutFields,
    },
}

/// Dome layout flags that live in UI layout state (not engine state) and must
/// be synced back after an undo/redo restore.
#[derive(Debug, Clone, Copy)]
pub struct DomeLayoutFields {
    pub dome_mode_active: bool,
    pub dome_preset: crate::engine::value::dome::DomePreset,
    pub dome_geometry: crate::engine::value::dome::DomeGeometry,
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
    SetTonemapMode(crate::engine::value::render::TonemapMode),
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
        channel_uuid: String,
        shader_name: String,
    },
    AddImageDeck {
        channel_uuid: String,
        path: std::path::PathBuf,
    },
    AddVideoDeck {
        channel_uuid: String,
        path: std::path::PathBuf,
    },
    AddSolidColorDeck {
        channel_uuid: String,
        color: [f32; 4],
    },
    AddCameraDeck {
        channel_uuid: String,
        camera_id: CameraId,
    },
    AddDepthSensorDeck {
        channel_uuid: String,
        depth_sensor_id: DepthSensorId,
    },
    /// Add a screen / window capture deck. The target is named, not handled, so
    /// the same payload works from the UI, HTTP, and a restored scene.
    AddScreenCaptureDeck {
        channel_uuid: String,
        target: crate::scene::CaptureTargetConfig,
        #[serde(default)]
        rate: Option<f32>,
        #[serde(default)]
        crop: Option<crate::scene::CaptureCropConfig>,
        #[serde(default)]
        show_cursor: Option<bool>,
        #[serde(default)]
        exclude_varda: Option<bool>,
    },
    /// Add a deck that re-enters Varda's own output. See spec/program-tap.md.
    AddTapDeck {
        channel_uuid: String,
        source: crate::scene::TapSourceConfig,
    },
    /// Repoint an existing tap deck at a different source.
    SetTapSource {
        deck_uuid: String,
        source: crate::scene::TapSourceConfig,
    },
    RemoveDeck {
        deck_uuid: String,
    },
    MoveDeck {
        deck_uuid: String,
        dst_channel_uuid: String,
    },
    /// Reposition a deck within its channel. `from_idx`/`to_idx` are ordinals,
    /// not addresses — the position is the payload. See `/spec/api-addressing.md`.
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
    SetDeckScalingMode {
        deck_uuid: String,
        mode: ScalingMode,
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
    /// Reposition an effect within its chain. `target` scopes the ordinals; the
    /// indices are positions, not addresses.
    MoveEffect {
        target: EffectTarget,
        from_idx: usize,
        to_idx: usize,
    },

    // ── Clipboard (see /spec/clipboard.md) ───────────────────
    /// Capture an object's config onto the clipboard. Mutates nothing on stage,
    /// so it is not undoable.
    ///
    /// `include_arrangement` carries a deck's regions, which the UI sets when
    /// the copy was made on the timeline: in the mixer a deck is a source, and
    /// in Arrangement mode it is a source and a placement.
    Copy {
        source: ClipboardSource,
        #[serde(default)]
        include_arrangement: bool,
    },
    /// Rebuild what the clipboard holds, with a fresh identity throughout.
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
    /// Toggle a parameter between its two extremes by path (keyboard-shortcut
    /// affordance): crossfader 0↔1, opacity 0↔1, mute/solo flip, etc. The
    /// two-value logic lives in `keymap::apply_keyboard_toggle_param`.
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
    /// mode, which is the "Add automation lane" gesture.
    /// See /spec/automation.md.
    AddAutomationLane {
        target: String,
        /// Timebase the curve is drawn against. Arrangement-authored lanes use
        /// `Transport`.
        timebase: crate::timebase::Timebase,
    },
    /// Replace an envelope's breakpoints wholesale. The engine sorts them, so
    /// callers do not have to maintain the ordering invariant.
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

    // ── Video Playback ────────────────────────────────────────────
    VideoTogglePlay {
        deck_uuid: String,
    },
    VideoSeek {
        deck_uuid: String,
        position_secs: f64,
    },
    VideoSetSpeed {
        deck_uuid: String,
        speed: f64,
    },
    VideoSetLoopMode {
        deck_uuid: String,
        mode: crate::engine::value::video::LoopMode,
    },
    VideoSetInPoint {
        deck_uuid: String,
        secs: f64,
    },
    VideoSetOutPoint {
        deck_uuid: String,
        secs: f64,
    },
    VideoClearInOutPoints {
        deck_uuid: String,
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

    // ── External I/O Deck Sources ──────────────────────────────
    AddNdiDeck {
        channel_uuid: String,
        source_name: String,
    },
    AddSyphonDeck {
        channel_uuid: String,
        server_name: String,
    },
    AddSrtDeck {
        channel_uuid: String,
        url: String,
        mode: crate::stream::SrtMode,
    },
    AddHlsDeck {
        channel_uuid: String,
        url: String,
    },
    AddDashDeck {
        channel_uuid: String,
        url: String,
    },
    AddRtmpDeck {
        channel_uuid: String,
        url: String,
        mode: crate::stream::RtmpMode,
    },
    AddHtmlDeck {
        channel_uuid: String,
        url: String,
    },
    ReloadHtmlDeck {
        deck_uuid: String,
    },
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
    // Steps are positional within their sequence: `step_idx` is an ordinal, not
    // an address. See `/spec/api-addressing.md`.
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
    AddHtmlLibraryEntry {
        url: String,
    },
    RemoveHtmlLibraryEntry {
        url: String,
    },

    // ── Output ─────────────────────────────────────────────────
    CreateOutput,
    CreateHeadlessOutput {
        target: crate::engine::value::render::OutputTarget,
    },
    CloseOutput {
        output_uuid: String,
    },
    SetOutputDisplay {
        output_uuid: String,
        monitor_name: String,
    },
    SetOutputTarget {
        output_uuid: String,
        target: crate::engine::value::render::OutputTarget,
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
    /// Import surfaces from a stage-plan file (image/SVG/DXF): detect contours
    /// and create surfaces. Composite of detect + confirm.
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
    // Absolute show position. See /spec/transport.md.
    /// Start the show position advancing. Rejected while chasing timecode.
    TransportPlay,
    /// Hold the show position. Anything reading it freezes rather than
    /// releasing, so a stop keeps the current look.
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
    /// Frame rate positions are displayed and quantised at.
    SetTimecodeRate {
        rate: crate::transport::TimecodeRate,
    },
    /// Locate to the cue before the playhead, or to zero when there is none.
    TransportPrevCue,
    /// Locate to the cue after the playhead, or stay put when there is none.
    TransportNextCue,
    /// Locate to one named cue, leaving the transport running or stopped as it
    /// was. What the Performance-mode cue bank's buttons send.
    TriggerCue {
        uuid: String,
    },

    // ── Arrangement ────────────────────────────────────────────
    // Deck activity positioned against transport time. See /spec/arrangement.md.
    /// Give a deck a row in the arrangement. Idempotent: a lane *is* the deck.
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
    /// Fold a lane's automation rows away. View state, but it belongs to the
    /// scene: which curves a show wants open is a property of the show.
    SetLaneCollapsed {
        deck_uuid: String,
        collapsed: bool,
    },
    /// What renders before the transport reaches the arranged range.
    SetIdleBehaviour {
        idle: crate::arrangement::IdleBehaviour,
    },
    /// Hand one overridden parameter back to the arrangement, ramping over
    /// `seconds` rather than snapping.
    RearmParam {
        param_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<f64>,
    },
    /// Hand every overridden parameter back at once.
    RearmAll {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seconds: Option<f64>,
    },
    /// Mark an instant worth returning to. Returns the cue's UUID.
    AddCue {
        at: f64,
        /// Left empty to be named by how many cues exist.
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
    /// Choose which notion of time a modulation source follows.
    /// See /spec/timebase.md.
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
    /// Live macro turn — fans out to all targets. Not undoable.
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
    RescanNdi,
    RescanSyphon,
    RescanCameras,
    RescanDepthSensors,
    /// Re-enumerate displays and windows. Manual: window lists churn constantly
    /// and polling them would thrash the library panel.
    RescanCaptureTargets,
    /// Trigger the platform screen-recording permission request.
    RequestScreenCapturePermission,
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

    // ── Resolution ─────────────────────────────────────────────
    SetRenderResolution {
        width: u32,
        height: u32,
    },

    /// Set the domemaster output size. Separate from the render resolution
    /// because a domemaster image is square by definition — it is sized by the
    /// dome's projector, not by the master canvas it samples from.
    SetDomemasterResolution {
        resolution: crate::engine::value::dome::DomemasterResolution,
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

    // ── Presets ────────────────────────────────────────────────
    /// Load a named deck preset as a new deck appended to a channel. Presets are
    /// addressed by name: the library is rescanned from disk, so its ordering is
    /// not stable across scans.
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

    // ── Persistence ────────────────────────────────────────────
    SaveWorkspace,
    LoadWorkspace,

    // ── History ─────────────────────────────────────────────────
    Undo,
    Redo,

    // ── System ──────────────────────────────────────────────────
    Shutdown,
}
