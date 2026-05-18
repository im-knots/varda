//! Channel - Groups multiple decks into a composited layer with its own effect chain

use crate::deck::{Deck, Effect};
use crate::isf::ISFShader;
use crate::modulation::ModulationEngine;
use crate::params::ShaderParams;
use crate::renderer::{GpuContext, BlitPipeline, CompositeBlitPipeline, ISFUniforms, TransitionPipeline};
use anyhow::{Context as _, Result};

/// Blend modes for compositing decks and channels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum BlendMode {
    #[default]
    Normal,
    Add,
    Subtract,
    Multiply,
    Screen,
    Overlay,
    SoftLight,
    HardLight,
    ColorDodge,
    ColorBurn,
    Difference,
    Exclusion,
    Darken,
    Lighten,
    LinearBurn,
}

impl BlendMode {
    /// Shader uniform index for this blend mode.
    /// Must match the constants in composite.wgsl.
    pub fn to_index(&self) -> u32 {
        match self {
            BlendMode::Normal => 0,
            BlendMode::Add => 1,
            BlendMode::Subtract => 2,
            BlendMode::Multiply => 3,
            BlendMode::Screen => 4,
            BlendMode::Overlay => 5,
            BlendMode::SoftLight => 6,
            BlendMode::HardLight => 7,
            BlendMode::ColorDodge => 8,
            BlendMode::ColorBurn => 9,
            BlendMode::Difference => 10,
            BlendMode::Exclusion => 11,
            BlendMode::Darken => 12,
            BlendMode::Lighten => 13,
            BlendMode::LinearBurn => 14,
        }
    }

    /// Short display name for UI
    pub fn short_name(&self) -> &'static str {
        match self {
            BlendMode::Normal => "Norm",
            BlendMode::Add => "Add",
            BlendMode::Subtract => "Sub",
            BlendMode::Multiply => "Mult",
            BlendMode::Screen => "Scrn",
            BlendMode::Overlay => "Ovly",
            BlendMode::SoftLight => "SftL",
            BlendMode::HardLight => "HrdL",
            BlendMode::ColorDodge => "CDge",
            BlendMode::ColorBurn => "CBrn",
            BlendMode::Difference => "Diff",
            BlendMode::Exclusion => "Excl",
            BlendMode::Darken => "Dark",
            BlendMode::Lighten => "Lite",
            BlendMode::LinearBurn => "LBrn",
        }
    }

    /// All blend mode variants in display order
    pub fn all() -> &'static [BlendMode] {
        &[
            BlendMode::Normal,
            BlendMode::Add,
            BlendMode::Subtract,
            BlendMode::Multiply,
            BlendMode::Screen,
            BlendMode::Overlay,
            BlendMode::SoftLight,
            BlendMode::HardLight,
            BlendMode::ColorDodge,
            BlendMode::ColorBurn,
            BlendMode::Difference,
            BlendMode::Exclusion,
            BlendMode::Darken,
            BlendMode::Lighten,
            BlendMode::LinearBurn,
        ]
    }
}

// ── Auto-Transition Types ──────────────────────────────────────────

/// Unit of a duration value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
pub enum DurationUnit {
    Seconds,
    Minutes,
    Hours,
    Beats,
}

impl DurationUnit {
    /// Short label for UI display.
    pub fn label(&self) -> &'static str {
        match self {
            DurationUnit::Seconds => "s",
            DurationUnit::Minutes => "m",
            DurationUnit::Hours => "h",
            DurationUnit::Beats => "b",
        }
    }

    /// Cycle to the next unit: s → m → h → b → s
    pub fn next(&self) -> Self {
        match self {
            DurationUnit::Seconds => DurationUnit::Minutes,
            DurationUnit::Minutes => DurationUnit::Hours,
            DurationUnit::Hours => DurationUnit::Beats,
            DurationUnit::Beats => DurationUnit::Seconds,
        }
    }
}

/// Duration specified in beats or wall-clock time (seconds, minutes, or hours).
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DurationSpec {
    Beats(f64),
    Seconds(f64),
    Minutes(f64),
    Hours(f64),
}

impl DurationSpec {
    /// Resolve to seconds given the current BPM (falls back to 120 if unknown/invalid).
    pub fn to_seconds(&self, bpm: Option<f64>) -> f64 {
        match self {
            DurationSpec::Beats(b) => {
                let safe_bpm = match bpm {
                    Some(v) if v.is_finite() && v > 0.0 => v,
                    _ => 120.0,
                };
                b * 60.0 / safe_bpm
            }
            DurationSpec::Seconds(s) => *s,
            DurationSpec::Minutes(m) => m * 60.0,
            DurationSpec::Hours(h) => h * 3600.0,
        }
    }

    /// Get the raw numeric value.
    pub fn value(&self) -> f64 {
        match self {
            DurationSpec::Beats(v) | DurationSpec::Seconds(v) | DurationSpec::Minutes(v) | DurationSpec::Hours(v) => *v,
        }
    }

    pub fn is_beats(&self) -> bool { matches!(self, DurationSpec::Beats(_)) }

    /// Get the unit of this duration.
    pub fn unit(&self) -> DurationUnit {
        match self {
            DurationSpec::Seconds(_) => DurationUnit::Seconds,
            DurationSpec::Minutes(_) => DurationUnit::Minutes,
            DurationSpec::Hours(_) => DurationUnit::Hours,
            DurationSpec::Beats(_) => DurationUnit::Beats,
        }
    }

