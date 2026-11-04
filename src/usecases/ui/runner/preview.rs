//! Previews draw inside egui's render pass, straight from their source
//! textures, through a paint callback. Panels name a [`PreviewSlot`], and the
//! runner resolves slots to sources after the frame's textures are drawn.

use super::UIRunner;
use crate::renderer::preview::{PreviewCorners, PreviewPipeline};
use std::collections::HashMap;

/// Radius of [`PreviewCorners::Rounded`], in points.
const ROUNDED_CORNER_POINTS: f32 = 4.0;

/// A preview a panel can draw.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PreviewSlot {
    Deck(String),
    Channel(usize),
    Main,
    Output(usize),
}

/// Draw `slot`'s preview into `rect`. Offscreen previews cost nothing: egui
/// skips callbacks with no visible area.
pub(crate) fn paint(ui: &egui::Ui, rect: egui::Rect, slot: PreviewSlot, corners: PreviewCorners) {
    ui.painter().add(egui_wgpu::Callback::new_paint_callback(
        rect,
        PreviewCallback { slot, corners },
    ));
}

/// Lay out a preview of `size`, as `ui.image` lays out an image.
pub(crate) fn image(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    slot: PreviewSlot,
    corners: PreviewCorners,
    sense: egui::Sense,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(size, sense);
    paint(ui, rect, slot, corners);
    response
}

struct PreviewCallback {
    slot: PreviewSlot,
    corners: PreviewCorners,
}

impl egui_wgpu::CallbackTrait for PreviewCallback {
    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        if let Some(previews) = resources.get::<PreviewResources>() {
            previews.draw(pass, &self.slot, self.corners);
        }
    }
}

struct PreviewSource {
    view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    /// The last [`PreviewResources::sync`] that listed this source.
    seen: u64,
}

/// The preview pipeline and a bind group per live slot, kept in egui's
/// `callback_resources`.
pub(crate) struct PreviewResources {
    pipeline: PreviewPipeline,
    sources: HashMap<PreviewSlot, PreviewSource>,
    syncs: u64,
}

impl PreviewResources {
    /// Resources for a UI render pass on `target_format`.
    pub(crate) fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            pipeline: PreviewPipeline::new(device, target_format)?,
            sources: HashMap::new(),
            syncs: 0,
        })
    }

    /// Make the live slots current: rebuild the bind groups of sources whose
    /// view changed and drop slots that are gone.
    pub(crate) fn sync<'a>(
        &mut self,
        device: &wgpu::Device,
        sources: impl IntoIterator<Item = (PreviewSlot, &'a wgpu::TextureView)>,
    ) {
        self.syncs += 1;
        for (slot, view) in sources {
            if let Some(source) = self.sources.get_mut(&slot)
                && source.view == *view
            {
                source.seen = self.syncs;
                continue;
            }
            let bind_group = self.pipeline.create_bind_group(device, view);
            self.sources.insert(
                slot,
                PreviewSource {
                    view: view.clone(),
                    bind_group,
                    seen: self.syncs,
                },
            );
        }
        let syncs = self.syncs;
        self.sources.retain(|_, source| source.seen == syncs);
    }

    pub(crate) fn set_pixels_per_point(&mut self, queue: &wgpu::Queue, pixels_per_point: f32) {
        self.pipeline
            .set_corner_radius(queue, ROUNDED_CORNER_POINTS * pixels_per_point);
    }

    fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'static>,
        slot: &PreviewSlot,
        corners: PreviewCorners,
    ) {
        if let Some(source) = self.sources.get(slot) {
            self.pipeline.draw(pass, &source.bind_group, corners);
        }
    }
}

impl UIRunner {
    /// Create the dome preview's render target and register it with egui.
    pub(super) fn register_dome_preview(&mut self) {
        let Some(varda) = &self.varda else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else {
            return;
        };
        let context = varda.gpu_context();

        if self.dome_preview_renderer.is_none() {
            match crate::renderer::dome_preview::DomePreviewRenderer::new(
                &context.device,
                wgpu::TextureFormat::Bgra8UnormSrgb,
            ) {
                Ok(renderer) => {
                    let tid = egui_renderer.register_native_texture(
                        &context.device,
                        &renderer.output_view,
                        wgpu::FilterMode::Linear,
                    );
                    self.dome_preview_texture = Some(tid);
                    self.dome_preview_renderer = Some(renderer);
                }
                Err(e) => log::error!("Failed to create dome preview renderer: {e}"),
            }
        }
    }

    /// Texture view for an output preview: the output's composed picture (surface
    /// geometry and warp) when its last frame produced one, otherwise its program.
    pub(super) fn output_preview_view<'a>(
        output: &'a crate::output::Output,
        mixer: &'a crate::mixer::Mixer,
    ) -> &'a wgpu::TextureView {
        output.preview_view().unwrap_or_else(|| {
            mixer.program_view(crate::mixer::ProgramKey::sdr(mixer.tonemap_mode()))
        })
    }

    /// Every live preview's slot and source view.
    fn preview_sources(varda: &crate::app::VardaApp) -> Vec<(PreviewSlot, &wgpu::TextureView)> {
        let mixer = varda.mixer_ref();
        let mut sources = Vec::new();
        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
            for slot in &ch.decks {
                sources.push((
                    PreviewSlot::Deck(slot.deck.uuid().to_string()),
                    &slot.deck.texture_view,
                ));
            }
            sources.push((PreviewSlot::Channel(ch_idx), &ch.composite_view));
        }
        sources.push((
            PreviewSlot::Main,
            mixer.program_view(crate::mixer::ProgramKey::sdr(mixer.tonemap_mode())),
        ));
        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
            sources.push((
                PreviewSlot::Output(out_idx),
                Self::output_preview_view(output, mixer),
            ));
        }
        sources
    }

    /// Point previews at this frame's sources.
    ///
    /// Runs after the mixer render and output windows draw, since window
    /// previews read their intermediate texture, and before egui paints.
    pub(super) fn sync_previews(&mut self) {
        let Some(varda) = &self.varda else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else {
            return;
        };
        let Some(previews) = egui_renderer
            .callback_resources
            .get_mut::<PreviewResources>()
        else {
            return;
        };
        let context = varda.gpu_context();
        previews.sync(&context.device, Self::preview_sources(varda));
        previews.set_pixels_per_point(&context.queue, self.egui_ctx.pixels_per_point());
    }
}

