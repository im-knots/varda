//! ApiRunner — HTTP/WS delivery layer for the Varda engine.
//!
//! Owns the axum server and tokio runtime. Runs on a background thread.
//! For windowed operation this runs alongside UIRunner.
//! For headless operation this is the primary consumer.

use crate::engine::{CommandEnvelope, EngineState};
use crate::usecases::api::routes;
use crate::usecases::api::SharedState;

use axum::Router;
use std::sync::{Arc, RwLock};
use tokio::sync::mpsc;
use tower_http::cors::{Any, CorsLayer};
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

#[derive(OpenApi)]
#[openapi(
    info(title = "Varda API", version = "0.1.0", description = "Real-time visual engine HTTP API"),
    paths(
        // System
        routes::system::health, routes::system::get_state,
        routes::system::shutdown, routes::system::undo, routes::system::redo,
        routes::system::set_resolution, routes::system::set_clock_preference,
        routes::system::set_manual_bpm, routes::system::save_workspace,
        routes::system::load_workspace,
        // Devices
        routes::system::scan_ndi, routes::system::scan_syphon,
        routes::system::scan_cameras, routes::system::scan_midi,
        routes::system::scan_audio, routes::system::set_audio_source_enabled,
        routes::system::set_midi_device_enabled, routes::system::clear_midi_mappings,
        routes::system::remove_midi_mapping,
        // Streams
        routes::system::add_stream_library_entry, routes::system::remove_stream_library_entry,
        routes::system::add_hls_library_entry, routes::system::remove_hls_library_entry,
        routes::system::add_dash_library_entry, routes::system::remove_dash_library_entry,
        // Mixer
        routes::mixer::set_crossfader, routes::mixer::auto_crossfade,
        routes::mixer::beat_crossfade,
        // Channels
        routes::channels::add_channel, routes::channels::remove_channel,
        routes::channels::set_opacity, routes::channels::set_blend_mode,
        // Decks
        routes::decks::add_shader_deck, routes::decks::remove_deck,
        routes::decks::set_opacity, routes::decks::set_blend_mode,
        routes::decks::set_solo, routes::decks::set_mute,
        routes::decks::add_image_deck, routes::decks::add_video_deck,
        routes::decks::add_solid_color_deck, routes::decks::add_camera_deck,
        routes::decks::move_deck, routes::decks::set_scaling_mode,
        routes::decks::set_transition, routes::decks::set_param,
        routes::decks::add_ndi_deck, routes::decks::add_syphon_deck,
        routes::decks::add_srt_deck, routes::decks::add_hls_deck, routes::decks::add_dash_deck,
        routes::decks::reset_generator_params,
        // Video
        routes::decks::video_toggle_play, routes::decks::video_seek,
        routes::decks::video_set_speed, routes::decks::video_set_loop_mode,
        routes::decks::video_set_in_point, routes::decks::video_set_out_point,
        routes::decks::video_clear_in_out,
        // Auto Transitions
        routes::decks::set_auto_transition_enabled, routes::decks::set_auto_transition_trigger,
        routes::decks::set_auto_transition_play_duration, routes::decks::set_auto_transition_duration,
        routes::decks::set_auto_transition_shader,
        // Effects
        routes::effects::add_effect, routes::effects::remove_effect,
        routes::effects::toggle_effect, routes::effects::move_effect,
        // Audio
        routes::audio::scan_devices, routes::audio::open_source,
        routes::audio::close_source,
        // Modulation
        routes::modulation::add_lfo, routes::modulation::add_audio_band,
        routes::modulation::add_adsr, routes::modulation::add_step_sequencer,
        routes::modulation::remove_source, routes::modulation::assign,
        routes::modulation::clear,
        routes::modulation::update_lfo_frequency, routes::modulation::update_lfo_waveform,
        routes::modulation::update_lfo_phase, routes::modulation::update_lfo_amplitude,
        routes::modulation::update_lfo_bipolar,
        routes::modulation::update_audio_smoothing, routes::modulation::update_audio_freq_range,
        routes::modulation::update_audio_gain, routes::modulation::update_audio_preset,
        routes::modulation::update_audio_mode,
        routes::modulation::update_adsr_attack, routes::modulation::update_adsr_decay,
        routes::modulation::update_adsr_sustain, routes::modulation::update_adsr_release,
        routes::modulation::trigger_adsr, routes::modulation::release_adsr,
        routes::modulation::update_step_seq_steps, routes::modulation::update_step_seq_rate,
        routes::modulation::update_step_seq_interpolation,
        routes::modulation::update_audio_source, routes::modulation::update_audio_noise_gate,
        routes::modulation::update_step_seq_bipolar, routes::modulation::set_step_seq_count,
        routes::modulation::update_step_seq_value,
        routes::modulation::assign_mod_on_mod, routes::modulation::remove_mod_on_mod,
        // Surfaces
        routes::surfaces::add_rect, routes::surfaces::add_polygon,
        routes::surfaces::add_circle, routes::surfaces::remove,
        routes::surfaces::set_source, routes::surfaces::set_output_type,
        routes::surfaces::set_content_mapping, routes::surfaces::rename,
        routes::surfaces::set_vertices, routes::surfaces::duplicate,
        routes::surfaces::flip_horizontal, routes::surfaces::flip_vertical,
        routes::surfaces::insert_vertex, routes::surfaces::set_circle_radius,
        routes::surfaces::set_circle_sides, routes::surfaces::convert_to_polygon,
        routes::surfaces::combine, routes::surfaces::move_surface,
        routes::surfaces::update_contour_vertices,
        // Outputs
        routes::outputs::create, routes::outputs::close,
        routes::outputs::set_display, routes::outputs::assign_surface,
        routes::outputs::unassign_surface, routes::outputs::create_headless,
        routes::outputs::start, routes::outputs::stop,
        routes::outputs::toggle_calibration, routes::outputs::set_warp,
        routes::outputs::reset_warp, routes::outputs::set_target,
        routes::outputs::set_edge_blend, routes::outputs::set_edge_blend_mode,
        // Sequences
        routes::sequences::create, routes::sequences::delete,
        routes::sequences::play, routes::sequences::stop,
        routes::sequences::toggle, routes::sequences::add_fade_step,
        routes::sequences::add_wait_step, routes::sequences::add_goto_step,
        routes::sequences::remove_step, routes::sequences::set_step_duration,
        routes::sequences::set_step_easing, routes::sequences::set_step_shader,
        routes::sequences::set_step_from_ch, routes::sequences::set_step_to_ch,
        routes::sequences::set_goto_target, routes::sequences::move_step,
    ),
    tags(
        (name = "System", description = "Health, state, shutdown, undo/redo, resolution, workspace"),
        (name = "Devices", description = "Device scanning, MIDI mappings, audio sources"),
        (name = "Streams", description = "Stream library management (SRT, HLS, DASH)"),
        (name = "Mixer", description = "Crossfader and transition controls"),
        (name = "Channels", description = "Channel CRUD and properties"),
        (name = "Decks", description = "Deck CRUD and properties"),
        (name = "Video", description = "Video playback controls"),
        (name = "Auto Transitions", description = "Auto-transition settings"),
        (name = "Effects", description = "Effect chain management"),
        (name = "Audio", description = "Audio device management"),
        (name = "Modulation", description = "LFO, audio-reactive, ADSR, step sequencer sources"),
        (name = "Surfaces", description = "Surface geometry and mapping"),
        (name = "Outputs", description = "Output window management and warp"),
        (name = "Sequences", description = "Transition sequence automation"),
        (name = "Params", description = "Shader parameter control"),
    )
)]
struct ApiDoc;

