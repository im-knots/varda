//! Host-inline preprocessors: one instance per deck and type, stepped on the
//! render thread before the deck's shader.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use super::AnalyzerRegistry;
use super::traits::{
    AnalyzerSnapshot, AnalyzerStateSnapshot, HostFrame, HostInlinePreprocessor,
};

/// How long one step may take before it is reported.
pub(crate) const STEP_BUDGET: Duration = Duration::from_millis(1);

struct Instance {
    preprocessor: Box<dyn HostInlinePreprocessor>,
    latest: AnalyzerSnapshot,
    /// The slowest step so far, for telemetry.
    slowest: Duration,
    over_budget_logged: bool,
}

/// A deck's host-inline preprocessors, keyed by type.
#[derive(Default)]
pub(crate) struct HostInlineSet {
    instances: HashMap<String, Instance>,
    /// Saved states for types not created yet.
    pending: HashMap<String, serde_json::Value>,
}

impl HostInlineSet {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Create `preprocessor_type` if it is not running. Returns false when
    /// the registry has no host-inline factory for it or `init` failed.
    pub(crate) fn ensure(
        &mut self,
        preprocessor_type: &str,
        registry: &AnalyzerRegistry,
        options: &serde_json::Value,
    ) -> bool {
        if self.instances.contains_key(preprocessor_type) {
            return true;
        }
        let Some(mut preprocessor) = registry.create_host_inline(preprocessor_type) else {
            return false;
        };
        if let Err(e) = preprocessor.init(options) {
            log::warn!("Host-inline preprocessor '{preprocessor_type}' failed to init: {e:#}");
            return false;
        }
        if let Some(state) = self.pending.remove(preprocessor_type) {
            restore(preprocessor.as_mut(), preprocessor_type, &state);
        }
        let latest = AnalyzerSnapshot::from_defaults(&preprocessor.output_schema());
        self.instances.insert(
            preprocessor_type.to_owned(),
            Instance {
                preprocessor,
                latest,
                slowest: Duration::ZERO,
                over_budget_logged: false,
            },
        );
        true
    }

    /// Drop instances whose type is not in `keep`.
    pub(crate) fn retain(&mut self, keep: &[&str]) {
        self.instances.retain(|ty, instance| {
            let keep = keep.contains(&ty.as_str());
            if !keep {
                instance.preprocessor.shutdown();
            }
            keep
        });
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.instances.is_empty()
    }

    /// Step every instance for one frame. `states` holds each type's bound
    /// values.
    pub(crate) fn step(
        &mut self,
        time_delta: f32,
        frame_index: u32,
        render_size: (u32, u32),
        states: &HashMap<String, AnalyzerStateSnapshot>,
    ) {
        let empty = AnalyzerStateSnapshot::default();
        for (ty, instance) in &mut self.instances {
            let frame = HostFrame {
                time_delta,
                frame_index,
                render_size,
                state: states.get(ty).unwrap_or(&empty),
            };
            let started = Instant::now();
            instance.latest = instance.preprocessor.step(&frame);
            let took = started.elapsed();
            instance.slowest = instance.slowest.max(took);
            if took > STEP_BUDGET && !instance.over_budget_logged {
                instance.over_budget_logged = true;
                log::warn!(
                    "Host-inline preprocessor '{ty}' took {:.2} ms, over its {} ms budget",
                    took.as_secs_f64() * 1000.0,
                    STEP_BUDGET.as_millis()
                );
            }
        }
    }

    /// This frame's outputs of `preprocessor_type`.
    pub(crate) fn latest(&self, preprocessor_type: &str) -> Option<&AnalyzerSnapshot> {
        self.instances.get(preprocessor_type).map(|i| &i.latest)
    }

    /// Every instance's outputs, as (type, snapshot).
    pub(crate) fn snapshots(&self) -> impl Iterator<Item = (&str, &AnalyzerSnapshot)> {
        self.instances
            .iter()
            .map(|(ty, instance)| (ty.as_str(), &instance.latest))
    }

    /// The slowest step of `preprocessor_type` so far.
    #[allow(dead_code)] // read by performance telemetry and tests
    pub(crate) fn slowest_step(&self, preprocessor_type: &str) -> Option<Duration> {
        self.instances.get(preprocessor_type).map(|i| i.slowest)
    }

    /// Saved state of every instance that has some, by type.
    pub(crate) fn persisted_states(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut states = serde_json::Map::new();
        for (ty, instance) in &self.instances {
            if let Some(state) = instance.preprocessor.persisted_state() {
                states.insert(ty.clone(), state);
            }
        }
        for (ty, state) in &self.pending {
            states.entry(ty.clone()).or_insert_with(|| state.clone());
        }
        states
    }

    /// Restore saved states by type. A type not running yet gets its state
    /// when it is created.
    pub(crate) fn restore_states(&mut self, states: &serde_json::Map<String, serde_json::Value>) {
        for (ty, state) in states {
            match self.instances.get_mut(ty) {
                Some(instance) => restore(instance.preprocessor.as_mut(), ty, state),
                None => {
                    self.pending.insert(ty.clone(), state.clone());
                }
            }
        }
    }
}

