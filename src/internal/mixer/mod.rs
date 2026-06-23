//! Mixer - Top-level compositor that owns channels, crossfader, master effects, and modulation

mod render;
mod transition;

pub use transition::{
    AutoCrossfade, BeatSyncCrossfade, CrossfadeEasing, SequencerState, StepKind, TransitionEffect,
    TransitionSequence, TransitionStep,
};

use crate::channel::Channel;
use crate::deck::Effect;
use crate::modulation::ModulationEngine;
use crate::renderer::lut::{LoadedLut, LutPipeline};
pub use crate::renderer::tonemap::TonemapMode;
use crate::renderer::{BlitPipeline, CompositeBlitPipeline, GpuContext, TonemapPipeline};
use anyhow::Result;

/// Per-frame GPU timing allocation context.
/// Hands out (begin_query, end_query) index pairs from a shared QuerySet.
pub struct GpuTimingFrame {
    /// Maximum number of queries in the set (must be even: pairs of begin/end)
    max_queries: u32,
    /// Next available query index
    next_index: u32,
    /// Records which (ch_idx, deck_idx) owns which query pair
    pub allocations: Vec<(usize, usize, u32, u32)>,
}

impl GpuTimingFrame {
    pub fn new(max_queries: u32) -> Self {
        Self {
            max_queries,
            next_index: 0,
            allocations: Vec::new(),
        }
    }

    /// Allocate a (begin, end) query index pair for a deck.
    /// Returns None if capacity exhausted.
    pub fn allocate(&mut self, ch_idx: usize, deck_idx: usize) -> Option<(u32, u32)> {
        if self.next_index + 2 > self.max_queries {
            return None;
        }
        let begin = self.next_index;
        let end = self.next_index + 1;
        self.next_index += 2;
        self.allocations.push((ch_idx, deck_idx, begin, end));
        Some((begin, end))
    }

    /// Number of queries actually written this frame.
    pub fn query_count(&self) -> u32 {
        self.next_index
    }
}

/// Mixer - Top-level compositor
pub struct Mixer {
    /// Channels (default 2: A and B)
    channels: Vec<Channel>,

    /// Monotonic counter for generating unique channel names (never decremented)
    next_channel_index: usize,

    /// Crossfader position (0.0 = Ch 0, 1.0 = Ch 1)
    crossfader: f32,

    /// Active auto-crossfade (if any)
    auto_crossfade: Option<AutoCrossfade>,

    /// Pending beat-synced crossfade (if any)
    beat_sync_crossfade: Option<BeatSyncCrossfade>,

    /// Global modulation engine
    modulation: ModulationEngine,

    /// Start time for TIME-based modulation
    start_time: std::time::Instant,

    /// Last render time for dt calculation
    last_render_time: std::time::Instant,

    /// Composite output texture (all channels mixed, pre-master effects)
    composite_texture: wgpu::Texture,
    composite_view: wgpu::TextureView,

    /// Ping-pong texture for master effect chain
    effect_ping_texture: wgpu::Texture,
    effect_ping_view: wgpu::TextureView,

    /// Master effect chain (applied to final composite)
    master_effects: Vec<Effect>,

    /// Frame counter
    frame_count: u32,

    /// Smoothed GPU load ratio (EMA): actual_frame_time / cpu_render_time.
    /// When > 1.0, GPU execution takes longer than CPU encoding — shaders are
    /// GPU-bound and render_cost_us underestimates true cost by this factor.
    gpu_load_ratio: f32,

    /// Smoothed GPU utilization % (0–100): sum of per-deck GPU render costs
    /// divided by frame budget. Uses GPU timestamp data when available,
    /// falls back to CPU-measured render cost × gpu_load_ratio.
    gpu_utilization: f32,

    /// Shader-based composite pipeline for blending channels (all blend modes via uniform)
    composite_pipeline: CompositeBlitPipeline,

    /// Simple blit pipeline for first-channel copy
    blit_pipeline: BlitPipeline,

    /// Tonemap pipeline (bypass/ACES) applied after master effects
    tonemap_pipeline: TonemapPipeline,

    /// Current tonemap mode
    tonemap_mode: TonemapMode,

