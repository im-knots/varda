//! Domain-specific traits for engine operations.
//!
//! Interface Segregation: one trait pair (Commands + Queries) per domain.
//! Consumers import only what they need.
//!
//! Traits MUST NOT expose wgpu, egui, or internal implementation types.
//! Parameters use primitives, strings, and engine-defined value types.

use super::types::{
    AnalyzerTypeInfo, AudioBandPreset, AudioSnapshot, AudioSourceId, BlendMode, CameraId,
    ContentMapping, CrossfadeEasing, DepthSensorId, EffectTarget, LFOWaveform, MixerSnapshot,
    ModulationSnapshot, OutputSnapshot, OutputSource, ParamValue, ScalingMode, SurfaceOutputType,
    SurfaceSnapshot,
};
use anyhow::Result;

// ── Mixer ───────────────────────────────────────────────────────────

/// Commands for controlling the mixer, channels, decks, and effects.
pub trait MixerCommands {
    fn set_crossfader(&mut self, position: f32);
    fn start_auto_crossfade(&mut self, target: f32, duration_secs: f32, easing: CrossfadeEasing);
    fn start_beat_crossfade(&mut self, target: f32, beats: f32);
    /// Deck-creating commands take the parent channel's UUID (there is no deck
    /// UUID yet) and return the new deck's stable UUID so callers can report it
    /// (`CommandResult::OkWithId`) and register a preview texture.
    ///
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, if no generator
    /// shader called `shader_name` is registered, if the GPU pipeline for the
    /// shader cannot be built, or if a required preprocessor the shader
    /// declares (e.g. `depth_sensor`) cannot be acquired.
    fn add_deck(&mut self, channel_uuid: &str, shader_name: &str) -> Result<String>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, or if the image at
    /// `path` cannot be read, decoded, or uploaded to the GPU.
    fn add_image_deck(&mut self, channel_uuid: &str, path: &std::path::Path) -> Result<String>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, or if the video at
    /// `path` cannot be opened or decoded.
    fn add_video_deck(&mut self, channel_uuid: &str, path: &std::path::Path) -> Result<String>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, or if the deck's
    /// GPU resources cannot be allocated.
    fn add_solid_color_deck(&mut self, channel_uuid: &str, color: [f32; 4]) -> Result<String>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, if the camera
    /// cannot be opened, or if the deck's GPU resources cannot be allocated.
    fn add_camera_deck(&mut self, channel_uuid: &str, camera_id: CameraId) -> Result<String>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, if the depth sensor
    /// cannot be opened, or if the deck's GPU resources cannot be allocated.
    fn add_depth_sensor_deck(
        &mut self,
        channel_uuid: &str,
        depth_sensor_id: DepthSensorId,
    ) -> Result<String>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck.
    fn remove_deck(&mut self, deck_uuid: &str) -> Result<()>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck or `dst_channel_uuid`
    /// names no channel.
    fn move_deck(&mut self, deck_uuid: &str, dst_channel_uuid: &str) -> Result<()>;
    /// Reposition a deck within `channel_uuid`. The indices are ordinals.
    ///
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, or if either
    /// ordinal is out of range for that channel's deck count.
    fn reorder_deck(&mut self, channel_uuid: &str, from_idx: usize, to_idx: usize) -> Result<()>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck.
    fn set_deck_opacity(&mut self, deck_uuid: &str, opacity: f32) -> Result<()>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck.
    fn set_deck_blend_mode(&mut self, deck_uuid: &str, mode: BlendMode) -> Result<()>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck.
    fn set_deck_solo(&mut self, deck_uuid: &str, solo: bool) -> Result<()>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck.
    fn set_deck_mute(&mut self, deck_uuid: &str, mute: bool) -> Result<()>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck.
    fn set_deck_scaling_mode(&mut self, deck_uuid: &str, mode: ScalingMode) -> Result<()>;
    /// # Errors
    /// Returns an error if `deck_uuid` names no deck.
    fn set_deck_transparent(&mut self, deck_uuid: &str, transparent: bool) -> Result<()>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel.
    fn set_channel_opacity(&mut self, channel_uuid: &str, opacity: f32) -> Result<()>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel.
    fn set_channel_blend_mode(&mut self, channel_uuid: &str, mode: BlendMode) -> Result<()>;
    /// Returns the new channel's UUID.
    ///
    /// # Errors
    /// Returns an error if the channel's GPU render targets cannot be
    /// allocated.
    fn add_channel(&mut self) -> Result<String>;
    /// # Errors
    /// Returns an error if `channel_uuid` names no channel, or if removing it
    /// would drop below the two-channel minimum the crossfader requires.
    fn remove_channel(&mut self, channel_uuid: &str) -> Result<()>;
    /// # Errors
    /// Returns an error if `target` names no deck or channel, if no filter
    /// shader called `shader_name` is registered, if the effect's GPU pipeline
    /// cannot be built, if the shader needs a depth sensor that cannot be
    /// acquired, or if a depth-sensor effect is targeted at a channel or master
    /// chain (only deck chains can host one).
    fn add_effect(&mut self, target: EffectTarget, shader_name: &str) -> Result<String>;
    /// # Errors
    /// Returns an error if `effect_uuid` names no effect.
    fn remove_effect(&mut self, effect_uuid: &str) -> Result<()>;
    /// # Errors
    /// Returns an error if `effect_uuid` names no effect.
    fn toggle_effect(&mut self, effect_uuid: &str) -> Result<()>;
    /// # Errors
    /// Returns an error if `target` names no deck or channel, or if either
    /// ordinal is out of range for that chain's effect count.
    fn move_effect(&mut self, target: EffectTarget, from_idx: usize, to_idx: usize) -> Result<()>;
    /// # Errors
    /// Returns an error if `shader_name` names no registered transition shader,
    /// or if its GPU pipeline cannot be built. `None` clears the transition and
    /// never fails.
    fn set_transition(&mut self, shader_name: Option<&str>) -> Result<()>;
    fn set_tonemap_mode(&mut self, mode: crate::engine::value::render::TonemapMode);
    /// # Errors
    /// Returns an error if `filename` does not exist under the workspace's
    /// `luts/` directory or cannot be parsed as a supported LUT file.
    fn load_lut(&mut self, filename: &str) -> Result<()>;
    fn unload_lut(&mut self);
    fn set_param(&mut self, path: &str, value: ParamValue);
}