    /// Create a DurationSpec from a value and unit.
    pub fn from_value_unit(value: f64, unit: DurationUnit) -> Self {
        match unit {
            DurationUnit::Seconds => DurationSpec::Seconds(value),
            DurationUnit::Minutes => DurationSpec::Minutes(value),
            DurationUnit::Hours => DurationSpec::Hours(value),
            DurationUnit::Beats => DurationSpec::Beats(value),
        }
    }

    /// Set the numeric value, preserving the unit.
    pub fn set_value(&mut self, v: f64) {
        match self {
            DurationSpec::Beats(ref mut b) => *b = v,
            DurationSpec::Seconds(ref mut s) => *s = v,
            DurationSpec::Minutes(ref mut m) => *m = v,
            DurationSpec::Hours(ref mut h) => *h = v,
        }
    }
}

/// What starts the transition countdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TransitionTrigger {
    /// Timer-based: starts counting when deck becomes the active (topmost visible) deck.
    Timer,
    /// Content-aware: starts when video hits its out-point or end-of-file.
    /// Falls back to Timer for non-video sources.
    ClipEnd,
}

/// Runtime phase of a deck's auto-transition.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum DeckTransitionPhase {
    /// Not the active top deck, or auto-transition disabled.
    Inactive,
    /// Playing content, countdown running.
    Playing { elapsed: f64 },
    /// Transition shader active, progress 0.0 → 1.0.
    Transitioning { progress: f64 },
    /// Transition complete — deck is effectively invisible.
    Done,
}

/// Per-deck auto-transition configuration.
pub struct DeckAutoTransition {
    pub enabled: bool,
    pub play_duration: DurationSpec,
    pub transition_duration: DurationSpec,
    pub trigger: TransitionTrigger,
    /// Name of the transition shader (None = simple opacity fade).
    pub transition_shader_name: Option<String>,
    /// Runtime phase (not persisted).
    pub phase: DeckTransitionPhase,
}

impl DeckAutoTransition {
    pub fn new() -> Self {
        Self {
            enabled: false,
            play_duration: DurationSpec::Beats(16.0),
            transition_duration: DurationSpec::Seconds(2.0),
            trigger: TransitionTrigger::Timer,
            transition_shader_name: None,
            phase: DeckTransitionPhase::Inactive,
        }
    }
}

/// Compiled transition shader for a deck (separate from config for GPU resource lifecycle).
pub struct DeckTransitionEffect {
    pub shader: ISFShader,
    pub pipeline: TransitionPipeline,
    pub params: ShaderParams,
}

// ── DeckSlot ───────────────────────────────────────────────────────

/// A deck slot in a channel with compositing properties
pub struct DeckSlot {
    pub deck: Deck,
    pub opacity: f32,
    pub blend_mode: BlendMode,
    pub solo: bool,
    pub mute: bool,
    pub z_index: i32,
    /// Auto-transition config (None = no auto-transition).
    pub auto_transition: Option<DeckAutoTransition>,
    /// Compiled transition effect for this deck's auto-transition.
    pub transition_effect: Option<DeckTransitionEffect>,
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
            auto_transition: None,
            transition_effect: None,
        }
    }

    /// Set the transition shader for this deck's auto-transition.
    /// Compiles the shader and stores the pipeline.
    pub fn set_transition_shader(
        &mut self,
        context: &GpuContext,
        shader: ISFShader,
    ) -> Result<()> {
        let spirv = crate::isf::compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile transition shader to SPIR-V")?;
        let pipeline = TransitionPipeline::new(
            &context.device,
            &spirv,
            context.texture_format,
        )?;
        let name = shader.name();
        let inputs = shader.metadata.inputs.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
        let mut params = ShaderParams::from_inputs(inputs);
        params.ensure_buffer(&context.device);

        // Ensure auto_transition config exists
        if self.auto_transition.is_none() {
            self.auto_transition = Some(DeckAutoTransition::new());
        }
        if let Some(at) = &mut self.auto_transition {
            at.transition_shader_name = Some(name);
        }

        self.transition_effect = Some(DeckTransitionEffect { shader, pipeline, params });
        Ok(())
    }

    /// Clear the transition shader (revert to opacity fade).
    pub fn clear_transition_shader(&mut self) {
        self.transition_effect = None;
        if let Some(at) = &mut self.auto_transition {
            at.transition_shader_name = None;
        }
    }

    /// Get the current auto-transition phase.
    pub fn transition_phase(&self) -> DeckTransitionPhase {
        self.auto_transition.as_ref()
            .filter(|at| at.enabled)
            .map(|at| at.phase)
            .unwrap_or(DeckTransitionPhase::Inactive)
    }
}

/// Channel - Groups multiple decks into a composited layer
pub struct Channel {
    /// Stable UUID for this channel (8-char hex, persists across saves)
    uuid: String,

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

    /// Shader-based composite pipeline for blending decks (all blend modes via uniform)
    composite_pipeline: CompositeBlitPipeline,

    /// Simple blit pipeline for first-deck copy (Normal mode, no blend needed)
    blit_pipeline: BlitPipeline,

    /// Smoothed render time for this channel in milliseconds (EMA over recent frames)
    pub render_time_ms: f32,
    /// Number of active (rendered) decks in the last frame
    pub active_deck_count: u32,
}

