//! View-model fixtures shared by panel tests, snapshot tests and the
//! `test-fixtures` feature.

use super::{
    AudioUIData, CameraDetectMode, ChannelRenderStats, ChannelUIInfo, DeckUIInfo, ModAssignmentUI,
    ModSourceUI, ModSourceUIEntry, ParamUIInfo, ShaderParamsUI, SurfaceUI, UIData,
};
use crate::BlendMode;
use crate::channel::DeckRenderFps;
use crate::engine::value::provider::{
    ControlStatus, LibraryEntry, LibrarySection, ProviderTypeSnapshot,
};
use crate::engine::value::source::{DeckSourceSnapshot, SourceConfig};
use crate::renderer::context::OutputSource;
use crate::renderer::slicer::{DomeGeometry, DomePreset};
use crate::surface::{ContentMapping, SurfaceOutputType};

/// A shader deck's source, as the fixture decks carry it.
#[cfg(any(test, feature = "test-fixtures"))]
fn shader_source() -> DeckSourceSnapshot {
    DeckSourceSnapshot {
        source_type: "Shader".into(),
        available: true,
        owns_alpha: false,
        status: ControlStatus::default(),
    }
}

/// The shader source type listing `generators`, then the image, video and
/// camera types as their providers describe themselves with no devices.
#[cfg(any(test, feature = "test-fixtures"))]
fn fixture_source_types(generators: &[&str]) -> Vec<ProviderTypeSnapshot> {
    let shaders = crate::registry::ShaderRegistry::new();
    let services = crate::source::Services::new();
    let mut providers = crate::source::SourceRegistry::new();
    providers
        .register(crate::still::ImageProvider)
        .register(crate::video::provider::VideoProvider)
        .register(crate::camera::provider::CameraProvider::new());
    let query = crate::source::SourceQuery {
        services: &services,
        shaders: &shaders,
        channels: &[],
    };
    let mut types = vec![fixture_shader_type(generators)];
    types.extend(providers.type_snapshots(&query));
    types
}

/// Every output sink type, as a windowed run with no monitors describes them.
#[cfg(any(test, feature = "test-fixtures"))]
fn fixture_sink_types() -> Vec<ProviderTypeSnapshot> {
    let services = crate::source::Services::new().with(crate::output::window::Monitors::default());
    crate::app::sources::output_sinks().type_snapshots(&crate::output::SinkQuery {
        services: &services,
    })
}

