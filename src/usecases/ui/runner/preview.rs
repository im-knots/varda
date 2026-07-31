//! Preview texture pipeline: gamma-encoding engine textures into egui-ready
//! targets, and keeping egui's registrations in step with the live deck, channel
//! and output set.
//!
//! Split out of the runner because it is a self-contained concern: given the
//! engine's current textures, produce the `TextureId`s the UI needs this frame.

use super::UIRunner;

/// Which preview a `PreviewEncoder` target belongs to.
#[derive(Clone, PartialEq, Eq, Hash)]
pub(super) enum PreviewSlot {
    Deck(String),
    Channel(usize),
    Main,
    Output(usize),
}

impl PreviewSlot {
    pub(super) fn key(&self) -> String {
        match self {
            PreviewSlot::Deck(uuid) => format!("deck:{uuid}"),
            PreviewSlot::Channel(idx) => format!("ch:{idx}"),
            PreviewSlot::Main => "main".to_string(),
            PreviewSlot::Output(idx) => format!("out:{idx}"),
        }
    }
}

/// Gamma-encodes linear engine textures so egui previews match the real output.
///
/// egui assumes every texture handed to it is already gamma-encoded ("We expect
/// 'normal' textures that are NOT sRGB-aware" — egui-wgpu's `egui.wgsl`), and
/// compensates by applying the inverse transfer function before writing to its
/// sRGB framebuffer. Handing it a linear texture therefore round-trips to raw
/// linear light on screen, which reads as far too dark: linear 0.2 displays as
/// 0.2 where the output window correctly shows 0.48.
///
/// Fix: blit each preview source through an explicit linear→sRGB encode into a
/// plain (non-sRGB) `Rgba8Unorm` texture, and register *that* with egui. The
/// target must not be `*UnormSrgb` or the hardware would decode on sample and
/// cancel the encode out.
///
/// One mechanism covers both kinds of source: linear float color-path textures
/// are sampled as-is, and already-sRGB output previews are hardware-decoded to
/// linear on sample. Either way the shader receives linear and emits gamma.
///
/// Targets are cached per key and recreated only when the source size changes.
/// They are capped to `MAX_DIM` on the long edge — previews are thumbnails, so
/// this also makes egui sample far less than the full render resolution.
pub(super) struct PreviewEncoder {
    pipeline: crate::renderer::BlitPipeline,
    targets: std::collections::HashMap<String, (wgpu::Texture, wgpu::TextureView)>,
}

impl PreviewEncoder {
    /// Long-edge cap for preview targets.
    const MAX_DIM: u32 = 960;

