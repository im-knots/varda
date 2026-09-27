//! Drawing a texture of its own size onto the deck: the one thing most
//! sources do last.

use super::{
    SourceControl, SourceFrame, SourceParamError, SourceParamSpec, SourceValue, choice_index,
    choice_value, expect_norm,
};
use crate::engine::value::source::ScalingMode;
use crate::renderer::{BlitPipeline, GpuContext};
use anyhow::Result;

/// Name and route of the scaling-mode control every blitted source shares.
pub const SCALING_MODE: &str = "scaling_mode";

/// The scaling-mode control, for a provider's schema.
pub fn scaling_mode_spec() -> SourceParamSpec {
    SourceParamSpec::choice(
        SCALING_MODE,
        "Scaling",
        &["Fill", "Fit", "Stretch", "Center"],
    )
    .routed(SCALING_MODE)
    .modulatable()
}

impl ScalingMode {
    /// The mode a fader at `value` (0.0–1.0) selects: four equal buckets.
    pub fn from_value(value: f32) -> Self {
        match choice_index(value, 4) {
            0 => ScalingMode::Fill,
            1 => ScalingMode::Fit,
            2 => ScalingMode::Stretch,
            _ => ScalingMode::Center,
        }
    }

    /// The value at the centre of this mode's bucket. Inverse of [`Self::from_value`].
    pub fn to_value(self) -> f32 {
        let index = match self {
            ScalingMode::Fill => 0,
            ScalingMode::Fit => 1,
            ScalingMode::Stretch => 2,
            ScalingMode::Center => 3,
        };
        choice_value(index, 4)
    }

    /// Compute UV scale and offset for blitting source into target
    /// Returns (`uv_scale`, `uv_offset`) to transform target UVs to source UVs
    pub fn compute_uv_transform(
        &self,
        source_w: u32,
        source_h: u32,
        target_w: u32,
        target_h: u32,
    ) -> ([f32; 2], [f32; 2]) {
        let src_aspect = source_w as f32 / source_h as f32;
        let tgt_aspect = target_w as f32 / target_h as f32;

        match self {
            ScalingMode::Stretch => ([1.0, 1.0], [0.0, 0.0]),
            ScalingMode::Fill => {
                if src_aspect > tgt_aspect {
                    let scale_x = tgt_aspect / src_aspect;
                    let offset_x = (1.0 - scale_x) * 0.5;
                    ([scale_x, 1.0], [offset_x, 0.0])
                } else {
                    let scale_y = src_aspect / tgt_aspect;
                    let offset_y = (1.0 - scale_y) * 0.5;
                    ([1.0, scale_y], [0.0, offset_y])
                }
            }
            ScalingMode::Fit => {
                if src_aspect > tgt_aspect {
                    let scale_y = src_aspect / tgt_aspect;
                    let offset_y = (1.0 - scale_y) * 0.5;
                    ([1.0, scale_y], [0.0, offset_y])
                } else {
                    let scale_x = tgt_aspect / src_aspect;
                    let offset_x = (1.0 - scale_x) * 0.5;
                    ([scale_x, 1.0], [offset_x, 0.0])
                }
            }
            ScalingMode::Center => {
                let scale_x = target_w as f32 / source_w as f32;
                let scale_y = target_h as f32 / source_h as f32;
                let offset_x = (1.0 - scale_x) * 0.5;
                let offset_y = (1.0 - scale_y) * 0.5;
                ([scale_x, scale_y], [offset_x, offset_y])
            }
        }
    }
}

/// How a blit treats the source's alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaPolicy {
    /// Write the source verbatim over a black clear, whatever the deck's
    /// transparency: files carry the alpha they were authored with.
    Verbatim,
    /// Follow the deck's `transparent` flag: verbatim over a transparent clear
    /// when set, otherwise flattened over black so a translucent live source
    /// does not punch holes in the mix. See /spec/html-source.md §2.
    FollowDeck,
}

