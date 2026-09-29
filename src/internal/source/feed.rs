//! A live texture from a device or another application, blitted onto the deck.

use super::{
    AlphaPolicy, ControlError, ControlStatus, ControlValue, ScaledBlit, SourceControl, SourceFrame,
};
use crate::renderer::GpuContext;
use anyhow::Result;

/// An externally fed source: the view its device manager publishes this
/// frame, and the scaled blit that draws it.
///
/// The provider passes the view in [`super::DeckSourceProvider::prepare`].
/// Without one the deck shows black, or transparent on a transparent deck.
pub struct Feed {
    pub blit: ScaledBlit,
    pub view: Option<wgpu::TextureView>,
}

impl Feed {
    /// # Errors
    ///
    /// Fails when the blit pipelines cannot be created.
    pub fn new(gpu: &GpuContext, label: &'static str, source_size: (u32, u32)) -> Result<Self> {
        Ok(Self {
            blit: ScaledBlit::new(gpu, AlphaPolicy::FollowDeck, label, source_size)?,
            view: None,
        })
    }

    /// Take this frame's view and its texture size. The size can change
    /// between frames when a stream reconnects or a capture is cropped.
    pub fn bind(&mut self, view: Option<wgpu::TextureView>, size: Option<(u32, u32)>) {
        self.view = view;
        if let Some(size) = size {
            self.blit.source_size = size;
        }
    }

    pub fn render(&self, frame: &mut SourceFrame) {
        match &self.view {
            Some(view) => self.blit.draw(frame, view),
            None => self.blit.clear(frame),
        }
    }

    pub fn control(&mut self, ctx: &mut SourceControl) {
        self.blit.control(ctx);
    }

    pub fn param(&self, name: &str) -> Option<ControlValue> {
        self.blit.param(name)
    }

    /// # Errors
    ///
    /// Fails for anything but the scaling-mode control.
    pub fn set_param(&mut self, name: &str, value: &ControlValue) -> Result<(), ControlError> {
        self.blit
            .set_param(name, value)
            .unwrap_or_else(|| Err(ControlError::Unknown(name.to_string())))
    }

    /// Status with the scaling mode and whether frames are arriving.
    pub fn status(&self, connected: Option<bool>) -> ControlStatus {
        let mut status = ControlStatus {
            connected,
            ..ControlStatus::default()
        };
        if let Some(v) = self.param(super::SCALING_MODE) {
            status.params.insert(super::SCALING_MODE.into(), v);
        }
        status
    }
}
