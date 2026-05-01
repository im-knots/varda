//! Mixer - Top-level compositor that owns channels, crossfader, master effects, and modulation

use crate::channel::{Channel, BlendMode};
use crate::deck::Effect;
use crate::isf::{ISFShader, compile_glsl_to_spirv};
use crate::modulation::{ModulationEngine, AudioValues};
use crate::params::ShaderParams;
use crate::renderer::{RenderContext, BlitPipeline, ISFUniforms, TransitionPipeline};
use anyhow::{Context as _, Result};

/// Easing curve for crossfade transitions
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CrossfadeEasing {
    Linear,
    EaseInOut,
    EaseIn,
    EaseOut,
}

impl CrossfadeEasing {
    /// Apply easing to normalized time t (0.0 to 1.0)
    pub fn apply(self, t: f32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        match self {
            CrossfadeEasing::Linear => t,
            CrossfadeEasing::EaseInOut => {
                // Smoothstep: 3t² - 2t³
                t * t * (3.0 - 2.0 * t)
            }
            CrossfadeEasing::EaseIn => t * t,
            CrossfadeEasing::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
        }
    }
}

/// Describes an in-progress auto-crossfade
#[derive(Debug, Clone)]
pub struct AutoCrossfade {
    /// Where the crossfader started
    pub from: f32,
    /// Where the crossfader is heading
    pub to: f32,
    /// Total duration in seconds
    pub duration: f32,
    /// Elapsed time in seconds
    pub elapsed: f32,
    /// Easing curve
    pub easing: CrossfadeEasing,
}

impl AutoCrossfade {
    /// Create a new auto-crossfade
    pub fn new(from: f32, to: f32, duration: f32, easing: CrossfadeEasing) -> Self {
        Self { from, to, duration, elapsed: 0.0, easing }
    }

    /// Tick the crossfade by dt seconds, return the new crossfader value.
    /// Returns None if the crossfade is complete.
    pub fn tick(&mut self, dt: f32) -> Option<f32> {
        self.elapsed += dt;
        if self.elapsed >= self.duration {
            return None; // Complete — caller should set crossfader to `self.to`
        }
        let t = self.easing.apply(self.elapsed / self.duration);
        Some(self.from + (self.to - self.from) * t)
    }

    /// Progress as 0.0 to 1.0
    pub fn progress(&self) -> f32 {
        (self.elapsed / self.duration).clamp(0.0, 1.0)
    }
}

/// Beat-synced crossfade configuration
#[derive(Debug, Clone)]
pub struct BeatSyncCrossfade {
    /// Target crossfader value
    pub to: f32,
    /// Duration in beats
    pub beats: f32,
    /// Whether we've started (waiting for next beat boundary)
    pub started: bool,
    /// The auto-crossfade that runs once triggered
    pub auto: Option<AutoCrossfade>,
}

/// Active transition effect between channels A and B
pub struct TransitionEffect {
    /// The ISF transition shader source
    pub shader: ISFShader,
    /// The compiled transition pipeline (two input textures + progress)
    pub pipeline: TransitionPipeline,
    /// User-controllable parameters (progress is always index 0)
    pub params: ShaderParams,
    /// Shader name for display
    pub name: String,
}

/// Mixer - Top-level compositor
pub struct Mixer {
    /// Channels (default 2: A and B)
    pub channels: Vec<Channel>,

    /// Monotonic counter for generating unique channel names (never decremented)
    next_channel_index: usize,

    /// Crossfader position (0.0 = Channel A, 1.0 = Channel B)
    pub crossfader: f32,

    /// Active auto-crossfade (if any)
    pub auto_crossfade: Option<AutoCrossfade>,

    /// Pending beat-synced crossfade (if any)
    pub beat_sync_crossfade: Option<BeatSyncCrossfade>,

    /// Global modulation engine
    pub modulation: ModulationEngine,

    /// Start time for TIME-based modulation
    start_time: std::time::Instant,

    /// Last render time for dt calculation
    last_render_time: std::time::Instant,

    /// Composite output texture (all channels mixed, pre-master effects)
    pub composite_texture: wgpu::Texture,
    pub composite_view: wgpu::TextureView,

    /// Ping-pong texture for master effect chain
    pub effect_ping_texture: wgpu::Texture,
    pub effect_ping_view: wgpu::TextureView,

    /// Master effect chain (applied to final composite)
    pub master_effects: Vec<Effect>,

    /// Frame counter
    frame_count: u32,

    /// Blit pipelines for channel compositing
    blend_blit_pipelines: std::collections::HashMap<BlendMode, BlitPipeline>,