/// A scaled blit of a source texture onto the deck, with the scaling-mode
/// control that goes with it.
pub struct ScaledBlit {
    replace: BlitPipeline,
    over_black: Option<BlitPipeline>,
    policy: AlphaPolicy,
    label: &'static str,
    pub scaling_mode: ScalingMode,
    /// Size of the texture being drawn, which the scaling mode fits.
    pub source_size: (u32, u32),
}

impl ScaledBlit {
    /// # Errors
    ///
    /// Fails when a blit pipeline cannot be created.
    pub fn new(
        gpu: &GpuContext,
        policy: AlphaPolicy,
        label: &'static str,
        source_size: (u32, u32),
    ) -> Result<Self> {
        let replace = BlitPipeline::new(&gpu.device, gpu.compositing_format)?;
        let over_black = match policy {
            AlphaPolicy::Verbatim => None,
            AlphaPolicy::FollowDeck => Some(BlitPipeline::with_blend(
                &gpu.device,
                gpu.compositing_format,
                wgpu::BlendState::ALPHA_BLENDING,
            )?),
        };
        Ok(Self {
            replace,
            over_black,
            policy,
            label,
            scaling_mode: ScalingMode::default(),
            source_size,
        })
    }

    /// Draw `view` onto the frame's target.
    pub fn draw(&self, frame: &mut SourceFrame, view: &wgpu::TextureView) {
        let (pipeline, clear) = match (self.policy, &self.over_black) {
            (AlphaPolicy::FollowDeck, Some(over_black)) if !frame.transparent => {
                (over_black, wgpu::Color::BLACK)
            }
            (AlphaPolicy::FollowDeck, _) => (&self.replace, wgpu::Color::TRANSPARENT),
            (AlphaPolicy::Verbatim, _) => (&self.replace, wgpu::Color::BLACK),
        };

        let (sw, sh) = self.source_size;
        let (uv_scale, uv_offset) =
            self.scaling_mode
                .compute_uv_transform(sw.max(1), sh.max(1), frame.width, frame.height);
        pipeline.set_uv_transform(&frame.gpu.queue, 1.0, uv_scale, uv_offset);

        let bind_group = pipeline.create_bind_group(&frame.gpu.device, view);
        let mut encoder =
            frame
                .gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some(self.label),
                });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(self.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: frame.target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pipeline.render(&mut pass, &bind_group);
        }
        frame.cmd_buffers.push(encoder.finish());
    }

    /// Clear the frame's target to what this blit shows when it has nothing to
    /// draw: black, or transparent on a transparent deck.
    pub fn clear(&self, frame: &mut SourceFrame) {
        let clear = if self.policy == AlphaPolicy::FollowDeck && frame.transparent {
            wgpu::Color::TRANSPARENT
        } else {
            wgpu::Color::BLACK
        };
        clear_target(frame, clear, self.label);
    }

    /// Let modulation drive the scaling mode. Discrete, so it is written only
    /// when the mode the modulator points at differs from the one in force.
    pub fn control(&mut self, ctx: &mut SourceControl) {
        if let Some(resolved) = ctx.resolve(SCALING_MODE) {
            let next = ScalingMode::from_value(super::discrete_value(&resolved));
            if next != self.scaling_mode {
                self.scaling_mode = next;
            }
        }
    }

    /// The scaling-mode control's value, if `name` is it.
    pub fn param(&self, name: &str) -> Option<SourceValue> {
        (name == SCALING_MODE).then(|| SourceValue::Float(self.scaling_mode.to_value()))
    }

    /// Write the scaling-mode control. `None` when `name` is some other control.
    pub fn set_param(
        &mut self,
        name: &str,
        value: &SourceValue,
    ) -> Option<Result<(), SourceParamError>> {
        if name != SCALING_MODE {
            return None;
        }
        Some(expect_norm(name, value).map(|v| self.scaling_mode = ScalingMode::from_value(v)))
    }
}

