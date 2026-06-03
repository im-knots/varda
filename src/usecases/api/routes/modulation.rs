//! Modulation source write routes.

use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use utoipa::ToSchema;

use crate::engine::{CommandResult, EngineCommand};
use crate::internal::modulation::{AudioBandPreset, LFOWaveform};
use crate::usecases::api::{command_response, SharedState};

#[derive(Deserialize, ToSchema)]
pub struct AddLfoBody {
    /// LFO waveform shape.
    pub waveform: LFOWaveform,
    /// Oscillation frequency in Hz.
    pub frequency: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct AddAudioBandBody {
    /// Frequency band preset to use.
    pub preset: AudioBandPreset,
    /// Audio source device ID, or null for the default source.
    pub source_id: Option<u32>,
}

#[derive(Deserialize, ToSchema)]
pub struct AddAdsrBody {
    /// Attack time in seconds.
    pub attack: f32,
    /// Decay time in seconds.
    pub decay: f32,
    /// Sustain level (0.0–1.0).
    pub sustain: f32,
    /// Release time in seconds.
    pub release: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct AddStepSequencerBody {
    /// Number of steps in the sequencer.
    pub num_steps: usize,
    /// Playback rate in steps per beat.
    pub rate: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct AssignModulationBody {
    /// Dot-separated path of the parameter to modulate.
    pub target: String,
    /// UUID of the modulation source.
    pub source_id: String,
    /// Modulation depth (0.0–1.0).
    pub amount: f32,
}

#[derive(Deserialize, ToSchema)]
pub struct ClearModulationBody {
    /// Dot-separated path of the parameter to un-modulate.
    pub target: String,
}

#[utoipa::path(post, path = "/api/modulation/lfo", request_body = AddLfoBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn add_lfo(
    State(state): State<SharedState>,
    Json(body): Json<AddLfoBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddLfo {
            waveform: body.waveform,
            frequency: body.frequency,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/modulation/audio-band", request_body = AddAudioBandBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn add_audio_band(
    State(state): State<SharedState>,
    Json(body): Json<AddAudioBandBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddAudioBand {
            preset: body.preset,
            source_id: body.source_id,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/modulation/adsr", request_body = AddAdsrBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn add_adsr(
    State(state): State<SharedState>,
    Json(body): Json<AddAdsrBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddAdsr {
            attack: body.attack,
            decay: body.decay,
            sustain: body.sustain,
            release: body.release,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/modulation/step-sequencer", request_body = AddStepSequencerBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn add_step_sequencer(
    State(state): State<SharedState>,
    Json(body): Json<AddStepSequencerBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AddStepSequencer {
            num_steps: body.num_steps,
            rate: body.rate,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(delete, path = "/api/modulation/{uuid}", params(("uuid" = String, Path, description = "Modulation source UUID")), responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn remove_source(
    State(state): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::RemoveModulationSource { uuid })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/modulation/assign", request_body = AssignModulationBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn assign(
    State(state): State<SharedState>,
    Json(body): Json<AssignModulationBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::AssignModulation {
            target: body.target,
            source_id: body.source_id,
            amount: body.amount,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

#[utoipa::path(post, path = "/api/modulation/clear", request_body = ClearModulationBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn clear(
    State(state): State<SharedState>,
    Json(body): Json<ClearModulationBody>,
) -> impl IntoResponse {
    match state
        .send_command(EngineCommand::ClearModulation {
            target: body.target,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(msg) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, msg).into_response(),
    }
}

// ── Modulation Parameter Updates ───────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct FloatBody {
    /// Numeric value.
    pub value: f32,
}
#[derive(Deserialize, ToSchema)]
pub struct BoolBody {
    /// Boolean toggle value.
    pub value: bool,
}
#[derive(Deserialize, ToSchema)]
pub struct FreqRangeBody {
    /// Lower bound of the frequency range in Hz.
    pub freq_low: f32,
    /// Upper bound of the frequency range in Hz.
    pub freq_high: f32,
}
#[derive(Deserialize, ToSchema)]
pub struct WaveformBody {
    /// LFO waveform shape.
    pub waveform: LFOWaveform,
}
#[derive(Deserialize, ToSchema)]
pub struct PresetBody {
    /// Frequency band preset.
    pub preset: AudioBandPreset,
}
#[derive(Deserialize, ToSchema)]
pub struct AudioModeBody {
    /// Audio reactivity mode.
    pub mode: crate::modulation::AudioReactMode,
}
#[derive(Deserialize, ToSchema)]
pub struct StepsBody {
    /// Step values for the sequencer (0.0–1.0 each).
    pub steps: Vec<f32>,
}
#[derive(Deserialize, ToSchema)]
pub struct InterpolationBody {
    /// Interpolation mode between steps.
    pub interpolation: crate::modulation::StepInterpolation,
}

#[utoipa::path(put, path = "/api/modulation/{uuid}/lfo/frequency", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_lfo_frequency(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateLfoFrequency {
            uuid,
            frequency: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/lfo/waveform", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = WaveformBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_lfo_waveform(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<WaveformBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateLfoWaveform {
            uuid,
            waveform: b.waveform,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/lfo/phase", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_lfo_phase(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateLfoPhase {
            uuid,
            phase: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/lfo/amplitude", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_lfo_amplitude(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateLfoAmplitude {
            uuid,
            amplitude: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/lfo/bipolar", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = BoolBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_lfo_bipolar(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<BoolBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateLfoBipolar {
            uuid,
            bipolar: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/audio/smoothing", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_audio_smoothing(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAudioSmoothing {
            uuid,
            smoothing: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/audio/freq-range", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FreqRangeBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_audio_freq_range(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FreqRangeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAudioFreqRange {
            uuid,
            freq_low: b.freq_low,
            freq_high: b.freq_high,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/audio/gain", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_audio_gain(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAudioGain {
            uuid,
            gain: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/audio/preset", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = PresetBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_audio_preset(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<PresetBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAudioPreset {
            uuid,
            preset: b.preset,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/audio/mode", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = AudioModeBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_audio_mode(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<AudioModeBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAudioMode { uuid, mode: b.mode })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/adsr/attack", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_adsr_attack(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAdsrAttack {
            uuid,
            attack: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/adsr/decay", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_adsr_decay(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAdsrDecay {
            uuid,
            decay: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/adsr/sustain", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_adsr_sustain(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAdsrSustain {
            uuid,
            sustain: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/adsr/release", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_adsr_release(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAdsrRelease {
            uuid,
            release: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/modulation/{uuid}/adsr/trigger", params(("uuid" = String, Path, description = "Modulation source UUID")), responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn trigger_adsr(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s.send_command(EngineCommand::TriggerAdsr { uuid }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/modulation/{uuid}/adsr/release-gate", params(("uuid" = String, Path, description = "Modulation source UUID")), responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn release_adsr(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
) -> impl IntoResponse {
    match s.send_command(EngineCommand::ReleaseAdsr { uuid }).await {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/step-seq/steps", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = StepsBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_step_seq_steps(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<StepsBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateStepSeqSteps {
            uuid,
            steps: b.steps,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/step-seq/rate", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_step_seq_rate(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateStepSeqRate {
            uuid,
            rate: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/step-seq/interpolation", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = InterpolationBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_step_seq_interpolation(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<InterpolationBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateStepSeqInterpolation {
            uuid,
            interpolation: b.interpolation,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}

// ── Missing Parity Routes ─────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct ModAudioSourceBody {
    /// Audio source device ID, or null for the default source.
    pub source_id: Option<crate::audio::AudioSourceId>,
}
#[derive(Deserialize, ToSchema)]
pub struct StepSeqCountBody {
    /// New number of steps.
    pub count: usize,
}
#[derive(Deserialize, ToSchema)]
pub struct StepSeqValueBody {
    /// Zero-based index of the step to update.
    pub step_idx: usize,
    /// New value for the step (0.0–1.0).
    pub value: f32,
}
#[derive(Deserialize, ToSchema)]
pub struct AssignModOnModBody {
    /// UUID of the modulation source being modulated.
    pub target_source_id: String,
    /// Parameter on the target source to modulate.
    pub param_name: String,
    /// UUID of the modulator driving this modulation.
    pub modulator_id: String,
    /// Modulation depth (0.0–1.0).
    pub amount: f32,
}
#[derive(Deserialize, ToSchema)]
pub struct RemoveModOnModBody {
    /// UUID of the modulation source being modulated.
    pub target_source_id: String,
    /// Parameter to remove modulation from.
    pub param_name: String,
}

#[utoipa::path(put, path = "/api/modulation/{uuid}/audio/source", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = ModAudioSourceBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_audio_source(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<ModAudioSourceBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAudioSource {
            uuid,
            source_id: b.source_id,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/audio/noise-gate", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = FloatBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_audio_noise_gate(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<FloatBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateAudioNoiseGate {
            uuid,
            noise_gate: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/step-seq/bipolar", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = BoolBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_step_seq_bipolar(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<BoolBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateStepSeqBipolar {
            uuid,
            bipolar: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/step-seq/count", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = StepSeqCountBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn set_step_seq_count(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<StepSeqCountBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::SetStepSeqCount {
            uuid,
            count: b.count,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(put, path = "/api/modulation/{uuid}/step-seq/value", params(("uuid" = String, Path, description = "Modulation source UUID")), request_body = StepSeqValueBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn update_step_seq_value(
    State(s): State<SharedState>,
    Path(uuid): Path<String>,
    Json(b): Json<StepSeqValueBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::UpdateStepSeqValue {
            uuid,
            step_idx: b.step_idx,
            value: b.value,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/modulation/mod-on-mod", request_body = AssignModOnModBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn assign_mod_on_mod(
    State(s): State<SharedState>,
    Json(b): Json<AssignModOnModBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::AssignModOnMod {
            target_source_id: b.target_source_id,
            param_name: b.param_name,
            modulator_id: b.modulator_id,
            amount: b.amount,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
#[utoipa::path(post, path = "/api/modulation/mod-on-mod/remove", request_body = RemoveModOnModBody, responses((status = 200, body = CommandResult)), tag = "Modulation")]
pub async fn remove_mod_on_mod(
    State(s): State<SharedState>,
    Json(b): Json<RemoveModOnModBody>,
) -> impl IntoResponse {
    match s
        .send_command(EngineCommand::RemoveModOnMod {
            target_source_id: b.target_source_id,
            param_name: b.param_name,
        })
        .await
    {
        Ok(r) => command_response(r),
        Err(m) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, m).into_response(),
    }
}