    /// Active transition effect (replaces opacity-based crossfade when set)
    pub active_transition: Option<TransitionEffect>,
}

impl Mixer {
    /// Create a new mixer with two default channels (A and B)
    pub fn new(context: &RenderContext, width: u32, height: u32) -> Result<Self> {
        let composite_texture = context.create_render_texture(width, height);
        let composite_view = composite_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let effect_ping_texture = context.create_render_texture(width, height);
        let effect_ping_view = effect_ping_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create blit pipelines for channel compositing
        let mut blend_blit_pipelines = std::collections::HashMap::new();
        for mode in [BlendMode::Normal, BlendMode::Add, BlendMode::Multiply,
                     BlendMode::Screen, BlendMode::Overlay, BlendMode::Difference] {
            let pipeline = BlitPipeline::with_blend(
                &context.device,
                context.surface_config.format,
                mode.to_blend_state(),
            )?;
            blend_blit_pipelines.insert(mode, pipeline);
        }

        // Create two default channels
        let channel_a = Channel::new("A".to_string(), context, width, height)?;
        let channel_b = Channel::new("B".to_string(), context, width, height)?;

        let now = std::time::Instant::now();
        Ok(Self {
            channels: vec![channel_a, channel_b],
            next_channel_index: 2, // A=0, B=1 already used
            crossfader: 0.0,
            auto_crossfade: None,
            beat_sync_crossfade: None,
            modulation: ModulationEngine::new(),
            start_time: now,
            last_render_time: now,
            composite_texture,
            composite_view,
            effect_ping_texture,
            effect_ping_view,
            master_effects: Vec::new(),
            frame_count: 0,
            blend_blit_pipelines,
            active_transition: None,
        })
    }

    /// Render all channels and composite them via crossfader, then apply master effects
    pub fn render(&mut self, context: &RenderContext, audio_data: &crate::AudioData) -> Result<()> {
        // Calculate dt
        let now = std::time::Instant::now();
        let dt = (now - self.last_render_time).as_secs_f32();
        self.last_render_time = now;

        // Tick auto-crossfade
        if let Some(auto) = &mut self.auto_crossfade {
            match auto.tick(dt) {
                Some(value) => self.crossfader = value,
                None => {
                    // Crossfade complete
                    let target = auto.to;
                    self.crossfader = target;
                    self.auto_crossfade = None;
                    log::info!("Auto-crossfade complete, crossfader = {:.2}", target);
                }
            }
        }

        // Handle beat-synced crossfade
        if let Some(bsc) = &mut self.beat_sync_crossfade {
            if !bsc.started {
                // Wait for beat boundary (beat_phase near 0)
                let phase = audio_data.beat_phase();
                if phase < 0.05 && audio_data.bpm.is_some() {
                    // Start the crossfade now
                    let bpm = audio_data.bpm.unwrap_or(120.0);
                    let duration_secs = bsc.beats * 60.0 / bpm;
                    bsc.auto = Some(AutoCrossfade::new(
                        self.crossfader, bsc.to, duration_secs, CrossfadeEasing::EaseInOut,
                    ));
                    bsc.started = true;
                    log::info!("Beat-synced crossfade started: {:.1} beats at {:.0} BPM = {:.2}s",
                        bsc.beats, bpm, duration_secs);
                }
            }

            if let Some(auto) = &mut bsc.auto {
                match auto.tick(dt) {
                    Some(value) => self.crossfader = value,
                    None => {
                        let target = bsc.to;
                        self.crossfader = target;
                        self.beat_sync_crossfade = None;
                        log::info!("Beat-synced crossfade complete, crossfader = {:.2}", target);
                    }
                }
            }
        }

        // Update global modulation engine
        let time = self.start_time.elapsed().as_secs_f32();
        let audio_values = AudioValues {
            level: audio_data.level,
            bass: audio_data.bass(),
            mid: audio_data.mid(),
            treble: audio_data.treble(),
        };
        self.modulation.update(time, &audio_values);

        // Render each channel
        for (ch_idx, channel) in self.channels.iter_mut().enumerate() {
            channel.render(context, audio_data, &self.modulation, ch_idx, time)?;
        }

        // Sync transition progress with crossfader before compositing
        self.sync_transition_progress();

        // Composite channels to mixer output using crossfader
        self.composite_channels(context)?;

        // Apply master effects
        self.apply_master_effects(context, audio_data, time)?;

        self.frame_count += 1;
        Ok(())
    }

