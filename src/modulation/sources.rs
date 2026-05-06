//! Modulation source types and their computation logic.

use serde::{Deserialize, Serialize};
use super::{LFOWaveform, AudioReactMode, ADSRStage, StepInterpolation, AudioBandPreset, AudioValues};

fn default_noise_gate() -> f32 { 0.1 }

/// Modulation source types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModulationSource {
    /// Low Frequency Oscillator
    LFO {
        waveform: LFOWaveform,
        frequency: f32,
        phase: f32,
        amplitude: f32,
        bipolar: bool,
    },
    /// Audio FFT reactivity with custom frequency range
    AudioBand {
        source_id: Option<crate::audio::AudioSourceId>,
        freq_low: f32,
        freq_high: f32,
        gain: f32,
        smoothing: f32,
        #[serde(default)]
        mode: AudioReactMode,
        #[serde(default = "default_noise_gate")]
        noise_gate: f32,
    },
    /// ADSR envelope generator
    ADSR {
        attack: f32,
        decay: f32,
        sustain: f32,
        release: f32,
        #[serde(skip)]
        stage: ADSRStage,
        #[serde(skip)]
        stage_time: f32,
        #[serde(skip)]
        gate: bool,
        #[serde(skip)]
        current_level: f32,
    },
    /// Step sequencer
    StepSequencer {
        steps: Vec<f32>,
        rate: f32,
        interpolation: StepInterpolation,
        bipolar: bool,
    },
}

impl ModulationSource {
    pub fn sine_lfo(frequency: f32) -> Self {
        ModulationSource::LFO {
            waveform: LFOWaveform::Sine, frequency, phase: 0.0, amplitude: 1.0, bipolar: false,
        }
    }

    pub fn audio_from_preset(preset: AudioBandPreset) -> Self {
        let (freq_low, freq_high) = preset.freq_range();
        ModulationSource::AudioBand {
            source_id: None, freq_low, freq_high, gain: 1.0, smoothing: 0.6,
            mode: AudioReactMode::Direct, noise_gate: 0.1,
        }
    }

    pub fn adsr(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        ModulationSource::ADSR {
            attack, decay, sustain, release,
            stage: ADSRStage::Idle, stage_time: 0.0, gate: false, current_level: 0.0,
        }
    }

    pub fn step_sequencer(num_steps: usize, rate: f32) -> Self {
        ModulationSource::StepSequencer {
            steps: vec![0.0; num_steps.max(2)], rate,
            interpolation: StepInterpolation::None, bipolar: false,
        }
    }

    pub fn gate_on(&mut self) {
        if let ModulationSource::ADSR { stage, stage_time, gate, .. } = self {
            *gate = true;
            *stage = ADSRStage::Attack;
            *stage_time = 0.0;
        }
    }

    pub fn gate_off(&mut self) {
        if let ModulationSource::ADSR { stage, stage_time, gate, .. } = self {
            *gate = false;
            if *stage != ADSRStage::Idle {
                *stage = ADSRStage::Release;
                *stage_time = 0.0;
            }
        }
    }