/// Read-only queries for mixer state.
pub trait MixerQueries {
    fn mixer_snapshot(&self) -> MixerSnapshot;
}

// ── Audio ───────────────────────────────────────────────────────────

/// Commands for controlling audio input.
pub trait AudioCommands {
    /// # Errors
    /// Returns an error if the device or loopback source cannot be opened —
    /// it has disappeared since the last scan, is already in exclusive use, or
    /// exposes no supported input stream configuration.
    fn open_audio_source(&mut self, source_id: AudioSourceId) -> Result<()>;
    fn close_audio_source(&mut self, source_id: AudioSourceId);
    fn scan_audio_devices(&mut self);
}

/// Read-only queries for audio state.
pub trait AudioQueries {
    fn audio_snapshot(&self) -> AudioSnapshot;
}

// ── Modulation ──────────────────────────────────────────────────────

/// Commands for controlling the modulation engine.
pub trait ModulationCommands {
    fn add_lfo(&mut self, waveform: LFOWaveform, frequency: f32) -> String;
    fn add_audio_band(
        &mut self,
        preset: AudioBandPreset,
        source_id: Option<AudioSourceId>,
    ) -> String;
    fn add_adsr(&mut self, attack: f32, decay: f32, sustain: f32, release: f32) -> String;
    fn add_step_sequencer(&mut self, num_steps: usize, rate: f32) -> String;
    fn remove_modulation_source(&mut self, uuid: &str);
    fn assign_modulation(&mut self, target: &str, source_id: &str, amount: f32);
    fn clear_modulation(&mut self, target: &str);
    fn clear_modulation_source(&mut self, target: &str, source_id: &str);
}

/// Read-only queries for modulation state.
pub trait ModulationQueries {
    fn modulation_snapshot(&self) -> ModulationSnapshot;
}

// ── Macros ──────────────────────────────────────────────────────────

/// Commands for the macro bank (one control → many parameter targets).
///
/// Config mutations (`add`/`remove`/`rename`/`kind`/target edits/button config)
/// are undoable via the scene snapshot. `set_macro_value` is a live performance
/// turn (fans out to targets) and is intentionally **not** undoable.
pub trait MacroCommands {
    /// Add a macro of `kind`; returns its UUID.
    fn add_macro(&mut self, kind: crate::macros::MacroKind) -> String;
    fn remove_macro(&mut self, uuid: &str);
    fn rename_macro(&mut self, uuid: &str, name: &str);
    fn set_macro_kind(&mut self, uuid: &str, kind: crate::macros::MacroKind);
    /// Drive a macro's value 0..1, fanning out to all targets (not undoable).
    fn set_macro_value(&mut self, uuid: &str, value: f32);
    /// Append a target on `path` (full-range linear by default).
    fn add_macro_target(&mut self, uuid: &str, path: &str);
    fn remove_macro_target(&mut self, uuid: &str, target_idx: usize);
    #[allow(clippy::too_many_arguments)]
    fn update_macro_target(
        &mut self,
        uuid: &str,
        target_idx: usize,
        min: f32,
        max: f32,
        curve: crate::macros::MacroCurve,
        invert: bool,
    );
    fn set_macro_button_behavior(&mut self, uuid: &str, behavior: crate::macros::ButtonBehavior);
    fn set_macro_triggers(&mut self, uuid: &str, actions: Vec<crate::macros::TriggerAction>);
}

/// Read-only queries for macro state.
pub trait MacroQueries {
    fn macro_snapshot(&self) -> Vec<crate::macros::Macro>;
}

// ── Output ──────────────────────────────────────────────────────────

