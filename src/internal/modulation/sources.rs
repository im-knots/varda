//! Modulation source types and their computation logic.

use super::{
    ADSRStage, AnalyzerValues, AudioBandPreset, AudioReactMode, AudioValues, LFOWaveform,
    StepInterpolation,
};
use serde::{Deserialize, Serialize};

fn default_noise_gate() -> f32 {
    0.1
}

fn default_analyzer_smoothing() -> f32 {
    0.3
}

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
    /// Analyzer output — reads scalar values from a running analyzer on a specific deck.
    Analyzer {
        /// UUID of the deck whose analyzer to read from.
        deck_id: String,
        /// Type of analyzer (e.g. "brightness", "face_detect").
        analyzer_type: String,
        /// Name of the scalar output (e.g. "brightness", "face_x").
        output_name: String,
        /// Smoothing factor (0.0 = no smoothing, 0.99 = heavy smoothing).
        #[serde(default = "default_analyzer_smoothing")]
        smoothing: f32,
    },
}

impl ModulationSource {
    /// Compare two sources by configuration fields only.
    /// Ignores ADSR runtime state (stage, stage_time, gate, current_level).
    pub fn config_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                ModulationSource::LFO {
                    waveform: w1,
                    frequency: f1,
                    phase: p1,
                    amplitude: a1,
                    bipolar: b1,
                },
                ModulationSource::LFO {
                    waveform: w2,
                    frequency: f2,
                    phase: p2,
                    amplitude: a2,
                    bipolar: b2,
                },
            ) => w1 == w2 && f1 == f2 && p1 == p2 && a1 == a2 && b1 == b2,
            (
                ModulationSource::AudioBand {
                    source_id: s1,
                    freq_low: fl1,
                    freq_high: fh1,
                    gain: g1,
                    smoothing: sm1,
                    mode: m1,
                    noise_gate: ng1,
                },
                ModulationSource::AudioBand {
                    source_id: s2,
                    freq_low: fl2,
                    freq_high: fh2,
                    gain: g2,
                    smoothing: sm2,
                    mode: m2,
                    noise_gate: ng2,
                },
            ) => {
                s1 == s2
                    && fl1 == fl2
                    && fh1 == fh2
                    && g1 == g2
                    && sm1 == sm2
                    && m1 == m2
                    && ng1 == ng2
            }
            (
                ModulationSource::ADSR {
                    attack: a1,
                    decay: d1,
                    sustain: s1,
                    release: r1,
                    ..
                },
                ModulationSource::ADSR {
                    attack: a2,
                    decay: d2,
                    sustain: s2,
                    release: r2,
                    ..
                },
            ) => a1 == a2 && d1 == d2 && s1 == s2 && r1 == r2,
            (
                ModulationSource::StepSequencer {
                    steps: s1,
                    rate: r1,
                    interpolation: i1,
                    bipolar: b1,
                },
                ModulationSource::StepSequencer {
                    steps: s2,
                    rate: r2,
                    interpolation: i2,
                    bipolar: b2,
                },
            ) => s1 == s2 && r1 == r2 && i1 == i2 && b1 == b2,
            (
                ModulationSource::Analyzer {
                    deck_id: d1,
                    analyzer_type: at1,
                    output_name: on1,
                    smoothing: sm1,
                },
                ModulationSource::Analyzer {
                    deck_id: d2,
                    analyzer_type: at2,
                    output_name: on2,
                    smoothing: sm2,
                },
            ) => d1 == d2 && at1 == at2 && on1 == on2 && sm1 == sm2,
            _ => false,
        }
    }

    pub fn sine_lfo(frequency: f32) -> Self {
        ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: false,
        }
    }

    pub fn audio_from_preset(preset: AudioBandPreset) -> Self {
        let (freq_low, freq_high) = preset.freq_range();
        ModulationSource::AudioBand {
            source_id: None,
            freq_low,
            freq_high,
            gain: 1.0,
            smoothing: 0.6,
            mode: AudioReactMode::Direct,
            noise_gate: 0.1,
        }
    }

    pub fn adsr(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        ModulationSource::ADSR {
            attack,
            decay,
            sustain,
            release,
            stage: ADSRStage::Idle,
            stage_time: 0.0,
            gate: false,
            current_level: 0.0,
        }
    }

    pub fn step_sequencer(num_steps: usize, rate: f32) -> Self {
        ModulationSource::StepSequencer {
            steps: vec![0.0; num_steps.max(2)],
            rate,
            interpolation: StepInterpolation::None,
            bipolar: false,
        }
    }

    pub fn gate_on(&mut self) {
        if let ModulationSource::ADSR {
            stage,
            stage_time,
            gate,
            ..
        } = self
        {
            *gate = true;
            *stage = ADSRStage::Attack;
            *stage_time = 0.0;
        }
    }

    pub fn gate_off(&mut self) {
        if let ModulationSource::ADSR {
            stage,
            stage_time,
            gate,
            ..
        } = self
        {
            *gate = false;
            if *stage != ADSRStage::Idle {
                *stage = ADSRStage::Release;
                *stage_time = 0.0;
            }
        }
    }

    /// Calculate current value of this modulation source.
    /// Returns value in range [-1, 1] for bipolar or [0, 1] for unipolar.
    pub fn calculate(
        &mut self,
        time: f32,
        dt: f32,
        audio: &AudioValues,
        analyzers: &AnalyzerValues,
        prev_value: f32,
    ) -> f32 {
        match self {
            ModulationSource::LFO {
                waveform,
                frequency,
                phase,
                amplitude,
                bipolar,
            } => {
                let t = (time * *frequency + *phase) % 1.0;
                let raw = match waveform {
                    LFOWaveform::Sine => (t * std::f32::consts::TAU).sin(),
                    LFOWaveform::Square => {
                        if t < 0.5 {
                            1.0
                        } else {
                            -1.0
                        }
                    }
                    LFOWaveform::Triangle => 1.0 - 4.0 * (t - 0.5).abs(),
                    LFOWaveform::Sawtooth => 2.0 * t - 1.0,
                    LFOWaveform::Random => {
                        let seed = (time * *frequency).floor() as u32;
                        let hash = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                        (hash as f32 / u32::MAX as f32) * 2.0 - 1.0
                    }
                };
                let scaled = raw * *amplitude;
                if *bipolar {
                    scaled
                } else {
                    scaled * 0.5 + 0.5
                }
            }
            ModulationSource::AudioBand {
                source_id,
                freq_low,
                freq_high,
                gain,
                smoothing,
                mode,
                noise_gate,
            } => {
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
                let raw = if raw_signal < *noise_gate {
                    0.0
                } else {
                    raw_signal
                };
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
                        if raw <= 0.0 {
                            prev_value
                        } else {
                            let speed = (1.0 - *smoothing * 0.9) * 4.0;
                            let step = raw * dt * speed;
                            let next = prev_value + step;
                            if next >= 1.0 {
                                next - 1.0
                            } else {
                                next
                            }
                        }
                    }
                    AudioReactMode::Decrease => {
                        if raw <= 0.0 {
                            prev_value
                        } else {
                            let speed = (1.0 - *smoothing * 0.9) * 4.0;
                            let step = raw * dt * speed;
                            let next = prev_value - step;
                            if next <= 0.0 {
                                next + 1.0
                            } else {
                                next
                            }
                        }
                    }
                }
            }
            ModulationSource::ADSR {
                attack,
                decay,
                sustain,
                release,
                stage,
                stage_time,
                current_level,
                ..
            } => {
                *stage_time += dt;
                match stage {
                    ADSRStage::Idle => {
                        *current_level = 0.0;
                    }
                    ADSRStage::Attack => {
                        let progress = if *attack > 0.001 {
                            *stage_time / *attack
                        } else {
                            1.0
                        };
                        if progress >= 1.0 {
                            *current_level = 1.0;
                            *stage = ADSRStage::Decay;
                            *stage_time = 0.0;
                        } else {
                            *current_level = progress;
                        }
                    }
                    ADSRStage::Decay => {
                        let progress = if *decay > 0.001 {
                            *stage_time / *decay
                        } else {
                            1.0
                        };
                        if progress >= 1.0 {
                            *current_level = *sustain;
                            *stage = ADSRStage::Sustain;
                            *stage_time = 0.0;
                        } else {
                            *current_level = 1.0 - (1.0 - *sustain) * progress;
                        }
                    }
                    ADSRStage::Sustain => {
                        *current_level = *sustain;
                    }
                    ADSRStage::Release => {
                        let start_level = *current_level;
                        let progress = if *release > 0.001 {
                            *stage_time / *release
                        } else {
                            1.0
                        };
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
            ModulationSource::StepSequencer {
                steps,
                rate,
                interpolation,
                bipolar,
            } => {
                if steps.is_empty() {
                    return 0.0;
                }
                let total_steps = steps.len() as f32;
                let position = (time * *rate) % total_steps;
                // position is already in [0, total_steps) after the modulo above,
                // so truncation to usize is bounded by [0, steps.len()-1].
                let current_idx = position as usize;
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
                if *bipolar {
                    raw * 2.0 - 1.0
                } else {
                    raw
                }
            }
            ModulationSource::Analyzer {
                deck_id,
                analyzer_type,
                output_name,
                smoothing,
            } => {
                let raw = analyzers.get(deck_id, analyzer_type, output_name);
                // Exponential smoothing: smoothed = α * raw + (1 - α) * prev
                let alpha = 1.0 - *smoothing;
                alpha * raw + *smoothing * prev_value
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::modulation::AudioSourceValues;

    /// Build an `AudioValues` whose single primary source reads full-scale
    /// energy (`energy_in_range ≈ 1.0`) across the whole spectrum. A flat
    /// FFT of 1.0 gives RMS = 1.0 → `(20*log10(1) + 60)/60 = 1.0`.
    fn loud_audio() -> AudioValues {
        let mut av = AudioValues::default();
        av.sources.insert(
            0,
            AudioSourceValues {
                fft: vec![1.0; 256],
                level: 1.0,
                sample_rate: 48000.0,
            },
        );
        av
    }

    /// Silent audio: no sources, so `raw` collapses to 0.0.
    fn silent_audio() -> AudioValues {
        AudioValues::default()
    }

    fn full_band(mode: AudioReactMode, smoothing: f32, noise_gate: f32) -> ModulationSource {
        ModulationSource::AudioBand {
            source_id: None,
            freq_low: 20.0,
            freq_high: 20000.0,
            gain: 1.0,
            smoothing,
            mode,
            noise_gate,
        }
    }

    #[test]
    fn audio_band_direct_tracks_rising_signal_immediately() {
        let mut src = full_band(AudioReactMode::Direct, 0.5, 0.0);
        let out = src.calculate(0.0, 0.01, &loud_audio(), &AnalyzerValues::default(), 0.0);
        // raw (~1.0) >= prev (0.0) → attack is instantaneous, clamped to 1.0.
        assert!((out - 1.0).abs() < 0.01, "expected ~1.0, got {out}");
    }

    #[test]
    fn audio_band_direct_releases_toward_zero_by_release_alpha() {
        // raw = 0 (silent) < prev → decay branch: prev + (1 - smoothing) * (0 - prev).
        let mut src = full_band(AudioReactMode::Direct, 0.5, 0.0);
        let out = src.calculate(0.0, 0.01, &silent_audio(), &AnalyzerValues::default(), 0.8);
        // 0.8 + 0.5 * (0.0 - 0.8) = 0.4
        assert!((out - 0.4).abs() < 1e-5, "expected 0.4, got {out}");
    }

    #[test]
    fn audio_band_direct_full_release_when_smoothing_zero() {
        // release_alpha = 1.0 → collapses straight to raw (0.0).
        let mut src = full_band(AudioReactMode::Direct, 0.0, 0.0);
        let out = src.calculate(0.0, 0.01, &silent_audio(), &AnalyzerValues::default(), 0.8);
        assert!(out.abs() < 1e-6, "expected 0.0, got {out}");
    }

    #[test]
    fn audio_band_increase_accumulates_upward() {
        // speed = (1 - 0) * 4 = 4.0; step = raw(~1.0) * dt(0.01) * 4 = ~0.04.
        let mut src = full_band(AudioReactMode::Increase, 0.0, 0.0);
        let out = src.calculate(0.0, 0.01, &loud_audio(), &AnalyzerValues::default(), 0.0);
        assert!((out - 0.04).abs() < 0.001, "expected ~0.04, got {out}");
    }

    #[test]
    fn audio_band_increase_wraps_past_one() {
        // prev 0.98 + ~0.04 = ~1.02 >= 1.0 → wraps to ~0.02.
        let mut src = full_band(AudioReactMode::Increase, 0.0, 0.0);
        let out = src.calculate(0.0, 0.01, &loud_audio(), &AnalyzerValues::default(), 0.98);
        assert!(
            (out - 0.02).abs() < 0.001,
            "expected ~0.02 after wrap, got {out}"
        );
    }

    #[test]
    fn audio_band_increase_holds_when_signal_idle() {
        // raw <= 0 → value is held, not advanced.
        let mut src = full_band(AudioReactMode::Increase, 0.0, 0.0);
        let out = src.calculate(0.0, 0.01, &silent_audio(), &AnalyzerValues::default(), 0.42);
        assert!((out - 0.42).abs() < 1e-6, "expected held 0.42, got {out}");
    }

    #[test]
    fn audio_band_decrease_accumulates_downward() {
        // step ~0.04; prev 1.0 - 0.04 = ~0.96.
        let mut src = full_band(AudioReactMode::Decrease, 0.0, 0.0);
        let out = src.calculate(0.0, 0.01, &loud_audio(), &AnalyzerValues::default(), 1.0);
        assert!((out - 0.96).abs() < 0.001, "expected ~0.96, got {out}");
    }

    #[test]
    fn audio_band_decrease_wraps_below_zero() {
        // prev 0.02 - ~0.04 = ~-0.02 <= 0.0 → wraps to ~0.98.
        let mut src = full_band(AudioReactMode::Decrease, 0.0, 0.0);
        let out = src.calculate(0.0, 0.01, &loud_audio(), &AnalyzerValues::default(), 0.02);
        assert!(
            (out - 0.98).abs() < 0.001,
            "expected ~0.98 after wrap, got {out}"
        );
    }

    #[test]
    fn audio_band_decrease_holds_when_signal_idle() {
        let mut src = full_band(AudioReactMode::Decrease, 0.0, 0.0);
        let out = src.calculate(0.0, 0.01, &silent_audio(), &AnalyzerValues::default(), 0.42);
        assert!((out - 0.42).abs() < 1e-6, "expected held 0.42, got {out}");
    }

    #[test]
    fn audio_band_noise_gate_zeroes_signal_below_threshold() {
        // raw ~1.0 but noise_gate 1.5 > raw → gated to 0.0. Under Direct with a
        // nonzero prev this drives the decay branch toward 0.
        let mut src = full_band(AudioReactMode::Direct, 0.0, 1.5);
        let out = src.calculate(0.0, 0.01, &loud_audio(), &AnalyzerValues::default(), 0.8);
        assert!(
            out.abs() < 1e-6,
            "gated signal should release to 0, got {out}"
        );
    }
}
