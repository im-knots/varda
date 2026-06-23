//! Domain-specific traits for engine operations.
//!
//! Interface Segregation: one trait pair (Commands + Queries) per domain.
//! Consumers import only what they need.
//!
//! Traits MUST NOT expose wgpu, egui, or internal implementation types.
//! Parameters use primitives, strings, and engine-defined value types.

use super::types::*;
use anyhow::Result;

// ── Mixer ───────────────────────────────────────────────────────────

/// Commands for controlling the mixer, channels, decks, and effects.
pub trait MixerCommands {
    fn set_crossfader(&mut self, position: f32);
    fn start_auto_crossfade(&mut self, target: f32, duration_secs: f32, easing: CrossfadeEasing);
    fn start_beat_crossfade(&mut self, target: f32, beats: f32);
    fn add_deck(&mut self, channel_idx: usize, shader_name: &str) -> Result<()>;
    fn add_image_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()>;
    fn add_video_deck(&mut self, channel_idx: usize, path: &std::path::Path) -> Result<()>;
    fn add_solid_color_deck(&mut self, channel_idx: usize, color: [f32; 4]) -> Result<()>;
    fn add_camera_deck(&mut self, channel_idx: usize, camera_id: CameraId) -> Result<()>;
    fn remove_deck(&mut self, channel_idx: usize, deck_idx: usize) -> Result<()>;
    fn move_deck(&mut self, src_ch: usize, src_deck: usize, dst_ch: usize) -> Result<()>;
    fn reorder_deck(&mut self, ch: usize, from_idx: usize, to_idx: usize);
    fn set_deck_opacity(&mut self, channel_idx: usize, deck_idx: usize, opacity: f32);
    fn set_deck_blend_mode(&mut self, channel_idx: usize, deck_idx: usize, mode: BlendMode);
    fn set_deck_solo(&mut self, channel_idx: usize, deck_idx: usize, solo: bool);
    fn set_deck_mute(&mut self, channel_idx: usize, deck_idx: usize, mute: bool);
    fn set_deck_scaling_mode(&mut self, channel_idx: usize, deck_idx: usize, mode: ScalingMode);
    fn set_channel_opacity(&mut self, channel_idx: usize, opacity: f32);
    fn set_channel_blend_mode(&mut self, channel_idx: usize, mode: BlendMode);
    fn add_channel(&mut self) -> Result<usize>;
    fn remove_channel(&mut self, channel_idx: usize) -> Result<()>;
    fn add_effect(&mut self, target: EffectTarget, shader_name: &str) -> Result<()>;
    fn remove_effect(&mut self, target: EffectTarget, effect_idx: usize);
    fn toggle_effect(&mut self, target: EffectTarget, effect_idx: usize);
    fn move_effect(&mut self, target: EffectTarget, from_idx: usize, to_idx: usize);
    fn set_transition(&mut self, shader_name: Option<&str>) -> Result<()>;
    fn set_tonemap_mode(&mut self, mode: crate::renderer::tonemap::TonemapMode);
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
}

/// Read-only queries for modulation state.
pub trait ModulationQueries {
    fn modulation_snapshot(&self) -> ModulationSnapshot;
}

// ── Output ──────────────────────────────────────────────────────────

/// Commands for controlling outputs and surfaces.
pub trait OutputCommands {
    fn request_create_output(&mut self);
    fn close_output(&mut self, idx: usize);
    fn set_output_display(&mut self, idx: usize, monitor_name: &str);
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
    fn unassign_surface_from_output(&mut self, output_uuid: &str, assignment_idx: usize);
}

/// Commands for surface auto-detection and import.
pub trait DetectCommands {
    /// Detect contours from raster image bytes.
    fn detect_from_image(
        &self,
        image_data: &[u8],
        params: &crate::surface::detect::DetectionParams,
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError>;
    /// Detect contours from SVG data.
    fn detect_from_svg(
        &self,
        svg_data: &[u8],
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError>;
    /// Detect contours from DXF data.
    fn detect_from_dxf(
        &self,
        dxf_data: &[u8],
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError>;
    /// Detect contours from a camera snapshot (RGBA frame data).
    fn detect_from_camera(
        &mut self,
        camera_id: CameraId,
        params: &crate::surface::detect::DetectionParams,
    ) -> Result<crate::surface::detect::DetectionResult, crate::surface::import::ImportError>;
    /// Create surfaces from confirmed detected contours.
    fn confirm_detected_contours(
        &mut self,
        contours: &[crate::surface::detect::DetectedContour],
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
    fn request_analyzer(
        &mut self,
        deck_id: &str,
        analyzer_type: &str,
        options: &serde_json::Value,
    ) -> anyhow::Result<()>;

    /// Release an analyzer on a deck. Stops it when refcount reaches zero.
    fn release_analyzer(&mut self, deck_id: &str, analyzer_type: &str);
}
