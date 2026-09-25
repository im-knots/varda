//! The engine snapshot every consumer reads. Published every 10th frame and
//! shared, not copied: readers take an `Arc` without a lock, and the JSON form
//! is serialized at most once per publication however many clients want it.
//! See /spec/state-publication.md.

use crate::engine::EngineState;
use arc_swap::ArcSwapOption;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

/// One published snapshot.
pub struct PublishedState {
    /// Increases with every publication, so a reader can tell whether anything
    /// new has arrived without comparing states.
    pub generation: u64,
    pub state: EngineState,
    json: OnceLock<serde_json::Value>,
}

impl PublishedState {
    /// The snapshot as JSON, serialized on first use and shared after that.
    pub fn json(&self) -> &serde_json::Value {
        self.json
            .get_or_init(|| match serde_json::to_value(&self.state) {
                Ok(value) => value,
                Err(e) => {
                    log::error!("Failed to serialize engine state: {e}");
                    serde_json::Value::Null
                }
            })
    }
}

impl std::ops::Deref for PublishedState {
    type Target = EngineState;

    fn deref(&self) -> &EngineState {
        &self.state
    }
}

/// Where the engine publishes its snapshot and consumers read it.
#[derive(Default)]
pub struct StatePublication {
    latest: ArcSwapOption<PublishedState>,
    generation: AtomicU64,
}

impl StatePublication {
    /// A publication that already holds `state`.
    pub fn with_state(state: EngineState) -> Self {
        let publication = Self::default();
        publication.publish(state);
        publication
    }

    /// Replace the published snapshot.
    pub fn publish(&self, state: EngineState) {
        let generation = self.generation.fetch_add(1, Ordering::Relaxed) + 1;
        self.latest.store(Some(Arc::new(PublishedState {
            generation,
            state,
            json: OnceLock::new(),
        })));
    }

    /// The latest snapshot, or `None` before the engine has published one.
    pub fn latest(&self) -> Option<Arc<PublishedState>> {
        self.latest.load_full()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real snapshot from a headless engine; `None` without a GPU adapter.
    fn state() -> Option<EngineState> {
        let gpu = crate::renderer::context::GpuContext::new_headless().ok()?;
        let app = crate::app::VardaApp::new(gpu, &crate::testing::headless_config()).ok()?;
        Some(app.build_engine_state())
    }

    #[test]
    fn nothing_is_published_until_the_engine_publishes() {
        assert!(StatePublication::default().latest().is_none());
    }

    #[test]
    fn each_publication_has_a_newer_generation() {
        let Some(state) = state() else {
            return;
        };
        let publication = StatePublication::default();
        publication.publish(state.clone());
        let first = publication.latest().expect("published");
        publication.publish(state);
        let second = publication.latest().expect("published");
        assert!(second.generation > first.generation);
    }

    #[test]
    fn readers_share_one_snapshot_and_one_serialization() {
        let Some(state) = state() else {
            return;
        };
        let publication = StatePublication::default();
        publication.publish(state);
        let a = publication.latest().expect("published");
        let b = publication.latest().expect("published");
        assert!(Arc::ptr_eq(&a, &b));
        assert!(std::ptr::eq(a.json(), b.json()));
    }
}
