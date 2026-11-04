//! Compositing a stack of layers into one texture. Up to [`FOLD_WIDTH`] layers
//! blend in one render pass, in the shader. A stack needs more passes when it
//! has more layers or a layer drawn by its own pipeline; those passes alternate
//! between two textures, so a pass never copies the composite. The first target
//! is chosen so the last write lands in the final texture.

use super::context::GpuContext;
use anyhow::Result;
use std::cell::{Cell, RefCell};
use std::num::NonZeroU64;

/// Layers blended in one render pass. Matches `FOLD_WIDTH` in `layer_fold.wgsl`.
pub const FOLD_WIDTH: usize = 8;

/// Initial and minimum parameter slots in the ring buffer. One slot per pass.
const INITIAL_SLOTS: usize = 4;

/// Binding of the first layer texture. The other layers follow it.
const FIRST_LAYER_BINDING: u32 = 1;
/// Binding of the composite from an earlier pass.
const BELOW_BINDING: u32 = FIRST_LAYER_BINDING + FOLD_WIDTH as u32;
/// Binding of the parameter block.
const PARAMS_BINDING: u32 = BELOW_BINDING + 1;

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

/// How a stack draws a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    /// Blended by the fold shader.
    Blended,
    /// Drawn by its own pipeline (e.g. a transition shader) in its own pass.
    OwnPass,
}

/// One layer's fold parameters. Matches `FoldLayer` in `layer_fold.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FoldLayer {
    opacity: f32,
    blend_mode: u32,
    premultiplied: u32,
    _pad: u32,
}

/// One pass's fold parameters. Matches `FoldParams` in `layer_fold.wgsl`.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct FoldParams {
    base: [f32; 4],
    layer_count: u32,
    has_below: u32,
    _pad: [u32; 2],
    layers: [FoldLayer; FOLD_WIDTH],
}

impl FoldParams {
    fn new(layers: &[BlendLayer<'_>], has_below: bool, base: wgpu::Color) -> Self {
        let mut folded = [bytemuck::Zeroable::zeroed(); FOLD_WIDTH];
        for (slot, layer) in folded.iter_mut().zip(layers) {
            *slot = FoldLayer {
                opacity: layer.opacity,
                blend_mode: layer.blend_mode,
                premultiplied: u32::from(layer.premultiplied),
                _pad: 0,
            };
        }
        Self {
            base: [base.r as f32, base.g as f32, base.b as f32, base.a as f32],
            layer_count: layers.len() as u32,
            has_below: u32::from(has_below),
            _pad: [0; 2],
            layers: folded,
        }
    }
}

/// The pipeline a stack draws its passes with. A pipeline holds one stack's
/// parameters at a time, so a stack's commands must be submitted before another
/// stack on the same pipeline writes.
pub struct LayerFoldPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Bound to layer slots a pass does not use, and to the composite slot when
    /// nothing is below.
    _empty_texture: wgpu::Texture,
    empty_view: wgpu::TextureView,
    /// Per-pass parameters, grown by [`Self::ensure_ring_slots`].
    ring_buffer: RefCell<wgpu::Buffer>,
    ring_slots: Cell<usize>,
    /// Byte stride between slots, aligned to the device minimum.
    ring_stride: usize,
    /// Reused staging for one run's parameters.
    scratch_params: RefCell<Vec<u8>>,
}