/// Build the axum router with all routes and middleware.
pub fn build_router(shared: SharedState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    use axum::routing::get;

    Router::new()
        // ── System ──────────────────────────────────────────────
        .route("/api/health", get(routes::system::health))
        .route("/api/state", get(routes::system::get_state))
        // ── Runtime state ───────────────────────────────────────
        .route("/api/state/mixer", get(routes::state::mixer))
        .route("/api/state/audio", get(routes::state::audio))
        .route("/api/state/modulation", get(routes::state::modulation))
        .route("/api/state/outputs", get(routes::state::outputs))
        .route("/api/state/surfaces", get(routes::state::surfaces))
        .route("/api/state/registry", get(routes::state::registry))
        .route("/api/state/midi", get(routes::state::midi))
        .route("/api/state/cameras", get(routes::state::cameras))
        .route("/api/state/clock", get(routes::state::clock))
        .route("/api/state/ndi", get(routes::state::ndi))
        .route("/api/state/syphon", get(routes::state::syphon))
        .route("/api/state/streams", get(routes::state::streams))
        .route("/api/state/performance", get(routes::state::performance))
        // ── Scene ───────────────────────────────────────────────
        .route("/api/scene", get(routes::scene::scene))
        .route("/api/scene/channels", get(routes::scene::channels))
        .route("/api/scene/channels/{uuid}", get(routes::scene::channel_by_uuid))
        .route("/api/scene/channels/{uuid}/decks", get(routes::scene::channel_decks))
        .route("/api/scene/channels/{ch_uuid}/decks/{deck_uuid}", get(routes::scene::deck_by_uuid))
        .route("/api/scene/modulation", get(routes::scene::modulation))
        .route("/api/scene/sequences", get(routes::scene::sequences))
        .route("/api/scene/streams", get(routes::scene::streams))
        // ── Stage ───────────────────────────────────────────────
        .route("/api/stage", get(routes::stage::stage))
        .route("/api/stage/surfaces", get(routes::stage::surfaces))
        .route("/api/stage/surfaces/{uuid}", get(routes::stage::surface_by_uuid))
        .route("/api/stage/outputs", get(routes::stage::outputs))
        .route("/api/stage/outputs/{uuid}", get(routes::stage::output_by_uuid))
        // ── Library ─────────────────────────────────────────────
        .route("/api/library/generators", get(routes::library::generators))
        .route("/api/library/effects", get(routes::library::effects))
        .route("/api/library/transitions", get(routes::library::transitions))
        .route("/api/library/cameras", get(routes::library::cameras))
        .route("/api/library/ndi", get(routes::library::ndi))
        .route("/api/library/syphon", get(routes::library::syphon))
        .route("/api/library/monitors", get(routes::library::monitors))
        // ── Write: Mixer ────────────────────────────────────────
        .route("/api/mixer/crossfader", axum::routing::put(routes::mixer::set_crossfader))
        .route("/api/mixer/auto-crossfade", axum::routing::post(routes::mixer::auto_crossfade))
        .route("/api/mixer/beat-crossfade", axum::routing::post(routes::mixer::beat_crossfade))
        // ── Write: Channels ─────────────────────────────────────
        .route("/api/channels", axum::routing::post(routes::channels::add_channel))
        .route("/api/channels/{idx}", axum::routing::delete(routes::channels::remove_channel))
        .route("/api/channels/{idx}/opacity", axum::routing::put(routes::channels::set_opacity))
        .route("/api/channels/{idx}/blend-mode", axum::routing::put(routes::channels::set_blend_mode))
        // ── Write: Decks ────────────────────────────────────────
        .route("/api/channels/{ch_idx}/decks/shader", axum::routing::post(routes::decks::add_shader_deck))
        .route("/api/channels/{ch_idx}/decks/{deck_idx}", axum::routing::delete(routes::decks::remove_deck))
        .route("/api/channels/{ch_idx}/decks/{deck_idx}/opacity", axum::routing::put(routes::decks::set_opacity))
        .route("/api/channels/{ch_idx}/decks/{deck_idx}/blend-mode", axum::routing::put(routes::decks::set_blend_mode))
        .route("/api/channels/{ch_idx}/decks/{deck_idx}/solo", axum::routing::put(routes::decks::set_solo))
        .route("/api/channels/{ch_idx}/decks/{deck_idx}/mute", axum::routing::put(routes::decks::set_mute))
        .route("/api/channels/{ch_idx}/decks/{deck_idx}/scaling-mode", axum::routing::put(routes::decks::set_scaling_mode))
        .route("/api/channels/{ch_idx}/decks/image", axum::routing::post(routes::decks::add_image_deck))
        .route("/api/channels/{ch_idx}/decks/video", axum::routing::post(routes::decks::add_video_deck))
        .route("/api/channels/{ch_idx}/decks/solid", axum::routing::post(routes::decks::add_solid_color_deck))
        .route("/api/channels/{ch_idx}/decks/camera", axum::routing::post(routes::decks::add_camera_deck))
        .route("/api/decks/move", axum::routing::post(routes::decks::move_deck))
        // ── Write: Effects ─────────────────────────────────────
        .route("/api/effects", axum::routing::post(routes::effects::add_effect).delete(routes::effects::remove_effect))
        .route("/api/effects/toggle", axum::routing::post(routes::effects::toggle_effect))
        .route("/api/effects/move", axum::routing::post(routes::effects::move_effect))
        // ── Write: Audio ───────────────────────────────────────
        .route("/api/audio/scan", axum::routing::post(routes::audio::scan_devices))
        .route("/api/audio/open", axum::routing::post(routes::audio::open_source))
        .route("/api/audio/close", axum::routing::post(routes::audio::close_source))
        // ── Write: Modulation ──────────────────────────────────
        .route("/api/modulation/lfo", axum::routing::post(routes::modulation::add_lfo))
        .route("/api/modulation/audio-band", axum::routing::post(routes::modulation::add_audio_band))
        .route("/api/modulation/adsr", axum::routing::post(routes::modulation::add_adsr))
        .route("/api/modulation/step-sequencer", axum::routing::post(routes::modulation::add_step_sequencer))
        .route("/api/modulation/{uuid}", axum::routing::delete(routes::modulation::remove_source))
        .route("/api/modulation/assign", axum::routing::post(routes::modulation::assign))
        .route("/api/modulation/clear", axum::routing::post(routes::modulation::clear))
        // ── Write: Surfaces ────────────────────────────────────
        .route("/api/surfaces/rect", axum::routing::post(routes::surfaces::add_rect))
        .route("/api/surfaces/polygon", axum::routing::post(routes::surfaces::add_polygon))
        .route("/api/surfaces/circle", axum::routing::post(routes::surfaces::add_circle))
        .route("/api/surfaces/{uuid}", axum::routing::delete(routes::surfaces::remove))
        .route("/api/surfaces/{uuid}/source", axum::routing::put(routes::surfaces::set_source))
        .route("/api/surfaces/{uuid}/output-type", axum::routing::put(routes::surfaces::set_output_type))
        .route("/api/surfaces/{uuid}/content-mapping", axum::routing::put(routes::surfaces::set_content_mapping))
        .route("/api/surfaces/{uuid}/name", axum::routing::put(routes::surfaces::rename))
        // ── Write: Outputs ─────────────────────────────────────
        .route("/api/outputs", axum::routing::post(routes::outputs::create))
        .route("/api/outputs/{idx}", axum::routing::delete(routes::outputs::close))
        .route("/api/outputs/{idx}/display", axum::routing::put(routes::outputs::set_display))
        .route("/api/outputs/{output_uuid}/surfaces", axum::routing::post(routes::outputs::assign_surface))
        .route("/api/outputs/{output_uuid}/surfaces/{assignment_idx}", axum::routing::delete(routes::outputs::unassign_surface))
        // ── Write: Video Playback ────────────────────────────────
        .route("/api/channels/{ch}/decks/{dk}/video/toggle-play", axum::routing::post(routes::decks::video_toggle_play))
        .route("/api/channels/{ch}/decks/{dk}/video/seek", axum::routing::put(routes::decks::video_seek))
        .route("/api/channels/{ch}/decks/{dk}/video/speed", axum::routing::put(routes::decks::video_set_speed))
        .route("/api/channels/{ch}/decks/{dk}/video/loop-mode", axum::routing::put(routes::decks::video_set_loop_mode))
        .route("/api/channels/{ch}/decks/{dk}/video/in-point", axum::routing::put(routes::decks::video_set_in_point))
        .route("/api/channels/{ch}/decks/{dk}/video/out-point", axum::routing::put(routes::decks::video_set_out_point))
        .route("/api/channels/{ch}/decks/{dk}/video/in-out-points", axum::routing::delete(routes::decks::video_clear_in_out))
        // ── Write: Auto-Transitions ──────────────────────────────
        .route("/api/channels/{ch}/decks/{dk}/auto-transition/enabled", axum::routing::put(routes::decks::set_auto_transition_enabled))
        .route("/api/channels/{ch}/decks/{dk}/auto-transition/trigger", axum::routing::put(routes::decks::set_auto_transition_trigger))
        .route("/api/channels/{ch}/decks/{dk}/auto-transition/play-duration", axum::routing::put(routes::decks::set_auto_transition_play_duration))
        .route("/api/channels/{ch}/decks/{dk}/auto-transition/duration", axum::routing::put(routes::decks::set_auto_transition_duration))
        .route("/api/channels/{ch}/decks/{dk}/auto-transition/shader", axum::routing::put(routes::decks::set_auto_transition_shader))
        // ── Write: External I/O Sources ──────────────────────────
        .route("/api/channels/{ch}/decks/ndi", axum::routing::post(routes::decks::add_ndi_deck))
        .route("/api/channels/{ch}/decks/syphon", axum::routing::post(routes::decks::add_syphon_deck))
        .route("/api/channels/{ch}/decks/srt", axum::routing::post(routes::decks::add_srt_deck))
        .route("/api/channels/{ch}/decks/hls", axum::routing::post(routes::decks::add_hls_deck))
        .route("/api/channels/{ch}/decks/dash", axum::routing::post(routes::decks::add_dash_deck))
        // ── Write: Modulation Updates ────────────────────────────
        .route("/api/modulation/{uuid}/lfo/frequency", axum::routing::put(routes::modulation::update_lfo_frequency))
        .route("/api/modulation/{uuid}/lfo/waveform", axum::routing::put(routes::modulation::update_lfo_waveform))
        .route("/api/modulation/{uuid}/lfo/phase", axum::routing::put(routes::modulation::update_lfo_phase))
        .route("/api/modulation/{uuid}/lfo/amplitude", axum::routing::put(routes::modulation::update_lfo_amplitude))
        .route("/api/modulation/{uuid}/lfo/bipolar", axum::routing::put(routes::modulation::update_lfo_bipolar))
        .route("/api/modulation/{uuid}/audio/smoothing", axum::routing::put(routes::modulation::update_audio_smoothing))
        .route("/api/modulation/{uuid}/audio/freq-range", axum::routing::put(routes::modulation::update_audio_freq_range))
        .route("/api/modulation/{uuid}/audio/gain", axum::routing::put(routes::modulation::update_audio_gain))
        .route("/api/modulation/{uuid}/audio/preset", axum::routing::put(routes::modulation::update_audio_preset))
        .route("/api/modulation/{uuid}/audio/mode", axum::routing::put(routes::modulation::update_audio_mode))
        .route("/api/modulation/{uuid}/adsr/attack", axum::routing::put(routes::modulation::update_adsr_attack))
        .route("/api/modulation/{uuid}/adsr/decay", axum::routing::put(routes::modulation::update_adsr_decay))
        .route("/api/modulation/{uuid}/adsr/sustain", axum::routing::put(routes::modulation::update_adsr_sustain))
        .route("/api/modulation/{uuid}/adsr/release", axum::routing::put(routes::modulation::update_adsr_release))
        .route("/api/modulation/{uuid}/adsr/trigger", axum::routing::post(routes::modulation::trigger_adsr))
        .route("/api/modulation/{uuid}/adsr/release-gate", axum::routing::post(routes::modulation::release_adsr))
        .route("/api/modulation/{uuid}/step-seq/steps", axum::routing::put(routes::modulation::update_step_seq_steps))
        .route("/api/modulation/{uuid}/step-seq/rate", axum::routing::put(routes::modulation::update_step_seq_rate))
        .route("/api/modulation/{uuid}/step-seq/interpolation", axum::routing::put(routes::modulation::update_step_seq_interpolation))
        .route("/api/modulation/{uuid}/audio/source", axum::routing::put(routes::modulation::update_audio_source))
        .route("/api/modulation/{uuid}/audio/noise-gate", axum::routing::put(routes::modulation::update_audio_noise_gate))
        .route("/api/modulation/{uuid}/step-seq/bipolar", axum::routing::put(routes::modulation::update_step_seq_bipolar))
        .route("/api/modulation/{uuid}/step-seq/count", axum::routing::put(routes::modulation::set_step_seq_count))
        .route("/api/modulation/{uuid}/step-seq/value", axum::routing::put(routes::modulation::update_step_seq_value))
        .route("/api/modulation/mod-on-mod", axum::routing::post(routes::modulation::assign_mod_on_mod))
        .route("/api/modulation/mod-on-mod/remove", axum::routing::post(routes::modulation::remove_mod_on_mod))
        // ── Write: Surfaces extras ──────────────────────────────
        .route("/api/surfaces/{uuid}/vertices", axum::routing::put(routes::surfaces::set_vertices))
        .route("/api/surfaces/{uuid}/duplicate", axum::routing::post(routes::surfaces::duplicate))
        .route("/api/surfaces/{uuid}/flip-horizontal", axum::routing::post(routes::surfaces::flip_horizontal))
        .route("/api/surfaces/{uuid}/flip-vertical", axum::routing::post(routes::surfaces::flip_vertical))
        .route("/api/surfaces/{uuid}/vertices/insert", axum::routing::post(routes::surfaces::insert_vertex))
        .route("/api/surfaces/{uuid}/circle/radius", axum::routing::put(routes::surfaces::set_circle_radius))
        .route("/api/surfaces/{uuid}/circle/sides", axum::routing::put(routes::surfaces::set_circle_sides))
        .route("/api/surfaces/{uuid}/convert-to-polygon", axum::routing::post(routes::surfaces::convert_to_polygon))
        .route("/api/surfaces/combine", axum::routing::post(routes::surfaces::combine))
        .route("/api/surfaces/{uuid}/move", axum::routing::put(routes::surfaces::move_surface))
        .route("/api/surfaces/{uuid}/contour-vertices", axum::routing::put(routes::surfaces::update_contour_vertices))
        // ── Write: Outputs extras ───────────────────────────────
        .route("/api/outputs/headless", axum::routing::post(routes::outputs::create_headless))
        .route("/api/outputs/{idx}/start", axum::routing::post(routes::outputs::start))
        .route("/api/outputs/{idx}/stop", axum::routing::post(routes::outputs::stop))
        .route("/api/outputs/{idx}/calibration", axum::routing::post(routes::outputs::toggle_calibration))
        .route("/api/outputs/{idx}/warp", axum::routing::put(routes::outputs::set_warp))
        .route("/api/outputs/{idx}/reset-warp", axum::routing::post(routes::outputs::reset_warp))
        .route("/api/outputs/{idx}/target", axum::routing::put(routes::outputs::set_target))
        .route("/api/outputs/{idx}/edge-blend", axum::routing::put(routes::outputs::set_edge_blend))
        .route("/api/outputs/{idx}/edge-blend-mode", axum::routing::put(routes::outputs::set_edge_blend_mode))
        // ── Write: Sequences ────────────────────────────────────
        .route("/api/sequences", axum::routing::post(routes::sequences::create))
        .route("/api/sequences/{idx}", axum::routing::delete(routes::sequences::delete))
        .route("/api/sequences/{idx}/play", axum::routing::post(routes::sequences::play))
        .route("/api/sequences/{idx}/stop", axum::routing::post(routes::sequences::stop))
        .route("/api/sequences/{idx}/toggle", axum::routing::post(routes::sequences::toggle))
        .route("/api/sequences/{idx}/steps/fade", axum::routing::post(routes::sequences::add_fade_step))
        .route("/api/sequences/{idx}/steps/wait", axum::routing::post(routes::sequences::add_wait_step))
        .route("/api/sequences/{idx}/steps/goto", axum::routing::post(routes::sequences::add_goto_step))
        .route("/api/sequences/{seq_idx}/steps/{step_idx}", axum::routing::delete(routes::sequences::remove_step))
        .route("/api/sequences/{seq_idx}/steps/{step_idx}/duration", axum::routing::put(routes::sequences::set_step_duration))
        .route("/api/sequences/{seq_idx}/steps/{step_idx}/easing", axum::routing::put(routes::sequences::set_step_easing))
        .route("/api/sequences/{seq_idx}/steps/{step_idx}/shader", axum::routing::put(routes::sequences::set_step_shader))
        .route("/api/sequences/{seq_idx}/steps/{step_idx}/from-ch", axum::routing::put(routes::sequences::set_step_from_ch))
        .route("/api/sequences/{seq_idx}/steps/{step_idx}/to-ch", axum::routing::put(routes::sequences::set_step_to_ch))
        .route("/api/sequences/{seq_idx}/steps/{step_idx}/goto-target", axum::routing::put(routes::sequences::set_goto_target))
        .route("/api/sequences/{idx}/steps/move", axum::routing::post(routes::sequences::move_step))
        // ── Write: System / Clock / Resolution / Persistence ────
        .route("/api/shutdown", axum::routing::post(routes::system::shutdown))
        .route("/api/undo", axum::routing::post(routes::system::undo))
        .route("/api/redo", axum::routing::post(routes::system::redo))
        .route("/api/resolution", axum::routing::put(routes::system::set_resolution))
        .route("/api/clock/preference", axum::routing::put(routes::system::set_clock_preference))
        .route("/api/clock/manual-bpm", axum::routing::put(routes::system::set_manual_bpm))
        .route("/api/workspace/save", axum::routing::post(routes::system::save_workspace))
        .route("/api/workspace/load", axum::routing::post(routes::system::load_workspace))
        // ── Write: Device scanning & MIDI ───────────────────────
        .route("/api/devices/ndi/scan", axum::routing::post(routes::system::scan_ndi))
        .route("/api/devices/syphon/scan", axum::routing::post(routes::system::scan_syphon))
        .route("/api/devices/cameras/scan", axum::routing::post(routes::system::scan_cameras))
        .route("/api/devices/midi/scan", axum::routing::post(routes::system::scan_midi))
        .route("/api/devices/audio/scan", axum::routing::post(routes::system::scan_audio))
        .route("/api/devices/audio/enabled", axum::routing::put(routes::system::set_audio_source_enabled))
        .route("/api/devices/midi/enabled", axum::routing::put(routes::system::set_midi_device_enabled))
        .route("/api/midi/mappings", axum::routing::delete(routes::system::clear_midi_mappings))
        .route("/api/midi/mappings/remove", axum::routing::post(routes::system::remove_midi_mapping))
        // ── Write: Stream Library ───────────────────────────────
        .route("/api/streams/library", axum::routing::post(routes::system::add_stream_library_entry).delete(routes::system::remove_stream_library_entry))
        .route("/api/streams/hls/library", axum::routing::post(routes::system::add_hls_library_entry).delete(routes::system::remove_hls_library_entry))
        .route("/api/streams/dash/library", axum::routing::post(routes::system::add_dash_library_entry).delete(routes::system::remove_dash_library_entry))
        // ── Write: Mixer extras ────────────────────────────────
        .route("/api/mixer/transition", axum::routing::put(routes::decks::set_transition))
        // ── Write: Params ──────────────────────────────────────
        .route("/api/params", axum::routing::put(routes::decks::set_param))
        .route("/api/params/reset", axum::routing::post(routes::decks::reset_generator_params))
        // ── Write: Generic ─────────────────────────────────────
        .route("/api/command", axum::routing::post(routes::decks::generic_command))
        // ── WebSocket ──────────────────────────────────────────
        .route("/api/ws", get(super::ws::ws_upgrade))
        // ── Static file serving for HLS/DASH stream segments ────
        .nest_service("/streams", tower_http::services::ServeDir::new(".varda/streams"))
        // ── OpenAPI / Swagger UI ─────────────────────────────────
        .merge(SwaggerUi::new("/api/docs").url("/api/openapi.json", ApiDoc::openapi()))
        // ── Middleware ───────────────────────────────────────────
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024 * 1024)) // 16 MB
        .layer(cors)
        .with_state(shared)
}