    fn composite_channels(&mut self, context: &RenderContext) -> Result<()> {
        let channel_count = self.channels.len();
        if channel_count == 0 {
            // Clear composite
            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Mixer Clear Encoder"),
            });
            {
                let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Mixer Clear Pass"),
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
            return Ok(());
        }

        // If we have exactly 2 channels and a transition shader is active, use it
        if channel_count == 2 {
            if let Some(transition) = &self.active_transition {
                let width = self.composite_texture.width();
                let height = self.composite_texture.height();

                let uniforms = ISFUniforms {
                    time: self.start_time.elapsed().as_secs_f32(),
                    time_delta: 1.0 / 60.0,
                    frame_index: self.frame_count,
                    pass_index: 0,
                    render_size: [width as f32, height as f32],
                    ..Default::default()
                };

                // Write crossfader position as the first float in the user params buffer
                let params_data = transition.params.build_buffer_data();
                if let Some(buf) = transition.params.buffer() {
                    context.queue.write_buffer(buf, 0, &params_data);
                }

                transition.pipeline.render_to(
                    context,
                    &self.channels[0].composite_view,
                    &self.channels[1].composite_view,
                    &self.composite_view,
                    &uniforms,
                    transition.params.buffer(),
                );

                return Ok(());
            }
        }

        // Fallback: opacity-based crossfade
        let opacities: Vec<f32> = if channel_count == 2 {
            vec![1.0 - self.crossfader, self.crossfader]
        } else {
            self.channels.iter().map(|ch| ch.opacity).collect()
        };

        // Composite each channel
        for (i, (channel, &opacity)) in self.channels.iter().zip(opacities.iter()).enumerate() {
            if opacity <= 0.0 {
                continue;
            }

            let blend_mode = channel.blend_mode;
            let pipeline = self.blend_blit_pipelines.get(&blend_mode)
                .unwrap_or_else(|| self.blend_blit_pipelines.get(&BlendMode::Normal).unwrap());

            let effective_opacity = opacity * channel.opacity;
            pipeline.set_opacity(&context.queue, effective_opacity);

            let bind_group = pipeline.create_bind_group(&context.device, &channel.composite_view);

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Mixer Composite Encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Mixer Composite Pass"),
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

                pipeline.render(&mut render_pass, &bind_group);
            }

