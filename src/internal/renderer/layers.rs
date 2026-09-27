//! Compositing a stack of layers into one texture. Two textures take turns as
//! the composite so far and the next pass's target, so a blend never copies the
//! composite first. The first write is chosen so the last one lands in the
//! final texture, which consumers hold views of.
//! See /spec/performance-hot-paths.md item C.

use super::blit::{BlitPipeline, CompositeBlitPipeline};
use super::context::GpuContext;

/// A texture and a view of it.
pub type TargetTexture<'a> = (&'a wgpu::Texture, &'a wgpu::TextureView);

/// One layer blended over the composite so far.
pub struct BlendLayer<'a> {
    pub view: &'a wgpu::TextureView,
    pub opacity: f32,
    /// A blend mode index, as `BlendMode::to_index` gives.
    pub blend_mode: u32,
    /// Whether the layer's color is premultiplied by its alpha.
    pub premultiplied: bool,
}

/// The pipelines a stack draws its blends with.
pub struct LayerPipelines<'a> {
    pub blit: &'a BlitPipeline,
    pub composite: &'a CompositeBlitPipeline,
}

/// A stack of layers composited into `final`, drawn one at a time.
pub struct LayerStack<'a> {
    /// The final texture, then the scratch texture.
    targets: [TargetTexture<'a>; 2],
    /// Which target the next layer writes.
    next: usize,
    /// Layers drawn so far, which is also the next layer's params slot.
    drawn: usize,
    /// Layers the stack was planned for.
    planned: usize,
    /// What the first layer is drawn over.
    base: wgpu::Color,
}

impl<'a> LayerStack<'a> {
    /// A stack of `layers` layers ending in `final_texture`, using `scratch`
    /// for every other one. The first layer is drawn over `base`.
    pub fn new(
        final_texture: TargetTexture<'a>,
        scratch: TargetTexture<'a>,
        layers: usize,
        base: wgpu::Color,
    ) -> Self {
        Self {
            targets: [final_texture, scratch],
            // Odd counts start in the final texture, even ones in scratch.
            next: 1 - layers % 2,
            drawn: 0,
            planned: layers,
            base,
        }
    }

    /// The composite so far and the target, for a layer drawn by its own
    /// pipeline, such as a transition shader, which must write every pixel of
    /// the target. `None`, drawing nothing, when no layer is below yet.
    pub fn pass_over(&mut self) -> Option<(&'a wgpu::TextureView, &'a wgpu::TextureView)> {
        let (below, target) = self.targets_for_next();
        let below = below?;
        self.advance();
        Some((below, target))
    }

    /// Blend `layer` over the composite so far, or draw it over the base when
    /// it is the first.
    pub fn blend(
        &mut self,
        context: &GpuContext,
        pipelines: &LayerPipelines<'_>,
        layer: &BlendLayer<'_>,
        out: &mut Vec<wgpu::CommandBuffer>,
    ) {
        let slot = self.drawn;
        let (below, target) = self.targets_for_next();
        self.advance();
        out.push(draw_layer(
            context, pipelines, slot, layer, below, target, self.base,
        ));
    }

    /// The composite so far, if any, and the next layer's target.
    fn targets_for_next(&self) -> (Option<&'a wgpu::TextureView>, &'a wgpu::TextureView) {
        let below = (self.drawn > 0).then(|| self.targets[1 - self.next].1);
        let target = self.targets[self.next].1;
        (below, target)
    }

    fn advance(&mut self) {
        debug_assert!(
            self.drawn < self.planned,
            "a layer stack drew more layers than it planned, so its result is not in the final texture"
        );
        self.drawn += 1;
        self.next = 1 - self.next;
    }
}

/// Encode one layer into `target`: blended over `below`, or drawn over `base`
/// when there is nothing below. `slot` is the layer's params slot.
fn draw_layer(
    context: &GpuContext,
    pipelines: &LayerPipelines<'_>,
    slot: usize,
    layer: &BlendLayer<'_>,
    below: Option<&wgpu::TextureView>,
    target: &wgpu::TextureView,
    base: wgpu::Color,
) -> wgpu::CommandBuffer {
    let mut encoder = context
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Layer Composite"),
        });
    let bind_group = if let Some(below) = below {
        pipelines.composite.write_params_slot(
            &context.queue,
            slot,
            layer.opacity,
            layer.blend_mode,
            [1.0, 1.0],
            [0.0, 0.0],
            layer.premultiplied,
        );
        pipelines
            .composite
            .create_ring_bind_group(&context.device, layer.view, below, slot)
    } else {
        pipelines.blit.write_params_slot(
            &context.queue,
            slot,
            layer.opacity,
            [1.0, 1.0],
            [0.0, 0.0],
            layer.premultiplied,
        );
        pipelines
            .blit
            .create_ring_bind_group(&context.device, layer.view, slot)
    };
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Layer Composite"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                ops: wgpu::Operations {
                    // A blend replaces every pixel, so only the first layer's
                    // clear is ever seen.
                    load: wgpu::LoadOp::Clear(base),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if below.is_some() {
            pipelines.composite.render_at_slot(&mut pass, &bind_group);
        } else {
            pipelines.blit.render_at_slot(&mut pass, &bind_group);
        }
    }
    encoder.finish()
}

