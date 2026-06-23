//! Projection layer — transforms `EngineState` into API response DTOs.
//!
//! Pure functions, no HTTP/axum dependency. This is the API consumer's
//! equivalent of `collect_ui_data()` in the UI consumer.

use crate::engine::types::*;
use serde::Serialize;
use utoipa::ToSchema;

/// Helper to read the engine state or return a 503-appropriate error.
pub fn read_state(
    engine_state: &std::sync::RwLock<Option<EngineState>>,
) -> Result<EngineState, StateReadError> {
    let guard = engine_state
        .read()
        .map_err(|_| StateReadError::LockPoisoned)?;
    guard.clone().ok_or(StateReadError::NotInitialized)
}

/// Errors when reading engine state.
#[derive(Debug)]
pub enum StateReadError {
    NotInitialized,
    LockPoisoned,
}

// ── Performance projection ──────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct PerformanceResponse {
    pub fps: f32,
    pub frame_count: u64,
    pub target_fps: u32,
}

// ── NDI / Syphon projections ────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct NdiResponse {
    pub available: bool,
    pub sources: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct SyphonResponse {
    pub available: bool,
    pub sources: Vec<String>,
}

// ── Scene projections ───────────────────────────────────────────────

#[derive(Serialize)]
pub struct SceneResponse {
    pub channels: Vec<ChannelSnapshot>,
    pub crossfader: f32,
    pub master_effects: Vec<EffectSnapshot>,
    pub active_transition_name: Option<String>,
    pub modulation: ModulationSnapshot,
    pub sequences: Vec<SequenceSnapshot>,
    pub streams: Vec<StreamReceiverSnapshot>,
}

pub fn project_scene(state: &EngineState) -> SceneResponse {
    SceneResponse {
        channels: state.mixer.channels.clone(),
        crossfader: state.mixer.crossfader,
        master_effects: state.mixer.master_effects.clone(),
        active_transition_name: state.mixer.active_transition_name.clone(),
        modulation: state.modulation.clone(),
        sequences: state.mixer.sequences.clone(),
        streams: state.stream_receivers.clone(),
    }
}

// ── Stage projections ───────────────────────────────────────────────

#[derive(Serialize)]
pub struct StageResponse {
    pub surfaces: Vec<SurfaceSnapshot>,
    pub outputs: Vec<OutputWindowSnapshot>,
    pub monitors: Vec<MonitorSnapshot>,
}

pub fn project_stage(state: &EngineState) -> StageResponse {
    StageResponse {
        surfaces: state.outputs.surfaces.clone(),
        outputs: state.outputs.windows.clone(),
        monitors: state.outputs.monitors.clone(),
    }
}

// ── Library projections ─────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct ShaderEntry {
    pub name: String,
    pub index: usize,
}

#[derive(Serialize, ToSchema)]
pub struct TransitionEntry {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct CameraEntry {
    pub name: String,
    pub id: CameraId,
}

#[derive(Serialize, ToSchema)]
pub struct NdiSourceEntry {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct SyphonSourceEntry {
    pub name: String,
}

#[derive(Serialize, ToSchema)]
pub struct MonitorEntry {
    pub name: String,
    pub index: usize,
    pub width: u32,
    pub height: u32,
}

// ── Lookup helpers ──────────────────────────────────────────────────

pub fn find_channel<'a>(state: &'a EngineState, uuid: &str) -> Option<&'a ChannelSnapshot> {
    state.mixer.channels.iter().find(|c| c.uuid == uuid)
}

pub fn find_deck<'a>(channel: &'a ChannelSnapshot, uuid: &str) -> Option<&'a DeckSnapshot> {
    channel.decks.iter().find(|d| d.uuid == uuid)
}

pub fn find_surface<'a>(state: &'a EngineState, uuid: &str) -> Option<&'a SurfaceSnapshot> {
    state.outputs.surfaces.iter().find(|s| s.uuid == uuid)
}