impl Drop for HostInlineSet {
    fn drop(&mut self) {
        for instance in self.instances.values_mut() {
            instance.preprocessor.shutdown();
        }
    }
}

fn restore(preprocessor: &mut dyn HostInlinePreprocessor, ty: &str, state: &serde_json::Value) {
    if let Err(e) = preprocessor.restore_state(state) {
        log::warn!("Host-inline preprocessor '{ty}': saved state ignored: {e:#}");
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::analyzer::traits::{AnalyzerSchema, ScalarOutputDef};
    use crate::params::ParamValue;

    /// Counts frames, adding its bound `step` value each frame. Persists the
    /// count.
    pub(crate) struct Counter {
        count: f32,
    }

    pub(crate) const COUNTER: &str = "test_counter";

    pub(crate) fn registry() -> AnalyzerRegistry {
        AnalyzerRegistry::new()
            .register_host_inline(COUNTER, || Box::new(Counter { count: 0.0 }))
    }

    impl HostInlinePreprocessor for Counter {
        fn output_schema(&self) -> AnalyzerSchema {
            AnalyzerSchema {
                scalars: vec![ScalarOutputDef {
                    name: "count".into(),
                    description: String::new(),
                    range: (0.0, f32::MAX),
                    default: 0.0,
                    default_smoothing: 0.0,
                }],
                textures: vec![],
            }
        }

        fn init(&mut self, _options: &serde_json::Value) -> anyhow::Result<()> {
            Ok(())
        }

        fn step(&mut self, frame: &HostFrame<'_>) -> AnalyzerSnapshot {
            let step = match frame.state.values.get("step") {
                Some(ParamValue::Float(v)) => *v,
                _ => 1.0,
            };
            self.count += step;
            let mut snapshot = AnalyzerSnapshot::from_defaults(&self.output_schema());
            snapshot.scalars.insert("count".into(), self.count);
            snapshot
        }

        fn persisted_state(&self) -> Option<serde_json::Value> {
            Some(serde_json::json!({ "count": self.count }))
        }

        fn restore_state(&mut self, state: &serde_json::Value) -> anyhow::Result<()> {
            self.count = state
                .get("count")
                .and_then(serde_json::Value::as_f64)
                .ok_or_else(|| anyhow::anyhow!("no count"))? as f32;
            Ok(())
        }
    }

    fn step_once(set: &mut HostInlineSet, step: Option<f32>) {
        let mut states = HashMap::new();
        if let Some(step) = step {
            let mut state = AnalyzerStateSnapshot::default();
            state.values.insert("step".into(), ParamValue::Float(step));
            states.insert(COUNTER.to_owned(), state);
        }
        set.step(1.0 / 60.0, 0, (64, 64), &states);
    }

    fn count(set: &HostInlineSet) -> f32 {
        set.latest(COUNTER).unwrap().scalar("count")
    }

    #[test]
    fn steps_every_frame_with_bound_values() {
        let mut set = HostInlineSet::new();
        assert!(set.ensure(COUNTER, &registry(), &serde_json::Value::Null));
        step_once(&mut set, None);
        step_once(&mut set, Some(2.0));
        assert_eq!(count(&set), 3.0);
    }

    #[test]
    fn unknown_type_is_not_created() {
        let mut set = HostInlineSet::new();
        assert!(!set.ensure("nope", &registry(), &serde_json::Value::Null));
        assert!(set.is_empty());
    }

    #[test]
    fn state_round_trips_and_waits_for_the_instance() {
        let mut set = HostInlineSet::new();
        set.ensure(COUNTER, &registry(), &serde_json::Value::Null);
        step_once(&mut set, Some(5.0));
        let saved = set.persisted_states();

        let mut restored = HostInlineSet::new();
        restored.restore_states(&saved);
        assert_eq!(
            restored.persisted_states(),
            saved,
            "a pending state is saved again unchanged"
        );
        restored.ensure(COUNTER, &registry(), &serde_json::Value::Null);
        step_once(&mut restored, Some(1.0));
        assert_eq!(count(&restored), 6.0);
    }

    #[test]
    fn a_malformed_state_keeps_the_current_one() {
        let mut set = HostInlineSet::new();
        set.ensure(COUNTER, &registry(), &serde_json::Value::Null);
        step_once(&mut set, Some(4.0));
        let mut bad = serde_json::Map::new();
        bad.insert(COUNTER.to_owned(), serde_json::json!("garbage"));
        set.restore_states(&bad);
        step_once(&mut set, Some(1.0));
        assert_eq!(count(&set), 5.0);
    }

    #[test]
    fn retain_drops_types_no_longer_declared() {
        let mut set = HostInlineSet::new();
        set.ensure(COUNTER, &registry(), &serde_json::Value::Null);
        set.retain(&[]);
        assert!(set.is_empty());
    }
}