/// Handle for gracefully shutting down the API server.
pub struct ApiServerHandle {
    shutdown_tx: tokio::sync::watch::Sender<bool>,
    thread_handle: std::thread::JoinHandle<()>,
}

impl ApiServerHandle {
    /// Signal the API server to shut down and wait for the thread to finish.
    pub fn shutdown(self) {
        let _ = self.shutdown_tx.send(true);
        if let Err(e) = self.thread_handle.join() {
            log::warn!("API server thread panicked: {:?}", e);
        }
    }
}

/// Start the HTTP API server on a background tokio runtime.
///
/// Returns an `ApiServerHandle` for graceful shutdown, or `None` if binding failed.
pub fn start(
    port: u16,
    command_tx: mpsc::UnboundedSender<CommandEnvelope>,
    engine_state: Arc<RwLock<Option<EngineState>>>,
) -> Option<ApiServerHandle> {
    let shared = SharedState {
        command_tx,
        engine_state,
    };

    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    // Pre-check: try to bind synchronously to fail fast with a clear message
    let test_bind = std::net::TcpListener::bind(std::net::SocketAddr::from(([0, 0, 0, 0], port)));
    if let Err(e) = test_bind {
        log::warn!("Cannot bind API server on port {}: {} — API disabled", port, e);
        return None;
    }
    // Drop the sync listener so the async one can bind
    drop(test_bind);

    let thread_handle = std::thread::spawn(move || {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    log::error!("Failed to create tokio runtime for API server: {}", e);
                    return;
                }
            };

            rt.block_on(async move {
                let app = build_router(shared);
                let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
                let listener = match tokio::net::TcpListener::bind(addr).await {
                    Ok(l) => l,
                    Err(e) => {
                        log::error!("Failed to bind API server on port {}: {}", port, e);
                        return;
                    }
                };
                let local_addr = listener.local_addr().unwrap();
                log::info!("HTTP API server listening on http://{}", local_addr);

                let mut shutdown_rx = shutdown_rx;
                if let Err(e) = axum::serve(listener, app)
                    .with_graceful_shutdown(async move {
                        let _ = shutdown_rx.wait_for(|&v| v).await;
                        log::info!("API server shutting down...");
                    })
                    .await
                {
                    log::error!("API server error: {}", e);
                }
                log::info!("API server stopped");
            });
        }));
        if let Err(_) = result {
            log::error!("API server thread panicked");
        }
    });

    Some(ApiServerHandle { shutdown_tx, thread_handle })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_router_creates_valid_router() {
        let shared = SharedState {
            command_tx: mpsc::unbounded_channel().0,
            engine_state: Arc::new(RwLock::new(None)),
        };
        // Should not panic
        let _router = build_router(shared);
    }

    #[tokio::test]
    async fn test_cors_headers_present() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let shared = SharedState {
            command_tx: mpsc::unbounded_channel().0,
            engine_state: Arc::new(RwLock::new(None)),
        };
        let app = build_router(shared);

        // Preflight OPTIONS request
        let resp = app
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/health")
                    .header("Origin", "http://localhost:3000")
                    .header("Access-Control-Request-Method", "GET")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert!(resp.headers().contains_key("access-control-allow-origin"));
    }

    // ── Offensive: catch_unwind wrapping thread panics ────────────────

    #[test]
    fn api_thread_catch_unwind_pattern_works() {
        let handle = std::thread::spawn(move || {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                panic!("simulated API server panic");
            }));
            if let Err(_) = result {
                log::error!("API server thread panicked");
            }
        });
        // Thread must complete without propagating the panic
        handle.join().expect("thread should join cleanly after catch_unwind");
    }
}