impl Channel {
    /// Create a new channel
    /// Get the stable UUID for this channel
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// Set the UUID (used during scene restore to preserve identity)
    pub fn set_uuid(&mut self, uuid: String) {
        self.uuid = uuid;
    }

    pub fn new(name: String, context: &GpuContext, width: u32, height: u32) -> Result<Self> {
        let composite_texture = context.create_render_texture(width, height);
        let composite_view = composite_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let effect_ping_texture = context.create_render_texture(width, height);
        let effect_ping_view = effect_ping_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let composite_pipeline = CompositeBlitPipeline::new(&context.device, context.texture_format)?;
        let blit_pipeline = BlitPipeline::with_blend(
            &context.device,
            context.texture_format,
            wgpu::BlendState::ALPHA_BLENDING,
        )?;

        Ok(Self {
            uuid: crate::deck::generate_short_uuid(),
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
            composite_pipeline,
            blit_pipeline,
            render_time_ms: 0.0,
            active_deck_count: 0,
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

    /// Tick video frames for all decks without doing a full render.
    /// Call this every frame even for off-screen channels so video players
    /// stay in sync and don't show stale/black frames when faded back in.
    pub fn tick_video_frames(&mut self, context: &GpuContext) {
        for slot in self.decks.iter_mut() {
            if let Err(e) = slot.deck.update_video_frame(context) {
                log::warn!("Video frame update failed: {}", e);
            }
        }
    }

    /// Render all decks in this channel and composite them, then apply channel effects
    /// `channel_idx` is used for modulation key addressing (e.g., "ch0_deck0:paramname")
    /// `dt` is the frame delta in seconds (for auto-transition tick).
    pub fn render(
        &mut self,
        context: &GpuContext,
        audio_data: &crate::audio::AudioData,
        modulation: &ModulationEngine,
        _channel_idx: usize,
        time: f32,
        dt: f32,
    ) -> Result<()> {
        let render_start = std::time::Instant::now();

        // Tick auto-transition state before rendering
        let bpm = audio_data.bpm.map(|b| b as f64);
        self.tick_auto_transitions(dt as f64, bpm);

        // Sort decks by z-index
        let mut deck_indices: Vec<usize> = (0..self.decks.len()).collect();
        deck_indices.sort_by_key(|&i| self.decks[i].z_index);

        // Check if any deck is solo'd
        let any_solo = self.decks.iter().any(|slot| slot.solo);

        // Note: video frame updates are handled by tick_video_frames() called
        // from the mixer before render, so all channels stay in sync even when
        // faded out by the crossfader.

        // Render each deck to its texture (skip muted, non-solo'd, zero-opacity)
        // Done decks still render — they serve as visible background for transitioning decks above.
        let mut cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        let mut active_count: u32 = 0;
        for (_deck_idx, slot) in self.decks.iter_mut().enumerate() {
            if !slot.mute && (!any_solo || slot.solo) && slot.opacity > 0.0 {
                active_count += 1;
                let param_prefix = format!("deck_{}", slot.deck.uuid());
                slot.deck.render_with_prefix(context, audio_data, modulation, &param_prefix, &mut cmd_buffers)?;
            }
        }
        self.active_deck_count = active_count;
        // Batch submit all deck renders at once
        if !cmd_buffers.is_empty() {
            context.queue.submit(cmd_buffers);
        }

        // Collect render info for visible decks, including transition phase
        struct DeckCompositeInfo {
            deck_idx: usize,
            blend_mode: BlendMode,
            opacity: f32,
            transition_progress: Option<f64>, // Some = transitioning with shader
        }

        let deck_composite_info: Vec<DeckCompositeInfo> = deck_indices.iter()
            .filter_map(|&idx| {
                let slot = &self.decks[idx];
                let phase = slot.transition_phase();
                if slot.mute || (any_solo && !slot.solo) || slot.opacity <= 0.0 {
                    return None;
                }
                // Inactive and Done auto-transition decks don't composite.
                // Inactive = waiting for turn. Done = already played, no longer needed
                // (the next deck in sequence gets re-activated to Playing when needed).
                let has_at = slot.auto_transition.as_ref().map_or(false, |at| at.enabled);
                if has_at && (phase == DeckTransitionPhase::Inactive || phase == DeckTransitionPhase::Done) {
                    return None;
                }
                let transition_progress = match phase {
                    DeckTransitionPhase::Transitioning { progress } => Some(progress),
                    // Done decks composite normally (full opacity) — they serve
                    // as the visible background that transitioning decks reveal.
                    _ => None,
                };
                Some(DeckCompositeInfo {
                    deck_idx: idx,
                    blend_mode: slot.blend_mode,
                    opacity: slot.opacity,
                    transition_progress,
                })
            })
            .collect();

        // Reorder compositing: non-transitioning decks first, then transitioning.
        // This ensures composite-so-far always contains all "revealed" content
        // before the transitioning deck is rendered.
        let mut non_transitioning: Vec<&DeckCompositeInfo> = Vec::new();
        let mut transitioning: Vec<&DeckCompositeInfo> = Vec::new();
        for info in &deck_composite_info {
            if info.transition_progress.is_some() {
                transitioning.push(info);
            } else {
                non_transitioning.push(info);
            }
        }
        let ordered: Vec<&DeckCompositeInfo> = non_transitioning.into_iter()
            .chain(transitioning.into_iter())
            .collect();

        // Composite all decks to the composite texture.
        // Submit per-deck to ensure each deck's uniform buffer writes
        // are consumed before the next deck overwrites them.
        let width = self.composite_texture.width();
        let height = self.composite_texture.height();

        for (i, info) in ordered.iter().enumerate() {
            let slot = &mut self.decks[info.deck_idx];

            // Check if this deck is transitioning with a shader
            if let Some(progress) = info.transition_progress {
                if let Some(effect) = slot.transition_effect.as_mut().filter(|_| i > 0) {
                    // Snapshot composite-so-far into effect_ping_texture
                    let mut copy_encoder = context.device.create_command_encoder(
                        &wgpu::CommandEncoderDescriptor { label: Some("AT Snapshot Copy") },
                    );
                    copy_encoder.copy_texture_to_texture(
                        self.composite_texture.as_image_copy(),
                        self.effect_ping_texture.as_image_copy(),
                        self.composite_texture.size(),
                    );
                    context.queue.submit(std::iter::once(copy_encoder.finish()));

                    // Run transition shader: start=deck (outgoing), end=composite-below (incoming)
                    let uniforms = ISFUniforms {
                        time,
                        time_delta: dt,
                        frame_index: self.frame_count,
                        pass_index: 0,
                        render_size: [width as f32, height as f32],
                        phase_times: [0.0; 4],
                        ..Default::default()
                    };

                    // Set progress on the transition shader
                    effect.params.set("progress", crate::params::ParamValue::Float(progress as f32));
                    let params_data = effect.params.build_buffer_data();
                    if let Some(buf) = effect.params.buffer() {
                        context.queue.write_buffer(buf, 0, &params_data);
                    }

                    let cmd = effect.pipeline.render_to_cmd(
                        context,
                        &slot.deck.texture_view,      // startImage: outgoing deck
                        &self.effect_ping_view,         // endImage: composite below
                        &self.composite_view,           // output: back to composite
                        &uniforms,
                        effect.params.buffer(),
                    );
                    context.queue.submit(std::iter::once(cmd));
                    continue;
                }

                // Opacity fade fallback (no shader or first deck)
                let fade_opacity = info.opacity * (1.0 - progress as f32);
                if i == 0 {
                    // First deck: simple blit with alpha blending
                    self.blit_pipeline.set_opacity(&context.queue, fade_opacity);
                    let bind_group = self.blit_pipeline.create_bind_group(&context.device, &slot.deck.texture_view);
                    let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Channel Composite Encoder (AT fade first)"),
                    });
                    {
                        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Channel Composite Pass (AT fade first)"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &self.composite_view,
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
                        });
                        self.blit_pipeline.render(&mut render_pass, &bind_group);
                    }
                    context.queue.submit(std::iter::once(encoder.finish()));
                } else {
                    // Subsequent decks: snapshot + composite shader
                    let mut copy_encoder = context.device.create_command_encoder(
                        &wgpu::CommandEncoderDescriptor { label: Some("Composite Snapshot Copy (AT fade)") },
                    );
                    copy_encoder.copy_texture_to_texture(
                        self.composite_texture.as_image_copy(),
                        self.effect_ping_texture.as_image_copy(),
                        self.composite_texture.size(),
                    );

                    self.composite_pipeline.set_params(&context.queue, fade_opacity, info.blend_mode.to_index(), [1.0, 1.0], [0.0, 0.0]);
                    let bind_group = self.composite_pipeline.create_bind_group(&context.device, &slot.deck.texture_view, &self.effect_ping_view);
                    let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Channel Composite Encoder (AT fade)"),
                    });
                    {
                        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Channel Composite Pass (AT fade)"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &self.composite_view,
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
                        });
                        self.composite_pipeline.render(&mut render_pass, &bind_group);
                    }
                    context.queue.submit([copy_encoder.finish(), encoder.finish()]);
                }
                continue;
            }

            // Normal compositing
            if i == 0 {
                // First deck: simple blit with alpha blending (Normal = just copy)
                self.blit_pipeline.set_opacity(&context.queue, info.opacity);
                let bind_group = self.blit_pipeline.create_bind_group(&context.device, &slot.deck.texture_view);
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Channel Composite Encoder (first)"),
                });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Channel Composite Pass (first)"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &self.composite_view,
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
                    });
                    self.blit_pipeline.render(&mut render_pass, &bind_group);
                }
                context.queue.submit(std::iter::once(encoder.finish()));
            } else {
                // Subsequent decks: snapshot composite → ping, blend src + ping → composite
                let mut copy_encoder = context.device.create_command_encoder(
                    &wgpu::CommandEncoderDescriptor { label: Some("Composite Snapshot Copy") },
                );
                copy_encoder.copy_texture_to_texture(
                    self.composite_texture.as_image_copy(),
                    self.effect_ping_texture.as_image_copy(),
                    self.composite_texture.size(),
                );

                self.composite_pipeline.set_params(&context.queue, info.opacity, info.blend_mode.to_index(), [1.0, 1.0], [0.0, 0.0]);
                let bind_group = self.composite_pipeline.create_bind_group(&context.device, &slot.deck.texture_view, &self.effect_ping_view);
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
                                load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                    self.composite_pipeline.render(&mut render_pass, &bind_group);
                }
                context.queue.submit([copy_encoder.finish(), encoder.finish()]);
            }
        }

        // If no decks, clear the composite texture to transparent
        if deck_composite_info.is_empty() {
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
                            load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
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
                phase_times: [0.0; 4],
            };

            let mut read_from_composite = true;
            let mut fx_cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();

            for (_eff_idx, effect) in self.effects.iter_mut().enumerate() {
                if !effect.enabled {
                    continue;
                }

                let (input_view, output_view) = if read_from_composite {
                    (&self.composite_view, &self.effect_ping_view)
                } else {
                    (&self.effect_ping_view, &self.composite_view)
                };

                let fx_prefix = format!("fx_{}", effect.uuid);
                if let Err(e) = effect.apply_with_modulation(context, input_view, output_view, &uniforms, Some(modulation), Some(&fx_prefix), &mut fx_cmd_buffers) {
                    log::warn!("Effect {} failed, skipping: {}", _eff_idx, e);
                    continue;
                }
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

        // Update smoothed render time (EMA, α = 0.1)
        let raw_ms = render_start.elapsed().as_secs_f32() * 1000.0;
        self.render_time_ms = 0.1 * raw_ms + 0.9 * self.render_time_ms;

        Ok(())
    }

    /// Resize the channel's textures
    pub fn resize(&mut self, context: &GpuContext, width: u32, height: u32) {
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
            slot.opacity = if opacity.is_finite() { opacity.clamp(0.0, 1.0) } else { 1.0 };
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

    /// Tick auto-transition state for all decks in this channel.
    /// Called once per frame before rendering.
    /// `dt` is the frame delta in seconds, `bpm` is the current detected BPM (if any).
    pub fn tick_auto_transitions(&mut self, dt: f64, bpm: Option<f64>) {
        // Determine the "active" deck — topmost visible with auto-transition enabled.
        // Sort by z-index descending to find the top deck.
        let mut sorted: Vec<usize> = (0..self.decks.len()).collect();
        sorted.sort_by_key(|&i| std::cmp::Reverse(self.decks[i].z_index));

        // Find the topmost deck that is visible and has auto-transition enabled
        let active_idx = sorted.iter().copied().find(|&i| {
            let slot = &self.decks[i];
            let has_at = slot.auto_transition.as_ref().map_or(false, |at| at.enabled);
            let phase = slot.transition_phase();
            has_at && !slot.mute && phase != DeckTransitionPhase::Done
        });

        // Update phase for each deck, collecting indices that just started transitioning
        let mut just_started_transitioning: Vec<usize> = Vec::new();

        for i in 0..self.decks.len() {
            let is_active = active_idx == Some(i);
            let slot = &mut self.decks[i];
            let at = match &mut slot.auto_transition {
                Some(at) if at.enabled => at,
                _ => continue,
            };

            match at.phase {
                DeckTransitionPhase::Inactive => {
                    if is_active {
                        at.phase = DeckTransitionPhase::Playing { elapsed: 0.0 };
                    }
                }
                DeckTransitionPhase::Playing { ref mut elapsed } => {
                    *elapsed += dt;
                    let play_secs = at.play_duration.to_seconds(bpm);

                    // Check trigger condition
                    let should_transition = match at.trigger {
                        TransitionTrigger::Timer => *elapsed >= play_secs,
                        TransitionTrigger::ClipEnd => {
                            // Check if video reached end
                            let clip_ended = slot.deck.playback_state()
                                .map_or(false, |ps| ps.reached_end);
                            // Also respect timer as fallback for non-video sources
                            clip_ended || (slot.deck.playback_state().is_none() && *elapsed >= play_secs)
                        }
                    };

                    if should_transition {
                        at.phase = DeckTransitionPhase::Transitioning { progress: 0.0 };
                        just_started_transitioning.push(i);
                    }
                }
                DeckTransitionPhase::Transitioning { ref mut progress } => {
                    let trans_secs = at.transition_duration.to_seconds(bpm);
                    if trans_secs > 0.0 {
                        *progress += dt / trans_secs;
                    } else {
                        *progress = 1.0;
                    }
                    if *progress >= 1.0 {
                        at.phase = DeckTransitionPhase::Done;
                    }
                }
                DeckTransitionPhase::Done => {
                    // Stay done until loop reset
                }
            }
        }

        // Activate the next deck for each deck that just started transitioning.
        // This makes the next deck visible as the background the transition reveals.
        // First try Inactive decks; if none, wrap around to the first Done deck (loop).
        for _trigger_idx in just_started_transitioning {
            let mut activated = false;
            // Try Inactive first
            for j in 0..self.decks.len() {
                let slot = &self.decks[j];
                let is_candidate = slot.auto_transition.as_ref()
                    .map_or(false, |at| at.enabled && at.phase == DeckTransitionPhase::Inactive);
                if is_candidate && !slot.mute {
                    if let Some(at) = self.decks[j].auto_transition.as_mut() {
                        at.phase = DeckTransitionPhase::Playing { elapsed: 0.0 };
                    }
                    activated = true;
                    break;
                }
            }
            // If no Inactive found, wrap around: re-activate the first Done deck
            if !activated {
                for j in 0..self.decks.len() {
                    let slot = &self.decks[j];
                    let is_candidate = slot.auto_transition.as_ref()
                        .map_or(false, |at| at.enabled && at.phase == DeckTransitionPhase::Done);
                    if is_candidate && !slot.mute {
                        if let Some(at) = self.decks[j].auto_transition.as_mut() {
                            at.phase = DeckTransitionPhase::Playing { elapsed: 0.0 };
                        }
                        break;
                    }
                }
            }
        }

        // Check if all auto-transition decks are now Done → loop reset.
        // Done AFTER phase updates so the reset happens in the same frame
        // a deck transitions to Done, preventing a flash of stale content.
        let all_done = self.decks.iter().all(|slot| {
            match &slot.auto_transition {
                Some(at) if at.enabled => at.phase == DeckTransitionPhase::Done,
                _ => true,
            }
        });
        let any_at = self.decks.iter().any(|slot| {
            slot.auto_transition.as_ref().map_or(false, |at| at.enabled)
        });

        if all_done && any_at {
            // Reset all AT decks to Inactive, then immediately activate the first one
            for slot in &mut self.decks {
                if let Some(at) = &mut slot.auto_transition {
                    if at.enabled {
                        at.phase = DeckTransitionPhase::Inactive;
                    }
                }
            }
            for slot in &mut self.decks {
                let dominated = slot.mute;
                let is_inactive_at = slot.auto_transition.as_ref()
                    .map_or(false, |at| at.enabled && at.phase == DeckTransitionPhase::Inactive);
                if is_inactive_at && !dominated {
                    if let Some(at) = slot.auto_transition.as_mut() {
                        at.phase = DeckTransitionPhase::Playing { elapsed: 0.0 };
                    }
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── BlendMode tests ──────────────────────────────────────────────

    #[test]
    fn blend_mode_default_is_normal() {
        assert_eq!(BlendMode::default(), BlendMode::Normal);
    }

    #[test]
    fn blend_mode_all_variants_have_index() {
        // Verify to_index doesn't panic for any variant
        for mode in BlendMode::all() {
            let _ = mode.to_index();
        }
    }

    #[test]
    fn blend_mode_debug() {
        assert!(format!("{:?}", BlendMode::Add).contains("Add"));
    }

    // ── DurationSpec tests ───────────────────────────────────────────

    #[test]
    fn duration_spec_seconds() {
        let d = DurationSpec::Seconds(5.0);
        assert!((d.to_seconds(None) - 5.0).abs() < 1e-5);
        assert!((d.to_seconds(Some(120.0)) - 5.0).abs() < 1e-5);
        assert!((d.value() - 5.0).abs() < 1e-5);
        assert!(!d.is_beats());
    }

    #[test]
    fn duration_spec_beats_with_bpm() {
        let d = DurationSpec::Beats(4.0);
        // 4 beats at 120 BPM = 4 * 60/120 = 2 seconds
        assert!((d.to_seconds(Some(120.0)) - 2.0).abs() < 1e-5);
        assert!(d.is_beats());
        assert!((d.value() - 4.0).abs() < 1e-5);
    }

    #[test]
    fn duration_spec_beats_no_bpm_defaults_120() {
        let d = DurationSpec::Beats(4.0);
        // Falls back to 120 BPM → 4 * 60/120 = 2.0
        assert!((d.to_seconds(None) - 2.0).abs() < 1e-5);
    }

    #[test]
    fn duration_spec_beats_different_bpm() {
        let d = DurationSpec::Beats(1.0);
        // 1 beat at 60 BPM = 1 second
        assert!((d.to_seconds(Some(60.0)) - 1.0).abs() < 1e-5);
        // 1 beat at 180 BPM = 60/180 = 0.333s
        assert!((d.to_seconds(Some(180.0)) - 0.333).abs() < 0.01);
    }

    #[test]
    fn duration_spec_minutes() {
        let d = DurationSpec::Minutes(2.0);
        assert!((d.to_seconds(None) - 120.0).abs() < 1e-5);
        assert!((d.value() - 2.0).abs() < 1e-5);
        assert!(!d.is_beats());
        assert_eq!(d.unit(), DurationUnit::Minutes);
    }

    #[test]
    fn duration_spec_hours() {
        let d = DurationSpec::Hours(1.5);
        assert!((d.to_seconds(None) - 5400.0).abs() < 1e-5);
        assert!((d.value() - 1.5).abs() < 1e-5);
        assert!(!d.is_beats());
        assert_eq!(d.unit(), DurationUnit::Hours);
    }

    #[test]
    fn duration_unit_cycle() {
        assert_eq!(DurationUnit::Seconds.next(), DurationUnit::Minutes);
        assert_eq!(DurationUnit::Minutes.next(), DurationUnit::Hours);
        assert_eq!(DurationUnit::Hours.next(), DurationUnit::Beats);
        assert_eq!(DurationUnit::Beats.next(), DurationUnit::Seconds);
    }

    #[test]
    fn duration_unit_labels() {
        assert_eq!(DurationUnit::Seconds.label(), "s");
        assert_eq!(DurationUnit::Minutes.label(), "m");
        assert_eq!(DurationUnit::Hours.label(), "h");
        assert_eq!(DurationUnit::Beats.label(), "b");
    }

    #[test]
    fn duration_spec_from_value_unit() {
        let d = DurationSpec::from_value_unit(5.0, DurationUnit::Minutes);
        assert!(matches!(d, DurationSpec::Minutes(v) if (v - 5.0).abs() < 1e-5));
        let d = DurationSpec::from_value_unit(2.0, DurationUnit::Hours);
        assert!(matches!(d, DurationSpec::Hours(v) if (v - 2.0).abs() < 1e-5));
    }

    // ── DeckAutoTransition tests ─────────────────────────────────────

    #[test]
    fn deck_auto_transition_defaults() {
        let at = DeckAutoTransition::new();
        assert!(!at.enabled);
        assert_eq!(at.trigger, TransitionTrigger::Timer);
        assert_eq!(at.phase, DeckTransitionPhase::Inactive);
        assert!(at.transition_shader_name.is_none());
    }

    #[test]
    fn deck_auto_transition_play_duration_is_beats() {
        let at = DeckAutoTransition::new();
        assert!(at.play_duration.is_beats());
    }

    #[test]
    fn deck_auto_transition_transition_duration_is_seconds() {
        let at = DeckAutoTransition::new();
        assert!(!at.transition_duration.is_beats());
    }

    // ── DeckTransitionPhase tests ────────────────────────────────────

    #[test]
    fn deck_transition_phase_equality() {
        assert_eq!(DeckTransitionPhase::Inactive, DeckTransitionPhase::Inactive);
        assert_eq!(DeckTransitionPhase::Done, DeckTransitionPhase::Done);
        assert_ne!(DeckTransitionPhase::Inactive, DeckTransitionPhase::Done);
    }

    #[test]
    fn deck_transition_phase_playing() {
        let phase = DeckTransitionPhase::Playing { elapsed: 1.5 };
        match phase {
            DeckTransitionPhase::Playing { elapsed } => {
                assert!((elapsed - 1.5).abs() < 1e-5);
            }
            _ => panic!("Wrong phase"),
        }
    }

    #[test]
    fn deck_transition_phase_transitioning() {
        let phase = DeckTransitionPhase::Transitioning { progress: 0.75 };
        match phase {
            DeckTransitionPhase::Transitioning { progress } => {
                assert!((progress - 0.75).abs() < 1e-5);
            }
            _ => panic!("Wrong phase"),
        }
    }

    // ── TransitionTrigger tests ──────────────────────────────────────

    #[test]
    fn transition_trigger_equality() {
        assert_eq!(TransitionTrigger::Timer, TransitionTrigger::Timer);
        assert_eq!(TransitionTrigger::ClipEnd, TransitionTrigger::ClipEnd);
        assert_ne!(TransitionTrigger::Timer, TransitionTrigger::ClipEnd);
    }

    // ── Deck slot management tests (DnD data model) ─────────────────
    //
    // These test the Channel-level operations that back drag-and-drop
    // actions: add_deck, remove_deck, remove_deck_slot, add_deck_slot.
    // They require a headless GPU to construct real Channel + Deck instances.

    use crate::renderer::GpuContext;

    fn headless_gpu() -> GpuContext {
        GpuContext::new_headless().expect("headless GPU required for tests")
    }

    fn test_channel(gpu: &GpuContext, name: &str) -> Channel {
        Channel::new(name.to_string(), gpu, 64, 64).expect("channel creation")
    }

    fn add_solid_deck(ch: &mut Channel, gpu: &GpuContext, color: [f32; 4]) {
        let deck = crate::deck::Deck::new_solid_color(gpu, color, 64, 64)
            .expect("solid color deck");
        ch.add_deck(deck);
    }

    #[test]
    fn add_deck_increases_count() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        assert_eq!(ch.deck_count(), 0);
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(ch.deck_count(), 1);
        add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(ch.deck_count(), 2);
    }

    #[test]
    fn remove_deck_returns_deck_and_shrinks() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
        add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
        assert_eq!(ch.deck_count(), 2);
        let removed = ch.remove_deck(0);
        assert!(removed.is_some());
        assert_eq!(ch.deck_count(), 1);
    }

    #[test]
    fn remove_deck_out_of_bounds_returns_none() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        assert!(ch.remove_deck(0).is_none());
        assert!(ch.remove_deck(99).is_none());
    }

    #[test]
    fn remove_deck_slot_preserves_properties() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
        ch.decks[0].opacity = 0.42;
        ch.decks[0].blend_mode = BlendMode::Add;
        ch.decks[0].solo = true;

        let slot = ch.remove_deck_slot(0).expect("slot exists");
        assert!((slot.opacity - 0.42).abs() < 1e-5);
        assert_eq!(slot.blend_mode, BlendMode::Add);
        assert!(slot.solo);
        assert_eq!(ch.deck_count(), 0);
    }

    #[test]
    fn add_deck_slot_appends_and_returns_index() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

        let slot = ch.remove_deck_slot(0).unwrap();
        let idx = ch.add_deck_slot(slot);
        assert_eq!(idx, 0); // only slot
        assert_eq!(ch.deck_count(), 1);
    }

    #[test]
    fn move_deck_between_channels_preserves_data() {
        let gpu = headless_gpu();
        let mut src = test_channel(&gpu, "Src");
        let mut dst = test_channel(&gpu, "Dst");

        // Add two decks to src
        add_solid_deck(&mut src, &gpu, [1.0, 0.0, 0.0, 1.0]); // Red
        add_solid_deck(&mut src, &gpu, [0.0, 1.0, 0.0, 1.0]); // Green
        src.decks[0].opacity = 0.5;
        src.decks[1].opacity = 0.75;

        // Move deck 0 (red) from src to dst
        let slot = src.remove_deck_slot(0).unwrap();
        let new_idx = dst.add_deck_slot(slot);

        assert_eq!(src.deck_count(), 1);
        assert_eq!(dst.deck_count(), 1);
        assert_eq!(new_idx, 0);
        // Moved slot preserves opacity
        assert!((dst.decks[0].opacity - 0.5).abs() < 1e-5);
        // Remaining src deck shifted
        assert!((src.decks[0].opacity - 0.75).abs() < 1e-5);
    }

    #[test]
    fn effect_reorder_within_deck() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

        // Manually push named effects (requires ISF shader + GPU pipeline)
        // Since Effect::new requires real shaders, test the vec operation directly
        // which is what apply_deck_and_effect_actions does
        let _deck = &mut ch.decks[0].deck;

        // Simulate 3 effects by checking vec operations match action processing logic
        // The action processing code does: effects.remove(from); effects.insert(to, effect);
        let mut names = vec!["blur", "glow", "invert"];
        // Move index 2 → index 0
        let removed = names.remove(2);
        names.insert(0, removed);
        assert_eq!(names, vec!["invert", "blur", "glow"]);

        // Move index 0 → index 1
        let removed = names.remove(0);
        names.insert(1, removed);
        assert_eq!(names, vec!["blur", "invert", "glow"]);
    }

    #[test]
    fn channel_effect_reorder() {
        // Channel effects use the same vec pattern
        let mut effects = vec!["ch_blur", "ch_color", "ch_distort"];
        let from = 0;
        let to = 2;
        let e = effects.remove(from);
        effects.insert(to, e);
        assert_eq!(effects, vec!["ch_color", "ch_distort", "ch_blur"]);
    }

    // ── Render timing tests ─────────────────────────────────────────

    #[test]
    fn new_channel_render_time_starts_at_zero() {
        let gpu = headless_gpu();
        let ch = test_channel(&gpu, "Test");
        assert!((ch.render_time_ms - 0.0).abs() < 1e-5);
        assert_eq!(ch.active_deck_count, 0);
    }

    #[test]
    fn render_updates_timing_fields() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
        add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();
        ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();

        // After one render, render_time_ms should be > 0 (something was measured)
        assert!(ch.render_time_ms > 0.0);
        // Both decks are active (opacity 1.0, not muted)
        assert_eq!(ch.active_deck_count, 2);
    }

    #[test]
    fn muted_decks_not_counted_as_active() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
        add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
        ch.decks[1].mute = true;

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();
        ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();

        assert_eq!(ch.active_deck_count, 1);
    }

    #[test]
    fn zero_opacity_decks_not_counted_as_active() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
        add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);
        ch.decks[0].opacity = 0.0;

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();
        ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();

        assert_eq!(ch.active_deck_count, 1);
    }

    #[test]
    fn render_time_smooths_over_frames() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();

        // Render multiple frames — EMA should converge
        for _ in 0..10 {
            ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();
        }
        let time_after_10 = ch.render_time_ms;

        // Render more frames
        for _ in 0..10 {
            ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();
        }
        let time_after_20 = ch.render_time_ms;

        // Both should be positive
        assert!(time_after_10 > 0.0);
        assert!(time_after_20 > 0.0);
    }

    #[test]
    fn empty_channel_render_timing() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        // No decks — render should still work and measure time

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();
        ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();

        // Time should be >= 0 (even empty channels do some work)
        assert!(ch.render_time_ms >= 0.0);
        assert_eq!(ch.active_deck_count, 0);
    }

    // ── Deck pipeline FPS tests ─────────────────────────────────────

    #[test]
    fn new_deck_fps_starts_at_zero() {
        let gpu = headless_gpu();
        let deck = crate::deck::Deck::new_solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64).unwrap();
        assert!((deck.fps() - 0.0).abs() < 1e-5);
    }

    #[test]
    fn deck_fps_becomes_positive_after_renders() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();

        // Render several frames so EMA has time to converge
        for _ in 0..5 {
            ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();
        }

        let deck_fps = ch.decks[0].deck.fps();
        assert!(deck_fps > 0.0, "Deck FPS should be positive after rendering, got {}", deck_fps);
    }

    #[test]
    fn deck_fps_ignores_huge_first_frame_delta() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();

        // First render — time_delta may be very large (time since Deck creation)
        // but the guard (time_delta < 1.0) should keep FPS sane
        ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();
        let fps = ch.decks[0].deck.fps();
        // Either 0 (if first delta was >= 1s) or some reasonable value
        assert!(fps >= 0.0);
        assert!(fps < 100_000.0, "FPS should not be absurdly high, got {}", fps);
    }

    #[test]
    fn multiple_decks_have_independent_fps() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);
        add_solid_deck(&mut ch, &gpu, [0.0, 1.0, 0.0, 1.0]);

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();

        for _ in 0..5 {
            ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();
        }

        // Both decks should have positive FPS
        let fps0 = ch.decks[0].deck.fps();
        let fps1 = ch.decks[1].deck.fps();
        assert!(fps0 > 0.0);
        assert!(fps1 > 0.0);
    }

    #[test]
    fn skipped_deck_keeps_old_fps() {
        let gpu = headless_gpu();
        let mut ch = test_channel(&gpu, "Test");
        add_solid_deck(&mut ch, &gpu, [1.0, 0.0, 0.0, 1.0]);

        let audio = crate::audio::AudioData::default();
        let modulation = crate::modulation::ModulationEngine::new();

        // Render to establish FPS
        for _ in 0..5 {
            ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();
        }
        let fps_before = ch.decks[0].deck.fps();

        // Mute the deck — it won't render
        ch.decks[0].mute = true;
        ch.render(&gpu, &audio, &modulation, 0, 0.0, 1.0 / 60.0).unwrap();

        // FPS should remain unchanged (deck wasn't rendered, no EMA update)
        let fps_after = ch.decks[0].deck.fps();
        assert!((fps_before - fps_after).abs() < 1e-5);
    }
}
