//! Channel - Groups multiple decks into a composited layer with its own effect chain

use crate::deck::{Deck, Effect};
use crate::modulation::ModulationEngine;
use crate::renderer::{RenderContext, BlitPipeline, ISFUniforms};
use anyhow::Result;

/// Blend modes for compositing decks and channels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Multiply,
    Screen,
    Overlay,
    Difference,
}

impl BlendMode {
    /// Get wgpu blend state for this mode
    pub fn to_blend_state(&self) -> wgpu::BlendState {
        match self {
            BlendMode::Normal => wgpu::BlendState::ALPHA_BLENDING,
            BlendMode::Add => wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
            BlendMode::Multiply => wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::Dst,
                    dst_factor: wgpu::BlendFactor::Zero,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
            BlendMode::Screen => wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::OneMinusSrc,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
            BlendMode::Overlay => wgpu::BlendState::ALPHA_BLENDING, // Requires shader
            BlendMode::Difference => wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Subtract,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
        }
    }
}

/// A deck slot in a channel with compositing properties
pub struct DeckSlot {
    pub deck: Deck,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    pub z_index: i32,
}

impl DeckSlot {
    pub fn new(deck: Deck) -> Self {
        Self {
            deck,
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            solo: false,
            mute: false,
            z_index: 0,
        }
    }
}

/// Channel - Groups multiple decks into a composited layer
pub struct Channel {
    /// Channel name (A, B, C, ...)
    pub name: String,

    /// Decks in this channel
    pub decks: Vec<DeckSlot>,

    /// Per-channel effect chain (applied to composited deck output)
    pub effects: Vec<Effect>,

    /// Channel opacity (0.0–1.0) for mixing into final output
    pub opacity: f32,

    /// Channel blend mode for mixing into final output
    pub blend_mode: BlendMode,

    /// Composite output texture (all decks blended together)
    pub composite_texture: wgpu::Texture,
    pub composite_view: wgpu::TextureView,

    /// Ping-pong texture for channel effect chain
    pub effect_ping_texture: wgpu::Texture,
    pub effect_ping_view: wgpu::TextureView,

    /// Frame counter for uniforms
    frame_count: u32,

    /// Blit pipelines for each blend mode (for compositing decks)
    blend_blit_pipelines: std::collections::HashMap<BlendMode, BlitPipeline>,
}

impl Channel {
    /// Create a new channel
    pub fn new(name: String, context: &RenderContext, width: u32, height: u32) -> Result<Self> {
        let composite_texture = context.create_render_texture(width, height);
        let composite_view = composite_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let effect_ping_texture = context.create_render_texture(width, height);
        let effect_ping_view = effect_ping_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create blit pipelines for each blend mode
        let mut blend_blit_pipelines = std::collections::HashMap::new();
        for mode in [BlendMode::Normal, BlendMode::Add, BlendMode::Multiply,
                     BlendMode::Screen, BlendMode::Overlay, BlendMode::Difference] {
            let pipeline = BlitPipeline::with_blend(
                &context.device,
                context.surface_config.format,
                mode.to_blend_state()
            )?;
            blend_blit_pipelines.insert(mode, pipeline);
        }

        Ok(Self {
            name,
            decks: Vec::new(),
            effects: Vec::new(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            composite_texture,
            composite_view,
            effect_ping_texture,
            effect_ping_view,
            frame_count: 0,
            blend_blit_pipelines,
        })
    }

    /// Add a deck to this channel
    pub fn add_deck(&mut self, deck: Deck) -> usize {
        let idx = self.decks.len();
        self.decks.push(DeckSlot::new(deck));
        idx
    }

    /// Remove a deck from this channel
    pub fn remove_deck(&mut self, index: usize) -> Option<Deck> {
        if index < self.decks.len() {
            Some(self.decks.remove(index).deck)
        } else {
            None
        }
    }

    /// Remove a deck slot (preserving all properties) from this channel
    pub fn remove_deck_slot(&mut self, index: usize) -> Option<DeckSlot> {
        if index < self.decks.len() {
            Some(self.decks.remove(index))
        } else {
            None
        }
    }

    /// Add a pre-configured deck slot to this channel
    pub fn add_deck_slot(&mut self, slot: DeckSlot) -> usize {
        let idx = self.decks.len();
        self.decks.push(slot);
        idx
    }

    /// Get number of decks
    pub fn deck_count(&self) -> usize {
        self.decks.len()
    }

    /// Render all decks in this channel and composite them, then apply channel effects
    /// `channel_idx` is used for modulation key addressing (e.g., "ch0_deck0:paramname")
    pub fn render(
        &mut self,
        context: &RenderContext,
        audio_data: &crate::AudioData,
        modulation: &ModulationEngine,
        channel_idx: usize,
        time: f32,
    ) -> Result<()> {
        // Sort decks by z-index
        let mut deck_indices: Vec<usize> = (0..self.decks.len()).collect();
        deck_indices.sort_by_key(|&i| self.decks[i].z_index);

        // Check if any deck is solo'd
        let any_solo = self.decks.iter().any(|slot| slot.solo);

        // Render each deck to its texture (skip muted, non-solo'd, and zero-opacity decks)
        // Collect command buffers for batch submission to reduce CPU-GPU sync overhead
        let mut cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        for (deck_idx, slot) in self.decks.iter_mut().enumerate() {
            if !slot.mute && (!any_solo || slot.solo) && slot.opacity > 0.0 {
                let param_prefix = format!("ch{}_deck{}", channel_idx, deck_idx);
                slot.deck.render_with_prefix(context, audio_data, modulation, &param_prefix, &mut cmd_buffers)?;
            }
        }
        // Batch submit all deck renders at once
        if !cmd_buffers.is_empty() {
            context.queue.submit(cmd_buffers);
        }

        // Create bind groups and blend modes for all visible decks BEFORE the render pass
        let deck_render_info: Vec<(usize, BlendMode, f32, wgpu::BindGroup)> = deck_indices.iter()
            .filter_map(|&idx| {
                let slot = &self.decks[idx];
                if slot.mute || (any_solo && !slot.solo) || slot.opacity <= 0.0 {
                    None
                } else {
                    let blend_mode = slot.blend_mode;
                    let opacity = slot.opacity;
                    let pipeline = self.blend_blit_pipelines.get(&blend_mode)
                        .unwrap_or_else(|| self.blend_blit_pipelines.get(&BlendMode::Normal).unwrap());
                    let bind_group = pipeline.create_bind_group(&context.device, &slot.deck.texture_view);
                    Some((idx, blend_mode, opacity, bind_group))
                }
            })
            .collect();

        // Composite all decks to the composite texture
        for (i, (_idx, blend_mode, opacity, bind_group)) in deck_render_info.iter().enumerate() {
            let pipeline = self.blend_blit_pipelines.get(blend_mode)
                .unwrap_or_else(|| self.blend_blit_pipelines.get(&BlendMode::Normal).unwrap());
            pipeline.set_opacity(&context.queue, *opacity);

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Channel Composite Encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Channel Composite Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.composite_view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if i == 0 {
                                wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                            } else {
                                wgpu::LoadOp::Load
                            },
                            store: wgpu::StoreOp::Store,
                        },
                        depth_slice: None,
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                pipeline.render(&mut render_pass, bind_group);
            }

            context.queue.submit(std::iter::once(encoder.finish()));
        }