    fn new(device: &wgpu::Device) -> anyhow::Result<Self> {
        Ok(Self {
            // Non-sRGB target on purpose — see the type-level comment.
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

    /// Create or resize the target for `key`. Returns true when the target was
    /// (re)created, meaning the caller must re-register it with egui — a
    /// `TextureId` is bound to a specific texture, so a resize invalidates it.
    fn ensure_target(
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

    /// Encode all `sources` into their cached targets in a single command buffer.
    ///
    /// Must run every frame *after* the engine has rendered, and after output
    /// windows have drawn (window previews source their intermediate texture).
    /// Registration is separate: `TextureId`s must already exist when the UI is
    /// built earlier in the frame.
    ///
    /// One encoder and one submit for every preview — a submit per thumbnail
    /// would add a dozen per frame, which is exactly the cost the compositing
    /// path batches away for weaker GPUs. All previews share the same blit
    /// params, so those are written once up front.
    fn encode_all(
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
        context.queue.submit(std::iter::once(encoder.finish()));
    }

    /// Drop cached targets whose key is no longer live (removed decks/outputs).
    fn retain_keys(&mut self, live: &std::collections::HashSet<String>) {
        self.targets.retain(|k, _| live.contains(k));
    }
}

impl UIRunner {
    /// Register GPU textures with egui for deck/channel/output previews and main output.
    pub(super) fn register_preview_textures(&mut self) {
        self.sync_preview_registrations();

        let Some(varda) = &self.varda else { return };
        let Some(egui_renderer) = &mut self.egui_renderer else {
            return;
        };
        let context = varda.gpu_context();

        // Dome preview renderer + texture
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

    /// Resolve an output preview's source view together with its dimensions, so
    /// the preview encoder can size its target to the right aspect ratio.
    ///
    /// Windowed outputs preview their own intermediate texture (which carries
    /// surface geometry and warp, and is the window's size); every headless
    /// source is a render-resolution composite, deck, or sub-mix.
    fn output_preview_source<'a>(
        output: &'a crate::renderer::context::UnifiedOutput,
        mixer: &'a crate::mixer::Mixer,
    ) -> (&'a wgpu::TextureView, u32, u32) {
        use crate::renderer::context::UnifiedOutput;
        let view = Self::output_preview_view(output, mixer);
        let (w, h) = match output {
            UnifiedOutput::Window(w) => (w.preview_texture.width(), w.preview_texture.height()),
            UnifiedOutput::Headless(_) => {
                let ct = mixer.composite_texture();
                (ct.width(), ct.height())
            }
        };
        (view, w, h)
    }

    /// Resolve the texture view to use for an output preview.
    /// Windowed outputs use their intermediate render texture (shows surface geometry + warp).
    /// Headless outputs resolve their source.
    pub(super) fn output_preview_view<'a>(
        output: &'a crate::renderer::context::UnifiedOutput,
        mixer: &'a crate::mixer::Mixer,
    ) -> &'a wgpu::TextureView {
        use crate::renderer::context::{OutputSource, UnifiedOutput};
        match output {
            UnifiedOutput::Window(w) => &w.preview_texture_view,
            UnifiedOutput::Headless(h) => match &h.source {
                OutputSource::Master => mixer.composite_view(),
                OutputSource::Channel(idx) => mixer
                    .channels()
                    .get(*idx)
                    .map_or_else(|| mixer.composite_view(), |c| &c.composite_view),
                OutputSource::Deck(ch, dk) => mixer
                    .channels()
                    .get(*ch)
                    .and_then(|c| c.decks.get(*dk))
                    .map_or_else(|| mixer.composite_view(), |s| &s.deck.texture_view),
                OutputSource::Channels(indices) => {
                    let mut sorted = indices.clone();
                    sorted.sort_unstable();
                    sorted.dedup();
                    mixer
                        .get_sub_mix_view(&sorted)
                        .unwrap_or_else(|| mixer.composite_view())
                }
                OutputSource::Domemaster => {
                    // Domemaster preview falls back to composite view;
                    // the actual domemaster texture is rendered in the output pipeline.
                    mixer.composite_view()
                }
            },
        }
    }

    /// Re-register GPU textures when deck/channel/output layout changes.
    /// Enumerate every live preview source: its slot, source view, and source size.
    ///
    /// Single source of truth shared by registration and encoding so the two
    /// passes cannot drift out of step.
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
            mixer.composite_view(),
            ct.width(),
            ct.height(),
        ));
        for (out_idx, output) in varda.outputs_ref().iter().enumerate() {
            let (view, w, h) = Self::output_preview_source(output, mixer);
            out.push((PreviewSlot::Output(out_idx), view, w, h));
        }
        out
    }

    /// Create/resize preview encode targets and keep egui registrations in sync.
    ///
    /// Runs early in the frame because the UI is built (and needs `TextureId`s)
    /// before any GPU work is submitted. Pixel content is filled in later by
    /// `encode_previews`.
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
            // Retire the previous registration for this slot, if any — a resized
            // target leaves the old TextureId dangling.
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

        // Retire registrations and targets for decks/outputs that are gone. This
        // is the only place they are retired, so skipping it leaks one texture
        // per removed entity.
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

    /// Gamma-encode every preview for this frame.
    ///
    /// Must run after the mixer render *and* after output windows have drawn, but
    /// before egui paints. See the frame sequence in `render_frame`.
    pub(super) fn encode_previews(&mut self) {
        let Some(varda) = &self.varda else { return };
        let Some(encoder) = &self.preview_encoder else {
            return;
        };
        let context = varda.gpu_context();
        encoder.encode_all(context, &Self::preview_sources(varda));
    }

    /// Per-frame egui texture sync. Previews are gamma-encoded into dedicated
    /// targets, so this only keeps targets and registrations in step; the pixels
    /// are written later by `encode_previews`.
    pub(super) fn refresh_textures(&mut self) {
        self.sync_preview_registrations();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── PreviewSlot keys ────────────────────────────────────────────

    /// Encoder targets are cached by this key, so two slots must never collide —
    /// a collision would show one preview's pixels in another's panel.
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