#[cfg(test)]
mod tests {
    use super::{BlendLayer, LayerPipelines, LayerStack, draw_layer};
    use crate::renderer::blit::{BlitPipeline, CompositeBlitPipeline};
    use crate::renderer::context::GpuContext;

    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    /// Every blend mode index, as `BlendMode::to_index` numbers them.
    const BLEND_MODES: std::ops::Range<u32> = 0..15;

    /// A layer whose color and alpha vary across it, so every blend mode sees
    /// a range of inputs.
    fn layer_texture(context: &GpuContext, seed: u32) -> wgpu::TextureView {
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("layer"),
            size: wgpu::Extent3d {
                width: WIDTH,
                height: HEIGHT,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let pixels: Vec<u8> = (0..WIDTH * HEIGHT)
            .flat_map(|i| {
                let v = i * 29 + seed * 71;
                [
                    (v % 251) as u8,
                    ((v * 3) % 241) as u8,
                    ((v * 7) % 239) as u8,
                    (128 + (v * 5) % 128) as u8,
                ]
            })
            .collect();
        context.queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 4),
                rows_per_image: Some(HEIGHT),
            },
            texture.size(),
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    fn target(context: &GpuContext) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = context.create_compositing_texture(WIDTH, HEIGHT);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    fn read(context: &GpuContext, texture: &wgpu::Texture) -> Vec<u8> {
        let texel = texture.format().block_copy_size(None).unwrap();
        let row = (WIDTH * texel).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("layer readback"),
            size: u64::from(row * HEIGHT),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(HEIGHT),
                },
            },
            texture.size(),
        );
        context.submit(std::iter::once(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
        buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback")
            .to_vec()
    }

    /// Ping-pong compositing gives the same picture as copying the composite
    /// into a scratch texture before every blend, the method it replaced, at
    /// every blend mode and for odd and even layer counts.
    #[test]
    fn ping_pong_matches_copying_the_composite_before_each_blend() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let blit = BlitPipeline::new(&context.device, context.compositing_format).unwrap();
        let composite =
            CompositeBlitPipeline::new(&context.device, context.compositing_format).unwrap();
        let pipelines = LayerPipelines {
            blit: &blit,
            composite: &composite,
        };
        let views: Vec<wgpu::TextureView> = (0..4).map(|i| layer_texture(&context, i)).collect();
        blit.ensure_ring_slots(&context.device, views.len());
        composite.ensure_ring_slots(&context.device, views.len());

        for count in 1..=views.len() {
            for blend_mode in BLEND_MODES {
                let layers: Vec<BlendLayer<'_>> = views[..count]
                    .iter()
                    .enumerate()
                    .map(|(i, view)| BlendLayer {
                        view,
                        opacity: 1.0 - i as f32 * 0.2,
                        blend_mode,
                        premultiplied: false,
                    })
                    .collect();

                let (final_texture, final_view) = target(&context);
                let (scratch, scratch_view) = target(&context);
                let mut commands = Vec::new();
                let mut stack = LayerStack::new(
                    (&final_texture, &final_view),
                    (&scratch, &scratch_view),
                    count,
                    wgpu::Color::TRANSPARENT,
                );
                for layer in &layers {
                    stack.blend(&context, &pipelines, layer, &mut commands);
                }
                context.submit(commands);
                let ping_pong = read(&context, &final_texture);

                let (copied, copied_view) = target(&context);
                let (snapshot, snapshot_view) = target(&context);
                for (slot, layer) in layers.iter().enumerate() {
                    let mut commands = Vec::new();
                    let below = if slot == 0 {
                        None
                    } else {
                        let mut encoder = context
                            .device
                            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                        encoder.copy_texture_to_texture(
                            copied.as_image_copy(),
                            snapshot.as_image_copy(),
                            copied.size(),
                        );
                        commands.push(encoder.finish());
                        Some(&snapshot_view)
                    };
                    commands.push(draw_layer(
                        &context,
                        &pipelines,
                        slot,
                        layer,
                        below,
                        &copied_view,
                        wgpu::Color::TRANSPARENT,
                    ));
                    context.submit(commands);
                }
                let copy_based = read(&context, &copied);

                assert_eq!(
                    ping_pong, copy_based,
                    "{count} layers at blend mode {blend_mode}"
                );
            }
        }
    }
}
