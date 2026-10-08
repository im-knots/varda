//! `ModulationEngine`: sources, assignments, and per-frame evaluation.

use super::{
    AnalyzerValues, AssignmentMode, AudioValues, ModulationSource, ModulationSourceEntry,
    ParamModulation,
};

/// The two ways a parameter's assignments contribute, resolved together.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolvedModulation {
    /// Summed additive contributions, already range-scaled.
    pub additive: f32,
    /// Normalized replacement for the base value, when an absolute source with content is
    /// assigned.
    pub absolute: Option<f32>,
}
use crate::timebase::{Timebase, TimebaseSet};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Modulation sources and assignments for a deck.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModulationEngine {
    /// Available modulation sources (with stable UUIDs)
    pub sources: Vec<ModulationSourceEntry>,
    /// Map from parameter name to list of modulations. Private so every change
    /// invalidates the evaluation order and the mod-on-mod index.
    assignments: HashMap<String, Vec<ParamModulation>>,
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
    /// Whether `cached_order` needs recomputation.
    #[serde(skip)]
    order_dirty: bool,
    /// Per source, the mod-on-mod inputs on each of its parameters. Rebuilt
    /// with `cached_order`; empty for a source nothing modulates.
    #[serde(skip)]
    mod_inputs: Vec<Vec<ModInput>>,
    /// Whether any source has mod-on-mod inputs, so a rig without any skips
    /// the per-source check.
    #[serde(skip)]
    any_mod_inputs: bool,
    /// Parameters a performer has taken back from the arrangement.
    ///
    /// Session state, never persisted, so a reopened file always plays the arrangement.
    #[serde(skip)]
    overrides: HashMap<String, ParamOverride>,
}

/// The modulators on one parameter of a source: `(source index, amount)` pairs.
#[derive(Debug, Clone)]
struct ModInput {
    param: String,
    from: Vec<(usize, f32)>,
}

/// One parameter's suspension of arrangement control.
#[derive(Debug, Clone, Copy)]
struct ParamOverride {
    /// Normalized value the performer left the parameter at, which the re-arm
    /// ramp starts from.
    held: f32,
    /// Progress back to the automated value. `None` while the performer still
    /// holds the parameter.
    rearm: Option<Rearm>,
}

#[derive(Debug, Clone, Copy)]
struct Rearm {
    elapsed: f64,
    duration: f64,
}

impl ParamOverride {
    /// How much of the envelope's output applies right now, from 0.0 (the
    /// performer owns it) to 1.0 (the arrangement has it back).
    fn envelope_weight(self) -> f32 {
        match self.rearm {
            None => 0.0,
            Some(r) if r.duration <= 0.0 => 1.0,
            Some(r) => (r.elapsed / r.duration).clamp(0.0, 1.0) as f32,
        }
    }
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

