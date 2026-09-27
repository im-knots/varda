//! NDI outputs: a named sender on the network. The finished texture is
//! converted to the sender's pixel format on the GPU and read back by the NDI
//! manager, so the output never reads RGBA back for it.
//! See /spec/performance-hot-paths.md A.

use super::{FrameConversion, NdiManager};
use crate::engine::value::render::PresentationRequest;
use crate::output::{
    ControlSpec, ControlValue, FramePath, OutputSinkInstance, OutputSinkProvider, ParamEffect,
    Presentation, SinkConfig, SinkEnv, SinkFrame, SinkQuery,
};
use crate::renderer::context::GpuContext;
use crate::source::{ControlError, expect_text};
use anyhow::Result;
use std::sync::LazyLock;

const SINK_TYPE: &str = "ndi_send";

static PARAMS: LazyLock<Vec<ControlSpec>> =
    LazyLock::new(|| vec![ControlSpec::text("name", "Sender name").routed("name")]);

/// NDI senders.
pub struct NdiSinkProvider;

impl OutputSinkProvider for NdiSinkProvider {
    fn id(&self) -> &'static str {
        SINK_TYPE
    }

    fn label(&self) -> &'static str {
        "NDI"
    }

    fn icon(&self) -> &'static str {
        "📡"
    }

    fn params(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn default_config(&self) -> SinkConfig {
        SinkConfig::new(SINK_TYPE).with("sender_name", "Varda NDI")
    }

    fn create(
        &mut self,
        config: &SinkConfig,
        _env: &mut SinkEnv,
    ) -> Result<Box<dyn OutputSinkInstance>> {
        Ok(Box::new(NdiSink {
            sender_name: config.str("sender_name").unwrap_or("Varda").to_string(),
        }))
    }
}

/// One NDI sender.
pub struct NdiSink {
    sender_name: String,
}

impl OutputSinkInstance for NdiSink {
    fn sink_type(&self) -> &str {
        SINK_TYPE
    }

    fn label(&self) -> String {
        self.sender_name.clone()
    }

    fn config(&self) -> SinkConfig {
        SinkConfig::new(SINK_TYPE).with("sender_name", &self.sender_name)
    }

    fn frame_path(&self) -> FramePath {
        FramePath::Converted
    }

    /// What NDI can carry comes from the loaded runtime rather than from the
    /// config. See /spec/presentation-mode-offering.md.
    fn configure(
        &mut self,
        _gpu: &GpuContext,
        query: &SinkQuery,
        request: PresentationRequest,
    ) -> Result<Presentation> {
        let ndi = query
            .services
            .get::<NdiManager>()
            .ok_or_else(|| anyhow::anyhow!("NDI is not running"))?;
        Ok(Presentation {
            resolved: ndi.resolve_presentation(request),
            modes: ndi.mode_availability(),
        })
    }

    fn encode(
        &mut self,
        frame: &mut SinkFrame,
        encoder: &mut wgpu::CommandEncoder,
    ) -> std::result::Result<(), String> {
        let Some(ndi) = frame.services.get_mut::<NdiManager>() else {
            return Ok(());
        };
        ndi.begin_frame(
            &self.sender_name,
            FrameConversion {
                device: &frame.gpu.device,
                queue: &frame.gpu.queue,
                encoder,
                source: frame.view,
                width: frame.width,
                height: frame.height,
                dither: frame.request.dither,
                pixel_format: frame.resolved.pixel_format.clone(),
            },
        )
        .map_err(|error| format!("NDI P216 conversion failed: {error}"))
    }

    fn publish(&mut self, frame: &mut SinkFrame) -> std::result::Result<(), String> {
        let Some(ndi) = frame.services.get_mut::<NdiManager>() else {
            return Ok(());
        };
        ndi.try_send(
            &self.sender_name,
            &frame.gpu.device,
            frame.width,
            frame.height,
            frame.fps,
        )
        .map_err(|error| format!("NDI P216 submission failed: {error}"))
    }

    fn schema(&self) -> &'static [ControlSpec] {
        &PARAMS
    }

    fn param(&self, name: &str) -> Option<ControlValue> {
        (name == "name").then(|| ControlValue::Text(self.sender_name.clone()))
    }

    fn set_param(
        &mut self,
        name: &str,
        value: &ControlValue,
    ) -> std::result::Result<ParamEffect, ControlError> {
        if name != "name" {
            return Err(ControlError::Unknown(name.to_string()));
        }
        self.sender_name = expect_text(name, value)?.to_string();
        Ok(ParamEffect::Applied)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