/// The shader source type with `generators` in its library.
#[cfg(any(test, feature = "test-fixtures"))]
fn fixture_shader_type(generators: &[&str]) -> ProviderTypeSnapshot {
    use crate::source::DeckSourceProvider;
    let provider = crate::generator::ShaderProvider;
    ProviderTypeSnapshot {
        type_id: provider.id().into(),
        label: provider.label().into(),
        icon: provider.icon().into(),
        available: true,
        unavailable_reason: None,
        listed: true,
        params: vec![],
        library: LibrarySection {
            entries: generators
                .iter()
                .map(|name| {
                    LibraryEntry::new(*name, SourceConfig::new("Shader").with("name", name))
                })
                .collect(),
            ..LibrarySection::default()
        },
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl SurfaceUI {
    /// An axis-aligned quad surface with no warp, for UI tests.
    ///
    /// Vertices wind clockwise from the top-left: `(x, y)` → `(x+w, y)` →
    /// `(x+w, y+h)` → `(x, y+h)`.
    pub fn test_quad(uuid: &str, x: f32, y: f32, w: f32, h: f32) -> Self {
        Self {
            uuid: uuid.to_string(),
            name: format!("Surface {uuid}"),
            vertices: vec![[x, y], [x + w, y], [x + w, y + h], [x, y + h]],
            extra_contours: vec![],
            source: OutputSource::Master,
            content_mapping: ContentMapping::Fill,
            output_type: SurfaceOutputType::Projection,
            circle_hint: None,
            warp: None,
            warp_bound: false,
            path: None,
            holes: vec![],
            hole_contours: vec![],
        }
    }
}

#[cfg(any(test, feature = "test-fixtures"))]
impl UIData {
    /// Representative fixture for UI tests.
    ///
    /// 2 channels with 2 decks each, effects, modulation, crossfader at 0.5,
    /// library panel open, deck (0,0) selected, and empty MIDI, audio, surface and
    /// sequence collections.
    pub fn test_fixture() -> Self {
        use crate::modulation::LFOWaveform;

        let alpha_lower = DeckUIInfo {
            deck_idx: 0,
            uuid: "a0000001".to_string(),
            name: "test_generator_a".to_string(),
            source: shader_source(),
            depth_prepro: None,
            is_interactive: false,
            opacity: 1.0,
            effective_opacity: 1.0,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            generator: ShaderParamsUI {
                shader_name: "test_generator_a".to_string(),
                params: vec![ParamUIInfo {
                    name: "speed".to_string(),
                    label: Some("Speed".to_string()),
                    value: crate::params::ParamValue::Float(1.0),
                    min: Some(0.0),
                    max: Some(5.0),
                    group: None,
                    choices: Vec::new(),
                }],
            },
            effects: vec![(
                "dfx00001".to_string(),
                "test_effect".to_string(),
                true,
                ShaderParamsUI {
                    shader_name: "test_effect".to_string(),
                    params: vec![ParamUIInfo {
                        name: "amount".to_string(),
                        label: Some("Amount".to_string()),
                        value: crate::params::ParamValue::Float(0.5),
                        min: Some(0.0),
                        max: Some(1.0),
                        group: None,
                        choices: Vec::new(),
                    }],
                },
            )],
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let alpha_upper = DeckUIInfo {
            deck_idx: 1,
            uuid: "a0000002".to_string(),
            name: "test_generator_b".to_string(),
            source: shader_source(),
            depth_prepro: None,
            is_interactive: false,
            opacity: 0.8,
            effective_opacity: 0.8,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            generator: ShaderParamsUI {
                shader_name: "test_generator_b".to_string(),
                params: vec![],
            },
            effects: vec![],
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let channel_a = ChannelUIInfo {
            ch_idx: 0,
            uuid: "ca000001".to_string(),
            name: "Ch A".to_string(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            decks: vec![alpha_lower, alpha_upper],
            effects: vec![(
                "cfx00001".to_string(),
                "ch_effect".to_string(),
                true,
                ShaderParamsUI {
                    shader_name: "ch_effect".to_string(),
                    params: vec![],
                },
            )],
        };

        let beta_lower = DeckUIInfo {
            deck_idx: 0,
            uuid: "b0000001".to_string(),
            name: "test_generator_c".to_string(),
            source: shader_source(),
            depth_prepro: None,
            is_interactive: false,
            opacity: 1.0,
            effective_opacity: 1.0,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            generator: ShaderParamsUI {
                shader_name: "test_generator_c".to_string(),
                params: vec![],
            },
            effects: vec![],
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let beta_upper = DeckUIInfo {
            deck_idx: 1,
            uuid: "b0000002".to_string(),
            name: "test_generator_d".to_string(),
            source: shader_source(),
            depth_prepro: None,
            is_interactive: false,
            opacity: 1.0,
            effective_opacity: 1.0,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            transparent: false,
            generator: ShaderParamsUI {
                shader_name: "test_generator_d".to_string(),
                params: vec![],
            },
            effects: vec![],
            auto_transition: None,
            render_fps: DeckRenderFps::Auto,
            effective_render_fps: 0.0,
            render_cost_us: 0.0,
            gpu_render_cost_us: 0.0,
        };

        let channel_b = ChannelUIInfo {
            ch_idx: 1,
            uuid: "cb000001".to_string(),
            name: "Ch B".to_string(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            decks: vec![beta_lower, beta_upper],
            effects: vec![],
        };

        UIData {
            filters: vec![
                ("test_effect".to_string(), 0),
                ("ch_effect".to_string(), 1),
                ("master_effect".to_string(), 2),
            ],
            shader_count: 7,
            channels: vec![channel_a, channel_b],
            master_effect_info: vec![(
                "mfx00001".to_string(),
                "master_effect".to_string(),
                true,
                ShaderParamsUI {
                    shader_name: "master_effect".to_string(),
                    params: vec![],
                },
            )],
            modulation_sources: vec![ModSourceUIEntry {
                uuid: "mod00001".to_string(),
                source: ModSourceUI::LFO {
                    waveform: LFOWaveform::Sine,
                    frequency: 1.0,
                    phase: 0.0,
                    amplitude: 1.0,
                    bipolar: false,
                },
                timebase: crate::timebase::Timebase::FreeRun,
            }],
            modulation_current_values: {
                let mut m = std::collections::HashMap::new();
                m.insert("mod00001".to_string(), 0.5);
                m
            },
            modulation_assignments: {
                let mut m = std::collections::HashMap::new();
                m.insert(
                    "deck/a0000001/param/speed".to_string(),
                    vec![ModAssignmentUI {
                        source_id: "mod00001".to_string(),
                        amount: 0.5,
                    }],
                );
                m
            },
            macros: Vec::new(),
            audio: AudioUIData {
                level: 0.0,
                bass: 0.0,
                mid: 0.0,
                treble: 0.0,
                bpm: None,
                beat_phase: 0.0,
                enabled: false,
                devices: vec![],
                fft: vec![0.0; 256],
                sample_rate: 44100.0,
            },
            deck_preview_textures: std::collections::HashMap::new(),
            channel_preview_textures: std::collections::HashMap::new(),
            output_preview_textures: std::collections::HashMap::new(),
            main_output_texture: None,
            notifications: vec![],
            crossfader: 0.5,
            auto_crossfade_active: false,
            auto_crossfade_progress: 0.0,
            tonemap_mode: crate::renderer::tonemap::TonemapMode::default(),
            active_lut_filename: None,
            look_lut_filename: None,
            available_luts: std::sync::Arc::default(),
            midi_learn_active: false,
            midi_learn_target: None,
            keyboard_learn_active: false,
            keyboard_learn_target: None,
            keymap_bindings: std::collections::HashMap::new(),
            transition_names: vec!["fade".to_string()],
            active_transition_name: None,
            selected_deck: Some((0, 0)),
            selected_channel: None,
            selected_master: false,
            selected_sequence: None,
            selected_sequence_step: None,
            selected_macro: None,
            outputs: vec![],
            surfaces: vec![],
            stage_editor_open: false,
            arrangement_mode_open: false,
            arrangement: None,
            arrangement_pixels_per_second: 40.0,
            arrangement_scroll: 0.0,
            arrangement_scroll_y: 0.0,
            arrangement_snap: true,
            arrangement_focus: None,
            clipboard: None,
            dome_preview_open: false,
            dome_preview_texture: None,
            dome_mode_active: false,
            dome_preset: DomePreset::Quad,
            dome_geometry: DomeGeometry::default(),
            domemaster_resolution: crate::renderer::dome::DomemasterResolution::default(),
            camera_detect_texture: None,
            camera_detect_mode: CameraDetectMode::Off,
            camera_detect_contours: vec![],
            library_panel_open: true,
            right_panel_open: true,
            stage_editor_grid_size: 0.05,
            stage_editor_snap: true,
            available_monitors: vec![],
            midi_devices: vec![],
            midi_mappings: vec![],
            cameras: vec![],
            sinks: std::sync::Arc::new(fixture_sink_types()),
            sources: std::sync::Arc::new(fixture_source_types(&[
                "test_generator_a",
                "test_generator_b",
                "test_generator_c",
                "test_generator_d",
            ])),

            sequences: vec![],
            channel_count: 2,
            fps: 60.0,
            channel_render_stats: vec![
                ChannelRenderStats {
                    name: "Ch A".to_string(),
                    avg_deck_fps: 60.0,
                    active_deck_count: 2,
                    render_time_ms: 1.5,
                },
                ChannelRenderStats {
                    name: "Ch B".to_string(),
                    avg_deck_fps: 58.0,
                    active_deck_count: 1,
                    render_time_ms: 0.8,
                },
            ],
            gpu_device_name: "Test GPU".to_string(),
            gpu_backend: "Metal".to_string(),
            gpu_driver: "Apple".to_string(),
            gpu_driver_info: "Metal 3".to_string(),
            gpu_device_type: "IntegratedGpu".to_string(),
            gpu_utilization: 45.0,
            cpu_usage: 25.0,
            ram_used: 8 * 1024 * 1024 * 1024,
            ram_total: 16 * 1024 * 1024 * 1024,
            clock_source: "Audio".to_string(),
            clock_bpm: None,
            clock_active: false,
            clock_device_name: None,
            clock_detected_midi: vec![],
            clock_osc_active: false,
            clock_osc_bpm: None,
            clock_audio_bpm: None,
            clock_preference: "Auto".to_string(),
            clock_preference_force_device_id: None,
            clock_manual_bpm: None,
            clock_beat_followers: 0,
            transport: crate::engine::types::TransportSnapshot::default(),
            timecode: crate::engine::types::TimecodeSnapshot::default(),
            render_width: 1920,
            render_height: 1080,
            max_render_dimension: 16384,
            target_fps: 60,
            can_undo: false,
            can_redo: false,
            pending_deck_loads: 0,
            deck_presets: vec![],
            channel_presets: vec![],
        }
    }
}