impl LayerFoldPipeline {
    /// Create the fold pipeline.
    ///
    /// # Errors
    ///
    /// Never returns `Err`; validation failures surface on the device's error
    /// scope. The `Result` matches the other pipeline constructors.
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let entries: [wgpu::BindGroupLayoutEntry; FOLD_WIDTH + 3] =
            std::array::from_fn(|index| match index as u32 {
                0 => wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                PARAMS_BINDING => wgpu::BindGroupLayoutEntry {
                    binding: PARAMS_BINDING,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(size_of::<FoldParams>() as u64),
                    },
                    count: None,
                },
                binding => texture_entry(binding),
            });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Layer Fold Bind Group Layout"),
            entries: &entries,
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Layer Fold Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let vertex_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Layer Fold Vertex Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/fullscreen.wgsl").into()),
        });
        let fragment_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Layer Fold Fragment Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/layer_fold.wgsl").into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Layer Fold Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &vertex_shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &fragment_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Layer Fold Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });
        let empty_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Layer Fold Empty Texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let empty_view = empty_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let align = device.limits().min_uniform_buffer_offset_alignment as usize;
        let ring_stride = size_of::<FoldParams>().next_multiple_of(align);
        Ok(Self {
            pipeline,
            bind_group_layout,
            sampler,
            _empty_texture: empty_texture,
            empty_view,
            ring_buffer: RefCell::new(Self::ring(device, INITIAL_SLOTS, ring_stride)),
            ring_slots: Cell::new(INITIAL_SLOTS),
            ring_stride,
            scratch_params: RefCell::new(Vec::new()),
        })
    }

    fn ring(device: &wgpu::Device, slots: usize, stride: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Layer Fold Params Ring Buffer"),
            size: (slots * stride) as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Grow the ring buffer to at least `needed` slots. Call once before a
    /// stack: growing mid-stack strands earlier bind groups on the old buffer.
    fn ensure_ring_slots(&self, device: &wgpu::Device, needed: usize) {
        if needed <= self.ring_slots.get() {
            return;
        }
        let slots = needed.next_power_of_two();
        *self.ring_buffer.borrow_mut() = Self::ring(device, slots, self.ring_stride);
        self.ring_slots.set(slots);
    }

    /// A bind group for one pass's `layers` over `below`, reading ring `slot`.
    ///
    /// # Panics
    ///
    /// Panics if `layers` holds more than [`FOLD_WIDTH`] layers, or if
    /// `size_of::<FoldParams>()` is zero, which cannot happen.
    pub fn create_bind_group(
        &self,
        device: &wgpu::Device,
        layers: &[BlendLayer<'_>],
        below: Option<&wgpu::TextureView>,
        slot: usize,
    ) -> wgpu::BindGroup {
        assert!(
            layers.len() <= FOLD_WIDTH,
            "a pass folds {FOLD_WIDTH} layers"
        );
        let ring = self.ring_buffer.borrow();
        let params = wgpu::BufferBinding {
            buffer: &ring,
            offset: (slot * self.ring_stride) as u64,
            size: NonZeroU64::new(size_of::<FoldParams>() as u64),
        };
        let entries: [wgpu::BindGroupEntry<'_>; FOLD_WIDTH + 3] = std::array::from_fn(|index| {
            let binding = index as u32;
            let resource = match binding {
                0 => wgpu::BindingResource::Sampler(&self.sampler),
                BELOW_BINDING => {
                    wgpu::BindingResource::TextureView(below.unwrap_or(&self.empty_view))
                }
                PARAMS_BINDING => wgpu::BindingResource::Buffer(params.clone()),
                _ => wgpu::BindingResource::TextureView(
                    layers
                        .get(index - FIRST_LAYER_BINDING as usize)
                        .map_or(&self.empty_view, |layer| layer.view),
                ),
            };
            wgpu::BindGroupEntry { binding, resource }
        });
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Layer Fold Bind Group"),
            layout: &self.bind_group_layout,
            entries: &entries,
        })
    }
}

/// Fold passes and own passes a stack draws for `kinds`, at `width` layers per pass.
fn pass_count(kinds: impl IntoIterator<Item = LayerKind>, width: usize) -> (usize, usize) {
    let (mut blended_passes, mut own_passes, mut run) = (0, 0, 0_usize);
    for kind in kinds {
        match kind {
            LayerKind::Blended => run += 1,
            LayerKind::OwnPass => {
                blended_passes += run.div_ceil(width);
                own_passes += 1;
                run = 0;
            }
        }
    }
    (blended_passes + run.div_ceil(width), own_passes)
}

/// A stack of layers composited into `final`, drawn run by run.
pub struct LayerStack<'a> {
    pipeline: &'a LayerFoldPipeline,
    /// The final texture, then the scratch texture.
    targets: [TargetTexture<'a>; 2],
    /// Which target the next pass writes.
    next: usize,
    /// Passes drawn so far.
    drawn: usize,
    /// Passes the stack was planned for.
    planned: usize,
    /// The next fold pass's params slot.
    slot: usize,
    /// What the first layer is drawn over.
    base: wgpu::Color,
    /// Layers folded per pass.
    width: usize,
}

impl<'a> LayerStack<'a> {
    /// A stack of the given layers ending in `final_texture`, using `scratch`
    /// for every other pass. The first layer is drawn over `base` and must be
    /// [`LayerKind::Blended`].
    pub fn new(
        pipeline: &'a LayerFoldPipeline,
        device: &wgpu::Device,
        final_texture: TargetTexture<'a>,
        scratch: TargetTexture<'a>,
        kinds: impl IntoIterator<Item = LayerKind>,
        base: wgpu::Color,
    ) -> Self {
        Self::with_width(
            pipeline,
            device,
            final_texture,
            scratch,
            kinds,
            base,
            FOLD_WIDTH,
        )
    }