#[cfg(test)]
mod tests {
    use super::{PreviewResources, PreviewSlot, paint};
    use crate::renderer::context::GpuContext;
    use crate::renderer::preview::PreviewCorners;

    const SIZE: u32 = 64;
    const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

    fn white_source(context: &GpuContext) -> wgpu::TextureView {
        let texture = context.create_compositing_texture(SIZE, SIZE);
        let one = half::f16::from_f32(1.0).to_le_bytes();
        let data: Vec<u8> = (0..SIZE * SIZE * 4).flat_map(|_| one).collect();
        context.queue.write_texture(
            texture.as_image_copy(),
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 8),
                rows_per_image: Some(SIZE),
            },
            texture.size(),
        );
        texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    /// Paint one frame with `draw_ui` through a real egui renderer and return
    /// the red channel of each pixel.
    fn paint_frame(
        context: &GpuContext,
        sources: &[(PreviewSlot, &wgpu::TextureView)],
        draw_ui: impl FnMut(&mut egui::Ui),
    ) -> Vec<u8> {
        let mut renderer = egui_wgpu::Renderer::new(
            &context.device,
            FORMAT,
            egui_wgpu::RendererOptions::default(),
        );
        let mut previews = PreviewResources::new(&context.device, FORMAT).unwrap();
        previews.sync(
            &context.device,
            sources.iter().map(|(slot, view)| (slot.clone(), *view)),
        );
        renderer.callback_resources.insert(previews);

        let egui_ctx = egui::Context::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(SIZE as f32, SIZE as f32),
            )),
            ..Default::default()
        };
        let mut draw_ui = draw_ui;
        let mut output = egui_ctx.run_ui(input, |ui| draw_ui(ui));
        for (id, deltas) in &output.textures_delta.set {
            for delta in deltas {
                renderer.update_texture(&context.device, &context.queue, *id, delta);
            }
        }
        output.textures_delta.clear();
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [SIZE, SIZE],
            pixels_per_point: output.pixels_per_point,
        };
        let jobs = egui_ctx.tessellate(output.shapes, output.pixels_per_point);
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview paint target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        let mut commands = renderer.update_buffers(
            &context.device,
            &context.queue,
            &mut encoder,
            &jobs,
            &screen,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview paint"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            renderer.render(&mut pass.forget_lifetime(), &jobs, &screen);
        }
        let row = (SIZE * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("preview paint readback"),
            size: u64::from(row * SIZE),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: Some(SIZE),
                },
            },
            texture.size(),
        );
        commands.push(encoder.finish());
        context.submit(commands);
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback");
        (0..SIZE)
            .flat_map(|y| (0..SIZE).map(move |x| (y, x)))
            .map(|(y, x)| mapped[(y * row + x * 4) as usize])
            .collect()
    }

    fn at(pixels: &[u8], x: u32, y: u32) -> u8 {
        pixels[(y * SIZE + x) as usize]
    }

    #[test]
    fn a_painted_preview_fills_its_rect_inside_its_clip() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let view = white_source(&context);
        let pixels = paint_frame(&context, &[(PreviewSlot::Main, &view)], |ui| {
            let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(32.0, 32.0));
            let clip = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(24.0, 64.0));
            let mut clipped = ui.new_child(egui::UiBuilder::new());
            clipped.set_clip_rect(clip);
            paint(&clipped, rect, PreviewSlot::Main, PreviewCorners::Square);
        });
        assert_eq!(at(&pixels, 12, 12), 255, "inside the rect and clip");
        assert_eq!(at(&pixels, 4, 12), 0, "left of the rect");
        assert_eq!(at(&pixels, 12, 44), 0, "below the rect");
        assert_eq!(at(&pixels, 30, 12), 0, "inside the rect, outside the clip");
    }

    #[test]
    fn a_slot_with_no_source_paints_nothing() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let pixels = paint_frame(&context, &[], |ui| {
            let rect = egui::Rect::from_min_size(egui::pos2(8.0, 8.0), egui::vec2(32.0, 32.0));
            paint(ui, rect, PreviewSlot::Main, PreviewCorners::Square);
        });
        assert_eq!(at(&pixels, 12, 12), 0);
    }

    #[test]
    fn bind_groups_follow_their_source_views() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let first = white_source(&context);
        let second = white_source(&context);
        let deck = PreviewSlot::Deck("a".to_string());
        let mut previews = PreviewResources::new(&context.device, FORMAT).unwrap();

        previews.sync(
            &context.device,
            [(deck.clone(), &first), (PreviewSlot::Main, &second)],
        );
        let bind_group = previews.sources[&deck].bind_group.clone();

        previews.sync(
            &context.device,
            [(deck.clone(), &first), (PreviewSlot::Main, &second)],
        );
        assert!(
            previews.sources[&deck].bind_group == bind_group,
            "unchanged view"
        );

        previews.sync(&context.device, [(deck.clone(), &second)]);
        assert!(
            previews.sources[&deck].bind_group != bind_group,
            "changed view"
        );
        assert!(
            !previews.sources.contains_key(&PreviewSlot::Main),
            "gone slot"
        );
    }
}
