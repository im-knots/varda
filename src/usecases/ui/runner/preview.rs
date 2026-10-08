//! Preview texture pipeline: gamma-encodes engine textures into egui-ready
//! targets and keeps egui's registrations in step with the live deck, channel
//! and output set.

use super::UIRunner;

/// Which preview a `PreviewEncoder` target belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) enum PreviewSlot {
    Deck(String),
    Channel(usize),
    Main,
    Output(usize),
}

impl PreviewSlot {
    pub(crate) fn key(&self) -> String {
        match self {
            PreviewSlot::Deck(uuid) => format!("deck:{uuid}"),
            PreviewSlot::Channel(idx) => format!("ch:{idx}"),
            PreviewSlot::Main => "main".to_string(),
            PreviewSlot::Output(idx) => format!("out:{idx}"),
        }
    }
}

/// Texture IDs of the shapes visible inside their clip rects this frame.
///
/// A preview in a collapsed panel or a view not shown has no shape, and one
/// scrolled out of its scroll area falls outside its clip.
pub(crate) fn textures_drawn(
    shapes: &[egui::epaint::ClippedShape],
) -> std::collections::HashSet<egui::TextureId> {
    fn collect(
        shape: &egui::epaint::Shape,
        clip: egui::Rect,
        drawn: &mut std::collections::HashSet<egui::TextureId>,
    ) {
        if let egui::epaint::Shape::Vec(shapes) = shape {
            for shape in shapes {
                collect(shape, clip, drawn);
            }
            return;
        }
        let id = shape.texture_id();
        if id != egui::TextureId::default() && shape.visual_bounding_rect().intersects(clip) {
            drawn.insert(id);
        }
    }
    let mut drawn = std::collections::HashSet::new();
    for clipped in shapes {
        collect(&clipped.shape, clipped.clip_rect, &mut drawn);
    }
    drawn
}

/// Whether `slot`'s registered texture was drawn this frame.
pub(crate) fn is_drawn(
    slot: &PreviewSlot,
    texture_id: impl Fn(&PreviewSlot) -> Option<egui::TextureId>,
    drawn: &std::collections::HashSet<egui::TextureId>,
) -> bool {
    texture_id(slot).is_some_and(|id| drawn.contains(&id))
}

/// Gamma-encodes linear engine textures so egui previews match the output.
///
/// egui expects gamma-encoded, non-sRGB-aware textures and decodes them before
/// writing to its sRGB framebuffer, so a linear texture displays too dark
/// (linear 0.2 shows as 0.2 instead of 0.48).
///
/// Each preview source is blitted through a linear→sRGB encode into a plain
/// `Rgba8Unorm` texture, which is registered with egui. The target must not be
/// `*UnormSrgb`, or sampling would decode it and cancel the encode. Linear float
/// sources are sampled as-is and sRGB output previews are decoded to linear on
/// sample, so the shader always receives linear.
///
/// Targets are cached per key, recreated only when the source size changes,
/// and capped to `MAX_DIM` on the long edge.
pub(crate) struct PreviewEncoder {
    pipeline: crate::renderer::BlitPipeline,
    targets: std::collections::HashMap<String, (wgpu::Texture, wgpu::TextureView)>,
}

impl PreviewEncoder {
    /// Long-edge cap for preview targets.
    const MAX_DIM: u32 = 960;

    pub(crate) fn new(device: &wgpu::Device) -> anyhow::Result<Self> {
        Ok(Self {
            // Non-sRGB target; see the type-level comment.
            pipeline: crate::renderer::BlitPipeline::new(device, wgpu::TextureFormat::Rgba8Unorm)?,
            targets: std::collections::HashMap::new(),
        })
    }

    /// Scale `(w, h)` down so the long edge is at most `MAX_DIM`, preserving aspect.
    fn preview_size(w: u32, h: u32) -> (u32, u32) {
        let long = w.max(h);
        if long <= Self::MAX_DIM || long == 0 {
            return (w.max(1), h.max(1));
        }
        let scale = Self::MAX_DIM as f32 / long as f32;
        (
            ((w as f32 * scale).round() as u32).max(1),
            ((h as f32 * scale).round() as u32).max(1),
        )
    }