    fn with_width(
        pipeline: &'a LayerFoldPipeline,
        device: &wgpu::Device,
        final_texture: TargetTexture<'a>,
        scratch: TargetTexture<'a>,
        kinds: impl IntoIterator<Item = LayerKind>,
        base: wgpu::Color,
        width: usize,
    ) -> Self {
        let (fold_passes, own_passes) = pass_count(kinds, width);
        pipeline.ensure_ring_slots(device, fold_passes);
        let planned = fold_passes + own_passes;
        Self {
            pipeline,
            targets: [final_texture, scratch],
            // Odd counts start in the final texture, even ones in scratch.
            next: 1 - planned % 2,
            drawn: 0,
            planned,
            slot: 0,
            base,
            width,
        }
    }

    /// The composite so far and the target, for a [`LayerKind::OwnPass`] layer,
    /// whose pipeline must write every pixel.
    ///
    /// # Panics
    ///
    /// Panics if no layer is below yet.
    pub fn pass_over(&mut self) -> (&'a wgpu::TextureView, &'a wgpu::TextureView) {
        let (below, target) = self.targets_for_next();
        self.advance();
        (
            below.expect("a layer with its own pass needs a layer below"),
            target,
        )
    }

    /// Blend a run of `layers` over the composite so far, in order. The first
    /// layer of the stack is drawn over the base.
    pub fn blend(
        &mut self,
        context: &GpuContext,
        layers: &[BlendLayer<'_>],
        out: &mut Vec<wgpu::CommandBuffer>,
    ) {
        if layers.is_empty() {
            return;
        }
        let pipeline = self.pipeline;
        let first_slot = self.slot;
        let mut params = pipeline.scratch_params.borrow_mut();
        params.clear();
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Layer Fold"),
            });
        for chunk in layers.chunks(self.width) {
            let (below, target) = self.targets_for_next();
            let chunk_params = FoldParams::new(chunk, below.is_some(), self.base);
            params.extend_from_slice(bytemuck::bytes_of(&chunk_params));
            params.resize((self.slot - first_slot + 1) * pipeline.ring_stride, 0);
            let bind_group = pipeline.create_bind_group(&context.device, chunk, below, self.slot);
            self.slot += 1;
            self.advance();

            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Layer Fold"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // The shader writes every pixel.
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&pipeline.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        context.queue.write_buffer(
            &pipeline.ring_buffer.borrow(),
            (first_slot * pipeline.ring_stride) as u64,
            &params,
        );
        out.push(encoder.finish());
    }

    /// The composite so far, if any, and the next pass's target.
    fn targets_for_next(&self) -> (Option<&'a wgpu::TextureView>, &'a wgpu::TextureView) {
        let below = (self.drawn > 0).then(|| self.targets[1 - self.next].1);
        let target = self.targets[self.next].1;
        (below, target)
    }

    fn advance(&mut self) {
        debug_assert!(
            self.drawn < self.planned,
            "a layer stack drew more passes than it planned, so its result is not in the final texture"
        );
        self.drawn += 1;
        self.next = 1 - self.next;
    }
}

#[cfg(test)]
mod tests {
    use super::{BlendLayer, FOLD_WIDTH, LayerFoldPipeline, LayerKind, LayerStack, pass_count};
    use crate::renderer::blit::BlitPipeline;
    use crate::renderer::context::GpuContext;
    use LayerKind::{Blended, OwnPass};

    const WIDTH: u32 = 8;
    const HEIGHT: u32 = 4;
    /// Every blend mode index, as `BlendMode::to_index` numbers them.
    const BLEND_MODES: std::ops::Range<u32> = 0..15;
    /// More layers than one pass holds, so chunks cross a pass boundary.
    const MAX_LAYERS: usize = FOLD_WIDTH + 4;
    /// Relative difference from one half-float write.
    const ONE_WRITE: f32 = 0.001;
    /// Relative difference from the half-float writes between layers, which a
    /// fold skips within a pass.
    const ROUNDING: f32 = 0.008;
    /// The same for Color Burn, which divides by the layer's color and so
    /// amplifies the rounding of the composite.
    const BURN_ROUNDING: f32 = 0.02;
    const COLOR_BURN: u32 = 9;