/// Commands for controlling outputs and surfaces.
pub trait OutputCommands {
    fn request_create_output(&mut self);
    /// # Errors
    /// Returns an error if `output_uuid` names no output.
    fn close_output(&mut self, output_uuid: &str) -> Result<()>;
    /// # Errors
    /// Returns an error if `output_uuid` names no output, or if no monitor
    /// called `monitor_name` is present in the cached monitor list.
    fn set_output_display(&mut self, output_uuid: &str, monitor_name: &str) -> Result<()>;
}

/// Read-only queries for output state.
pub trait OutputQueries {
    fn output_snapshot(&self) -> OutputSnapshot;
}

// ── Surfaces ────────────────────────────────────────────────────────

/// Commands for controlling surfaces.
pub trait SurfaceCommands {
    fn add_surface(&mut self, name: &str, source: OutputSource) -> String;
    fn add_polygon_surface(
        &mut self,
        name: &str,
        vertices: &[[f32; 2]],
        source: OutputSource,
    ) -> String;
    fn add_circle_surface(
        &mut self,
        name: &str,
        center: [f32; 2],
        radius: f32,
        sides: u32,
        aspect_ratio: f32,
        source: OutputSource,
    ) -> String;
    fn remove_surface(&mut self, uuid: &str);
    fn set_surface_source(&mut self, uuid: &str, source: OutputSource);
    fn set_surface_output_type(&mut self, uuid: &str, output_type: SurfaceOutputType);
    fn set_surface_content_mapping(&mut self, uuid: &str, mapping: ContentMapping);
    fn rename_surface(&mut self, uuid: &str, name: &str);
    fn assign_surface_to_output(&mut self, output_uuid: &str, surface_uuid: &str);
    fn unassign_surface_from_output(&mut self, output_uuid: &str, surface_uuid: &str);
}

/// Commands for surface auto-detection and import.
pub trait DetectCommands {
    /// Detect contours from raster image bytes.
    ///
    /// # Errors
    /// Returns [`ImportError`](crate::engine::value::detect::ImportError) if
    /// the bytes are not a decodable image, or if contour tracing produces no
    /// usable geometry at the given `params`.
    fn detect_from_image(
        &self,
        image_data: &[u8],
        params: &crate::engine::value::detect::DetectionParams,
    ) -> Result<
        crate::engine::value::detect::DetectionResult,
        crate::engine::value::detect::ImportError,
    >;
    /// Detect contours from SVG data.
    ///
    /// # Errors
    /// Returns [`ImportError`](crate::engine::value::detect::ImportError) if
    /// the bytes are not well-formed SVG or contain no convertible paths.
    fn detect_from_svg(
        &self,
        svg_data: &[u8],
    ) -> Result<
        crate::engine::value::detect::DetectionResult,
        crate::engine::value::detect::ImportError,
    >;
    /// Detect contours from DXF data.
    ///
    /// # Errors
    /// Returns [`ImportError`](crate::engine::value::detect::ImportError) if
    /// the bytes are not parseable DXF or contain no convertible entities.
    fn detect_from_dxf(
        &self,
        dxf_data: &[u8],
    ) -> Result<
        crate::engine::value::detect::DetectionResult,
        crate::engine::value::detect::ImportError,
    >;
    /// Detect contours from a camera snapshot (RGBA frame data).
    ///
    /// # Errors
    /// Returns [`ImportError`](crate::engine::value::detect::ImportError) if
    /// the camera cannot be opened, if no frame arrives within the capture
    /// budget, or if contour tracing fails on the captured frame.
    fn detect_from_camera(
        &mut self,
        camera_id: CameraId,
        params: &crate::engine::value::detect::DetectionParams,
    ) -> Result<
        crate::engine::value::detect::DetectionResult,
        crate::engine::value::detect::ImportError,
    >;
    /// Create surfaces from confirmed detected contours.
    fn confirm_detected_contours(
        &mut self,
        contours: &[crate::engine::value::detect::DetectedContour],
    ) -> Vec<String>;
}

/// Read-only queries for surface state.
pub trait SurfaceQueries {
    fn surface_snapshot(&self) -> Vec<SurfaceSnapshot>;
}

// ── Analyzers ──────────────────────────────────────────────────────

/// Read-only queries for analyzer state.
pub trait AnalyzerQueries {
    /// List available analyzer types and their output schemas.
    fn available_analyzers(&self) -> Vec<AnalyzerTypeInfo>;

    /// Check if an analyzer is running on a specific deck.
    fn is_analyzer_running(&self, deck_id: &str, analyzer_type: &str) -> bool;
}

/// Commands for managing analyzer lifecycle on decks.
pub trait AnalyzerCommands {
    /// Request an analyzer on a deck. If already running, increments refcount.
    ///
    /// # Errors
    /// Returns an error if `deck_id` names no deck, or if `analyzer_type` is
    /// not in the analyzer registry or rejects `options` and so cannot start.
    fn request_analyzer(
        &mut self,
        deck_id: &str,
        analyzer_type: &str,
        options: &serde_json::Value,
    ) -> anyhow::Result<()>;

    /// Release an analyzer on a deck. Stops it when refcount reaches zero.
    fn release_analyzer(&mut self, deck_id: &str, analyzer_type: &str);
}