    /// Populate `uuid_to_idx`, needed after deserialization.
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
        self.uuid_to_idx
            .insert(uuid.clone(), self.sources.len() - 1);
        self.invalidate_order();
        uuid
    }
    /// Add an automation lane: an empty envelope on `timebase`, assigned absolutely to `target` so
    /// a curve's value does not depend on the saved fader position. Returns its UUID.
    pub fn add_automation_lane(&mut self, target: &str, timebase: Timebase) -> String {
        let uuid = self.add_source(ModulationSource::envelope(Vec::new()));
        self.set_timebase(&uuid, timebase);
        self.assign_with_mode(target, &uuid, 1.0, AssignmentMode::Absolute);
        uuid
    }

    /// Add a source with a specific UUID (for preset loading)
    pub fn add_source_with_uuid(&mut self, uuid: String, source: ModulationSource) -> String {
        let entry = ModulationSourceEntry::with_uuid(uuid.clone(), source);
        self.sources.push(entry);
        self.prev_values.push(0.0);
        self.current_values.push(0.0);
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
            // Remove assignments referencing this source (no reindexing needed)
            for mods in self.assignments.values_mut() {
                mods.retain(|m| m.source_id != uuid);
            }
            // Remove mod-on-mod assignments targeting this source
            let mod_prefix = crate::engine::value::param::modulator_prefix(uuid);
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
            log::info!("Removed {removed} orphaned modulation assignments with prefix '{prefix}'");
        }
    }

    /// Re-apply an assignment from a preset or clipboard. A component index saved before scene
    /// version 9 is kept for [`Self::rekey_legacy_components`].
    pub fn assign_saved(
        &mut self,
        param_name: &str,
        source_id: &str,
        amount: f32,
        legacy_component: Option<usize>,
    ) {
        self.assign(param_name, source_id, amount);
        if legacy_component.is_some()
            && let Some(last) = self
                .assignments
                .get_mut(param_name)
                .and_then(|mods| mods.last_mut())
                .filter(|m| m.source_id == source_id)
        {
            last.legacy_component = legacy_component;
        }
    }

    pub fn assign(&mut self, param_name: &str, source_id: &str, amount: f32) {
        self.assign_with_mode(param_name, source_id, amount, AssignmentMode::default());
    }

    /// Assign with an explicit mode. Envelopes use `Absolute`, so a curve's value does not depend
    /// on the saved fader position.
    pub fn assign_with_mode(
        &mut self,
        param_name: &str,
        source_id: &str,
        amount: f32,
        mode: AssignmentMode,
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
            mode,
            legacy_component: None,
        };
        self.assignments
            .entry(param_name.to_string())
            .or_default()
            .push(modulation);
        self.invalidate_order();
    }

    /// Rewrite assignments saved with a component index (before scene
    /// version 9) to their component path, `<key>/r` or `<key>/x`, using
    /// `kind_of` to tell a color from a point. Assignments whose target is
    /// neither, or whose index is out of range, are dropped with a warning.
    /// Returns how many were rewritten.
    pub fn rekey_legacy_components(
        &mut self,
        kind_of: impl Fn(&str) -> Option<crate::engine::value::param::ComponentKind>,
    ) -> usize {
        let keys: Vec<String> = self
            .assignments
            .iter()
            .filter(|(_, mods)| mods.iter().any(|m| m.legacy_component.is_some()))
            .map(|(key, _)| key.clone())
            .collect();
        let mut rekeyed = 0;
        for key in keys {
            let Some(mods) = self.assignments.get_mut(&key) else {
                continue;
            };
            let (legacy, kept): (Vec<_>, Vec<_>) = std::mem::take(mods)
                .into_iter()
                .partition(|m| m.legacy_component.is_some());
            if kept.is_empty() {
                self.assignments.remove(&key);
            } else {
                *mods = kept;
            }
            let kind = kind_of(&key);
            for mut m in legacy {
                let index = m.legacy_component.take().unwrap_or_default();
                let Some(component) = kind.and_then(|k| k.component(index)) else {
                    log::warn!(
                        "Dropped modulation on '{key}' component {index}: the target is not a color or point"
                    );
                    continue;
                };
                self.assignments
                    .entry(component.path(&key))
                    .or_default()
                    .push(m);
                rekeyed += 1;
            }
        }
        if rekeyed > 0 {
            log::info!("Rewrote {rekeyed} component modulation assignment(s) to component paths");
            self.invalidate_order();
        }
        rekeyed
    }

    /// Move every assignment to the key `rekey` gives its current one,
    /// merging assignments that land on the same key. Returns how many keys
    /// changed.
    pub fn rekey_assignments(&mut self, mut rekey: impl FnMut(&str) -> String) -> usize {
        let mut changed = 0;
        for (key, mods) in std::mem::take(&mut self.assignments) {
            let new_key = rekey(&key);
            if new_key != key {
                changed += 1;
            }
            self.assignments.entry(new_key).or_default().extend(mods);
        }
        self.invalidate_order();
        changed
    }

    pub fn assign_mod_on_mod(
        &mut self,
        target_uuid: &str,
        param_name: &str,
        modulator_uuid: &str,
        amount: f32,
    ) {
        let key =
            crate::engine::value::param::ParamAddress::modulator_param(target_uuid, param_name)
                .to_string();
        self.assign(&key, modulator_uuid, amount);
        // assign() already calls invalidate_order()
    }

    pub fn clear_mod_on_mod(&mut self, target_uuid: &str, param_name: &str) {
        let key =
            crate::engine::value::param::ParamAddress::modulator_param(target_uuid, param_name)
                .to_string();
        self.assignments.remove(&key);
        self.invalidate_order();
    }

    pub fn clear_assignments(&mut self, param_name: &str) {
        self.assignments.remove(param_name);
        self.invalidate_order();
    }

    /// Remove only the assignment(s) from a specific source on a target, leaving
    /// any other sources on that target intact. Drops the target entry entirely
    /// once its last source is removed.
    pub fn clear_assignment_source(&mut self, param_name: &str, source_id: &str) {
        if let Some(list) = self.assignments.get_mut(param_name) {
            list.retain(|a| a.source_id != source_id);
            if list.is_empty() {
                self.assignments.remove(param_name);
            }
            self.invalidate_order();
        }
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

    /// Set which timebase a source follows. Returns false if the UUID is unknown.
    pub fn set_timebase(&mut self, uuid: &str, timebase: Timebase) -> bool {
        self.ensure_index();
        match self.uuid_to_idx.get(uuid).copied() {
            Some(idx) => {
                self.sources[idx].timebase = timebase;
                true
            }
            None => false,
        }
    }

    /// Which timebase a source follows, or `None` if the UUID is unknown.
    pub fn timebase(&self, uuid: &str) -> Option<Timebase> {
        self.sources
            .iter()
            .find(|e| e.uuid == uuid)
            .map(|e| e.timebase)
    }

    /// Replace an envelope's breakpoints and sort them by position. Returns false if the UUID is
    /// unknown or not an envelope.
    pub fn set_envelope_breakpoints(
        &mut self,
        uuid: &str,
        mut breakpoints: Vec<super::Breakpoint>,
    ) -> bool {
        self.ensure_index();
        let Some(&idx) = self.uuid_to_idx.get(uuid) else {
            return false;
        };
        let ModulationSource::Envelope {
            breakpoints: target,
            cursor,
        } = &mut self.sources[idx].source
        else {
            return false;
        };
        breakpoints.sort_by(|a, b| a.position.total_cmp(&b.position));
        *target = breakpoints;
        *cursor = 0;
        true
    }

    /// How many sources read a given timebase.
    ///
    /// Audio bands and analyzers carry a timebase field but ignore it, so they are not counted.
    pub fn followers_of(&self, timebase: Timebase) -> usize {
        self.sources
            .iter()
            .filter(|e| e.timebase == timebase && e.source.follows_timebase())
            .count()
    }

    /// Get a mutable reference to a source by UUID
    pub fn source_mut(&mut self, uuid: &str) -> Option<&mut ModulationSource> {
        self.ensure_index();
        self.uuid_to_idx
            .get(uuid)
            .copied()
            .map(|idx| &mut self.sources[idx].source)
    }

    /// Whether a source with this UUID exists.
    pub fn has_source(&self, uuid: &str) -> bool {
        self.sources.iter().any(|e| e.uuid == uuid)
    }

    /// The mod-on-mod offset on `param_name` of the source at `idx`: each
    /// modulator's current value times its amount.
    fn get_mod_source_offset(&self, idx: usize, param_name: &str) -> f32 {
        self.mod_inputs[idx]
            .iter()
            .find(|input| input.param == param_name)
            .map_or(0.0, |input| {
                input
                    .from
                    .iter()
                    .map(|&(source, amount)| self.current_values[source] * amount)
                    .sum()
            })
    }

    fn apply_mod_on_mod(&self, idx: usize, source: &ModulationSource) -> ModulationSource {
        let mut modified = source.clone();
        match &mut modified {
            ModulationSource::LFO {
                frequency,
                phase,
                amplitude,
                ..
            } => {
                *frequency = (*frequency + self.get_mod_source_offset(idx, "frequency")).max(0.001);
                *phase = (*phase + self.get_mod_source_offset(idx, "phase")).clamp(0.0, 1.0);
                *amplitude =
                    (*amplitude + self.get_mod_source_offset(idx, "amplitude")).clamp(0.0, 1.0);
            }
            ModulationSource::AudioBand {
                gain, smoothing, ..
            } => {
                *gain = (*gain + self.get_mod_source_offset(idx, "gain")).max(0.0);
                *smoothing =
                    (*smoothing + self.get_mod_source_offset(idx, "smoothing")).clamp(0.0, 0.99);
            }
            ModulationSource::ADSR {
                attack,
                decay,
                sustain,
                release,
                ..
            } => {
                *attack = (*attack + self.get_mod_source_offset(idx, "attack")).max(0.001);
                *decay = (*decay + self.get_mod_source_offset(idx, "decay")).max(0.001);
                *sustain = (*sustain + self.get_mod_source_offset(idx, "sustain")).clamp(0.0, 1.0);
                *release = (*release + self.get_mod_source_offset(idx, "release")).max(0.001);
            }
            ModulationSource::StepSequencer { rate, .. } => {
                *rate = (*rate + self.get_mod_source_offset(idx, "rate")).max(0.01);
            }
            ModulationSource::Analyzer { smoothing, .. } => {
                *smoothing =
                    (*smoothing + self.get_mod_source_offset(idx, "smoothing")).clamp(0.0, 0.99);
            }
            // Envelopes stay out of the mod-on-mod dependency scan, since an arrangement can hold
            // hundreds of them.
            ModulationSource::Envelope { .. } => {}
        }
        modified
    }

    /// Recompute the cached evaluation order and per-source mod-on-mod flags.
    fn recompute_order(&mut self) {
        const MAX_MOD_DEPTH: usize = 4;
        let n = self.sources.len();

        self.mod_inputs.clear();
        self.mod_inputs.resize(n, Vec::new());
        self.any_mod_inputs = false;

        self.cached_order.clear();
        if n == 0 {
            self.order_dirty = false;
            return;
        }

        let mut deps: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (key, mods) in &self.assignments {
            if let Some(target_uuid) = Self::parse_mod_target(key)
                && let Some(&target_idx) = self.uuid_to_idx.get(target_uuid)
                && target_idx < n
            {
                let mut input = ModInput {
                    param: key["mod/".len() + target_uuid.len() + 1..].to_string(),
                    from: Vec::with_capacity(mods.len()),
                };
                for m in mods {
                    let Some(&src_idx) = self.uuid_to_idx.get(&m.source_id) else {
                        continue;
                    };
                    input.from.push((src_idx, m.amount));
                    if src_idx != target_idx {
                        deps[target_idx].push(src_idx);
                    }
                }
                self.mod_inputs[target_idx].push(input);
            }
        }

        self.any_mod_inputs = self.mod_inputs.iter().any(|inputs| !inputs.is_empty());
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

    /// Parse mod-on-mod key: "mod/{uuid}/{param}" → Some(uuid)
    pub(crate) fn parse_mod_target(key: &str) -> Option<&str> {
        // Find the delimiters instead of allocating a Vec for splitn.
        let key = key.as_bytes();
        if key.len() < 5 || &key[..4] != b"mod/" {
            return None;
        }
        let rest = &key[4..];
        // Find the next '/' separating uuid from param_name
        rest.iter()
            .position(|&b| b == b'/')
            .map(|pos| std::str::from_utf8(&rest[..pos]).unwrap_or(""))
    }

    /// Update with every timebase free-running at `time`, deriving `dt` from
    /// the previous call.
    ///
    /// For callers that have no clock to resolve: headless tests, benchmarks,
    /// and the offline parameter-preview path.
    pub fn update_free_running(
        &mut self,
        time: f32,
        audio: &AudioValues,
        analyzers: &AnalyzerValues,
    ) {
        let dt = self.prev_time.map_or(0.016, |prev| time - prev);
        self.update(&TimebaseSet::free_running(time, dt), audio, analyzers);
    }

    /// Update all source values for the current frame.
    ///
    /// Each source reads its own timebase, so one LFO can follow the beat while another
    /// free-runs.
    pub fn update(
        &mut self,
        timebases: &TimebaseSet,
        audio: &AudioValues,
        analyzers: &AnalyzerValues,
    ) {
        self.ensure_index();
        self.prev_time = Some(timebases.free_run().time);
        // Wall-clock time, not the envelope's timebase: the re-arm ramp is visual smoothing, not a
        // musical duration.
        self.advance_rearms(f64::from(timebases.free_run().dt));

        while self.prev_values.len() < self.sources.len() {
            self.prev_values.push(0.0);
        }
        while self.current_values.len() < self.sources.len() {
            self.current_values.push(0.0);
        }

        if self.order_dirty {
            self.recompute_order();
        }

        // Iterate the cached order by index to avoid a borrow conflict.
        let order_len = self.cached_order.len();
        for oi in 0..order_len {
            let i = self.cached_order[oi];

            // Checking the timebase first skips the variant match for `FreeRun` sources, which is
            // nearly all of them. Sources that integrate or follow a signal always read free-run
            // time.
            let tb = self.sources[i].timebase;
            let tc = if tb == Timebase::FreeRun || !self.sources[i].source.follows_timebase() {
                *timebases.free_run()
            } else {
                *timebases.get(tb)
            };
            let (time, dt) = (tc.time, tc.dt);

            // Only clone + apply mod-on-mod if this source actually has mod-on-mod assignments
            let value = if self.any_mod_inputs && !self.mod_inputs[i].is_empty() {
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
        self.resolve(param_name).additive
    }

    /// Resolve every assignment on a parameter in one pass.
    ///
    /// Returns both halves together so the per-frame path does one hash lookup per parameter.
    pub fn resolve(&self, param_name: &str) -> ResolvedModulation {
        let mut out = ResolvedModulation::default();
        let Some(mods) = self.assignments.get(param_name) else {
            return out;
        };
        // Overrides suspend only envelopes; other modulators on the parameter keep running.
        let override_record = self.overrides.get(param_name).copied();
        for m in mods {
            // Waiting to be rewritten to its component path; see
            // `rekey_legacy_components`.
            if m.legacy_component.is_some() {
                continue;
            }
            let idx = if let Some(&i) = self.uuid_to_idx.get(&m.source_id) {
                i
            } else {
                // Fallback: linear scan (handles deserialized state before ensure_index)
                match self.sources.iter().position(|e| e.uuid == m.source_id) {
                    Some(i) => i,
                    None => continue,
                }
            };
            if idx >= self.current_values.len() {
                continue;
            }
            let source = &self.sources[idx].source;
            let is_envelope = matches!(source, ModulationSource::Envelope { .. });
            let weight = match override_record {
                Some(record) if is_envelope => record.envelope_weight(),
                _ => 1.0,
            };
            if weight <= 0.0 {
                continue;
            }

            if m.mode == AssignmentMode::Absolute {
                // An envelope with no breakpoints is inert; overriding the base with zero would
                // black out the parameter before any point is drawn.
                if source.provides_absolute_value() {
                    // Last assignment wins. Stacking absolute sources has no meaning, so it is
                    // allowed rather than rejected.
                    let automated = self.current_values[idx] * m.amount;
                    out.absolute = Some(match override_record {
                        // Ramp out of the value the performer left rather than
                        // snapping to the envelope.
                        Some(record) if is_envelope && weight < 1.0 => {
                            record.held + (automated - record.held) * weight
                        }
                        _ => automated,
                    });
                }
            } else {
                out.additive += self.current_values[idx] * m.amount * source.range_scale() * weight;
            }
        }
        out
    }

    // ── Live override ───────────────────────────────────────────
    // The performer's change wins, only on the parameter they touched.

    /// Suspend arrangement control of one parameter, holding the value the performer left it at.
    ///
    /// Overriding mid-ramp restarts from the new value.
    pub fn override_param(&mut self, param_key: &str, held: f32) {
        if let Some(existing) = self.overrides.get_mut(param_key) {
            existing.held = held;
            existing.rearm = None;
            return;
        }
        self.overrides
            .insert(param_key.to_string(), ParamOverride { held, rearm: None });
    }

    /// Hand one parameter back to the arrangement, ramping over `duration` seconds. A zero or
    /// negative duration hands over immediately.
    pub fn rearm_param(&mut self, param_key: &str, duration: f64) {
        let Some(record) = self.overrides.get_mut(param_key) else {
            return;
        };
        if duration <= 0.0 {
            self.overrides.remove(param_key);
            return;
        }
        record.rearm = Some(Rearm {
            elapsed: 0.0,
            duration,
        });
    }

    /// Hand every overridden parameter back at once.
    pub fn rearm_all(&mut self, duration: f64) {
        if duration <= 0.0 {
            self.overrides.clear();
            return;
        }
        for record in self.overrides.values_mut() {
            record.rearm = Some(Rearm {
                elapsed: 0.0,
                duration,
            });
        }
    }

    pub fn is_overridden(&self, param_key: &str) -> bool {
        self.overrides
            .get(param_key)
            .is_some_and(|o| o.rearm.is_none())
    }

    /// Parameters currently held by a performer, for the lane header and the
    /// deck thumbnail to render.
    pub fn overridden_params(&self) -> impl Iterator<Item = &str> {
        self.overrides
            .iter()
            .filter(|(_, o)| o.rearm.is_none())
            .map(|(k, _)| k.as_str())
    }

    pub fn override_count(&self) -> usize {
        self.overrides
            .values()
            .filter(|o| o.rearm.is_none())
            .count()
    }

    /// Drop every override. Called on scene load, since overrides are session
    /// state and a reload restores full arrangement authority.
    pub fn clear_overrides(&mut self) {
        self.overrides.clear();
    }

    /// Advance re-arm ramps and retire the ones that have completed.
    fn advance_rearms(&mut self, dt: f64) {
        if self.overrides.is_empty() {
            return;
        }
        self.overrides.retain(|_, record| {
            let Some(rearm) = record.rearm.as_mut() else {
                return true;
            };
            rearm.elapsed += dt.max(0.0);
            rearm.elapsed < rearm.duration
        });
    }

    /// Check if a parameter has any modulations assigned
    pub fn has_modulation(&self, param_name: &str) -> bool {
        self.assignments
            .get(param_name)
            .is_some_and(|v| !v.is_empty())
    }

    /// Whether anything is assigned. Lets per-frame callers skip building parameter keys on scenes
    /// with no modulation.
    pub fn has_modulation_for_any(&self) -> bool {
        !self.assignments.is_empty()
    }

    /// Get number of sources
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Device selection of every `AudioBand` modulator (`None` = default input).
    ///
    /// Drives the per-frame capture reconcile, so a device is captured only while a modulator
    /// references it.
    pub fn audio_band_source_ids(&self) -> Vec<Option<crate::audio::AudioSourceId>> {
        self.sources
            .iter()
            .filter_map(|e| match &e.source {
                ModulationSource::AudioBand { source_id, .. } => Some(*source_id),
                _ => None,
            })
            .collect()
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

    /// Find an existing source by UUID (mutable). Used by the parameter router
    /// to address modulators by stable identity rather than positional index.
    pub fn find_source_by_uuid_mut(&mut self, uuid: &str) -> Option<&mut ModulationSourceEntry> {
        self.sources.iter_mut().find(|e| e.uuid == uuid)
    }

    /// Every modulation assigned to one parameter, empty when it has none.
    pub fn assignments_for(&self, param_name: &str) -> &[super::ParamModulation] {
        self.assignments.get(param_name).map_or(&[], Vec::as_slice)
    }

    /// Iterate over all assignments (key → modulations).
    pub fn assignments_iter(
        &self,
    ) -> impl Iterator<Item = (&String, &Vec<super::ParamModulation>)> {
        self.assignments.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modulation::{AnalyzerValues, AudioValues};

    fn update(engine: &mut ModulationEngine, time: f32) {
        engine.update_free_running(time, &AudioValues::default(), &AnalyzerValues::default());
    }

    /// The offset mod-on-mod applies to `param` of `target` this frame.
    fn offset(engine: &mut ModulationEngine, target: &str, param: &str) -> f32 {
        update(engine, 0.37);
        let idx = engine.uuid_to_idx[target];
        engine.get_mod_source_offset(idx, param)
    }

    fn value(engine: &ModulationEngine, uuid: &str) -> f32 {
        engine.current_value_for(uuid)
    }

    fn key(target: &str, param: &str) -> String {
        crate::engine::value::param::ParamAddress::modulator_param(target, param).to_string()
    }

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-6
    }

    /// Each parameter sums its own modulators, scaled by their amounts.
    #[test]
    fn offsets_sum_each_parameters_modulators() {
        let mut engine = ModulationEngine::new();
        let target = engine.add_source(ModulationSource::sine_lfo(1.0));
        let a = engine.add_source(ModulationSource::sine_lfo(0.3));
        let b = engine.add_source(ModulationSource::sine_lfo(0.7));
        let c = engine.add_source(ModulationSource::sine_lfo(1.3));
        engine.assign_mod_on_mod(&target, "frequency", &a, 0.5);
        engine.assign_mod_on_mod(&target, "frequency", &b, 0.25);
        engine.assign_mod_on_mod(&target, "amplitude", &c, 0.1);

        let frequency = offset(&mut engine, &target, "frequency");
        let expected = 0.5 * value(&engine, &a) + 0.25 * value(&engine, &b);
        assert!(close(frequency, expected), "{frequency} vs {expected}");
        let amplitude = offset(&mut engine, &target, "amplitude");
        assert!(close(amplitude, 0.1 * value(&engine, &c)));
        assert_eq!(offset(&mut engine, &target, "phase"), 0.0);
    }

    /// In a cycle every offset is still its modulators' values times amounts.
    #[test]
    fn a_cycle_still_sums_its_modulators() {
        let mut engine = ModulationEngine::new();
        let a = engine.add_source(ModulationSource::sine_lfo(0.4));
        let b = engine.add_source(ModulationSource::sine_lfo(0.9));
        engine.assign_mod_on_mod(&a, "frequency", &b, 0.5);
        engine.assign_mod_on_mod(&b, "frequency", &a, 0.5);
        let on_a = offset(&mut engine, &a, "frequency");
        assert!(close(on_a, 0.5 * value(&engine, &b)));
        let on_b = offset(&mut engine, &b, "frequency");
        assert!(close(on_b, 0.5 * value(&engine, &a)));
    }

    /// Every way the assignments change shows up in the next update.
    #[test]
    fn each_assignment_change_reaches_the_next_update() {
        let mut engine = ModulationEngine::new();
        let spare = engine.add_source(ModulationSource::sine_lfo(0.2));
        let target = engine.add_source(ModulationSource::sine_lfo(1.0));
        let a = engine.add_source(ModulationSource::sine_lfo(0.3));
        let b = engine.add_source(ModulationSource::sine_lfo(0.7));
        let frequency = key(&target, "frequency");

        engine.assign_mod_on_mod(&target, "frequency", &a, 0.5);
        let got = offset(&mut engine, &target, "frequency");
        assert!(close(got, 0.5 * value(&engine, &a)), "assign");

        engine.assign_mod_on_mod(&target, "frequency", &b, 0.25);
        let got = offset(&mut engine, &target, "frequency");
        let both = 0.5 * value(&engine, &a) + 0.25 * value(&engine, &b);
        assert!(close(got, both), "second assign");

        engine.clear_assignment_source(&frequency, &a);
        let got = offset(&mut engine, &target, "frequency");
        assert!(close(got, 0.25 * value(&engine, &b)), "clear one source");

        engine.clear_mod_on_mod(&target, "frequency");
        assert_eq!(offset(&mut engine, &target, "frequency"), 0.0, "clear");

        engine.assign_mod_on_mod(&target, "frequency", &a, 0.5);
        engine.clear_assignments(&frequency);
        assert_eq!(offset(&mut engine, &target, "frequency"), 0.0, "clear key");

        engine.assign_mod_on_mod(&target, "frequency", &a, 0.5);
        engine.remove_assignments_with_prefix(&format!("mod/{target}/"));
        assert_eq!(offset(&mut engine, &target, "frequency"), 0.0, "prefix");

        // Removing an earlier source shifts every index after it.
        engine.assign_mod_on_mod(&target, "frequency", &b, 0.25);
        engine.remove_source(&spare);
        let got = offset(&mut engine, &target, "frequency");
        assert!(close(got, 0.25 * value(&engine, &b)), "shifted indices");

        engine.remove_source(&b);
        assert_eq!(
            offset(&mut engine, &target, "frequency"),
            0.0,
            "modulator removed"
        );

        engine.assign_mod_on_mod(&a, "frequency", &target, 0.5);
        engine.remove_source(&target);
        assert_eq!(offset(&mut engine, &a, "frequency"), 0.0, "target removed");
    }

    /// A saved engine indexes its mod-on-mod on load.
    #[test]
    fn a_loaded_engine_indexes_its_mod_on_mod() {
        let mut engine = ModulationEngine::new();
        let target = engine.add_source(ModulationSource::sine_lfo(1.0));
        let a = engine.add_source(ModulationSource::sine_lfo(0.3));
        engine.assign_mod_on_mod(&target, "frequency", &a, 0.5);
        let json = serde_json::to_string(&engine).expect("serializes");
        let mut loaded: ModulationEngine = serde_json::from_str(&json).expect("loads");
        let got = offset(&mut loaded, &target, "frequency");
        assert!(close(got, 0.5 * value(&loaded, &a)));
    }

    /// Re-keying moves assignments onto mod-on-mod keys, and drops legacy ones
    /// whose target has no components.
    #[test]
    fn rekeying_reaches_the_next_update() {
        let mut engine = ModulationEngine::new();
        let target = engine.add_source(ModulationSource::sine_lfo(1.0));
        let a = engine.add_source(ModulationSource::sine_lfo(0.3));
        update(&mut engine, 0.1);
        engine.assign(&format!("old/{target}/frequency"), &a, 0.5);
        assert_eq!(offset(&mut engine, &target, "frequency"), 0.0);
        engine.rekey_assignments(|k| k.replacen("old/", "mod/", 1));
        let got = offset(&mut engine, &target, "frequency");
        assert!(close(got, 0.5 * value(&engine, &a)), "rekey");

        engine.clear_mod_on_mod(&target, "frequency");
        engine.assign_saved(&key(&target, "frequency"), &a, 0.5, Some(0));
        engine.rekey_legacy_components(|_| None);
        assert_eq!(
            offset(&mut engine, &target, "frequency"),
            0.0,
            "legacy dropped"
        );
    }
}
