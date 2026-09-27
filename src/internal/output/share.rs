//! Inter-app sharing sinks: a named server another application on this machine
//! reads the finished texture from, on the GPU. Syphon and Spout are the same
//! sink around different managers.

use super::{
    ControlSpec, ControlValue, FramePath, OutputSinkInstance, OutputSinkProvider, ParamEffect,
    Presentation, SinkConfig, SinkEnv, SinkFrame, SinkQuery,
};
use crate::delivery::presentation::modes_for;
use crate::engine::value::render::{PresentationRequest, ResolvedPresentation};
use crate::renderer::context::GpuContext;
use crate::source::{ControlError, expect_text};
use anyhow::Result;
use std::sync::LazyLock;

/// A manager that publishes textures under a name.
pub trait ShareSender: 'static {
    fn is_available(&self) -> bool;
    fn publish(
        &mut self,
        gpu: &GpuContext,
        name: &str,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
    );
}

/// What differs between sharing protocols.
pub struct ShareOutput {
    pub id: &'static str,
    pub label: &'static str,
    pub icon: &'static str,
    /// The config field holding the published name, as `stage.json` saves it.
    pub name_field: &'static str,
    pub unavailable: &'static str,
    pub presentation: fn(PresentationRequest) -> ResolvedPresentation,
}

static NAME_PARAMS: LazyLock<Vec<ControlSpec>> =
    LazyLock::new(|| vec![ControlSpec::text("name", "Name").routed("name")]);

/// A sharing sink type over manager `M`.
pub struct ShareSinkProvider<M> {
    output: &'static ShareOutput,
    _manager: std::marker::PhantomData<fn() -> M>,
}

impl<M: ShareSender> ShareSinkProvider<M> {
    pub const fn new(output: &'static ShareOutput) -> Self {
        Self {
            output,
            _manager: std::marker::PhantomData,
        }
    }
}

impl<M: ShareSender> OutputSinkProvider for ShareSinkProvider<M> {
    fn id(&self) -> &'static str {
        self.output.id
    }

    fn label(&self) -> &'static str {
        self.output.label
    }

    fn icon(&self) -> &'static str {
        self.output.icon
    }

    fn params(&self) -> &'static [ControlSpec] {
        &NAME_PARAMS
    }

    fn availability(&self, query: &SinkQuery) -> std::result::Result<(), String> {
        if query.services.get::<M>().is_some_and(M::is_available) {
            Ok(())
        } else {
            Err(self.output.unavailable.to_string())
        }
    }

    fn default_config(&self) -> SinkConfig {
        SinkConfig::new(self.output.id).with(self.output.name_field, "Varda")
    }

    fn create(
        &mut self,
        config: &SinkConfig,
        _env: &mut SinkEnv,
    ) -> Result<Box<dyn OutputSinkInstance>> {
        let name = config
            .str(self.output.name_field)
            .unwrap_or("Varda")
            .to_string();
        Ok(Box::new(ShareSink::<M> {
            output: self.output,
            name,
            _manager: std::marker::PhantomData,
        }))
    }
}

/// One published server.
pub struct ShareSink<M> {
    output: &'static ShareOutput,
    name: String,
    _manager: std::marker::PhantomData<fn() -> M>,
}

impl<M: ShareSender> OutputSinkInstance for ShareSink<M> {
    fn sink_type(&self) -> &str {
        self.output.id
    }

    fn label(&self) -> String {
        self.name.clone()
    }

    fn config(&self) -> SinkConfig {
        SinkConfig::new(self.output.id).with(self.output.name_field, &self.name)
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
            resolved: (self.output.presentation)(request),
            modes: modes_for(self.output.presentation),
        })
    }

    fn publish(&mut self, frame: &mut SinkFrame) -> std::result::Result<(), String> {
        if let Some(manager) = frame.services.get_mut::<M>() {
            manager.publish(frame.gpu, &self.name, frame.view, frame.width, frame.height);
        }
        Ok(())
    }

    fn schema(&self) -> &'static [ControlSpec] {
        &NAME_PARAMS
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        (name == "name").then(|| ControlValue::Text(self.name.clone()))
    }

    fn set_param(
        &mut self,
        name: &str,
        value: &ControlValue,
    ) -> std::result::Result<ParamEffect, ControlError> {
        if name != "name" {
            return Err(ControlError::Unknown(name.to_string()));
        }
        self.name = expect_text(name, value)?.to_string();
        Ok(ParamEffect::Applied)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
