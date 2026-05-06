//! Parameter modulation engine for automating shader parameters
//!
//! Supports LFOs, envelopes, and audio-reactive modulation sources.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// LFO waveform types
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AudioReactMode {
    /// Direct: output = audio energy (standard envelope follower)
    Direct,
    /// Increase: audio energy sweeps the value upward (accumulates)
    Increase,
    /// Decrease: audio energy sweeps the value downward (accumulates)
    Decrease,
}

impl Default for AudioReactMode {
    fn default() -> Self { AudioReactMode::Direct }
}

fn default_noise_gate() -> f32 { 0.1 }

/// Audio frequency band presets (convenience for UI quick-select).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum AudioBandPreset {
    Low,    // 20–250 Hz
    Mid,    // 250–2000 Hz
    High,   // 2000–20000 Hz
    Full,   // 20–20000 Hz (overall level)
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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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

/// Modulation source types
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ModulationSource {
    /// Low Frequency Oscillator
    LFO {
        waveform: LFOWaveform,
        frequency: f32, // Hz (0.01 - 20.0)
        phase: f32,     // Phase offset 0.0 - 1.0
        amplitude: f32, // 0.0 - 1.0, scales the output range
        bipolar: bool,  // true: -1 to 1, false: 0 to 1
    },
    /// Audio FFT reactivity with custom frequency range
    AudioBand {
        /// Which audio source device to read from (None = first/default)
        source_id: Option<crate::audio::AudioSourceId>,
        /// Low frequency bound in Hz (default 20.0)
        freq_low: f32,
        /// High frequency bound in Hz (default 250.0)
        freq_high: f32,
        /// Gain multiplier (1.0 = unity, 0.0-10.0)
        gain: f32,
        /// Smoothing / fall-off (0.0 = instant, 0.99 = very slow)
        smoothing: f32,
        /// How audio energy drives the output value
        #[serde(default)]
        mode: AudioReactMode,
        /// Noise gate threshold (0.0–1.0). Signal below this reads as zero.
        #[serde(default = "default_noise_gate")]
        noise_gate: f32,
    },
    /// ADSR envelope generator
    ADSR {
        attack: f32,         // Attack time in seconds (0.001 - 10.0)
        decay: f32,          // Decay time in seconds (0.001 - 10.0)
        sustain: f32,        // Sustain level (0.0 - 1.0)
        release: f32,        // Release time in seconds (0.001 - 10.0)
        #[serde(skip)]
        stage: ADSRStage,    // Current envelope stage
        #[serde(skip)]
        stage_time: f32,     // Time within current stage
        #[serde(skip)]
        gate: bool,          // Gate signal (true = held)
        #[serde(skip)]
        current_level: f32,  // Current envelope output level
    },
    /// Step sequencer
    StepSequencer {
        steps: Vec<f32>,          // Step values (0.0 - 1.0), typically 4/8/16 steps
        rate: f32,                // Rate in Hz (steps per second)
        interpolation: StepInterpolation,
        bipolar: bool,            // true: -1 to 1, false: 0 to 1
    },
}

impl ModulationSource {
    /// Create a new sine LFO
    pub fn sine_lfo(frequency: f32) -> Self {
        ModulationSource::LFO {
            waveform: LFOWaveform::Sine,
            frequency,
            phase: 0.0,
            amplitude: 1.0,
            bipolar: false,
        }
    }

    /// Create a new audio FFT source from a preset band
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

    /// Create a new ADSR envelope with default parameters
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

    /// Create a new step sequencer with default steps
    pub fn step_sequencer(num_steps: usize, rate: f32) -> Self {
        ModulationSource::StepSequencer {
            steps: vec![0.0; num_steps.max(2)],
            rate,
            interpolation: StepInterpolation::None,
            bipolar: false,
        }
    }

    /// Trigger ADSR gate on (start attack)
    pub fn gate_on(&mut self) {
        if let ModulationSource::ADSR { stage, stage_time, gate, .. } = self {
            *gate = true;
            *stage = ADSRStage::Attack;
            *stage_time = 0.0;
        }
    }

    /// Trigger ADSR gate off (start release)
    pub fn gate_off(&mut self) {
        if let ModulationSource::ADSR { stage, stage_time, gate, .. } = self {
            *gate = false;
            if *stage != ADSRStage::Idle {
                *stage = ADSRStage::Release;
                *stage_time = 0.0;
            }
        }
    }

