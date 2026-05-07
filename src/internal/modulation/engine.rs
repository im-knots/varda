//! ModulationEngine — manages sources, assignments, and per-frame evaluation.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use super::{ModulationSource, AudioValues, ParamModulation};

/// Modulation engine manages sources and assignments for a deck
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModulationEngine {
    /// Available modulation sources
    pub sources: Vec<ModulationSource>,
    /// Map from parameter name to list of modulations
    pub assignments: HashMap<String, Vec<ParamModulation>>,
    #[serde(skip)]
    prev_values: Vec<f32>,
    #[serde(skip)]
    current_values: Vec<f32>,
    #[serde(skip)]
    prev_time: Option<f32>,
}

impl ModulationEngine {
    pub fn new() -> Self { Self::default() }

    pub fn add_source(&mut self, source: ModulationSource) -> usize {
        let idx = self.sources.len();
        self.sources.push(source);
        self.prev_values.push(0.0);
        self.current_values.push(0.0);
        idx
    }

    pub fn remove_source(&mut self, idx: usize) {
        if idx < self.sources.len() {
            self.sources.remove(idx);
            self.prev_values.remove(idx);
            self.current_values.remove(idx);
            for mods in self.assignments.values_mut() {
                mods.retain(|m| m.source_idx != idx);
                for m in mods.iter_mut() {
                    if m.source_idx > idx { m.source_idx -= 1; }
                }
            }
        }
    }

    pub fn assign(&mut self, param_name: &str, source_idx: usize, amount: f32, component: Option<usize>) {
        if source_idx >= self.sources.len() { return; }
        let modulation = ParamModulation { source_idx, amount, component };
        self.assignments.entry(param_name.to_string()).or_insert_with(Vec::new).push(modulation);
    }

    pub fn assign_mod_on_mod(&mut self, target_source_idx: usize, param_name: &str, modulator_idx: usize, amount: f32) {
        let key = format!("mod:{}:{}", target_source_idx, param_name);
        self.assign(&key, modulator_idx, amount, None);
    }

    pub fn clear_mod_on_mod(&mut self, target_source_idx: usize, param_name: &str) {
        let key = format!("mod:{}:{}", target_source_idx, param_name);
        self.assignments.remove(&key);
    }

    pub fn clear_assignments(&mut self, param_name: &str) {
        self.assignments.remove(param_name);
    }

    pub fn trigger_adsr(&mut self, idx: usize) {
        if idx < self.sources.len() { self.sources[idx].gate_on(); }
    }

    pub fn release_adsr(&mut self, idx: usize) {
        if idx < self.sources.len() { self.sources[idx].gate_off(); }
    }

    fn get_mod_source_offset(&self, source_idx: usize, param_name: &str) -> f32 {
        let key = format!("mod:{}:{}", source_idx, param_name);
        self.get_modulation(&key)
    }

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

    pub(crate) fn evaluation_order(&self) -> Vec<usize> {
        const MAX_MOD_DEPTH: usize = 4;
        let n = self.sources.len();
        if n == 0 { return vec![]; }

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

        let mut order = Vec::with_capacity(n);
        let mut evaluated = vec![false; n];
        for _pass in 0..MAX_MOD_DEPTH {
            let mut progress = false;
            for i in 0..n {
                if evaluated[i] { continue; }
                if deps[i].iter().all(|&d| evaluated[d]) {
                    order.push(i);
                    evaluated[i] = true;
                    progress = true;
                }
            }
            if !progress { break; }
        }
        for i in 0..n {
            if !evaluated[i] { order.push(i); }
        }
        order
    }

    pub(crate) fn parse_mod_target(key: &str) -> Option<usize> {
        let parts: Vec<&str> = key.splitn(3, ':').collect();
        if parts.len() >= 2 && parts[0] == "mod" { parts[1].parse().ok() } else { None }
    }


    /// Update all source values for the current frame
    pub fn update(&mut self, time: f32, audio: &AudioValues) {
        let dt = self.prev_time.map_or(0.016, |prev| time - prev);
        self.prev_time = Some(time);

        while self.prev_values.len() < self.sources.len() { self.prev_values.push(0.0); }
        while self.current_values.len() < self.sources.len() { self.current_values.push(0.0); }

        let order = self.evaluation_order();
        for i in order {
            let mut effective = self.apply_mod_on_mod(i, &self.sources[i]);
            let value = effective.calculate(time, dt, audio, self.prev_values[i]);

            // Copy back mutable state changes (ADSR stage progression)
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
        let Some(mods) = self.assignments.get(param_name) else { return 0.0; };
        let mut total = 0.0;
        for m in mods {
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
    pub fn source_count(&self) -> usize { self.sources.len() }

    /// Get current computed values for all sources (for UI visualization)
    pub fn current_values(&self) -> &[f32] { &self.current_values }
}