            context.queue.submit(std::iter::once(encoder.finish()));
        }

        Ok(())
    }

    fn apply_master_effects(&mut self, context: &RenderContext, audio_data: &crate::AudioData, time: f32) -> Result<()> {
        if self.master_effects.is_empty() {
            return Ok(());
        }

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
        let mut cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();

        for effect in self.master_effects.iter_mut() {
            if !effect.enabled {
                continue;
            }

            let (input_view, output_view) = if read_from_composite {
                (&self.composite_view, &self.effect_ping_view)
            } else {
                (&self.effect_ping_view, &self.composite_view)
            };

            effect.apply(context, input_view, output_view, &uniforms, &mut cmd_buffers)?;
            read_from_composite = !read_from_composite;
        }

        // If result is in ping texture, copy back to composite
        if !read_from_composite {
            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Master Effect Final Copy Encoder"),
            });
            encoder.copy_texture_to_texture(
                self.effect_ping_texture.as_image_copy(),
                self.composite_texture.as_image_copy(),
                self.composite_texture.size(),
            );
            cmd_buffers.push(encoder.finish());
        }

        // Batch submit all master effects
        if !cmd_buffers.is_empty() {
            context.queue.submit(cmd_buffers);
        }

        Ok(())
    }

    /// Resize mixer and all channel textures
    pub fn resize(&mut self, context: &RenderContext, width: u32, height: u32) {
        self.composite_texture = context.create_render_texture(width, height);
        self.composite_view = self.composite_texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.effect_ping_texture = context.create_render_texture(width, height);
        self.effect_ping_view = self.effect_ping_texture.create_view(&wgpu::TextureViewDescriptor::default());

        for channel in &mut self.channels {
            channel.resize(context, width, height);
        }
    }

    /// Add a master effect
    pub fn add_master_effect(&mut self, effect: Effect) {
        self.master_effects.push(effect);
    }

    /// Remove a master effect by index
    pub fn remove_master_effect(&mut self, index: usize) -> bool {
        if index < self.master_effects.len() {
            self.master_effects.remove(index);
            true
        } else {
            false
        }
    }

    /// Add a new channel with an auto-generated name (C, D, E, ...)
    pub fn add_channel(&mut self, context: &RenderContext, width: u32, height: u32) -> Result<usize> {
        let name = channel_name(self.next_channel_index);
        self.next_channel_index += 1;
        let channel = Channel::new(name, context, width, height)?;
        let idx = self.channels.len();
        self.channels.push(channel);
        log::info!("Added channel {} (index {})", self.channels[idx].name, idx);
        Ok(idx)
    }

    /// Remove a channel by index. Returns true if removed.
    /// Cannot remove below 2 channels (minimum A and B).
    pub fn remove_channel(&mut self, index: usize) -> bool {
        if self.channels.len() <= 2 || index >= self.channels.len() {
            return false;
        }
        let name = self.channels[index].name.clone();
        self.channels.remove(index);
        log::info!("Removed channel {} (was index {})", name, index);
        true
    }

    /// Get a reference to channel by index
    pub fn channel(&self, index: usize) -> Option<&Channel> {
        self.channels.get(index)
    }

    /// Get a mutable reference to channel by index
    pub fn channel_mut(&mut self, index: usize) -> Option<&mut Channel> {
        self.channels.get_mut(index)
    }

    /// Start a timed auto-crossfade to the target value
    pub fn start_crossfade(&mut self, target: f32, duration_secs: f32, easing: CrossfadeEasing) {
        let target = target.clamp(0.0, 1.0);
        if (self.crossfader - target).abs() < 0.001 {
            return; // Already at target
        }
        self.beat_sync_crossfade = None; // Cancel any pending beat-sync
        self.auto_crossfade = Some(AutoCrossfade::new(self.crossfader, target, duration_secs, easing));
        log::info!("Starting auto-crossfade: {:.2} → {:.2} over {:.1}s ({:?})",
            self.crossfader, target, duration_secs, easing);
    }

    /// Start a beat-synced crossfade (waits for next beat boundary, then transitions over N beats)
    pub fn start_beat_crossfade(&mut self, target: f32, beats: f32) {
        let target = target.clamp(0.0, 1.0);
        self.auto_crossfade = None; // Cancel any timed crossfade
        self.beat_sync_crossfade = Some(BeatSyncCrossfade {
            to: target,
            beats,
            started: false,
            auto: None,
        });
        log::info!("Queued beat-synced crossfade: → {:.2} over {:.1} beats", target, beats);
    }

    /// Snap crossfader to a value immediately (cancels any in-progress transitions)
    pub fn snap_crossfader(&mut self, value: f32) {
        self.crossfader = value.clamp(0.0, 1.0);
        self.auto_crossfade = None;
        self.beat_sync_crossfade = None;
    }

    /// Whether a crossfade transition is currently in progress
    pub fn is_crossfading(&self) -> bool {
        self.auto_crossfade.is_some() || self.beat_sync_crossfade.as_ref().map_or(false, |b| b.started)
    }

    /// Set the active transition shader. Compiles the shader and creates the pipeline.
    /// The `progress` parameter is automatically synced from the crossfader position.
    pub fn set_transition(
        &mut self,
        context: &RenderContext,
        shader: ISFShader,
    ) -> Result<()> {
        let name = shader.name();
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &name)
            .context("Failed to compile transition shader")?;

        let target_format = wgpu::TextureFormat::Rgba8Unorm;
        let pipeline = TransitionPipeline::new(&context.device, &spirv, target_format)
            .context("Failed to create transition pipeline")?;

        // Build params — filter out image inputs, keep floats/bools/etc.
        let inputs = shader.metadata.inputs.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
        let mut params = ShaderParams::from_inputs(inputs);
        params.ensure_buffer(&context.device);

        log::info!("Active transition set: {}", name);
        self.active_transition = Some(TransitionEffect {
            shader,
            pipeline,
            params,
            name,
        });
        Ok(())
    }

    /// Clear the active transition (revert to opacity-based crossfade)
    pub fn clear_transition(&mut self) {
        if self.active_transition.is_some() {
            log::info!("Transition cleared, reverting to opacity crossfade");
        }
        self.active_transition = None;
    }

    /// Sync the transition's `progress` parameter with the crossfader value.
    /// Called automatically during render.
    fn sync_transition_progress(&mut self) {
        if let Some(transition) = &mut self.active_transition {
            transition.params.set("progress", crate::params::ParamValue::Float(self.crossfader));
        }
    }
}


/// Generate a channel name from its index: 0→"A", 1→"B", 2→"C", ..., 25→"Z", 26→"A2", etc.
fn channel_name(index: usize) -> String {
    let letter = (b'A' + (index % 26) as u8) as char;
    if index < 26 {
        letter.to_string()
    } else {
        format!("{}{}", letter, index / 26 + 1)
    }
}