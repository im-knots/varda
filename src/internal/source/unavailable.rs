//! The placeholder a deck gets when its source type cannot run.

use super::{ControlStatus, DeckSourceInstance, SourceConfig, SourceFrame};
use anyhow::Result;

/// A deck whose source type this build cannot run, or that failed to open.
///
/// It renders black and saves its config back unchanged, so a scene moved
/// between machines keeps every deck it had. See
/// /spec/deck-source-providers.md Decision 4.
pub struct UnavailableSource {
    config: SourceConfig,
    reason: String,
}

impl UnavailableSource {
    pub fn new(config: SourceConfig, reason: impl Into<String>) -> Self {
        Self {
            config,
            reason: reason.into(),
        }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl DeckSourceInstance for UnavailableSource {
    fn source_type(&self) -> &str {
        self.config.type_id()
    }

    fn label(&self) -> String {
        format!("⚠ {} (unavailable)", self.config.type_id())
    }

    fn config(&self) -> SourceConfig {
        self.config.clone()
    }

    fn render(&mut self, frame: &mut SourceFrame) -> Result<()> {
        super::clear_target(frame, wgpu::Color::BLACK, "Unavailable Source");
        Ok(())
    }

    fn status(&self) -> ControlStatus {
        let mut status = ControlStatus {
            bound: Some(false),
            ..ControlStatus::default()
        };
        status.info.insert(
            "unavailable_reason".into(),
            serde_json::Value::String(self.reason.clone()),
        );
        status
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
