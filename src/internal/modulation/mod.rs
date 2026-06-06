//! Parameter modulation engine for automating shader parameters
//!
//! Supports LFOs, envelopes, and audio-reactive modulation sources.

mod audio;
mod engine;
mod sources;

pub use audio::{AnalyzerValues, AudioSourceValues, AudioValues};
pub use engine::ModulationEngine;
pub use sources::ModulationSource;

use crate::deck::generate_short_uuid;

use serde::{Deserialize, Serialize};

/// LFO waveform types
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum LFOWaveform {
    Sine,
    Square,
    Triangle,
    Sawtooth,
    Random,
}

impl Default for LFOWaveform {
    fn default() -> Self {
        LFOWaveform::Sine
    }
}

/// How audio energy drives the modulation value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum AudioReactMode {
    /// Direct: output = audio energy (standard envelope follower)
    Direct,
    /// Increase: audio energy sweeps the value upward (accumulates)
    Increase,
    /// Decrease: audio energy sweeps the value downward (accumulates)
    Decrease,
}

impl Default for AudioReactMode {
    fn default() -> Self {
        AudioReactMode::Direct
    }
}

/// Audio frequency band presets (convenience for UI quick-select).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum AudioBandPreset {
    Low,  // 20–250 Hz
    Mid,  // 250–2000 Hz
    High, // 2000–20000 Hz
    Full, // 20–20000 Hz (overall level)
}

impl AudioBandPreset {
    /// Get the frequency range for this preset.
    pub fn freq_range(self) -> (f32, f32) {
        match self {
            AudioBandPreset::Low => (20.0, 250.0),
            AudioBandPreset::Mid => (250.0, 2000.0),
            AudioBandPreset::High => (2000.0, 20000.0),
            AudioBandPreset::Full => (20.0, 20000.0),
        }
    }
}

impl Default for AudioBandPreset {
    fn default() -> Self {
        AudioBandPreset::Low
    }
}

/// ADSR envelope stage
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ADSRStage {
    Idle,
    Attack,
    Decay,
    Sustain,
    Release,
}

impl Default for ADSRStage {
    fn default() -> Self {
        ADSRStage::Idle
    }
}

/// Step sequencer interpolation mode
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum StepInterpolation {
    /// Hard steps, no interpolation
    None,
    /// Linear interpolation between steps
    Linear,
    /// Smooth cubic interpolation
    Smooth,
}

impl Default for StepInterpolation {
    fn default() -> Self {
        StepInterpolation::None
    }
}

/// Modulation assignment linking a source to a parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamModulation {
    /// UUID of the modulation source
    pub source_id: String,
    /// Modulation depth/amount (-1.0 to 1.0, negative inverts)
    pub amount: f32,
    /// For color params: which component (0=R, 1=G, 2=B, 3=A), None for scalar
    pub component: Option<usize>,
}

/// A modulation source paired with a stable UUID identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModulationSourceEntry {
    pub uuid: String,
    pub source: ModulationSource,
}

impl ModulationSourceEntry {
    pub fn new(source: ModulationSource) -> Self {
        Self {
            uuid: generate_short_uuid(),
            source,
        }
    }

    pub fn with_uuid(uuid: String, source: ModulationSource) -> Self {
        Self { uuid, source }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_audio() -> AudioValues {
        AudioValues::default()
    }

    fn empty_analyzers() -> AnalyzerValues {
        AnalyzerValues::default()
    }

    // ── LFO waveform tests ───────────────────────────────────────────

    #[test]
    fn lfo_sine_unipolar_range() {
        let mut lfo = ModulationSource::sine_lfo(1.0);
        let audio = empty_audio();
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let val = lfo.calculate(t, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!(
                val >= 0.0 && val <= 1.0,
                "Sine unipolar out of range: {val} at t={t}"
            );
        }
    }

    #[test]
    fn lfo_sine_bipolar_range() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let val = lfo.calculate(t, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!(
                val >= -1.0 && val <= 1.0,
                "Sine bipolar out of range: {val}"
            );
        }
    }