pub fn find_output<'a>(state: &'a EngineState, uuid: &str) -> Option<&'a OutputWindowSnapshot> {
    state.outputs.windows.iter().find(|o| o.uuid == uuid)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    pub(crate) fn make_test_state() -> EngineState {
        EngineState {
            mixer: MixerSnapshot {
                channels: vec![ChannelSnapshot {
                    idx: 0,
                    uuid: "ch-001".into(),
                    name: "Channel A".into(),
                    opacity: 1.0,
                    blend_mode: BlendMode::Normal,
                    decks: vec![DeckSnapshot {
                        idx: 0,
                        uuid: "dk-001".into(),
                        name: "Sine".into(),
                        opacity: 1.0,
                        effective_opacity: 1.0,
                        blend_mode: BlendMode::Normal,
                        solo: false,
                        mute: false,
                        scaling_mode: None,
                        generator: ShaderParamsSnapshot {
                            shader_name: "Sine".into(),
                            params: vec![],
                        },
                        effects: vec![],
                        video_playback: None,
                        auto_transition: None,
                        render_fps: DeckRenderFps::Auto,
                        effective_render_fps: 0.0,
                        render_cost_us: 0.0,
                        gpu_render_cost_us: 0.0,
                        fps: 60.0,
                        running_analyzers: vec![],
                    }],
                    effects: vec![],
                    render_time_ms: 0.5,
                    active_deck_count: 1,
                }],
                crossfader: 0.5,
                auto_crossfade_active: false,
                auto_crossfade_progress: 0.0,
                master_effects: vec![],
                active_transition_name: None,
                transition_names: vec!["Dissolve".into()],
                sequences: vec![],
                tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
                active_lut: None,
            },
            audio: AudioSnapshot {
                level: 0.0,
                bass: 0.0,
                mid: 0.0,
                treble: 0.0,
                bpm: Some(120.0),
                beat_phase: 0.0,
                enabled: true,
                devices: vec![],
                fft: vec![],
                sample_rate: 48000.0,
            },
            modulation: ModulationSnapshot {
                sources: vec![],
                current_values: Default::default(),
                assignments: Default::default(),
            },
            outputs: OutputSnapshot {
                windows: vec![OutputWindowSnapshot {
                    uuid: "out-001".into(),
                    name: "Output 1".into(),
                    target_label: "HDMI-1".into(),
                    is_on_display: true,
                    surface_assignments: vec![],
                    calibration_mode: false,
                }],
                surfaces: vec![SurfaceSnapshot {
                    uuid: "srf-001".into(),
                    name: "Main".into(),
                    vertices: vec![[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]],
                    extra_contours: vec![],
                    source: OutputSource::Master,
                    content_mapping: ContentMapping::Fill,
                    output_type: SurfaceOutputType::Projection,
                    circle_hint: None,
                    default_warp: None,
                }],
                monitors: vec![MonitorSnapshot {
                    name: "HDMI-1".into(),
                    index: 0,
                    width: 1920,
                    height: 1080,
                }],
            },
            registry: RegistrySnapshot {
                generators: vec![("Sine".into(), 0)],
                filters: vec![("Blur".into(), 0)],
                shader_count: 2,
            },
            midi: MidiSnapshot {
                devices: vec![],
                mappings: vec![],
                learn_active: false,
                learn_target: None,
            },
            cameras: CameraSnapshot {
                devices: vec![("FaceTime".into(), 0u32)],
            },
            clock: ClockSnapshot {
                bpm: Some(120.0),
                beat_phase: 0.0,
                source_label: "Audio".into(),
                device_name: None,
                active: true,
                detected_midi_sources: vec![],
                osc_active: false,
                osc_bpm: None,
                audio_bpm: Some(120.0),
                preference_label: "Auto".into(),
                preference_force_device_id: None,
                manual_bpm: None,
            },
            fps: 60.0,
            frame_count: 100,
            target_fps: 60,
            ndi_sources: vec!["OBS".into()],
            ndi_available: true,
            syphon_sources: vec![],
            syphon_available: false,
            stream_receivers: vec![],
            analyzers: vec![],
        }
    }

    #[test]
    fn test_read_state_not_initialized() {
        let lock = std::sync::RwLock::new(None);
        assert!(matches!(
            read_state(&lock),
            Err(StateReadError::NotInitialized)
        ));
    }

    #[test]
    fn test_read_state_returns_clone() {
        let state = make_test_state();
        let lock = std::sync::RwLock::new(Some(state));
        let result = read_state(&lock).unwrap();
        assert!((result.fps - 60.0).abs() < 1e-5);
    }

    #[test]
    fn test_project_scene() {
        let state = make_test_state();
        let scene = project_scene(&state);
        assert_eq!(scene.channels.len(), 1);
        assert!((scene.crossfader - 0.5).abs() < 1e-5);
        assert_eq!(scene.channels[0].uuid, "ch-001");
    }

    #[test]
    fn test_project_stage() {
        let state = make_test_state();
        let stage = project_stage(&state);
        assert_eq!(stage.surfaces.len(), 1);
        assert_eq!(stage.outputs.len(), 1);
        assert_eq!(stage.monitors.len(), 1);
    }

    #[test]
    fn test_find_channel() {
        let state = make_test_state();
        assert!(find_channel(&state, "ch-001").is_some());
        assert!(find_channel(&state, "nonexistent").is_none());
    }

    #[test]
    fn test_find_deck() {
        let state = make_test_state();
        let ch = find_channel(&state, "ch-001").unwrap();
        assert!(find_deck(ch, "dk-001").is_some());
        assert!(find_deck(ch, "nonexistent").is_none());
    }

    #[test]
    fn test_find_surface() {
        let state = make_test_state();
        assert!(find_surface(&state, "srf-001").is_some());
        assert!(find_surface(&state, "nonexistent").is_none());
    }

    #[test]
    fn test_find_output() {
        let state = make_test_state();
        assert!(find_output(&state, "out-001").is_some());
        assert!(find_output(&state, "nonexistent").is_none());
    }
}
