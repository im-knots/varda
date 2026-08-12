//! `ModulationEngine` — manages sources, assignments, and per-frame evaluation.

use super::{
    AnalyzerValues, AssignmentMode, AudioValues, ModulationSource, ModulationSourceEntry,
    ParamModulation,
};

/// The two ways a parameter's assignments contribute, resolved together.
/// See /spec/automation.md § Absolute vs Additive.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolvedModulation {
    /// Summed additive contributions, already range-scaled.
    pub additive: f32,
    /// Normalized replacement for the base value, if an absolute source is
    /// assigned and actually has something to say.
    pub absolute: Option<f32>,
}
use crate::timebase::{Timebase, TimebaseSet};
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
    /// Whether `cached_order` needs recomputation.
    #[serde(skip)]
    order_dirty: bool,
    /// Per-source flag: does this source have any mod-on-mod assignments targeting it?
    #[serde(skip)]
    has_mod_on_mod: Vec<bool>,
    /// Parameters a performer has taken back from the arrangement.
    ///
    /// Session state, never persisted: a saved override is an invisible trap
    /// that would silently break the show the next time the file is opened.
    #[serde(skip)]
    overrides: HashMap<String, ParamOverride>,
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

    /// Ensure `uuid_to_idx` is populated (needed after deserialization)
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
            let mod_prefix = format!("mod:{uuid}:");
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

    pub fn assign(
        &mut self,
        param_name: &str,
        source_id: &str,
        amount: f32,
        component: Option<usize>,
    ) {
        self.assign_with_mode(
            param_name,
            source_id,
            amount,
            component,
            AssignmentMode::default(),
        );
    }

    /// Assign with an explicit mode. Envelopes want `Absolute`, so that a curve
    /// drawn to a value produces that value rather than depending on wherever
    /// the fader happened to be saved.
    pub fn assign_with_mode(
        &mut self,
        param_name: &str,
        source_id: &str,
        amount: f32,
        component: Option<usize>,
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
            component,
            mode,
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
        let key = format!("mod:{target_uuid}:{param_name}");
        self.assign(&key, modulator_uuid, amount, None);
        // assign() already calls invalidate_order()
    }

    pub fn clear_mod_on_mod(&mut self, target_uuid: &str, param_name: &str) {
        let key = format!("mod:{target_uuid}:{param_name}");
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

    /// Replace an envelope's breakpoints, restoring the sorted-by-position
    /// invariant so callers do not have to. Returns false if the UUID is
    /// unknown or does not name an envelope.
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

    /// How many sources actually read a given timebase.
    ///
    /// Signal-driven sources (audio bands, analyzers) carry a timebase field but
    /// ignore it, so they are excluded: the count answers "would anything stop
    /// moving if this clock went away", which is what the readouts report.
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
                    let Some(&idx) = self.uuid_to_idx.get(&m.source_id) else {
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
            // Envelopes take no part in the mod-on-mod graph: an arrangement may
            // hold hundreds of them, and keeping them out of the dependency scan
            // is one of the three properties that make that affordable.
            // See /spec/automation.md § Performance.
            ModulationSource::Envelope { .. } => {}
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
    /// Each source reads whichever timebase it is assigned, so one LFO can ride
    /// the beat while another free-runs. See /spec/timebase.md.
    pub fn update(
        &mut self,
        timebases: &TimebaseSet,
        audio: &AudioValues,
        analyzers: &AnalyzerValues,
    ) {
        self.ensure_index();
        self.prev_time = Some(timebases.free_run().time);
        // Wall-clock, not the envelope's timebase: this is a smoothing ramp on
        // the way a handover looks, not a musical duration.
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

        // Iterate over cached order (clone the slice to avoid borrow conflict)
        let order_len = self.cached_order.len();
        for oi in 0..order_len {
            let i = self.cached_order[oi];

            // Free-run resolves to the same context either way, so testing the
            // timebase first skips the variant match for every source that has
            // not opted in — which, since `FreeRun` is the default, is nearly
            // all of them. Sources that integrate or follow a signal read
            // free-run time whatever they are set to.
            let tb = self.sources[i].timebase;
            let tc = if tb == Timebase::FreeRun || !self.sources[i].source.follows_timebase() {
                *timebases.free_run()
            } else {
                *timebases.get(tb)
            };
            let (time, dt) = (tc.time, tc.dt);

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
        self.resolve(param_name, component).additive
    }

    /// Resolve every assignment on a parameter in one pass.
    ///
    /// Returns both halves together because callers need both and a parameter's
    /// assignment list is looked up by string key: splitting this into two
    /// entry points would double the hash lookups on a per-parameter,
    /// per-frame path. See /spec/automation.md § Absolute vs Additive.
    pub fn resolve(&self, param_name: &str, component: Option<usize>) -> ResolvedModulation {
        let mut out = ResolvedModulation::default();
        let Some(mods) = self.assignments.get(param_name) else {
            return out;
        };
        // Only envelopes answer to an override: an LFO the performer never
        // took is still theirs to run, and suspending the whole parameter
        // would stop it too.
        let override_record = self.overrides.get(param_name).copied();
        for m in mods {
            if m.component != component {
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
                // An envelope with no breakpoints is inert: a lane exists before
                // any point is drawn on it, and overriding the base with zero
                // would black the parameter out in the meantime.
                if source.provides_absolute_value() {
                    // Last assignment wins. Stacking absolute sources is
                    // meaningless rather than dangerous, so the engine takes one
                    // rather than rejecting the configuration.
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
    //
    // See /spec/arrangement.md § Live override. The performer's hand wins,
    // always, and only over the parameter they actually touched.

    /// Suspend arrangement control of one parameter, holding the value the
    /// performer left it at.
    ///
    /// Re-taking a parameter mid-ramp restarts from the new value rather than
    /// continuing the ramp, since the performer has spoken more recently than
    /// the re-arm did.
    pub fn override_param(&mut self, param_key: &str, held: f32) {
        if let Some(existing) = self.overrides.get_mut(param_key) {
            existing.held = held;
            existing.rearm = None;
            return;
        }
        self.overrides
            .insert(param_key.to_string(), ParamOverride { held, rearm: None });
    }

    /// Hand one parameter back to the arrangement, ramping over `duration`
    /// seconds rather than snapping.
    ///
    /// A snap is the correct state and the wrong look; this runs in front of an
    /// audience. A zero or negative duration is honoured as an immediate
    /// handover, which is what a test or an API caller asking for one means.
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

    /// Whether anything at all is assigned.
    ///
    /// Lets a per-frame caller skip building a parameter key on a scene that has
    /// no modulation in it, which is most of them.
    pub fn has_modulation_for_any(&self) -> bool {
        !self.assignments.is_empty()
    }

    /// Get number of sources
    pub fn source_count(&self) -> usize {
        self.sources.len()
    }

    /// Device selection of every `AudioBand` modulator (`None` = default input).
    ///
    /// Drives the per-frame audio-capture reconcile so a device is captured only
    /// while at least one modulator references it. See
    /// [/spec/audio-capture-lifecycle.md](/spec/audio-capture-lifecycle.md).
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