    /// A layer with the given alpha for each pixel, and varied color.
    fn layer_with_alpha(
        context: &GpuContext,
        seed: u32,
        alpha: impl Fn(u32) -> u8,
    ) -> wgpu::TextureView {
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
                    alpha(i),
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

    /// A layer whose color and alpha vary across it, so every blend mode sees
    /// a range of inputs.
    fn layer_texture(context: &GpuContext, seed: u32) -> wgpu::TextureView {
        layer_with_alpha(context, seed, |i| {
            (128 + ((i * 29 + seed * 71) * 5) % 128) as u8
        })
    }

    fn target(context: &GpuContext) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = context.create_compositing_texture(WIDTH, HEIGHT);
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// The channels of a half-float texture.
    fn read(context: &GpuContext, texture: &wgpu::Texture) -> Vec<f32> {
        assert_eq!(texture.format(), wgpu::TextureFormat::Rgba16Float);
        let row_bytes = WIDTH * 8;
        let row = row_bytes.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
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
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback");
        mapped
            .chunks_exact(row as usize)
            .flat_map(|line| line[..row_bytes as usize].as_chunks::<2>().0)
            .map(|bytes| half::f16::from_le_bytes(*bytes).to_f32())
            .collect()
    }

    /// Fails unless every channel matches within `tolerance`, relative above 1.
    fn assert_close(left: &[f32], right: &[f32], tolerance: f32, what: &str) {
        assert_eq!(left.len(), right.len());
        for (channel, (l, r)) in left.iter().zip(right).enumerate() {
            assert!(
                l == r || (l - r).abs() <= tolerance * l.abs().max(r.abs()).max(1.0),
                "{what}: channel {channel} is {l} and {r}"
            );
        }
    }

    fn composite(
        context: &GpuContext,
        pipeline: &LayerFoldPipeline,
        layers: &[BlendLayer<'_>],
        width: usize,
        base: wgpu::Color,
    ) -> Vec<f32> {
        let (final_texture, final_view) = target(context);
        let (scratch, scratch_view) = target(context);
        let mut commands = Vec::new();
        let mut stack = LayerStack::with_width(
            pipeline,
            &context.device,
            (&final_texture, &final_view),
            (&scratch, &scratch_view),
            std::iter::repeat_n(Blended, layers.len()),
            base,
            width,
        );
        stack.blend(context, layers, &mut commands);
        context.submit(commands);
        read(context, &final_texture)
    }

    #[test]
    fn passes_follow_the_fold_width_and_split_at_own_passes() {
        let blended = |count| std::iter::repeat_n(Blended, count);
        assert_eq!(pass_count(blended(0), 8), (0, 0));
        assert_eq!(pass_count(blended(1), 8), (1, 0));
        assert_eq!(pass_count(blended(8), 8), (1, 0));
        assert_eq!(pass_count(blended(9), 8), (2, 0));
        assert_eq!(pass_count(blended(17), 8), (3, 0));
        assert_eq!(pass_count(blended(5), 1), (5, 0));
        assert_eq!(pass_count([Blended, OwnPass], 8), (1, 1));
        assert_eq!(pass_count([Blended, Blended, OwnPass, Blended], 8), (2, 1));
        assert_eq!(
            pass_count([Blended, OwnPass, OwnPass, Blended, Blended], 8),
            (2, 2)
        );
    }

    /// The first layer over the base ignores its blend mode and matches a blit
    /// with hardware blending, including alpha 0 and premultiplied color there.
    #[test]
    fn first_layer_matches_the_blit_in_every_blend_mode() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let fold = LayerFoldPipeline::new(&context.device, context.compositing_format).unwrap();
        let view = layer_with_alpha(&context, 3, |i| (i % 4) as u8 * 2);
        let bases = [
            wgpu::Color::TRANSPARENT,
            wgpu::Color::BLACK,
            wgpu::Color {
                r: 0.5,
                g: 0.25,
                b: 2.0,
                a: 1.0,
            },
        ];
        for premultiplied in [false, true] {
            let blend = if premultiplied {
                wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING
            } else {
                wgpu::BlendState::ALPHA_BLENDING
            };
            let blit = BlitPipeline::with_blend(&context.device, context.compositing_format, blend)
                .unwrap();
            for base in bases {
                for opacity in [1.0, 0.4] {
                    let (texture, target_view) = target(&context);
                    let mut encoder = context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
                    blit.write_params_slot(
                        &context.queue,
                        0,
                        opacity,
                        [1.0, 1.0],
                        [0.0, 0.0],
                        premultiplied,
                    );
                    let bind_group = blit.create_ring_bind_group(&context.device, &view, 0);
                    {
                        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("expected first layer"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &target_view,
                                resolve_target: None,
                                ops: wgpu::Operations {
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
                        blit.render_at_slot(&mut pass, &bind_group);
                    }
                    context.submit(std::iter::once(encoder.finish()));
                    let expected = read(&context, &texture);

                    for blend_mode in BLEND_MODES {
                        let layer = BlendLayer {
                            view: &view,
                            opacity,
                            blend_mode,
                            premultiplied,
                        };
                        let drawn = composite(&context, &fold, &[layer], FOLD_WIDTH, base);
                        assert_close(
                            &drawn,
                            &expected,
                            ONE_WRITE,
                            &format!(
                                "mode {blend_mode}, premultiplied {premultiplied}, base {base:?}, \
                                 opacity {opacity}"
                            ),
                        );
                    }
                }
            }
        }
    }

    /// Folding gives the result of one pass per layer, at every blend mode, for
    /// odd and even layer counts, and across a pass boundary.
    #[test]
    fn folded_layers_match_one_pass_per_layer() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let fold = LayerFoldPipeline::new(&context.device, context.compositing_format).unwrap();
        let views: Vec<wgpu::TextureView> = (0..MAX_LAYERS as u32)
            .map(|i| layer_texture(&context, i))
            .collect();
        for count in 1..=MAX_LAYERS {
            for blend_mode in BLEND_MODES {
                for premultiplied in [false, true] {
                    let layers: Vec<BlendLayer<'_>> = views[..count]
                        .iter()
                        .enumerate()
                        .map(|(i, view)| BlendLayer {
                            view,
                            opacity: 1.0 - i as f32 * 0.05,
                            blend_mode,
                            premultiplied,
                        })
                        .collect();
                    let base = wgpu::Color::TRANSPARENT;
                    let folded = composite(&context, &fold, &layers, FOLD_WIDTH, base);
                    let one_by_one = composite(&context, &fold, &layers, 1, base);
                    let tolerance = if blend_mode == COLOR_BURN {
                        BURN_ROUNDING
                    } else {
                        ROUNDING
                    };
                    assert_close(
                        &folded,
                        &one_by_one,
                        tolerance,
                        &format!("{count} layers, mode {blend_mode}, {premultiplied}"),
                    );
                }
            }
        }
    }

    /// A layer with its own pass reads the fold below it, and folding resumes
    /// above it. The layer here copies the composite, so the stack matches the
    /// same stack without it, whichever texture the plan starts in.
    #[test]
    fn own_pass_layers_sit_between_folded_runs() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let fold = LayerFoldPipeline::new(&context.device, context.compositing_format).unwrap();
        let copy = BlitPipeline::new(&context.device, context.compositing_format).unwrap();
        let views: Vec<wgpu::TextureView> = (0..MAX_LAYERS as u32)
            .map(|i| layer_texture(&context, i))
            .collect();
        let layers: Vec<BlendLayer<'_>> = views
            .iter()
            .map(|view| BlendLayer {
                view,
                opacity: 0.8,
                blend_mode: 3,
                premultiplied: false,
            })
            .collect();
        let base = wgpu::Color::TRANSPARENT;

        for (below, above) in [
            (1, 1),
            (2, 1),
            (3, 2),
            (FOLD_WIDTH + 1, 2),
            (5, MAX_LAYERS - 5),
        ] {
            let runs = [&layers[..below], &layers[below..below + above]];
            let (final_texture, final_view) = target(&context);
            let (scratch, scratch_view) = target(&context);
            let mut commands = Vec::new();
            let kinds = std::iter::repeat_n(Blended, below)
                .chain([OwnPass])
                .chain(std::iter::repeat_n(Blended, above));
            let mut stack = LayerStack::new(
                &fold,
                &context.device,
                (&final_texture, &final_view),
                (&scratch, &scratch_view),
                kinds,
                base,
            );
            stack.blend(&context, runs[0], &mut commands);
            let (composite_below, own_target) = stack.pass_over();
            let bind_group = copy.create_bind_group(&context.device, composite_below);
            let mut encoder = context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
            {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("own pass layer"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: own_target,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                copy.render(&mut pass, &bind_group);
            }
            commands.push(encoder.finish());
            stack.blend(&context, runs[1], &mut commands);
            context.submit(commands);

            let without = composite(&context, &fold, &layers[..below + above], FOLD_WIDTH, base);
            assert_close(
                &read(&context, &final_texture),
                &without,
                ROUNDING,
                &format!("{below} below, {above} above"),
            );
        }
    }
}
