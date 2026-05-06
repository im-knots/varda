//! Mixer - Top-level compositor that owns channels, crossfader, master effects, and modulation

mod transition;
mod render;

pub use transition::{
    CrossfadeEasing, AutoCrossfade, BeatSyncCrossfade,
    TransitionSequence, TransitionStep, StepKind, SequencerState,
    TransitionEffect,
};

use crate::channel::{Channel, BlendMode};
use crate::deck::Effect;
use crate::modulation::ModulationEngine;
use crate::renderer::{GpuContext, BlitPipeline};
use anyhow::Result;

/// Mixer - Top-level compositor
pub struct Mixer {
    /// Channels (default 2: A and B)
    pub channels: Vec<Channel>,

    /// Monotonic counter for generating unique channel names (never decremented)
    pub(crate) next_channel_index: usize,

    /// Crossfader position (0.0 = Ch 0, 1.0 = Ch 1)
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

    /// Transition sequences (channel-to-channel automation). Multiple named sequences supported.
    pub transition_sequences: Vec<TransitionSequence>,

    /// Cached sub-mix textures for multi-channel surface assignments.
    /// Key: sorted channel indices, Value: (texture, view).
    sub_mix_cache: std::collections::HashMap<Vec<usize>, (wgpu::Texture, wgpu::TextureView)>,
}