        // If no decks, clear the composite texture
        if deck_render_info.is_empty() {
            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Channel Clear Encoder"),
            });
            {
                let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Channel Clear Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.composite_view,
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
                });
            }
            context.queue.submit(std::iter::once(encoder.finish()));
        }

        // Apply channel effect chain (if any)
        if !self.effects.is_empty() {
            let width = self.composite_texture.width();
            let height = self.composite_texture.height();

            let uniforms = ISFUniforms {
                time,
                time_delta: 1.0 / 60.0,
                frame_index: self.frame_count,
                pass_index: 0,
                render_size: [width as f32, height as f32],
                audio_level: audio_data.level,
                audio_bass: audio_data.bass(),
                audio_mid: audio_data.mid(),
                audio_treble: audio_data.treble(),
                audio_bpm: audio_data.bpm.unwrap_or(0.0),
                audio_beat_phase: audio_data.beat_phase(),
                date: crate::deck::get_current_date(),
            };

            let mut read_from_composite = true;
            let mut fx_cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();

            for (eff_idx, effect) in self.effects.iter_mut().enumerate() {
                if !effect.enabled {
                    continue;
                }

                let (input_view, output_view) = if read_from_composite {
                    (&self.composite_view, &self.effect_ping_view)
                } else {
                    (&self.effect_ping_view, &self.composite_view)
                };

                let fx_prefix = format!("ch{}_fx{}", channel_idx, eff_idx);
                effect.apply_with_modulation(context, input_view, output_view, &uniforms, Some(modulation), Some(&fx_prefix), &mut fx_cmd_buffers)?;
                read_from_composite = !read_from_composite;
            }

            // If result is in ping texture, copy back to composite
            if !read_from_composite {
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Channel Effect Final Copy Encoder"),
                });
                encoder.copy_texture_to_texture(
                    self.effect_ping_texture.as_image_copy(),
                    self.composite_texture.as_image_copy(),
                    self.composite_texture.size(),
                );
                fx_cmd_buffers.push(encoder.finish());
            }

            // Batch submit all channel effects
            if !fx_cmd_buffers.is_empty() {
                context.queue.submit(fx_cmd_buffers);
            }
        }

        self.frame_count += 1;
        Ok(())
    }

    /// Resize the channel's textures
    pub fn resize(&mut self, context: &RenderContext, width: u32, height: u32) {
        self.composite_texture = context.create_render_texture(width, height);
        self.composite_view = self.composite_texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.effect_ping_texture = context.create_render_texture(width, height);
        self.effect_ping_view = self.effect_ping_texture.create_view(&wgpu::TextureViewDescriptor::default());

        for slot in &mut self.decks {
            slot.deck.resize(context, width, height);
        }
    }

    /// Add a channel effect
    pub fn add_effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    /// Remove a channel effect by index
    pub fn remove_effect(&mut self, index: usize) -> bool {
        if index < self.effects.len() {
            self.effects.remove(index);
            true
        } else {
            false
        }
    }

    /// Set deck opacity
    pub fn set_deck_opacity(&mut self, index: usize, opacity: f32) {
        if let Some(slot) = self.decks.get_mut(index) {
            slot.opacity = opacity.clamp(0.0, 1.0);
        }
    }

    /// Set deck solo
    pub fn set_deck_solo(&mut self, index: usize, solo: bool) {
        if let Some(slot) = self.decks.get_mut(index) {
            slot.solo = solo;
        }
    }

    /// Set deck mute
    pub fn set_deck_mute(&mut self, index: usize, mute: bool) {
        if let Some(slot) = self.decks.get_mut(index) {
            slot.mute = mute;
        }
    }

    /// Set deck blend mode
    pub fn set_deck_blend_mode(&mut self, index: usize, blend_mode: BlendMode) {
        if let Some(slot) = self.decks.get_mut(index) {
            slot.blend_mode = blend_mode;
        }
    }
}