    /// Create or resize the target for `key`. Returns true when it was
    /// (re)created, in which case the caller must re-register it with egui: a
    /// `TextureId` is bound to one texture.
    pub(crate) fn ensure_target(
        &mut self,
        context: &crate::renderer::GpuContext,
        key: &str,
        src_w: u32,
        src_h: u32,
    ) -> bool {
        let (w, h) = Self::preview_size(src_w, src_h);
        let stale = self
            .targets
            .get(key)
            .is_none_or(|(t, _)| t.width() != w || t.height() != h);
        if stale {
            let texture = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Preview Encode Target"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.targets.insert(key.to_string(), (texture, view));
        }
        stale
    }

    fn target_view(&self, key: &str) -> Option<&wgpu::TextureView> {
        self.targets.get(key).map(|(_, v)| v)
    }

    /// Encode all `sources` into their cached targets in one command buffer.
    ///
    /// Runs every frame after the engine renders and after output windows draw
    /// (window previews read their intermediate texture). `TextureId`s are
    /// registered separately, earlier in the frame, before the UI is built.
    ///
    /// One submit for all previews avoids a dozen submits per frame on weak GPUs.
    /// All previews share the same blit params, written once up front.
    pub(crate) fn encode_all(
        &self,
        context: &crate::renderer::GpuContext,
        sources: &[(PreviewSlot, &wgpu::TextureView, u32, u32)],
    ) {
        if sources.is_empty() {
            return;
        }
        self.pipeline.set_srgb_encode(&context.queue, true);
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Preview Encode"),
            });
        for (slot, src, ..) in sources {
            let key = slot.key();
            let Some(target_view) = self.target_view(&key) else {
                continue;
            };
            let bind_group = self.pipeline.create_bind_group(&context.device, src);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Preview Encode Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
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
            self.pipeline.render(&mut pass, &bind_group);
        }
        context.submit(std::iter::once(encoder.finish()));
    }

    /// Drop cached targets whose key is no longer live.
    fn retain_keys(&mut self, live: &std::collections::HashSet<String>) {
        self.targets.retain(|k, _| live.contains(k));
    }
}

impl UIRunner {
    /// Register deck, channel, output and main output preview textures with egui.
    pub(super) fn register_preview_textures(&mut self) {
        self.sync_preview_registrations();

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

    /// An output preview's source view and its dimensions, so the encoder can size
    /// the target to the right aspect.
    ///
    /// Windowed outputs preview their intermediate texture (surface geometry and
    /// warp, at the window's size); headless sources are render-resolution
    /// composites, decks, or sub-mixes.
    fn output_preview_source<'a>(
        output: &'a crate::output::Output,
        mixer: &'a crate::mixer::Mixer,
    ) -> (&'a wgpu::TextureView, u32, u32) {
        let view = Self::output_preview_view(output, mixer);
        let (w, h) = if output.preview_view().is_some() {
            output.preview_size()
        } else {
            let ct = mixer.composite_texture();
            (ct.width(), ct.height())
        };
        (view, w, h)
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

    /// Every live preview source: its slot, source view, and source size.
    ///
    /// Shared by registration and encoding so the two stay in step.
    fn preview_sources(
        varda: &crate::app::VardaApp,
    ) -> Vec<(PreviewSlot, &wgpu::TextureView, u32, u32)> {
        let mixer = varda.mixer_ref();
        let mut out: Vec<(PreviewSlot, &wgpu::TextureView, u32, u32)> = Vec::new();
        for (ch_idx, ch) in mixer.channels().iter().enumerate() {
            for slot in &ch.decks {
                out.push((
                    PreviewSlot::Deck(slot.deck.uuid().to_string()),
                    &slot.deck.texture_view,
                    slot.deck.texture.width(),
                    slot.deck.texture.height(),
                ));
            }
            out.push((
                PreviewSlot::Channel(ch_idx),
                &ch.composite_view,
                ch.composite_texture.width(),
                ch.composite_texture.height(),
            ));
        }
        let ct = mixer.composite_texture();
        out.push((
            PreviewSlot::Main,
            mixer.program_view(crate::mixer::ProgramKey::sdr(mixer.tonemap_mode())),
            ct.width(),
            ct.height(),
        ));
        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
            let (view, w, h) = Self::output_preview_source(output, mixer);
            out.push((PreviewSlot::Output(out_idx), view, w, h));
        }
        out
    }