impl Mixer {
    /// Create a new mixer with two default channels (A and B)
    pub fn new(context: &GpuContext, width: u32, height: u32) -> Result<Self> {
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
                context.texture_format,
                mode.to_blend_state(),
            )?;
            blend_blit_pipelines.insert(mode, pipeline);
        }

        // Create two default channels
        let channel_0 = Channel::new("Ch 0".to_string(), context, width, height)?;
        let channel_1 = Channel::new("Ch 1".to_string(), context, width, height)?;

        let now = std::time::Instant::now();
        Ok(Self {
            channels: vec![channel_0, channel_1],
            next_channel_index: 2, // Ch 0, Ch 1 already used
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
            transition_sequences: Vec::new(),
            sub_mix_cache: std::collections::HashMap::new(),
        })
    }

    /// Resize mixer and all channel textures
    pub fn resize(&mut self, context: &GpuContext, width: u32, height: u32) {
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
    pub fn add_channel(&mut self, context: &GpuContext, width: u32, height: u32) -> Result<usize> {
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

}

/// Generate a channel name from its index: 0→"Ch 0", 1→"Ch 1", 2→"Ch 2", etc.
fn channel_name(index: usize) -> String {
    format!("Ch {}", index)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── CrossfadeEasing tests ────────────────────────────────────────

    #[test]
    fn easing_linear() {
        assert!((CrossfadeEasing::Linear.apply(0.0) - 0.0).abs() < 1e-5);
        assert!((CrossfadeEasing::Linear.apply(0.5) - 0.5).abs() < 1e-5);
        assert!((CrossfadeEasing::Linear.apply(1.0) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn easing_ease_in_out() {
        let e = CrossfadeEasing::EaseInOut;
        assert!((e.apply(0.0) - 0.0).abs() < 1e-5);
        assert!((e.apply(1.0) - 1.0).abs() < 1e-5);
        // Midpoint of smoothstep = 0.5
        assert!((e.apply(0.5) - 0.5).abs() < 1e-5);
        // Should be slow at start (below linear)
        assert!(e.apply(0.25) < 0.25);
    }

    #[test]
    fn easing_ease_in() {
        let e = CrossfadeEasing::EaseIn;
        assert!((e.apply(0.0) - 0.0).abs() < 1e-5);
        assert!((e.apply(1.0) - 1.0).abs() < 1e-5);
        // Ease-in is t² → slower at start
        assert!(e.apply(0.5) < 0.5);
        assert!((e.apply(0.5) - 0.25).abs() < 1e-5);
    }

    #[test]
    fn easing_ease_out() {
        let e = CrossfadeEasing::EaseOut;
        assert!((e.apply(0.0) - 0.0).abs() < 1e-5);
        assert!((e.apply(1.0) - 1.0).abs() < 1e-5);
        // Ease-out: 1-(1-t)² → faster at start
        assert!(e.apply(0.5) > 0.5);
    }

    #[test]
    fn easing_clamps_input() {
        assert!((CrossfadeEasing::Linear.apply(-0.5) - 0.0).abs() < 1e-5);
        assert!((CrossfadeEasing::Linear.apply(1.5) - 1.0).abs() < 1e-5);
    }

    // ── AutoCrossfade tests ──────────────────────────────────────────

    #[test]
    fn auto_crossfade_new() {
        let ac = AutoCrossfade::new(0.0, 1.0, 2.0, CrossfadeEasing::Linear);
        assert_eq!(ac.from, 0.0);
        assert_eq!(ac.to, 1.0);
        assert_eq!(ac.duration, 2.0);
        assert_eq!(ac.elapsed, 0.0);
    }

    #[test]
    fn auto_crossfade_tick_returns_value() {
        let mut ac = AutoCrossfade::new(0.0, 1.0, 2.0, CrossfadeEasing::Linear);
        let val = ac.tick(0.5);
        assert!(val.is_some());
        let v = val.unwrap();
        assert!((v - 0.25).abs() < 1e-5); // 25% through linear
    }

    #[test]
    fn auto_crossfade_tick_completes() {
        let mut ac = AutoCrossfade::new(0.0, 1.0, 1.0, CrossfadeEasing::Linear);
        let val = ac.tick(1.5); // Past duration
        assert!(val.is_none()); // Complete
    }

    #[test]
    fn auto_crossfade_tick_exact_duration() {
        let mut ac = AutoCrossfade::new(0.0, 1.0, 1.0, CrossfadeEasing::Linear);
        let val = ac.tick(1.0);
        assert!(val.is_none()); // Complete at exact duration
    }

    #[test]
    fn auto_crossfade_reverse() {
        let mut ac = AutoCrossfade::new(1.0, 0.0, 2.0, CrossfadeEasing::Linear);
        let val = ac.tick(1.0).unwrap();
        assert!((val - 0.5).abs() < 1e-5); // Halfway back
    }

    #[test]
    fn auto_crossfade_progress() {
        let mut ac = AutoCrossfade::new(0.0, 1.0, 4.0, CrossfadeEasing::Linear);
        assert!((ac.progress() - 0.0).abs() < 1e-5);
        ac.tick(2.0);
        assert!((ac.progress() - 0.5).abs() < 1e-5);
        ac.tick(2.0);
        assert!((ac.progress() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn auto_crossfade_with_easing() {
        let mut ac = AutoCrossfade::new(0.0, 1.0, 2.0, CrossfadeEasing::EaseInOut);
        let val = ac.tick(1.0).unwrap(); // 50% through with ease-in-out
        // Smoothstep at 0.5 = 0.5
        assert!((val - 0.5).abs() < 1e-5);
    }

    // ── SequencerState tests ─────────────────────────────────────────

    #[test]
    fn sequencer_state_new() {
        let state = SequencerState::new();
        assert!(!state.playing);
        assert_eq!(state.current_step, 0);
        assert_eq!(state.step_elapsed, 0.0);
    }

    #[test]
    fn sequencer_state_reset() {
        let mut state = SequencerState::new();
        state.playing = true;
        state.current_step = 5;
        state.step_elapsed = 3.14;
        state.reset();
        assert!(!state.playing);
        assert_eq!(state.current_step, 0);
        assert_eq!(state.step_elapsed, 0.0);
    }

    // ── TransitionSequence tests ─────────────────────────────────────

    #[test]
    fn transition_sequence_new() {
        let seq = TransitionSequence::new("Test".into());
        assert_eq!(seq.name, "Test");
        assert!(seq.enabled);
        assert!(seq.steps.is_empty());
        assert!(!seq.state.playing);
    }

    // ── channel_name tests ───────────────────────────────────────────

    #[test]
    fn channel_name_format() {
        assert_eq!(channel_name(0), "Ch 0");
        assert_eq!(channel_name(1), "Ch 1");
        assert_eq!(channel_name(42), "Ch 42");
    }
}