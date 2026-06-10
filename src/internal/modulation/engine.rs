//! ModulationEngine — manages sources, assignments, and per-frame evaluation.

use super::{
    AnalyzerValues, AudioValues, ModulationSource, ModulationSourceEntry, ParamModulation,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Modulation engine manages sources and assignments for a deck
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModulationEngine {
    /// Available modulation sources (with stable UUIDs)
    pub sources: Vec<ModulationSourceEntry>,
    /// Map from parameter name to list of modulations
    pub assignments: HashMap<String, Vec<ParamModulation>>,
    /// UUID → index cache for O(1) lookups during tick
    #[serde(skip)]
    uuid_to_idx: HashMap<String, usize>,
    #[serde(skip)]
    prev_values: Vec<f32>,
    #[serde(skip)]
    current_values: Vec<f32>,
    #[serde(skip)]
    prev_time: Option<f32>,
    /// Cached topological evaluation order. Invalidated when assignments change.
    #[serde(skip)]
    cached_order: Vec<usize>,
    /// Whether cached_order needs recomputation.
    #[serde(skip)]
    order_dirty: bool,
    /// Per-source flag: does this source have any mod-on-mod assignments targeting it?
    #[serde(skip)]
    has_mod_on_mod: Vec<bool>,
}

impl ModulationEngine {
    pub fn new() -> Self {
        Self::default()
    }

    fn rebuild_uuid_index(&mut self) {
        self.uuid_to_idx.clear();
        for (i, entry) in self.sources.iter().enumerate() {
            self.uuid_to_idx.insert(entry.uuid.clone(), i);
        }
    }

    /// Ensure uuid_to_idx is populated (needed after deserialization)
    pub fn ensure_index(&mut self) {
        if self.uuid_to_idx.len() != self.sources.len() {
            self.rebuild_uuid_index();
            self.invalidate_order();
        }
    }

    /// Mark the cached evaluation order as stale.
    fn invalidate_order(&mut self) {
        self.order_dirty = true;
    }

    /// Add a new source, returns its UUID
    pub fn add_source(&mut self, source: ModulationSource) -> String {
        let entry = ModulationSourceEntry::new(source);
        let uuid = entry.uuid.clone();
        self.sources.push(entry);
        self.prev_values.push(0.0);
        self.current_values.push(0.0);
        self.has_mod_on_mod.push(false);
        self.uuid_to_idx
            .insert(uuid.clone(), self.sources.len() - 1);
        self.invalidate_order();
        uuid
    }

    /// Add a source with a specific UUID (for preset loading)
    pub fn add_source_with_uuid(&mut self, uuid: String, source: ModulationSource) -> String {
        let entry = ModulationSourceEntry::with_uuid(uuid.clone(), source);
        self.sources.push(entry);
        self.prev_values.push(0.0);
        self.current_values.push(0.0);
        self.has_mod_on_mod.push(false);
        self.uuid_to_idx
            .insert(uuid.clone(), self.sources.len() - 1);
        self.invalidate_order();
        uuid
    }

    /// Remove a source by UUID
    pub fn remove_source(&mut self, uuid: &str) {
        if let Some(idx) = self.uuid_to_idx.get(uuid).copied() {
            self.sources.remove(idx);
            if idx < self.prev_values.len() {
                self.prev_values.remove(idx);
            }
            if idx < self.current_values.len() {
                self.current_values.remove(idx);
            }
            if idx < self.has_mod_on_mod.len() {
                self.has_mod_on_mod.remove(idx);
            }
            // Remove assignments referencing this source (no reindexing needed)
            for mods in self.assignments.values_mut() {
                mods.retain(|m| m.source_id != uuid);
            }
            // Remove mod-on-mod assignments targeting this source
            let mod_prefix = format!("mod:{}:", uuid);
            self.assignments.retain(|k, _| !k.starts_with(&mod_prefix));
            self.rebuild_uuid_index();
            self.invalidate_order();
        }
    }

    /// Remove all assignments whose key starts with the given prefix.
    /// Used to clean up orphaned assignments when a deck or effect is removed.
    pub fn remove_assignments_with_prefix(&mut self, prefix: &str) {
        let before = self.assignments.len();
        self.assignments.retain(|k, _| !k.starts_with(prefix));
        let removed = before - self.assignments.len();
        if removed > 0 {
            self.invalidate_order();
            log::info!(
                "Removed {} orphaned modulation assignments with prefix '{}'",
                removed,
                prefix
            );
        }
    }

    pub fn assign(
        &mut self,
        param_name: &str,
        source_id: &str,
        amount: f32,
        component: Option<usize>,
    ) {
        if !self.uuid_to_idx.contains_key(source_id) {
            self.ensure_index();
            if !self.uuid_to_idx.contains_key(source_id) {
                return;
            }
        }
        let modulation = ParamModulation {
            source_id: source_id.to_string(),
            amount,
            component,
        };
        self.assignments
            .entry(param_name.to_string())
            .or_default()
            .push(modulation);
        self.invalidate_order();
    }

    pub fn assign_mod_on_mod(
        &mut self,
        target_uuid: &str,
        param_name: &str,
        modulator_uuid: &str,
        amount: f32,
    ) {
        let key = format!("mod:{}:{}", target_uuid, param_name);
        self.assign(&key, modulator_uuid, amount, None);
        // assign() already calls invalidate_order()
    }

    pub fn clear_mod_on_mod(&mut self, target_uuid: &str, param_name: &str) {
        let key = format!("mod:{}:{}", target_uuid, param_name);
        self.assignments.remove(&key);
        self.invalidate_order();
    }

    pub fn clear_assignments(&mut self, param_name: &str) {
        self.assignments.remove(param_name);
        self.invalidate_order();
    }

    pub fn trigger_adsr(&mut self, uuid: &str) {
        if let Some(&idx) = self.uuid_to_idx.get(uuid) {
            self.sources[idx].source.gate_on();
        }
    }

    pub fn release_adsr(&mut self, uuid: &str) {
        if let Some(&idx) = self.uuid_to_idx.get(uuid) {
            self.sources[idx].source.gate_off();
        }
    }

    /// Get a mutable reference to a source by UUID
    pub fn source_mut(&mut self, uuid: &str) -> Option<&mut ModulationSource> {
        self.ensure_index();
        self.uuid_to_idx
            .get(uuid)
            .copied()
            .map(|idx| &mut self.sources[idx].source)
    }

    /// Find source by UUID (returns exists check)
    pub fn has_source(&self, uuid: &str) -> bool {
        self.sources.iter().any(|e| e.uuid == uuid)
    }

    fn get_mod_source_offset(&self, source_uuid: &str, param_name: &str) -> f32 {
        // Look up "mod:{uuid}:{param}" without allocating a String.
        // We scan assignments for keys matching this pattern.
        let prefix = "mod:";
        for (key, mods) in &self.assignments {
            if key.starts_with(prefix)
                && key[prefix.len()..].starts_with(source_uuid)
                && key.len() > prefix.len() + source_uuid.len()
                && key.as_bytes()[prefix.len() + source_uuid.len()] == b':'
                && &key[prefix.len() + source_uuid.len() + 1..] == param_name
            {
                let mut total = 0.0;
                for m in mods {
                    let idx = if let Some(&i) = self.uuid_to_idx.get(&m.source_id) {
                        i
                    } else {
                        continue;
                    };
                    if idx < self.current_values.len() {
                        total += self.current_values[idx] * m.amount;
                    }
                }
                return total;
            }
        }
        0.0
    }

    fn apply_mod_on_mod(&self, idx: usize, source: &ModulationSource) -> ModulationSource {
        let uuid = &self.sources[idx].uuid;
        let mut modified = source.clone();
        match &mut modified {
            ModulationSource::LFO {
                frequency,
                phase,
                amplitude,
                ..
            } => {
                *frequency =
                    (*frequency + self.get_mod_source_offset(uuid, "frequency")).max(0.001);
                *phase = (*phase + self.get_mod_source_offset(uuid, "phase")).clamp(0.0, 1.0);
                *amplitude =
                    (*amplitude + self.get_mod_source_offset(uuid, "amplitude")).clamp(0.0, 1.0);
            }
            ModulationSource::AudioBand {
                gain, smoothing, ..
            } => {
                *gain = (*gain + self.get_mod_source_offset(uuid, "gain")).max(0.0);
                *smoothing =
                    (*smoothing + self.get_mod_source_offset(uuid, "smoothing")).clamp(0.0, 0.99);
            }
            ModulationSource::ADSR {
                attack,
                decay,
                sustain,
                release,
                ..
            } => {
                *attack = (*attack + self.get_mod_source_offset(uuid, "attack")).max(0.001);
                *decay = (*decay + self.get_mod_source_offset(uuid, "decay")).max(0.001);
                *sustain = (*sustain + self.get_mod_source_offset(uuid, "sustain")).clamp(0.0, 1.0);
                *release = (*release + self.get_mod_source_offset(uuid, "release")).max(0.001);
            }
            ModulationSource::StepSequencer { rate, .. } => {
                *rate = (*rate + self.get_mod_source_offset(uuid, "rate")).max(0.01);
            }
            ModulationSource::Analyzer { smoothing, .. } => {
                *smoothing =
                    (*smoothing + self.get_mod_source_offset(uuid, "smoothing")).clamp(0.0, 0.99);
            }
        }
        modified
    }

    /// Recompute the cached evaluation order and per-source mod-on-mod flags.
    fn recompute_order(&mut self) {
        const MAX_MOD_DEPTH: usize = 4;
        let n = self.sources.len();

        // Rebuild has_mod_on_mod flags
        self.has_mod_on_mod.clear();
        self.has_mod_on_mod.resize(n, false);

        self.cached_order.clear();
        if n == 0 {
            self.order_dirty = false;
            return;
        }

        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (key, mods) in &self.assignments {
            if let Some(target_uuid) = Self::parse_mod_target(key) {
                if let Some(&target_idx) = self.uuid_to_idx.get(target_uuid) {
                    if target_idx < n {
                        self.has_mod_on_mod[target_idx] = true;
                    }
                    for m in mods {
                        if let Some(&src_idx) = self.uuid_to_idx.get(&m.source_id) {
                            if src_idx != target_idx {
                                deps[target_idx].push(src_idx);
                            }
                        }
                    }
                }
            }
        }

        self.cached_order.reserve(n);
        let mut evaluated = vec![false; n];
        for _pass in 0..MAX_MOD_DEPTH {
            let mut progress = false;
            for i in 0..n {
                if evaluated[i] {
                    continue;
                }
                if deps[i].iter().all(|&d| evaluated[d]) {
                    self.cached_order.push(i);
                    evaluated[i] = true;
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        for (i, done) in evaluated.iter().enumerate().take(n) {
            if !done {
                self.cached_order.push(i);
            }
        }
        self.order_dirty = false;
    }

    /// Get the evaluation order, recomputing if stale. Used by tests.
    #[cfg(test)]
    pub(crate) fn evaluation_order(&mut self) -> Vec<usize> {
        if self.order_dirty {
            self.recompute_order();
        }
        self.cached_order.clone()
    }

    /// Parse mod-on-mod key: "mod:{uuid}:{param}" → Some(uuid)
    pub(crate) fn parse_mod_target(key: &str) -> Option<&str> {
        // Avoid allocating a Vec for splitn — just find the delimiters.
        let key = key.as_bytes();
        if key.len() < 5 || &key[..4] != b"mod:" {
            return None;
        }
        let rest = &key[4..];
        // Find the next ':' separating uuid from param_name
        rest.iter()
            .position(|&b| b == b':')
            .map(|pos| std::str::from_utf8(&rest[..pos]).unwrap_or(""))
    }

    /// Update all source values for the current frame
    pub fn update(&mut self, time: f32, audio: &AudioValues, analyzers: &AnalyzerValues) {
        self.ensure_index();
        let dt = self.prev_time.map_or(0.016, |prev| time - prev);
        self.prev_time = Some(time);

        while self.prev_values.len() < self.sources.len() {
            self.prev_values.push(0.0);
        }
        while self.current_values.len() < self.sources.len() {
            self.current_values.push(0.0);
        }

        if self.order_dirty {
            self.recompute_order();
        }

        // Iterate over cached order (clone the slice to avoid borrow conflict)
        let order_len = self.cached_order.len();
        for oi in 0..order_len {
            let i = self.cached_order[oi];

            // Only clone + apply mod-on-mod if this source actually has mod-on-mod assignments
            let value = if i < self.has_mod_on_mod.len() && self.has_mod_on_mod[i] {
                let mut effective = self.apply_mod_on_mod(i, &self.sources[i].source);
                let v = effective.calculate(time, dt, audio, analyzers, self.prev_values[i]);

                // Copy back mutable state changes (ADSR stage progression)
                if let (
                    ModulationSource::ADSR {
                        stage,
                        stage_time,
                        current_level,
                        ..
                    },
                    ModulationSource::ADSR {
                        stage: eff_stage,
                        stage_time: eff_st,
                        current_level: eff_cl,
                        ..
                    },
                ) = (&mut self.sources[i].source, &effective)
                {
                    *stage = *eff_stage;
                    *stage_time = *eff_st;
                    *current_level = *eff_cl;
                }
                v
            } else {
                // No mod-on-mod: calculate directly on the source (no clone)
                self.sources[i]
                    .source
                    .calculate(time, dt, audio, analyzers, self.prev_values[i])
            };

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
            if m.component == component {
                let idx = if let Some(&i) = self.uuid_to_idx.get(&m.source_id) {
                    i
                } else {
                    // Fallback: linear scan (handles deserialized state before ensure_index)
                    match self.sources.iter().position(|e| e.uuid == m.source_id) {
                        Some(i) => i,
                        None => continue,
                    }
                };
                if idx < self.current_values.len() {
                    total += self.current_values[idx] * m.amount;
                }
            }
        }
        total
    }

    /// Check if a parameter has any modulations assigned
    pub fn has_modulation(&self, param_name: &str) -> bool {
        self.assignments
            .get(param_name)
            .is_some_and(|v| !v.is_empty())
    }

    /// Get number of sources
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Get current computed values for all sources (for UI visualization)
    pub fn current_values(&self) -> &[f32] {
        &self.current_values
    }

    /// Get current value for a source by UUID
    pub fn current_value_for(&self, uuid: &str) -> f32 {
        self.sources
            .iter()
            .position(|e| e.uuid == uuid)
            .and_then(|idx| self.current_values.get(idx).copied())
            .unwrap_or(0.0)
    }

    /// Find an existing source by UUID
    pub fn find_source_by_uuid(&self, uuid: &str) -> Option<&ModulationSourceEntry> {
        self.sources.iter().find(|e| e.uuid == uuid)
    }

    /// Iterate over all assignments (key → modulations).
    pub fn assignments_iter(
        &self,
    ) -> impl Iterator<Item = (&String, &Vec<super::ParamModulation>)> {
        self.assignments.iter()
    }
}