    /// Calculate current value of this modulation source.
    /// Returns value in range [-1, 1] for bipolar or [0, 1] for unipolar.
    pub fn calculate(&mut self, time: f32, dt: f32, audio: &AudioValues, prev_value: f32) -> f32 {
        match self {
            ModulationSource::LFO { waveform, frequency, phase, amplitude, bipolar } => {
                let t = (time * *frequency + *phase) % 1.0;
                let raw = match waveform {
                    LFOWaveform::Sine => (t * std::f32::consts::TAU).sin(),
                    LFOWaveform::Square => if t < 0.5 { 1.0 } else { -1.0 },
                    LFOWaveform::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                    LFOWaveform::Sawtooth => 2.0 * t - 1.0,
                    LFOWaveform::Random => {
                        let seed = (time * *frequency).floor() as u32;
                        let hash = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        (hash as f32 / u32::MAX as f32) * 2.0 - 1.0
                    }
                };
                let scaled = raw * *amplitude;
                if *bipolar { scaled } else { scaled * 0.5 + 0.5 }
            }
            ModulationSource::AudioBand { source_id, freq_low, freq_high, gain, smoothing, mode, noise_gate } => {
                let source_vals = if let Some(id) = source_id {
                    audio.sources.get(id)
                } else {
                    audio.primary()
                };
                let raw_signal = if let Some(vals) = source_vals {
                    vals.energy_in_range(*freq_low, *freq_high) * *gain
                } else {
                    0.0
                };
                let raw = if raw_signal < *noise_gate { 0.0 } else { raw_signal };
                match mode {
                    AudioReactMode::Direct => {
                        if raw >= prev_value {
                            raw.clamp(0.0, 1.0)
                        } else {
                            let release_alpha = 1.0 - *smoothing;
                            (prev_value + release_alpha * (raw - prev_value)).clamp(0.0, 1.0)
                        }
                    }
                    AudioReactMode::Increase => {
                        if raw <= 0.0 { prev_value } else {
                            let speed = (1.0 - *smoothing * 0.9) * 4.0;
                            let step = raw * dt * speed;
                            let next = prev_value + step;
                            if next >= 1.0 { next - 1.0 } else { next }
                        }
                    }
                    AudioReactMode::Decrease => {
                        if raw <= 0.0 { prev_value } else {
                            let speed = (1.0 - *smoothing * 0.9) * 4.0;
                            let step = raw * dt * speed;
                            let next = prev_value - step;
                            if next <= 0.0 { next + 1.0 } else { next }
                        }
                    }
                }
            }
            ModulationSource::ADSR { attack, decay, sustain, release, stage, stage_time, current_level, .. } => {
                *stage_time += dt;
                match stage {
                    ADSRStage::Idle => { *current_level = 0.0; }
                    ADSRStage::Attack => {
                        let progress = if *attack > 0.001 { *stage_time / *attack } else { 1.0 };
                        if progress >= 1.0 {
                            *current_level = 1.0;
                            *stage = ADSRStage::Decay;
                            *stage_time = 0.0;
                        } else {
                            *current_level = progress;
                        }
                    }
                    ADSRStage::Decay => {
                        let progress = if *decay > 0.001 { *stage_time / *decay } else { 1.0 };
                        if progress >= 1.0 {
                            *current_level = *sustain;
                            *stage = ADSRStage::Sustain;
                            *stage_time = 0.0;
                        } else {
                            *current_level = 1.0 - (1.0 - *sustain) * progress;
                        }
                    }
                    ADSRStage::Sustain => { *current_level = *sustain; }
                    ADSRStage::Release => {
                        let start_level = *current_level;
                        let progress = if *release > 0.001 { *stage_time / *release } else { 1.0 };
                        if progress >= 1.0 {
                            *current_level = 0.0;
                            *stage = ADSRStage::Idle;
                            *stage_time = 0.0;
                        } else {
                            *current_level = start_level * (1.0 - progress);
                        }
                    }
                }
                *current_level
            }
            ModulationSource::StepSequencer { steps, rate, interpolation, bipolar } => {
                if steps.is_empty() { return 0.0; }
                let total_steps = steps.len() as f32;
                let position = (time * *rate) % total_steps;
                let current_idx = position.floor() as usize % steps.len();
                let raw = match interpolation {
                    StepInterpolation::None => steps[current_idx],
                    StepInterpolation::Linear => {
                        let next_idx = (current_idx + 1) % steps.len();
                        let frac = position.fract();
                        steps[current_idx] * (1.0 - frac) + steps[next_idx] * frac
                    }
                    StepInterpolation::Smooth => {
                        let next_idx = (current_idx + 1) % steps.len();
                        let frac = position.fract();
                        let t = frac * frac * (3.0 - 2.0 * frac);
                        steps[current_idx] * (1.0 - t) + steps[next_idx] * t
                    }
                };
                if *bipolar { raw * 2.0 - 1.0 } else { raw }
            }
        }
    }
}