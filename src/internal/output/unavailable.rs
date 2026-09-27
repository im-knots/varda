//! A sink that holds a saved output this build or host cannot run, so the
//! output keeps its surfaces, warp and settings and saves back unchanged.
//! Mirrors the deck placeholder of /spec/deck-source-providers.md Decision 4.

use super::{FramePath, OutputSinkInstance, Presentation, SinkConfig, SinkQuery};
use crate::delivery::presentation::{modes_for, resolve_eight_bit_sdr};
use crate::engine::value::render::PresentationRequest;
use crate::renderer::context::GpuContext;
use anyhow::Result;

/// A saved output whose type cannot run here.
pub struct UnavailableSink {
    config: SinkConfig,
    reason: String,
}

impl UnavailableSink {
    pub fn new(config: SinkConfig, reason: String) -> Self {
        Self { config, reason }
    }

    pub fn reason(&self) -> &str {
        &self.reason
    }
}

impl OutputSinkInstance for UnavailableSink {
    fn sink_type(&self) -> &str {
        self.config.type_id()
    }

    fn label(&self) -> String {
        format!("Unavailable: {}", self.reason)
    }

    fn config(&self) -> SinkConfig {
        self.config.clone()
    }

    fn frame_path(&self) -> FramePath {
        FramePath::Gpu
    }

    fn configure(
        &mut self,
        _gpu: &GpuContext,
        _query: &SinkQuery,
        request: PresentationRequest,
    ) -> Result<Presentation> {
        Ok(Presentation {
            resolved: resolve_eight_bit_sdr(request, &self.reason),
            modes: modes_for(|r| resolve_eight_bit_sdr(r, &self.reason)),
        })
    }

    fn start(
        &mut self,
        _start: super::SinkStart,
    ) -> Result<Option<crate::engine::value::render::ResolvedPresentation>> {
        anyhow::bail!("{}", self.reason)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