    /// Calculate current value of this modulation source
    /// time: current time in seconds, dt: delta time since last frame
    /// audio: current audio analysis data
    /// Returns value in range [-1, 1] for bipolar or [0, 1] for unipolar
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
                // Look up the audio source: specific ID, or fall back to primary
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
                // Apply noise gate: signal below threshold reads as zero
                let raw = if raw_signal < *noise_gate { 0.0 } else { raw_signal };
                match mode {
                    AudioReactMode::Direct => {
                        // Asymmetric envelope: instant attack, smooth release
                        if raw >= prev_value {
                            raw.clamp(0.0, 1.0)
                        } else {
                            let release_alpha = 1.0 - *smoothing;
                            (prev_value + release_alpha * (raw - prev_value)).clamp(0.0, 1.0)
                        }
                    }
                    AudioReactMode::Increase => {
                        if raw <= 0.0 {
                            prev_value // hold when gated/quiet
                        } else {
                            // Speed: smoothing 0.0 = very fast, 0.99 = slow
                            // At smoothing=0.5 with raw=0.5, sweep 0→1 in ~0.5s
                            let speed = (1.0 - *smoothing * 0.9) * 4.0;
                            let step = raw * dt * speed;
                            let next = prev_value + step;
                            if next >= 1.0 { next - 1.0 } else { next }
                        }
                    }
                    AudioReactMode::Decrease => {
                        if raw <= 0.0 {
                            prev_value // hold when gated/quiet
                        } else {
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
                    ADSRStage::Idle => {
                        *current_level = 0.0;
                    }
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
                    ADSRStage::Sustain => {
                        *current_level = *sustain;
                    }
                    ADSRStage::Release => {
                        let start_level = *current_level;
                        let progress = if *release > 0.001 { *stage_time / *release } else { 1.0 };
                        if progress >= 1.0 {
                            *current_level = 0.0;
                            *stage = ADSRStage::Idle;
                            *stage_time = 0.0;
                        } else {
                            // Release from wherever we were (handles early release during attack/decay)
                            *current_level = start_level * (1.0 - progress);
                        }
                    }
                }
                *current_level
            }
            ModulationSource::StepSequencer { steps, rate, interpolation, bipolar } => {
                if steps.is_empty() {
                    return 0.0;
                }
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
                        // Smoothstep interpolation
                        let t = frac * frac * (3.0 - 2.0 * frac);
                        steps[current_idx] * (1.0 - t) + steps[next_idx] * t
                    }
                };
                if *bipolar { raw * 2.0 - 1.0 } else { raw }
            }
        }
    }
}

/// Audio analysis values for a single source, passed to modulation engine.
#[derive(Debug, Clone)]
pub struct AudioSourceValues {
    pub fft: Vec<f32>,
    pub level: f32,
    pub sample_rate: f32,
}

impl AudioSourceValues {
    /// Compute energy in a frequency range from the FFT data.
    /// Returns a perceptually-scaled value in roughly 0.0–1.0 range
    /// suitable for driving modulation (dB-based mapping).
    pub fn energy_in_range(&self, freq_low: f32, freq_high: f32) -> f32 {
        if self.fft.is_empty() || self.sample_rate <= 0.0 { return 0.0; }
        let fft_size = self.fft.len() * 2; // fft stores half (positive frequencies)
        let bin_width = self.sample_rate / fft_size as f32;
        let bin_low = ((freq_low / bin_width).floor() as usize).min(self.fft.len() - 1);
        let bin_high = ((freq_high / bin_width).ceil() as usize).min(self.fft.len());
        if bin_high <= bin_low { return 0.0; }
        let slice = &self.fft[bin_low..bin_high];
        // RMS energy
        let rms = (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt();
        // Convert to dB-based perceptual scale:
        // -60dB (0.001) → 0.0, 0dB (1.0) → 1.0
        // This maps typical mic signals into a usable range
        if rms < 1e-6 { return 0.0; }
        let db = 20.0 * rms.log10(); // negative dB value
        ((db + 60.0) / 60.0).clamp(0.0, 1.0)
    }
}

/// All audio source data for the current frame.
#[derive(Debug, Clone, Default)]
pub struct AudioValues {
    /// Per-source audio data, keyed by AudioSourceId.
    pub sources: std::collections::HashMap<crate::audio::AudioSourceId, AudioSourceValues>,
}

impl AudioValues {
    /// Get the first/primary source's data (convenience).
    pub fn primary(&self) -> Option<&AudioSourceValues> {
        // Return the source with the lowest ID
        self.sources.iter().min_by_key(|(id, _)| **id).map(|(_, v)| v)
    }
}

/// Modulation assignment linking a source to a parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamModulation {
    /// Index into ModulationEngine.sources
    pub source_idx: usize,
    /// Modulation depth/amount (-1.0 to 1.0, negative inverts)
    pub amount: f32,
    /// For color params: which component (0=R, 1=G, 2=B, 3=A), None for scalar
    pub component: Option<usize>,
}