    /// Create or resize preview targets and keep egui registrations in sync.
    ///
    /// Runs early in the frame because the UI needs `TextureId`s before any GPU
    /// work is submitted. `encode_previews` fills in the pixels later.
    fn sync_preview_registrations(&mut self) {
        let Some(varda) = &self.varda else { return };
        let context = varda.gpu_context();

        if self.preview_encoder.is_none() {
            match PreviewEncoder::new(&context.device) {
                Ok(e) => self.preview_encoder = Some(e),
                Err(e) => {
                    log::error!("Failed to create preview encoder: {e}");
                    return;
                }
            }
        }
        let Some(egui_renderer) = self.egui_renderer.as_mut() else {
            return;
        };
        let Some(encoder) = self.preview_encoder.as_mut() else {
            return;
        };

        let sources = Self::preview_sources(varda);
        let mut live: std::collections::HashSet<String> =
            std::collections::HashSet::with_capacity(sources.len());

        for (slot, _src, w, h) in &sources {
            let key = slot.key();
            live.insert(key.clone());
            let recreated = encoder.ensure_target(context, &key, *w, *h);
            let known = match slot {
                PreviewSlot::Deck(uuid) => self.deck_preview_textures.contains_key(uuid),
                PreviewSlot::Channel(idx) => self.channel_preview_textures.contains_key(idx),
                PreviewSlot::Main => self.main_output_texture.is_some(),
                PreviewSlot::Output(idx) => self.output_preview_textures.contains_key(idx),
            };
            if !recreated && known {
                continue;
            }
            let Some(view) = encoder.target_view(&key) else {
                continue;
            };
            let tid = egui_renderer.register_native_texture(
                &context.device,
                view,
                wgpu::FilterMode::Linear,
            );
            // Retire the slot's previous registration; a resized target leaves the old
            // TextureId dangling.
            let stale = match slot {
                PreviewSlot::Deck(uuid) => self.deck_preview_textures.insert(uuid.clone(), tid),
                PreviewSlot::Channel(idx) => self.channel_preview_textures.insert(*idx, tid),
                PreviewSlot::Main => self.main_output_texture.replace(tid),
                PreviewSlot::Output(idx) => self.output_preview_textures.insert(*idx, tid),
            };
            if let Some(old) = stale {
                egui_renderer.free_texture(&old);
            }
        }

        // Retire registrations and targets for removed decks and outputs. This is the
        // only place they are retired; skipping it leaks a texture per removed entity.
        let live_decks: std::collections::HashSet<String> = sources
            .iter()
            .filter_map(|(s, ..)| match s {
                PreviewSlot::Deck(u) => Some(u.clone()),
                _ => None,
            })
            .collect();
        let stale_decks: Vec<String> = self
            .deck_preview_textures
            .keys()
            .filter(|u| !live_decks.contains(*u))
            .cloned()
            .collect();
        for uuid in stale_decks {
            if let Some(tid) = self.deck_preview_textures.remove(&uuid) {
                egui_renderer.free_texture(&tid);
            }
        }
        let live_outputs: std::collections::HashSet<usize> = sources
            .iter()
            .filter_map(|(s, ..)| match s {
                PreviewSlot::Output(i) => Some(*i),
                _ => None,
            })
            .collect();
        let stale_outputs: Vec<usize> = self
            .output_preview_textures
            .keys()
            .copied()
            .filter(|i| !live_outputs.contains(i))
            .collect();
        for idx in stale_outputs {
            if let Some(tid) = self.output_preview_textures.remove(&idx) {
                egui_renderer.free_texture(&tid);
            }
        }
        encoder.retain_keys(&live);
    }

    /// Gamma-encode the previews this frame's UI drew. Others keep their last
    /// pixels until they are drawn again.
    ///
    /// Runs after the mixer render and output windows draw, before egui paints.
    /// See the frame sequence in `render_frame`.
    pub(super) fn encode_previews(&mut self, drawn: &std::collections::HashSet<egui::TextureId>) {
        let Some(varda) = &self.varda else { return };
        let Some(encoder) = &self.preview_encoder else {
            return;
        };
        let context = varda.gpu_context();
        let mut sources = Self::preview_sources(varda);
        sources.retain(|(slot, ..)| is_drawn(slot, |s| self.preview_texture_id(s), drawn));
        encoder.encode_all(context, &sources);
    }

    /// The egui texture registered for `slot`, if any.
    fn preview_texture_id(&self, slot: &PreviewSlot) -> Option<egui::TextureId> {
        match slot {
            PreviewSlot::Deck(uuid) => self.deck_preview_textures.get(uuid).copied(),
            PreviewSlot::Channel(idx) => self.channel_preview_textures.get(idx).copied(),
            PreviewSlot::Main => self.main_output_texture,
            PreviewSlot::Output(idx) => self.output_preview_textures.get(idx).copied(),
        }
    }