    /// LUT pipeline for applying 3D LUTs after tonemapping
    lut_pipeline: LutPipeline,

    /// Currently loaded LUT (applied after tonemap, before output)
    active_lut: Option<LoadedLut>,

    /// Active transition effect (replaces opacity-based crossfade when set)
    active_transition: Option<TransitionEffect>,

    /// Transition sequences (channel-to-channel automation). Multiple named sequences supported.
    transition_sequences: Vec<TransitionSequence>,

    /// Cached sub-mix textures for multi-channel surface assignments.
    /// Key: sorted channel indices, Value: (texture, view).
    sub_mix_cache: std::collections::HashMap<Vec<usize>, (wgpu::Texture, wgpu::TextureView)>,
    /// Cached tonemapped copies of individual channel composites.
    /// Used when surfaces source from Channel(idx) — the raw channel composite
    /// can't be tonemapped in-place since it feeds into the mixer composite.
    tonemapped_channel_cache: std::collections::HashMap<usize, (wgpu::Texture, wgpu::TextureView)>,

    /// GPU performance profiling: when > 0, insert device.poll(Wait) between
    /// GPU work stages to measure actual GPU drain time per category.
    /// Auto-decrements each frame until 0 (self-disabling).
    pub perf_profile_frames: u32,

    /// GPU timestamp query set (128 queries = 64 deck measurements)
    pub(crate) query_set: Option<wgpu::QuerySet>,
    /// Buffer for resolving query results (QUERY_RESOLVE | COPY_SRC)
    resolve_buffer: Option<wgpu::Buffer>,
    /// Double-buffered staging buffers for readback (COPY_DST | MAP_READ)
    staging_buffers: Option<[wgpu::Buffer; 2]>,
    /// Which staging buffer to write next (alternates 0/1)
    staging_index: usize,
    /// Nanoseconds per timestamp tick (from queue.get_timestamp_period())
    timestamp_period: f32,
    /// Per-deck GPU times from the previous frame: (ch_idx, deck_idx) -> microseconds
    pub(crate) last_frame_gpu_times: std::collections::HashMap<(usize, usize), f32>,
    /// Timing allocations from the frame whose results are in the readable staging buffer
    prev_timing_allocations: Vec<(usize, usize, u32, u32)>,
    /// Index of the staging buffer whose map_async has completed (ready to read).
    /// `usize::MAX` means no buffer is pending/ready. Set by the map_async callback.
    staging_mapped_idx: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    /// True while a timing-staging `map_async` is outstanding — from the moment it
    /// is *issued* until the read path consumes and unmaps the buffer. Gates the
    /// resolve/readback path so at most one map is ever in flight. The callback
    /// only sets `staging_mapped_idx` later, so relying on that alone would let a
    /// second map_async be issued during the pending window, leaving a buffer
    /// permanently mapped and crashing the next submit with "still mapped".
    timing_map_inflight: bool,
}