/// Modulation engine manages sources and assignments for a deck
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModulationEngine {
    /// Available modulation sources
    pub sources: Vec<ModulationSource>,
    /// Map from parameter name to list of modulations
    pub assignments: HashMap<String, Vec<ParamModulation>>,
    /// Cached previous values for smoothing (source_idx -> value)
    #[serde(skip)]
    prev_values: Vec<f32>,
    /// Cached current source values (updated per frame)
    #[serde(skip)]
    current_values: Vec<f32>,
    /// Previous frame time for computing dt
    #[serde(skip)]
    prev_time: Option<f32>,
}

impl ModulationEngine {
    /// Create a new empty modulation engine
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a new modulation source, returns its index
    pub fn add_source(&mut self, source: ModulationSource) -> usize {
        let idx = self.sources.len();
        self.sources.push(source);
        self.prev_values.push(0.0);
        self.current_values.push(0.0);
        idx
    }

    /// Remove a modulation source by index
    pub fn remove_source(&mut self, idx: usize) {
        if idx < self.sources.len() {
            self.sources.remove(idx);
            self.prev_values.remove(idx);
            self.current_values.remove(idx);
            // Update all assignments to reflect removed index
            for mods in self.assignments.values_mut() {
                mods.retain(|m| m.source_idx != idx);
                for m in mods.iter_mut() {
                    if m.source_idx > idx {
                        m.source_idx -= 1;
                    }
                }
            }
        }
    }

    /// Assign a modulation source to a parameter
    pub fn assign(&mut self, param_name: &str, source_idx: usize, amount: f32, component: Option<usize>) {
        if source_idx >= self.sources.len() {
            return;
        }
        let modulation = ParamModulation {
            source_idx,
            amount,
            component,
        };
        self.assignments
            .entry(param_name.to_string())
            .or_insert_with(Vec::new)
            .push(modulation);
    }

    /// Assign a modulation source to another source's parameter (mod-on-mod)
    pub fn assign_mod_on_mod(&mut self, target_source_idx: usize, param_name: &str, modulator_idx: usize, amount: f32) {
        let key = format!("mod:{}:{}", target_source_idx, param_name);
        self.assign(&key, modulator_idx, amount, None);
    }

    /// Remove mod-on-mod assignments for a source parameter
    pub fn clear_mod_on_mod(&mut self, target_source_idx: usize, param_name: &str) {
        let key = format!("mod:{}:{}", target_source_idx, param_name);
        self.assignments.remove(&key);
    }

    /// Remove all modulations from a parameter
    pub fn clear_assignments(&mut self, param_name: &str) {
        self.assignments.remove(param_name);
    }

    /// Trigger ADSR gate on for source at index
    pub fn trigger_adsr(&mut self, idx: usize) {
        if idx < self.sources.len() {
            self.sources[idx].gate_on();
        }
    }

    /// Trigger ADSR gate off for source at index
    pub fn release_adsr(&mut self, idx: usize) {
        if idx < self.sources.len() {
            self.sources[idx].gate_off();
        }
    }

    /// Get the modulation offset for a mod-source parameter (for mod-on-mod)
    /// Uses only already-computed current_values (sources evaluated before this one)
    fn get_mod_source_offset(&self, source_idx: usize, param_name: &str) -> f32 {
        let key = format!("mod:{}:{}", source_idx, param_name);
        self.get_modulation(&key)
    }

    /// Apply mod-on-mod offsets to a source's parameters before evaluation.
    /// Returns a clone of the source with modified parameters.
    fn apply_mod_on_mod(&self, idx: usize, source: &ModulationSource) -> ModulationSource {
        let mut modified = source.clone();
        match &mut modified {
            ModulationSource::LFO { frequency, phase, amplitude, .. } => {
                *frequency = (*frequency + self.get_mod_source_offset(idx, "frequency")).max(0.001);
                *phase = (*phase + self.get_mod_source_offset(idx, "phase")).clamp(0.0, 1.0);
                *amplitude = (*amplitude + self.get_mod_source_offset(idx, "amplitude")).clamp(0.0, 1.0);
            }
            ModulationSource::AudioBand { gain, smoothing, .. } => {
                *gain = (*gain + self.get_mod_source_offset(idx, "gain")).max(0.0);
                *smoothing = (*smoothing + self.get_mod_source_offset(idx, "smoothing")).clamp(0.0, 0.99);
            }
            ModulationSource::ADSR { attack, decay, sustain, release, .. } => {
                *attack = (*attack + self.get_mod_source_offset(idx, "attack")).max(0.001);
                *decay = (*decay + self.get_mod_source_offset(idx, "decay")).max(0.001);
                *sustain = (*sustain + self.get_mod_source_offset(idx, "sustain")).clamp(0.0, 1.0);
                *release = (*release + self.get_mod_source_offset(idx, "release")).max(0.001);
            }
            ModulationSource::StepSequencer { rate, .. } => {
                *rate = (*rate + self.get_mod_source_offset(idx, "rate")).max(0.01);
            }
        }
        modified
    }