    /// Per-frame egui texture sync: keeps targets and registrations in step.
    /// `encode_previews` writes the pixels later.
    pub(super) fn refresh_textures(&mut self) {
        self.sync_preview_registrations();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::{ClippedShape, RectShape, Shape};
    use egui::{Color32, Rect, TextureId, pos2, vec2};

    fn image(id: u64, at: Rect) -> Shape {
        let mut rect = RectShape::filled(at, 0.0, Color32::WHITE);
        rect = rect.with_texture(
            TextureId::User(id),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
        );
        Shape::Rect(rect)
    }

    fn clipped(clip: Rect, shape: Shape) -> ClippedShape {
        ClippedShape {
            clip_rect: clip,
            shape,
        }
    }

    /// Only previews visible inside their clip count; scrolled-away ones and
    /// untextured shapes do not.
    #[test]
    fn drawn_textures_are_the_visible_textured_shapes() {
        let screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0));
        let panel = Rect::from_min_size(pos2(0.0, 0.0), vec2(200.0, 200.0));
        let shapes = [
            clipped(
                screen,
                image(1, Rect::from_min_size(pos2(10.0, 10.0), vec2(96.0, 54.0))),
            ),
            clipped(
                panel,
                image(2, Rect::from_min_size(pos2(10.0, 400.0), vec2(96.0, 54.0))),
            ),
            clipped(
                screen,
                Shape::Vec(vec![
                    Shape::Noop,
                    image(3, Rect::from_min_size(pos2(300.0, 10.0), vec2(96.0, 54.0))),
                ]),
            ),
            clipped(screen, Shape::rect_filled(screen, 0.0, Color32::BLACK)),
        ];
        let drawn = textures_drawn(&shapes);
        assert_eq!(
            drawn,
            [TextureId::User(1), TextureId::User(3)]
                .into_iter()
                .collect()
        );
    }

    /// A slot is encoded only when its registered texture was drawn.
    #[test]
    fn only_drawn_slots_are_encoded() {
        let slots = [
            PreviewSlot::Deck("a".to_string()),
            PreviewSlot::Deck("b".to_string()),
            PreviewSlot::Channel(0),
            PreviewSlot::Main,
        ];
        let id = |slot: &PreviewSlot| match slot {
            PreviewSlot::Deck(uuid) if uuid == "a" => Some(TextureId::User(1)),
            PreviewSlot::Deck(_) => Some(TextureId::User(2)),
            PreviewSlot::Channel(_) => None,
            _ => Some(TextureId::User(4)),
        };
        let drawn = [TextureId::User(2), TextureId::User(4)]
            .into_iter()
            .collect();
        let encoded: Vec<&PreviewSlot> = slots.iter().filter(|s| is_drawn(s, id, &drawn)).collect();
        assert_eq!(encoded, [&slots[1], &slots[3]]);
    }

    /// Targets are cached by this key, so a collision would show one preview's
    /// pixels in another's panel.
    #[test]
    fn preview_slot_keys_are_distinct_across_variants() {
        let keys = [
            PreviewSlot::Deck("0".to_string()).key(),
            PreviewSlot::Channel(0).key(),
            PreviewSlot::Main.key(),
            PreviewSlot::Output(0).key(),
        ];
        let unique: std::collections::HashSet<&String> = keys.iter().collect();
        assert_eq!(unique.len(), keys.len(), "keys collide: {keys:?}");
    }

    #[test]
    fn preview_slot_keys_are_stable_and_namespaced() {
        assert_eq!(PreviewSlot::Deck("a1b2".to_string()).key(), "deck:a1b2");
        assert_eq!(PreviewSlot::Channel(3).key(), "ch:3");
        assert_eq!(PreviewSlot::Main.key(), "main");
        assert_eq!(PreviewSlot::Output(2).key(), "out:2");
    }

    #[test]
    fn preview_slot_keys_differ_per_index_and_uuid() {
        assert_ne!(PreviewSlot::Channel(1).key(), PreviewSlot::Channel(2).key());
        assert_ne!(PreviewSlot::Output(1).key(), PreviewSlot::Output(2).key());
        assert_ne!(
            PreviewSlot::Deck("a".to_string()).key(),
            PreviewSlot::Deck("b".to_string()).key()
        );
    }
}