/// Clear the frame's target to `color`.
pub fn clear_target(frame: &mut SourceFrame, color: wgpu::Color, label: &'static str) {
    let mut encoder = frame
        .gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some(label) });
    {
        let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: frame.target,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(color),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    frame.cmd_buffers.push(encoder.finish());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_mode_buckets() {
        assert_eq!(ScalingMode::from_value(0.0), ScalingMode::Fill);
        assert_eq!(ScalingMode::from_value(0.3), ScalingMode::Fit);
        assert_eq!(ScalingMode::from_value(0.6), ScalingMode::Stretch);
        assert_eq!(ScalingMode::from_value(1.0), ScalingMode::Center);
    }

    #[test]
    fn scaling_modes_round_trip_through_their_buckets() {
        // Both directions are written out by hand, so a reordering of either
        // match has to show up here rather than as a mode that silently becomes
        // its neighbour when a live gesture is recorded.
        for mode in ScalingMode::ALL {
            assert_eq!(ScalingMode::from_value(mode.to_value()), mode);
        }
    }

    #[test]
    fn scaling_mode_default_is_fill() {
        assert_eq!(ScalingMode::default(), ScalingMode::Fill);
    }

    #[test]
    fn stretch_returns_identity() {
        let (scale, offset) = ScalingMode::Stretch.compute_uv_transform(800, 600, 1920, 1080);
        assert_eq!(scale, [1.0, 1.0]);
        assert_eq!(offset, [0.0, 0.0]);
    }

    #[test]
    fn fill_same_aspect_is_identity() {
        let (scale, offset) = ScalingMode::Fill.compute_uv_transform(1920, 1080, 960, 540);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((offset[0]).abs() < 1e-5);
        assert!((offset[1]).abs() < 1e-5);
    }

    #[test]
    fn fill_wide_source_crops_horizontal() {
        let (scale, offset) = ScalingMode::Fill.compute_uv_transform(200, 100, 100, 100);
        assert!((scale[0] - 0.5).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((offset[0] - 0.25).abs() < 1e-5);
        assert!((offset[1]).abs() < 1e-5);
    }

    #[test]
    fn fill_tall_source_crops_vertical() {
        let (scale, offset) = ScalingMode::Fill.compute_uv_transform(100, 200, 100, 100);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 0.5).abs() < 1e-5);
        assert!((offset[0]).abs() < 1e-5);
        assert!((offset[1] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn fit_wide_source_letterboxes() {
        let (scale, _offset) = ScalingMode::Fit.compute_uv_transform(200, 100, 100, 100);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 2.0).abs() < 1e-5);
    }

    #[test]
    fn fit_tall_source_pillarboxes() {
        let (scale, _offset) = ScalingMode::Fit.compute_uv_transform(100, 200, 100, 100);
        assert!((scale[0] - 2.0).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn center_smaller_source() {
        let (scale, offset) = ScalingMode::Center.compute_uv_transform(100, 100, 200, 200);
        assert!((scale[0] - 2.0).abs() < 1e-5);
        assert!((scale[1] - 2.0).abs() < 1e-5);
        assert!((offset[0] - -0.5).abs() < 1e-5);
        assert!((offset[1] - -0.5).abs() < 1e-5);
    }

    #[test]
    fn center_larger_source() {
        let (scale, offset) = ScalingMode::Center.compute_uv_transform(400, 400, 200, 200);
        assert!((scale[0] - 0.5).abs() < 1e-5);
        assert!((scale[1] - 0.5).abs() < 1e-5);
        assert!((offset[0] - 0.25).abs() < 1e-5);
        assert!((offset[1] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn center_same_size_is_identity() {
        let (scale, offset) = ScalingMode::Center.compute_uv_transform(1920, 1080, 1920, 1080);
        assert!((scale[0] - 1.0).abs() < 1e-5);
        assert!((scale[1] - 1.0).abs() < 1e-5);
        assert!((offset[0]).abs() < 1e-5);
        assert!((offset[1]).abs() < 1e-5);
    }
}
