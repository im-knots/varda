//! A live texture from a device or another application, blitted onto the deck.

use super::{
    AlphaPolicy, ScaledBlit, SourceControl, SourceFrame, SourceParamError, SourceStatus,
    SourceValue,
};
use crate::renderer::GpuContext;
use anyhow::Result;

/// What every externally fed source shares: the view its device manager
/// publishes this frame, and the scaled blit that draws it.
///
/// A provider hands the instance its view in
/// [`super::DeckSourceProvider::prepare`]; with none (not yet connected, or
/// the device went away) the deck shows black, or transparent on a
/// transparent deck.
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

    /// Take this frame's view and the size of the texture behind it. The size
    /// is pushed each frame because a stream that reconnects at a new
    /// resolution, or a capture being cropped, reallocates its texture.
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

    pub fn param(&self, name: &str) -> Option<SourceValue> {
        self.blit.param(name)
    }

    /// # Errors
    ///
    /// Fails for anything but the scaling-mode control.
    pub fn set_param(&mut self, name: &str, value: &SourceValue) -> Result<(), SourceParamError> {
        self.blit
            .set_param(name, value)
            .unwrap_or_else(|| Err(SourceParamError::Unknown(name.to_string())))
    }

    /// Status with the scaling mode and whether frames are arriving.
    pub fn status(&self, connected: Option<bool>) -> SourceStatus {
        let mut status = SourceStatus {
            connected,
            ..SourceStatus::default()
        };
        if let Some(v) = self.param(super::SCALING_MODE) {
            status.params.insert(super::SCALING_MODE.into(), v);
        }
        status
    }
}