    /// Build topological evaluation order for mod-on-mod dependencies.
    /// Returns indices in evaluation order (leaves first). Depth limited to MAX_MOD_DEPTH.
    fn evaluation_order(&self) -> Vec<usize> {
        const MAX_MOD_DEPTH: usize = 4;
        let n = self.sources.len();
        if n == 0 { return vec![]; }

        // Build dependency graph: deps[i] = set of source indices that source i depends on
        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (key, mods) in &self.assignments {
            if let Some(target_idx) = Self::parse_mod_target(key) {
                if target_idx < n {
                    for m in mods {
                        if m.source_idx < n && m.source_idx != target_idx {
                            deps[target_idx].push(m.source_idx);
                        }
                    }
                }
            }
        }

        // Simple topological sort with depth limit
        let mut order = Vec::with_capacity(n);
        let mut evaluated = vec![false; n];

        for _pass in 0..MAX_MOD_DEPTH {
            let mut progress = false;
            for i in 0..n {
                if evaluated[i] { continue; }
                // Check if all dependencies are evaluated
                let all_deps_met = deps[i].iter().all(|&d| evaluated[d]);
                if all_deps_met {
                    order.push(i);
                    evaluated[i] = true;
                    progress = true;
                }
            }
            if !progress { break; }
        }

        // Add any remaining (cycle-breaking: evaluate with whatever values are available)
        for i in 0..n {
            if !evaluated[i] {
                order.push(i);
            }
        }

        order
    }

    /// Parse a mod-on-mod target key like "mod:3:frequency" → Some(3)
    fn parse_mod_target(key: &str) -> Option<usize> {
        let parts: Vec<&str> = key.splitn(3, ':').collect();
        if parts.len() >= 2 && parts[0] == "mod" {
            parts[1].parse().ok()
        } else {
            None
        }
    }

    /// Update all source values for the current frame
    pub fn update(&mut self, time: f32, audio: &AudioValues) {
        let dt = self.prev_time.map_or(0.016, |prev| time - prev);
        self.prev_time = Some(time);

        // Ensure vectors are sized correctly
        while self.prev_values.len() < self.sources.len() {
            self.prev_values.push(0.0);
        }
        while self.current_values.len() < self.sources.len() {
            self.current_values.push(0.0);
        }

        // Evaluate in dependency order for mod-on-mod support
        let order = self.evaluation_order();
        for i in order {
            // Apply mod-on-mod offsets to get effective parameters
            let mut effective = self.apply_mod_on_mod(i, &self.sources[i]);
            let value = effective.calculate(time, dt, audio, self.prev_values[i]);

            // Copy back any mutable state changes (ADSR stage progression)
            match (&mut self.sources[i], &effective) {
                (ModulationSource::ADSR { stage, stage_time, current_level, .. },
                 ModulationSource::ADSR { stage: eff_stage, stage_time: eff_st, current_level: eff_cl, .. }) => {
                    *stage = *eff_stage;
                    *stage_time = *eff_st;
                    *current_level = *eff_cl;
                }
                _ => {}
            }

            self.current_values[i] = value;
            self.prev_values[i] = value;
        }
    }

    /// Get the total modulation offset for a scalar parameter
    pub fn get_modulation(&self, param_name: &str) -> f32 {
        self.get_modulation_for_component(param_name, None)
    }

    /// Get the total modulation offset for a specific component (color params)
    pub fn get_modulation_for_component(&self, param_name: &str, component: Option<usize>) -> f32 {
        let Some(mods) = self.assignments.get(param_name) else {
            return 0.0;
        };

        let mut total = 0.0;
        for m in mods {
            // Match component: None matches None, or Some(x) matches Some(x)
            if m.component == component {
                if m.source_idx < self.current_values.len() {
                    total += self.current_values[m.source_idx] * m.amount;
                }
            }
        }
        total
    }

    /// Check if a parameter has any modulations assigned
    pub fn has_modulation(&self, param_name: &str) -> bool {
        self.assignments.get(param_name).map_or(false, |v| !v.is_empty())
    }

    /// Get number of sources
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Get current computed values for all sources (for UI visualization)
    pub fn current_values(&self) -> &[f32] {
        &self.current_values
    }
}