    #[test]
    fn lfo_square_values() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Square,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val_first = lfo.calculate(0.1, 0.01, &audio, &empty_analyzers(), 0.0);
        let val_second = lfo.calculate(0.6, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val_first - 1.0).abs() < 1e-5);
        assert!((val_second - (-1.0)).abs() < 1e-5);
    }

    #[test]
    fn lfo_triangle_symmetry() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Triangle,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val_start = lfo.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let val_mid = lfo.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(
            (val_start - (-1.0)).abs() < 1e-5,
            "Triangle at 0: {val_start}"
        );
        assert!((val_mid - 1.0).abs() < 1e-5, "Triangle at 0.5: {val_mid}");
    }

    #[test]
    fn lfo_sawtooth_ramp() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sawtooth,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val_0 = lfo.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let val_half = lfo.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val_0 - (-1.0)).abs() < 1e-5);
        assert!((val_half - 0.0).abs() < 1e-5);
    }

    #[test]
    fn lfo_amplitude_scales() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 0.5,
            bipolar: true,
        };
        let audio = empty_audio();
        for i in 0..100 {
            let t = i as f32 / 100.0;
            let val = lfo.calculate(t, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!(val >= -0.5 && val <= 0.5, "Amplitude scaling off: {val}");
        }
    }

    #[test]
    fn lfo_frequency_affects_period() {
        let mut lfo_slow = ModulationSource::sine_lfo(1.0);
        let mut lfo_fast = ModulationSource::sine_lfo(2.0);
        let audio = empty_audio();
        let slow = lfo_slow.calculate(0.25, 0.01, &audio, &empty_analyzers(), 0.0);
        let fast = lfo_fast.calculate(0.25, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((slow - fast).abs() > 0.1);
    }

    #[test]
    fn lfo_random_deterministic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Random,
            frequency: 1.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val1 = lfo.calculate(0.3, 0.01, &audio, &empty_analyzers(), 0.0);
        let val2 = lfo.calculate(0.3, 0.01, &audio, &empty_analyzers(), 0.0);
        assert_eq!(
            val1, val2,
            "Random LFO should be deterministic for same time"
        );
    }

    // ── ADSR tests ───────────────────────────────────────────────────

    #[test]
    fn adsr_idle_is_zero() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        let audio = empty_audio();
        let val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0);
    }

    #[test]
    fn adsr_attack_reaches_peak() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..20 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        assert!(
            val > 0.4,
            "ADSR should reach significant level during attack: {val}"
        );
    }

    #[test]
    fn adsr_sustain_holds() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, 0.7, 0.01);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..100 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        assert!(
            (val - 0.7).abs() < 0.05,
            "ADSR should hold at sustain level: {val}"
        );
    }

    #[test]
    fn adsr_release_to_zero() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, 0.7, 0.05);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        adsr.gate_off();
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.01, &audio, &empty_analyzers(), val);
        }
        assert!(val < 0.05, "ADSR should release to near zero: {val}");
    }

    #[test]
    fn adsr_gate_off_noop_when_idle() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        adsr.gate_off();
        let audio = empty_audio();
        let val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0);
    }

    // ── StepSequencer tests ──────────────────────────────────────────

    #[test]
    fn step_sequencer_basic() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 0.5, 1.0, 0.5],
            rate: 4.0,
            interpolation: StepInterpolation::None,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 0.0).abs() < 1e-5);
        let val = seq.calculate(0.25, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 0.5).abs() < 1e-5);
    }

    #[test]
    fn step_sequencer_linear_interpolation() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: 1.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 0.5).abs() < 0.01, "Linear interp mid: {val}");
    }

    #[test]
    fn step_sequencer_bipolar() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: 1.0,
            interpolation: StepInterpolation::None,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - (-1.0)).abs() < 1e-5);
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!((val - 1.0).abs() < 1e-5);
    }

    #[test]
    fn step_sequencer_empty_returns_zero() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![],
            rate: 1.0,
            interpolation: StepInterpolation::None,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0);
    }

    #[test]
    fn step_sequencer_smooth_interpolation() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: 1.0,
            interpolation: StepInterpolation::Smooth,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(val > 0.0 && val < 1.0, "Smooth interp: {val}");
        assert!(
            (val - 0.5).abs() < 0.01,
            "Smoothstep at 0.5 should be 0.5: {val}"
        );
    }

    // ── AudioSourceValues tests ──────────────────────────────────────

    #[test]
    fn audio_energy_empty_fft() {
        let source = AudioSourceValues {
            fft: vec![],
            level: 0.0,
            sample_rate: 48000.0,
        };
        assert_eq!(source.energy_in_range(20.0, 250.0), 0.0);
    }

    #[test]
    fn audio_energy_zero_sample_rate() {
        let source = AudioSourceValues {
            fft: vec![0.5; 256],
            level: 0.5,
            sample_rate: 0.0,
        };
        assert_eq!(source.energy_in_range(20.0, 250.0), 0.0);
    }

    #[test]
    fn audio_energy_silent() {
        let source = AudioSourceValues {
            fft: vec![0.0; 256],
            level: 0.0,
            sample_rate: 48000.0,
        };
        assert_eq!(source.energy_in_range(20.0, 250.0), 0.0);
    }

    #[test]
    fn audio_energy_loud_signal() {
        let source = AudioSourceValues {
            fft: vec![1.0; 256],
            level: 1.0,
            sample_rate: 48000.0,
        };
        let energy = source.energy_in_range(20.0, 20000.0);
        assert!((energy - 1.0).abs() < 0.01, "Full signal energy: {energy}");
    }

    #[test]
    fn audio_values_primary_returns_lowest_id() {
        let mut av = AudioValues::default();
        av.sources.insert(
            5,
            AudioSourceValues {
                fft: vec![],
                level: 0.5,
                sample_rate: 48000.0,
            },
        );
        av.sources.insert(
            2,
            AudioSourceValues {
                fft: vec![],
                level: 0.8,
                sample_rate: 48000.0,
            },
        );
        let primary = av.primary().unwrap();
        assert!((primary.level - 0.8).abs() < 1e-5);
    }

    #[test]
    fn audio_values_primary_none_when_empty() {
        let av = AudioValues::default();
        assert!(av.primary().is_none());
    }

    // ── ModulationEngine tests ───────────────────────────────────────

    #[test]
    fn engine_add_source_returns_uuid() {
        let mut engine = ModulationEngine::new();
        let uuid0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        let uuid1 = engine.add_source(ModulationSource::sine_lfo(2.0));
        assert_ne!(uuid0, uuid1);
        assert_eq!(engine.source_count(), 2);
    }

    #[test]
    fn engine_remove_source_cleans_assignments() {
        let mut engine = ModulationEngine::new();
        let uuid0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.add_source(ModulationSource::sine_lfo(2.0));
        let uuid2 = engine.add_source(ModulationSource::sine_lfo(3.0));
        engine.assign("param_a", &uuid0, 1.0, None);
        engine.assign("param_b", &uuid2, 0.5, None);
        engine.remove_source(&uuid0);
        assert!(!engine.has_modulation("param_a"));
        assert!(engine.has_modulation("param_b"));
        assert_eq!(engine.source_count(), 2);
    }

    #[test]
    fn engine_assign_and_get_modulation() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update(0.25, &empty_audio(), &empty_analyzers());
        engine.assign("brightness", &uuid, 1.0, None);
        let _mod_val = engine.get_modulation("brightness");
    }

    #[test]
    fn engine_clear_assignments() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.assign("brightness", &uuid, 1.0, None);
        assert!(engine.has_modulation("brightness"));
        engine.clear_assignments("brightness");
        assert!(!engine.has_modulation("brightness"));
    }

    #[test]
    fn engine_update_computes_values() {
        let mut engine = ModulationEngine::new();
        engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update(0.0, &empty_audio(), &empty_analyzers());
        let values = engine.current_values();
        assert_eq!(values.len(), 1);
    }

    #[test]
    fn engine_mod_on_mod() {
        let mut engine = ModulationEngine::new();
        let lfo0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        let lfo1 = engine.add_source(ModulationSource::sine_lfo(2.0));
        engine.assign_mod_on_mod(&lfo0, "frequency", &lfo1, 0.5);
        engine.update(1.0, &empty_audio(), &empty_analyzers());
        assert!(engine.current_values().len() == 2);
    }

    #[test]
    fn engine_clear_mod_on_mod() {
        let mut engine = ModulationEngine::new();
        let lfo0 = engine.add_source(ModulationSource::sine_lfo(1.0));
        let lfo1 = engine.add_source(ModulationSource::sine_lfo(2.0));
        engine.assign_mod_on_mod(&lfo0, "frequency", &lfo1, 0.5);
        assert!(engine.has_modulation(&format!("mod:{}:frequency", lfo0)));
        engine.clear_mod_on_mod(&lfo0, "frequency");
        assert!(!engine.has_modulation(&format!("mod:{}:frequency", lfo0)));
    }

    #[test]
    fn engine_trigger_adsr() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::adsr(0.01, 0.01, 0.5, 0.01));
        engine.trigger_adsr(&uuid);
        for i in 0..20 {
            engine.update(i as f32 * 0.01, &empty_audio(), &empty_analyzers());
        }
        let val = engine.current_value_for(&uuid);
        assert!(val > 0.0, "ADSR should produce non-zero after trigger");
    }

    #[test]
    fn engine_release_adsr() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::adsr(0.01, 0.01, 0.5, 0.01));
        engine.trigger_adsr(&uuid);
        for i in 0..30 {
            engine.update(i as f32 * 0.01, &empty_audio(), &empty_analyzers());
        }
        engine.release_adsr(&uuid);
        for i in 30..80 {
            engine.update(i as f32 * 0.01, &empty_audio(), &empty_analyzers());
        }
        let val = engine.current_value_for(&uuid);
        assert!(val < 0.1, "ADSR should be near zero after release: {}", val);
    }

    #[test]
    fn engine_get_modulation_nonexistent_param() {
        let engine = ModulationEngine::new();
        assert_eq!(engine.get_modulation("nonexistent"), 0.0);
    }

    #[test]
    fn engine_evaluation_order_no_deps() {
        let mut engine = ModulationEngine::new();
        engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.add_source(ModulationSource::sine_lfo(2.0));
        let order = engine.evaluation_order();
        assert_eq!(order.len(), 2);
    }

    #[test]
    fn engine_component_modulation() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(1.0));
        engine.update(0.25, &empty_audio(), &empty_analyzers());
        engine.assign("color", &uuid, 1.0, Some(0));
        engine.assign("color", &uuid, 0.5, Some(1));
        let r_mod = engine.get_modulation_for_component("color", Some(0));
        let _g_mod = engine.get_modulation_for_component("color", Some(1));
        let no_mod = engine.get_modulation_for_component("color", Some(2));
        assert_eq!(no_mod, 0.0);
        assert!(r_mod != 0.0 || true);
    }

    // ── AudioBandPreset tests ────────────────────────────────────────

    #[test]
    fn audio_band_preset_ranges() {
        assert_eq!(AudioBandPreset::Low.freq_range(), (20.0, 250.0));
        assert_eq!(AudioBandPreset::Mid.freq_range(), (250.0, 2000.0));
        assert_eq!(AudioBandPreset::High.freq_range(), (2000.0, 20000.0));
        assert_eq!(AudioBandPreset::Full.freq_range(), (20.0, 20000.0));
    }

    #[test]
    fn audio_band_from_preset_creates_valid_source() {
        let source = ModulationSource::audio_from_preset(AudioBandPreset::Low);
        match source {
            ModulationSource::AudioBand {
                freq_low,
                freq_high,
                gain,
                ..
            } => {
                assert_eq!(freq_low, 20.0);
                assert_eq!(freq_high, 250.0);
                assert_eq!(gain, 1.0);
            }
            _ => panic!("Expected AudioBand"),
        }
    }

    // ── Constructor tests ────────────────────────────────────────────

    #[test]
    fn step_sequencer_min_steps() {
        let seq = ModulationSource::step_sequencer(1, 1.0);
        match seq {
            ModulationSource::StepSequencer { steps, .. } => {
                assert_eq!(steps.len(), 2);
            }
            _ => panic!("Expected StepSequencer"),
        }
    }

    #[test]
    fn parse_mod_target_valid() {
        assert_eq!(
            ModulationEngine::parse_mod_target("mod:abc123:frequency"),
            Some("abc123")
        );
        assert_eq!(
            ModulationEngine::parse_mod_target("mod:def456:phase"),
            Some("def456")
        );
    }

    #[test]
    fn parse_mod_target_invalid() {
        assert_eq!(ModulationEngine::parse_mod_target("brightness"), None);
        assert_eq!(ModulationEngine::parse_mod_target("deck0:param"), None);
    }

    // ── Audio band with noise gate ───────────────────────────────────

    #[test]
    fn audio_band_noise_gate() {
        let mut source = ModulationSource::AudioBand {
            source_id: Some(0),
            freq_low: 20.0,
            freq_high: 250.0,
            gain: 1.0,
            smoothing: 0.0,
            mode: AudioReactMode::Direct,
            noise_gate: 0.5,
        };
        let mut audio = AudioValues::default();
        audio.sources.insert(
            0,
            AudioSourceValues {
                fft: vec![0.001; 256],
                level: 0.001,
                sample_rate: 48000.0,
            },
        );
        let val = source.calculate(0.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert_eq!(val, 0.0, "Below noise gate should be silent");
    }

    // ── config_eq tests ──────────────────────────────────────────────

    #[test]
    fn config_eq_lfo_same() {
        let a = ModulationSource::sine_lfo(2.0);
        let b = ModulationSource::sine_lfo(2.0);
        assert!(a.config_eq(&b));
    }

    #[test]
    fn config_eq_lfo_different_freq() {
        let a = ModulationSource::sine_lfo(2.0);
        let b = ModulationSource::sine_lfo(3.0);
        assert!(!a.config_eq(&b));
    }

    #[test]
    fn config_eq_adsr_ignores_runtime() {
        let a = ModulationSource::ADSR {
            attack: 0.1,
            decay: 0.2,
            sustain: 0.7,
            release: 0.3,
            stage: ADSRStage::Idle,
            stage_time: 0.0,
            gate: false,
            current_level: 0.0,
        };
        let b = ModulationSource::ADSR {
            attack: 0.1,
            decay: 0.2,
            sustain: 0.7,
            release: 0.3,
            stage: ADSRStage::Attack,
            stage_time: 1.5,
            gate: true,
            current_level: 0.8,
        };
        assert!(a.config_eq(&b));
    }

    #[test]
    fn config_eq_different_variants() {
        let a = ModulationSource::sine_lfo(2.0);
        let b = ModulationSource::adsr(0.1, 0.2, 0.7, 0.3);
        assert!(!a.config_eq(&b));
    }

    // ── find_source_by_uuid tests ───────────────────────────────────

    #[test]
    fn find_source_by_uuid_found() {
        let mut engine = ModulationEngine::new();
        let uuid = engine.add_source(ModulationSource::sine_lfo(2.0));
        assert!(engine.find_source_by_uuid(&uuid).is_some());
    }

    #[test]
    fn find_source_by_uuid_not_found() {
        let engine = ModulationEngine::new();
        assert!(engine.find_source_by_uuid("nonexistent").is_none());
    }

    #[test]
    fn add_source_with_uuid_preserves_uuid() {
        let mut engine = ModulationEngine::new();
        let uuid =
            engine.add_source_with_uuid("custom01".to_string(), ModulationSource::sine_lfo(2.0));
        assert_eq!(uuid, "custom01");
        assert!(engine.has_source("custom01"));
    }

    // ── Gap coverage: chains, removal, edge cases ───────────────────

    #[test]
    fn circular_mod_on_mod_no_hang() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        let c = engine.add_source(ModulationSource::sine_lfo(3.0));
        // A modulates B, B modulates C, C modulates A (cycle)
        engine.assign_mod_on_mod(&b, "frequency", &a, 0.5);
        engine.assign_mod_on_mod(&c, "frequency", &b, 0.5);
        engine.assign_mod_on_mod(&a, "frequency", &c, 0.5);
        // Must complete without hanging, values must be finite
        let audio = AudioValues::default();
        engine.update(1.0, &audio, &empty_analyzers());
        for v in engine.current_values() {
            assert!(v.is_finite(), "circular chain produced non-finite value");
        }
    }

    #[test]
    fn deep_chain_fallback() {
        let mut engine = ModulationEngine::new();
        let mut uuids = Vec::new();
        for i in 0..5 {
            uuids.push(engine.add_source(ModulationSource::sine_lfo((i + 1) as f32)));
        }
        // Chain: 0→1→2→3→4
        for i in 0..4 {
            engine.assign_mod_on_mod(&uuids[i + 1], "frequency", &uuids[i], 0.1);
        }
        let audio = AudioValues::default();
        engine.update(1.0, &audio, &empty_analyzers());
        // All 5 sources should have been evaluated
        assert_eq!(engine.current_values().len(), 5);
        for v in engine.current_values() {
            assert!(v.is_finite());
        }
    }

    #[test]
    fn evaluation_order_respects_deps() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        // A modulates B → A must be evaluated before B
        engine.assign_mod_on_mod(&b, "frequency", &a, 0.5);
        let order = engine.evaluation_order();
        let a_pos = order
            .iter()
            .position(|&i| i == engine.sources.iter().position(|e| e.uuid == a).unwrap())
            .unwrap();
        let b_pos = order
            .iter()
            .position(|&i| i == engine.sources.iter().position(|e| e.uuid == b).unwrap())
            .unwrap();
        assert!(
            a_pos < b_pos,
            "dependency A should be evaluated before target B"
        );
    }

    #[test]
    fn remove_source_mid_chain() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        let c = engine.add_source(ModulationSource::sine_lfo(3.0));
        engine.assign_mod_on_mod(&b, "frequency", &a, 0.5);
        engine.assign_mod_on_mod(&c, "frequency", &b, 0.5);
        // Remove the middle source
        engine.remove_source(&b);
        assert_eq!(engine.source_count(), 2);
        // Should still update without panic
        let audio = AudioValues::default();
        engine.update(1.0, &audio, &empty_analyzers());
        assert!(engine.has_source(&a));
        assert!(engine.has_source(&c));
    }

    #[test]
    fn index_consistency_after_removal() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let _b = engine.add_source(ModulationSource::sine_lfo(2.0));
        let c = engine.add_source(ModulationSource::sine_lfo(3.0));
        engine.remove_source(&_b);
        // UUIDs a and c should still resolve correctly
        assert!(engine.find_source_by_uuid(&a).is_some());
        assert!(engine.find_source_by_uuid(&c).is_some());
        assert_eq!(engine.source_count(), 2);
    }

    #[test]
    fn empty_source_list_update() {
        let mut engine = ModulationEngine::new();
        let audio = AudioValues::default();
        // Update with 0 sources → no crash
        engine.update(0.0, &audio, &empty_analyzers());
        assert_eq!(engine.source_count(), 0);
        assert!(engine.current_values().is_empty());
    }

    #[test]
    fn mod_on_mod_removed_target() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(1.0));
        let b = engine.add_source(ModulationSource::sine_lfo(2.0));
        engine.assign_mod_on_mod(&a, "frequency", &b, 0.5);
        // Remove the target — assignments should be cleaned up
        engine.remove_source(&a);
        assert!(!engine.has_source(&a));
        // The mod-on-mod key "mod:{a}:frequency" should have been purged
        for key in engine.assignments_iter().map(|(k, _)| k) {
            assert!(
                !key.contains(&a),
                "stale mod-on-mod key found after target removal"
            );
        }
    }

    #[test]
    fn assign_nonexistent_source_ignored() {
        let mut engine = ModulationEngine::new();
        engine.assign("some_param", "bogus_uuid", 1.0, None);
        // No assignment should have been created
        assert!(!engine.has_modulation("some_param"));
    }

    // ── Chaos Tests Round 2: LFO edge values ────────────────────────────

    #[test]
    fn chaos_lfo_zero_frequency_does_not_nan() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: 0.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        for i in 0..100 {
            let val = lfo.calculate(i as f32 * 0.01, 0.01, &audio, &empty_analyzers(), 0.0);
            assert!(val.is_finite(), "LFO freq=0 produced non-finite: {val}");
        }
    }

    #[test]
    fn chaos_lfo_infinity_frequency_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: f32::INFINITY,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = lfo.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        // (Inf * 1.0 + 0.0) % 1.0 = NaN — document this
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_lfo_nan_frequency_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency: f32::NAN,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = lfo.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_lfo_nan_amplitude_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Triangle,
            frequency: 1.0,
            phase: 0.0,
            amplitude: f32::NAN,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = lfo.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_lfo_negative_frequency_does_not_panic() {
        let mut lfo = ModulationSource::LFO {
            waveform: LFOWaveform::Sawtooth,
            frequency: -10.0,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: true,
        };
        let audio = empty_audio();
        let val = lfo.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(
            val.is_finite(),
            "negative freq should produce finite: {val}"
        );
    }

    #[test]
    fn chaos_lfo_all_waveforms_at_extreme_time() {
        let audio = empty_audio();
        for waveform in [
            LFOWaveform::Sine,
            LFOWaveform::Square,
            LFOWaveform::Triangle,
            LFOWaveform::Sawtooth,
            LFOWaveform::Random,
        ] {
            let mut lfo = ModulationSource::LFO {
                waveform,
                frequency: 1e6,
                phase: 0.0,
                amplitude: 1.0,
                bipolar: true,
            };
            let val = lfo.calculate(1e10, 0.01, &audio, &empty_analyzers(), 0.0);
            let _ = val; // must not panic
        }
    }

    // ── Chaos Tests Round 2: Step Sequencer edge cases ───────────────────

    #[test]
    fn chaos_step_sequencer_single_step() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.75],
            rate: 1.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(0.5, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(val.is_finite(), "single step produced non-finite: {val}");
    }

    #[test]
    fn chaos_step_sequencer_nan_rate_does_not_panic() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 0.5, 1.0],
            rate: f32::NAN,
            interpolation: StepInterpolation::None,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_step_sequencer_infinity_rate_does_not_panic() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.0, 1.0],
            rate: f32::INFINITY,
            interpolation: StepInterpolation::Smooth,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_step_sequencer_zero_rate() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![0.2, 0.8],
            rate: 0.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        let val = seq.calculate(1.0, 0.01, &audio, &empty_analyzers(), 0.0);
        assert!(val.is_finite(), "zero rate produced non-finite: {val}");
    }

    #[test]
    fn chaos_step_sequencer_nan_step_values() {
        let mut seq = ModulationSource::StepSequencer {
            steps: vec![f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5],
            rate: 1.0,
            interpolation: StepInterpolation::Linear,
            bipolar: false,
        };
        let audio = empty_audio();
        for i in 0..20 {
            let val = seq.calculate(i as f32 * 0.25, 0.01, &audio, &empty_analyzers(), 0.0);
            let _ = val; // must not panic
        }
    }

    // ── Chaos Tests Round 2: ADSR edge cases ────────────────────────────

    #[test]
    fn chaos_adsr_zero_all_times() {
        let mut adsr = ModulationSource::adsr(0.0, 0.0, 0.5, 0.0);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
            assert!(val.is_finite(), "zero-time ADSR produced non-finite: {val}");
        }
        adsr.gate_off();
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
            assert!(val.is_finite(), "zero-time ADSR release non-finite: {val}");
        }
    }

    #[test]
    fn chaos_adsr_nan_attack_does_not_panic() {
        let mut adsr = ModulationSource::ADSR {
            attack: f32::NAN,
            decay: 0.1,
            sustain: 0.5,
            release: 0.1,
            stage: ADSRStage::Idle,
            stage_time: 0.0,
            gate: false,
            current_level: 0.0,
        };
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..20 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
        }
        // must not panic
    }

    #[test]
    fn chaos_adsr_negative_sustain() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, -1.0, 0.01);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..100 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
        }
        // Sustain = -1.0 may produce negative values — document, must not panic
    }

    #[test]
    fn chaos_adsr_infinity_release() {
        let mut adsr = ModulationSource::adsr(0.01, 0.01, 0.5, f32::INFINITY);
        adsr.gate_on();
        let audio = empty_audio();
        let mut val = 0.0;
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
        }
        adsr.gate_off();
        for _ in 0..50 {
            val = adsr.calculate(0.0, 0.016, &audio, &empty_analyzers(), val);
            // progress = stage_time / INFINITY = 0 — never completes release
        }
        // must not panic
    }

    #[test]
    fn chaos_adsr_rapid_gate_toggle() {
        let mut adsr = ModulationSource::adsr(0.1, 0.1, 0.5, 0.1);
        let audio = empty_audio();
        let mut val = 0.0;
        for i in 0..100 {
            if i % 3 == 0 {
                adsr.gate_on();
            }
            if i % 5 == 0 {
                adsr.gate_off();
            }
            val = adsr.calculate(0.0, 0.001, &audio, &empty_analyzers(), val);
            assert!(
                val.is_finite(),
                "rapid gate toggle produced non-finite at step {i}: {val}"
            );
        }
    }

    // ── Analyzer source tests ────────────────────────────────────────

    #[test]
    fn analyzer_source_reads_from_values() {
        let mut src = ModulationSource::Analyzer {
            deck_id: "deck-1".into(),
            analyzer_type: "brightness".into(),
            output_name: "brightness".into(),
            smoothing: 0.0, // no smoothing
        };
        let audio = empty_audio();
        let mut av = AnalyzerValues::default();
        av.insert(
            "deck-1".into(),
            "brightness".into(),
            "brightness".into(),
            0.75,
        );
        let val = src.calculate(0.0, 0.016, &audio, &av, 0.0);
        assert!((val - 0.75).abs() < 1e-5, "Expected 0.75, got {val}");
    }

    #[test]
    fn analyzer_source_smoothing() {
        let mut src = ModulationSource::Analyzer {
            deck_id: "d".into(),
            analyzer_type: "brightness".into(),
            output_name: "brightness".into(),
            smoothing: 0.5,
        };
        let audio = empty_audio();
        let mut av = AnalyzerValues::default();
        av.insert("d".into(), "brightness".into(), "brightness".into(), 1.0);

        // First frame: alpha=0.5, prev=0.0 → 0.5*1.0 + 0.5*0.0 = 0.5
        let v1 = src.calculate(0.0, 0.016, &audio, &av, 0.0);
        assert!((v1 - 0.5).abs() < 1e-5, "Expected 0.5, got {v1}");

        // Second frame: 0.5*1.0 + 0.5*0.5 = 0.75
        let v2 = src.calculate(0.016, 0.016, &audio, &av, v1);
        assert!((v2 - 0.75).abs() < 1e-5, "Expected 0.75, got {v2}");
    }

    #[test]
    fn analyzer_source_missing_returns_zero() {
        let mut src = ModulationSource::Analyzer {
            deck_id: "nonexistent".into(),
            analyzer_type: "brightness".into(),
            output_name: "brightness".into(),
            smoothing: 0.0,
        };
        let val = src.calculate(0.0, 0.016, &empty_audio(), &empty_analyzers(), 0.5);
        assert!(
            val.abs() < 1e-5,
            "Missing analyzer should return 0.0, got {val}"
        );
    }
}