impl Mixer {
    /// Create a new mixer with two default channels (A and B)
    pub fn new(context: &GpuContext, width: u32, height: u32) -> Result<Self> {
        let composite_texture = context.create_compositing_texture(width, height);
        let composite_view = composite_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let effect_ping_texture = context.create_compositing_texture(width, height);
        let effect_ping_view =
            effect_ping_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let composite_pipeline =
            CompositeBlitPipeline::new(&context.device, context.compositing_format)?;
        let blit_pipeline = BlitPipeline::with_blend(
            &context.device,
            context.compositing_format,
            wgpu::BlendState::ALPHA_BLENDING,
        )?;
        let tonemap_pipeline = TonemapPipeline::new(&context.device, context.compositing_format)?;
        let lut_pipeline = LutPipeline::new(&context.device, context.compositing_format)?;

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
            gpu_load_ratio: 1.0,
            gpu_utilization: 0.0,
            composite_pipeline,
            blit_pipeline,
            tonemap_pipeline,
            tonemap_mode: TonemapMode::default(),
            lut_pipeline,
            active_lut: None,
            active_transition: None,
            transition_sequences: Vec::new(),
            sub_mix_cache: std::collections::HashMap::new(),
            tonemapped_channel_cache: std::collections::HashMap::new(),
            perf_profile_frames: 0,
            query_set: if context.timestamp_supported {
                Some(context.device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: Some("GPU Timing QuerySet"),
                    ty: wgpu::QueryType::Timestamp,
                    count: 128,
                }))
            } else {
                None
            },
            resolve_buffer: if context.timestamp_supported {
                Some(context.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("GPU Timing Resolve"),
                    size: 128 * 8, // 128 queries * 8 bytes each
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                }))
            } else {
                None
            },
            staging_buffers: if context.timestamp_supported {
                Some([
                    context.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("GPU Timing Staging 0"),
                        size: 128 * 8,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                    context.device.create_buffer(&wgpu::BufferDescriptor {
                        label: Some("GPU Timing Staging 1"),
                        size: 128 * 8,
                        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                        mapped_at_creation: false,
                    }),
                ])
            } else {
                None
            },
            staging_index: 0,
            timestamp_period: if context.timestamp_supported {
                context.queue.get_timestamp_period()
            } else {
                0.0
            },
            last_frame_gpu_times: std::collections::HashMap::new(),
            prev_timing_allocations: Vec::new(),
            staging_mapped_idx: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(
                usize::MAX,
            )),
            timing_map_inflight: false,
        })
    }

    /// Resize mixer and all channel textures
    pub fn resize(&mut self, context: &GpuContext, width: u32, height: u32) {
        self.composite_texture = context.create_compositing_texture(width, height);
        self.composite_view = self
            .composite_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.effect_ping_texture = context.create_compositing_texture(width, height);
        self.effect_ping_view = self
            .effect_ping_texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        for channel in &mut self.channels {
            channel.resize(context, width, height);
        }
        self.sub_mix_cache.clear();
        self.tonemapped_channel_cache.clear();
    }

    /// Clear the sub-mix texture cache (e.g. after resolution change).
    pub fn clear_sub_mix_cache(&mut self) {
        self.sub_mix_cache.clear();
    }

    /// Get the tonemapped view for a channel, if available.
    /// Returns None in Bypass mode or if the channel isn't used as a direct source.
    pub fn get_tonemapped_channel_view(&self, ch_idx: usize) -> Option<&wgpu::TextureView> {
        self.tonemapped_channel_cache.get(&ch_idx).map(|(_, v)| v)
    }

    /// Start GPU performance profiling for the next N frames.
    /// Inserts device.poll(Wait) between GPU stages to measure actual GPU
    /// execution time per category. Logs every frame (not every 120).
    pub fn start_perf_profile(&mut self, frames: u32) {
        self.perf_profile_frames = frames;
        log::info!(
            "[PERF_PROFILE] Starting GPU profiling for {} frames",
            frames
        );
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

    // ── Accessor methods ─────────────────────────────────────────────

    /// Read-only access to all channels.
    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    /// Mutable access to all channels.
    pub fn channels_mut(&mut self) -> &mut Vec<Channel> {
        &mut self.channels
    }

    /// Number of channels.
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    /// Current crossfader position (0.0 = Ch 0, 1.0 = Ch 1).
    pub fn crossfader(&self) -> f32 {
        self.crossfader
    }

    /// Read-only access to the auto-crossfade state.
    pub fn auto_crossfade(&self) -> Option<&AutoCrossfade> {
        self.auto_crossfade.as_ref()
    }

    /// Read-only access to master effects.
    pub fn master_effects(&self) -> &[Effect] {
        &self.master_effects
    }

    /// Mutable access to master effects.
    pub fn master_effects_mut(&mut self) -> &mut Vec<Effect> {
        &mut self.master_effects
    }

    /// Read-only access to the modulation engine.
    pub fn modulation(&self) -> &ModulationEngine {
        &self.modulation
    }

    /// Mutable access to the modulation engine.
    pub fn modulation_mut(&mut self) -> &mut ModulationEngine {
        &mut self.modulation
    }

    /// Read-only access to the active transition effect.
    pub fn active_transition(&self) -> Option<&TransitionEffect> {
        self.active_transition.as_ref()
    }

    /// Read-only access to transition sequences.
    pub fn transition_sequences(&self) -> &[TransitionSequence] {
        &self.transition_sequences
    }

    /// Mutable access to transition sequences.
    pub fn transition_sequences_mut(&mut self) -> &mut Vec<TransitionSequence> {
        &mut self.transition_sequences
    }

    /// The composited output texture view (post-crossfade, post-master-effects, post-tonemap).
    pub fn composite_view(&self) -> &wgpu::TextureView {
        &self.composite_view
    }

    /// Current tonemap mode.
    pub fn tonemap_mode(&self) -> TonemapMode {
        self.tonemap_mode
    }

    /// Set tonemap mode and update GPU uniform.
    pub fn set_tonemap_mode(&mut self, queue: &wgpu::Queue, mode: TonemapMode) {
        self.tonemap_mode = mode;
        self.tonemap_pipeline.set_mode(queue, mode);
    }

    /// Load a LUT from a parsed file and upload to GPU.
    pub fn load_lut(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        parsed: &crate::renderer::lut::ParsedLut,
        filename: String,
    ) {
        self.active_lut = Some(LoadedLut::from_parsed(device, queue, parsed, filename));
    }

    /// Unload the active LUT.
    pub fn unload_lut(&mut self) {
        self.active_lut = None;
    }

    /// Get the active LUT filename (if any).
    pub fn active_lut_filename(&self) -> Option<&str> {
        self.active_lut.as_ref().map(|l| l.filename.as_str())
    }

    /// Whether a LUT is currently active.
    pub fn has_active_lut(&self) -> bool {
        self.active_lut.is_some()
    }

    /// GPU utilization % (0–100), smoothed. Based on GPU timestamp data
    /// (sum of per-deck GPU costs / frame budget).
    pub fn gpu_utilization(&self) -> f32 {
        self.gpu_utilization
    }

    // ── UUID lookup helpers ────────────────────────────────────────────

    /// Find a mutable deck slot by deck UUID. Returns (channel_index, deck_index) if found.
    pub fn find_deck_by_uuid(&self, uuid: &str) -> Option<(usize, usize)> {
        for (ch_idx, ch) in self.channels.iter().enumerate() {
            for (dk_idx, slot) in ch.decks.iter().enumerate() {
                if slot.deck.uuid() == uuid {
                    return Some((ch_idx, dk_idx));
                }
            }
        }
        None
    }

    /// Find a channel index by channel UUID.
    pub fn find_channel_by_uuid(&self, uuid: &str) -> Option<usize> {
        self.channels.iter().position(|ch| ch.uuid() == uuid)
    }

    // ── Persistence restore helpers ──────────────────────────────────

    /// Replace all channels (used by persistence restore).
    /// Also updates next_channel_index based on the highest "Ch N" name.
    pub fn replace_channels(&mut self, channels: Vec<Channel>) {
        let max_idx = channels
            .iter()
            .filter_map(|ch| {
                ch.name
                    .strip_prefix("Ch ")
                    .and_then(|s| s.parse::<usize>().ok())
            })
            .max()
            .map(|n| n + 1)
            .unwrap_or(channels.len());
        self.next_channel_index = max_idx;
        self.channels = channels;
    }

    /// Set the crossfader position directly (used by persistence restore).
    pub fn set_crossfader(&mut self, value: f32) {
        self.crossfader = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.5
        };
    }

    /// Replace the modulation engine (used by persistence restore).
    pub fn set_modulation(&mut self, engine: ModulationEngine) {
        self.modulation = engine;
    }

    /// Replace transition sequences (used by persistence restore).
    pub fn set_transition_sequences(&mut self, sequences: Vec<TransitionSequence>) {
        self.transition_sequences = sequences;
    }

    /// Set the next_channel_index counter (used by persistence restore).
    pub fn set_next_channel_index(&mut self, idx: usize) {
        self.next_channel_index = idx;
    }

    /// Consume the next channel name and advance the counter.
    /// Use this when manually constructing a channel outside of `add_channel`.
    pub fn take_next_channel_name(&mut self) -> String {
        let name = channel_name(self.next_channel_index);
        self.next_channel_index += 1;
        name
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
        state.step_elapsed = 2.5;
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

    // ── Mixer-level DnD data model tests ─────────────────────────────
    //
    // Tests for cross-channel deck moves and master effect reordering,
    // matching the logic in apply_deck_and_effect_actions.

    use crate::renderer::GpuContext;

    fn headless_gpu() -> GpuContext {
        GpuContext::new_headless().expect("headless GPU required for tests")
    }

    #[test]
    fn mixer_new_has_two_channels() {
        let gpu = headless_gpu();
        let mixer = Mixer::new(&gpu, 64, 64).unwrap();
        assert_eq!(mixer.channel_count(), 2);
    }

    #[test]
    fn mixer_add_channel() {
        let gpu = headless_gpu();
        let mut mixer = Mixer::new(&gpu, 64, 64).unwrap();
        let idx = mixer.add_channel(&gpu, 64, 64).unwrap();
        assert_eq!(idx, 2);
        assert_eq!(mixer.channel_count(), 3);
    }

    #[test]
    fn mixer_move_deck_between_channels() {
        let gpu = headless_gpu();
        let mut mixer = Mixer::new(&gpu, 64, 64).unwrap();

        // Add a solid color deck to channel 0
        let deck = crate::deck::Deck::new_solid_color(&gpu, [1.0, 0.0, 0.0, 1.0], 64, 64).unwrap();
        mixer.channel_mut(0).unwrap().add_deck(deck);
        mixer.channel_mut(0).unwrap().decks[0].opacity = 0.33;

        assert_eq!(mixer.channel(0).unwrap().deck_count(), 1);
        assert_eq!(mixer.channel(1).unwrap().deck_count(), 0);

        // Move deck from ch0 to ch1 (mirrors apply_deck_and_effect_actions logic)
        let slot = mixer.channels_mut()[0].remove_deck_slot(0).unwrap();
        let new_idx = mixer.channels_mut()[1].add_deck_slot(slot);

        assert_eq!(new_idx, 0);
        assert_eq!(mixer.channel(0).unwrap().deck_count(), 0);
        assert_eq!(mixer.channel(1).unwrap().deck_count(), 1);
        assert!((mixer.channel(1).unwrap().decks[0].opacity - 0.33).abs() < 1e-5);
    }

    #[test]
    fn mixer_master_effect_reorder() {
        // Master effects are Vec<Effect> — test the vec reorder pattern
        // used in apply_deck_and_effect_actions
        let mut effects = vec!["master_blur", "master_color", "master_feedback"];
        // Move last to first (from=2, to=0)
        let e = effects.remove(2);
        effects.insert(0, e);
        assert_eq!(
            effects,
            vec!["master_feedback", "master_blur", "master_color"]
        );
    }

    // ── Chaos Tests Round 2: Crossfader/opacity arithmetic ──────────────

    #[test]
    fn chaos_crossfader_opacity_arithmetic_oob() {
        // Simulate the opacity calculation from composite_sub_mix
        let crossfader = 1.5_f32;
        let opacities = [0.8_f32, 0.9];
        let op_a = (1.0 - crossfader) * opacities[0]; // -0.5 * 0.8 = -0.4
        let op_b = crossfader * opacities[1]; // 1.5 * 0.9 = 1.35
        assert!(op_a.is_finite() && op_b.is_finite());
    }

    #[test]
    fn chaos_crossfader_nan_arithmetic() {
        let crossfader = f32::NAN;
        let opacity = 0.8_f32;
        let result = (1.0 - crossfader) * opacity;
        // NaN propagates — document this behavior
        assert!(result.is_nan(), "NaN crossfader should propagate NaN");
    }

    #[test]
    fn chaos_crossfader_infinity_arithmetic() {
        let crossfader = f32::INFINITY;
        let opacity = 0.8_f32;
        let result = (1.0 - crossfader) * opacity;
        assert!(result.is_infinite(), "Inf crossfader produces Inf opacity");
    }

    #[test]
    fn chaos_opacity_nan_does_not_panic() {
        let opacity = f32::NAN;
        let crossfader = 0.5_f32;
        let result = crossfader * opacity;
        assert!(result.is_nan());
    }
}
