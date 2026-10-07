//! Channel: groups decks into a composited layer with its own effect chain.

use crate::arrangement::SourceDemand;
use crate::deck::{Deck, Effect, FramePacing};
use crate::isf::ISFShader;
use crate::modulation::ModulationEngine;
use crate::params::ShaderParams;
use crate::renderer::{
    BlitPipeline, CompositeBlitPipeline, GpuContext, ISFUniforms, TransitionPipeline,
};
use anyhow::Result;

/// Blend modes for compositing decks and channels
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
    utoipa::ToSchema,
)]
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
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
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
    #[must_use]
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
            DurationSpec::Beats(v)
            | DurationSpec::Seconds(v)
            | DurationSpec::Minutes(v)
            | DurationSpec::Hours(v) => *v,
        }
    }

    pub fn is_beats(&self) -> bool {
        matches!(self, DurationSpec::Beats(_))
    }

    /// Get the unit of this duration.
    pub fn unit(&self) -> DurationUnit {
        match self {
            DurationSpec::Seconds(_) => DurationUnit::Seconds,
            DurationSpec::Minutes(_) => DurationUnit::Minutes,
            DurationSpec::Hours(_) => DurationUnit::Hours,
            DurationSpec::Beats(_) => DurationUnit::Beats,
        }
    }

    /// Create a `DurationSpec` from a value and unit.
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
            DurationSpec::Beats(b) => *b = v,
            DurationSpec::Seconds(s) => *s = v,
            DurationSpec::Minutes(m) => *m = v,
            DurationSpec::Hours(h) => *h = v,
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

impl Default for DeckAutoTransition {
    fn default() -> Self {
        Self::new()
    }
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

/// Per-deck render FPS setting.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum DeckRenderFps {
    /// Automatic adaptive skipping based on render cost
    #[default]
    Auto,
    /// Fixed render rate (renders every Nth frame to hit target)
    Fixed(u32),
}

impl std::fmt::Display for DeckRenderFps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Auto => write!(f, "Auto"),
            Self::Fixed(fps) => write!(f, "{fps}"),
        }
    }
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
    /// A picked transition shader still building.
    pending_transition: Option<crate::renderer::PendingTransition>,
    /// Per-deck render FPS setting (Auto = adaptive skipping)
    pub render_fps: DeckRenderFps,
    /// Smoothed render cost in microseconds (EMA)
    pub render_cost_us: f32,
    /// Frame counter for skip logic
    pub skip_counter: u32,
    /// GPU-measured render cost in microseconds (EMA, 0 = no data yet)
    pub gpu_render_cost_us: f32,
    /// True while the arrangement drives this deck, suspending its auto-transition. Recomputed
    /// every frame by `Mixer::apply_arrangement`; never persisted.
    pub arrangement_authority: bool,
    /// Whether the arrangement wants this deck's source soon, or it is far enough from any region
    /// to stop decoding. Recomputed every frame alongside `arrangement_authority`; never
    /// persisted.
    pub source_demand: SourceDemand,
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
            pending_transition: None,
            render_fps: DeckRenderFps::default(),
            render_cost_us: 0.0,
            skip_counter: 0,
            gpu_render_cost_us: 0.0,
            arrangement_authority: false,
            source_demand: SourceDemand::default(),
        }
    }

    /// Pick the transition shader for this deck's auto-transition. The choice is recorded at
    /// once; the shader builds off the render thread and the current one stays in use until
    /// [`Self::poll_transition_build`] finds it ready.
    pub fn set_transition_shader(&mut self, context: &GpuContext, shader: ISFShader) {
        let at = self
            .auto_transition
            .get_or_insert_with(DeckAutoTransition::new);
        at.transition_shader_name = Some(shader.name());
        self.pending_transition = Some(crate::renderer::PendingTransition::spawn(context, shader));
    }

    /// The transition shader picked last: one still building, else the one in use.
    pub fn chosen_transition_shader(&self) -> Option<String> {
        self.pending_transition
            .as_ref()
            .map(|p| p.shader.name())
            .or_else(|| self.transition_effect.as_ref().map(|t| t.shader.name()))
    }

    /// Use a finished transition build.
    pub fn poll_transition_build(&mut self, context: &GpuContext) {
        let Some(result) = self
            .pending_transition
            .as_ref()
            .and_then(crate::renderer::PendingTransition::poll)
        else {
            return;
        };
        let Some(pending) = self.pending_transition.take() else {
            return;
        };
        match result {
            Ok(pipeline) => {
                let inputs = pending.shader.metadata.inputs.as_deref().unwrap_or(&[]);
                let mut params = ShaderParams::from_inputs(inputs);
                params.ensure_buffer(&context.device);
                self.transition_effect = Some(DeckTransitionEffect {
                    shader: pending.shader,
                    pipeline,
                    params,
                });
            }
            Err(e) => log::warn!(
                "Transition '{}' failed to build: {e:#}",
                pending.shader.name()
            ),
        }
    }

    /// Clear the transition shader (revert to opacity fade).
    pub fn clear_transition_shader(&mut self) {
        self.transition_effect = None;
        self.pending_transition = None;
        if let Some(at) = &mut self.auto_transition {
            at.transition_shader_name = None;
        }
    }

    /// Get the current auto-transition phase.
    pub fn transition_phase(&self) -> DeckTransitionPhase {
        self.auto_transition
            .as_ref()
            .filter(|at| at.enabled)
            .map_or(DeckTransitionPhase::Inactive, |at| at.phase)
    }
}

/// Per-frame GPU timing allocation context.
/// Hands out (`begin_query`, `end_query`) index pairs from a shared `QuerySet`.
pub struct GpuTimingFrame {
    /// Maximum number of queries in the set (must be even: pairs of begin/end)
    max_queries: u32,
    /// Next available query index
    next_index: u32,
    /// Records which (`ch_idx`, `deck_idx`) owns which query pair
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

/// One frame's timing as the mixer hands it to a channel.
#[derive(Debug, Clone, Copy)]
pub struct FrameClock {
    /// Mixer time in seconds.
    pub time: f32,
    /// Frame delta in seconds.
    pub dt: f32,
    /// Whether the wall clock paces this frame. Offline renders are not live.
    pub live: bool,
}

/// Channel: groups decks into a composited layer.
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

    /// Previous frame's composite, kept only while a deck taps this channel. Swapped with
    /// `composite_texture` each frame, so reading it costs no copy and always yields frame N-1.
    tap_prev: Option<(wgpu::Texture, wgpu::TextureView)>,

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
    /// Stable UUID for this channel.
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// Set the UUID (used during scene restore to preserve identity)
    pub fn set_uuid(&mut self, uuid: String) {
        self.uuid = uuid;
    }

    /// Create a new channel.
    ///
    /// # Errors
    ///
    /// Returns an error if the composite-blit or alpha-blend blit pipelines cannot
    /// be created on the given GPU device.
    pub fn new(name: String, context: &GpuContext, width: u32, height: u32) -> Result<Self> {
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

        Ok(Self {
            uuid: crate::ids::generate_short_uuid(),
            name,
            decks: Vec::new(),
            effects: Vec::new(),
            opacity: 1.0,
            blend_mode: BlendMode::Normal,
            composite_texture,
            composite_view,
            effect_ping_texture,
            effect_ping_view,
            tap_prev: None,
            frame_count: 0,
            composite_pipeline,
            blit_pipeline,
            render_time_ms: 0.0,
            active_deck_count: 0,
        })
    }

    /// Previous frame's composite, or `None` when nothing taps this channel.
    pub fn tap_view(&self) -> Option<&wgpu::TextureView> {
        self.tap_prev.as_ref().map(|(_, v)| v)
    }

    /// Allocate the tap target if it does not exist yet. Idempotent, so the
    /// per-frame sync can call it unconditionally for every tapped channel.
    pub fn ensure_tap(&mut self, context: &GpuContext, width: u32, height: u32) {
        if self.tap_prev.is_some() {
            return;
        }
        let tex = context.create_compositing_texture(width, height);
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        self.tap_prev = Some((tex, view));
    }

    /// Release the tap target so an untapped channel costs no extra memory.
    pub fn clear_tap(&mut self) {
        self.tap_prev = None;
    }

    /// Exchange the composite and tap targets. Called once per frame before any deck renders, so
    /// the tap is one frame old wherever the tapping deck sits.
    pub(crate) fn swap_tap(&mut self) {
        if let Some((mut tex, mut view)) = self.tap_prev.take() {
            std::mem::swap(&mut self.composite_texture, &mut tex);
            std::mem::swap(&mut self.composite_view, &mut view);
            self.tap_prev = Some((tex, view));
        }
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

    /// Record every awake deck's source uploads for this frame, without a full render. Runs for
    /// every channel, visible or not, so a clip faded out by the crossfader never shows a stale
    /// frame when it returns.
    ///
    /// Decks the arrangement has put to sleep are skipped; they hold their last frame and resume
    /// from there.
    pub fn upload_sources(&mut self, encoder: &mut wgpu::CommandEncoder) {
        for slot in &mut self.decks {
            if slot.source_demand.wants_frames() {
                slot.deck.upload_source(encoder);
            }
        }
    }

    /// Called after the frame's uploads were submitted.
    pub fn after_source_submit(&mut self) {
        for slot in &mut self.decks {
            slot.deck.after_source_submit();
        }
    }

    /// Render all decks in this channel, composite them, then apply channel effects.
    /// `channel_idx` identifies the channel's GPU timing queries.
    /// `clock.dt` ticks auto-transitions and paces decks when `target_fps` is 0.
    /// `target_fps` is the global target FPS for the adaptive skip budget.
    /// `total_active_decks` is the active deck count across all channels, from the last frame.
    /// `gpu_load_ratio` scales CPU-measured render costs to estimate GPU time; >1.0 means
    /// GPU-bound.
    ///
    /// # Errors
    ///
    /// Returns an error if any deck fails to render its frame (shader/pipeline or
    /// source-decode failure), propagated from `Deck::render_with_prefix`.
    // Hot-path render entry; the args are distinct per-frame inputs with no shared invariant.
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        &mut self,
        context: &GpuContext,
        audio_data: &crate::audio::AudioData,
        modulation: &ModulationEngine,
        channel_idx: usize,
        clock: FrameClock,
        target_fps: u32,
        total_active_decks: u32,
        gpu_load_ratio: f32,
        prefix_cmds: &mut Vec<wgpu::CommandBuffer>,
        mut timing: Option<&mut GpuTimingFrame>,
        query_set: Option<&wgpu::QuerySet>,
    ) -> Result<()> {
        struct DeckCompositeInfo {
            deck_idx: usize,
            blend_mode: BlendMode,
            opacity: f32,
            transition_progress: Option<f64>, // Some = transitioning with shader
        }

        let FrameClock { time, dt, live } = clock;
        let render_start = std::time::Instant::now();

        // Tick auto-transition state before rendering
        let bpm = audio_data.bpm.map(f64::from);
        self.tick_auto_transitions(f64::from(dt), bpm);

        // Sort decks by z-index
        let mut deck_indices: Vec<usize> = (0..self.decks.len()).collect();
        deck_indices.sort_by_key(|&i| self.decks[i].z_index);

        // Check if any deck is solo'd
        let any_solo = self.decks.iter().any(|slot| slot.solo);

        // Video frame updates happen in tick_video_frames(), called by the mixer before render, so
        // all channels stay in sync even when faded out.

        // Calculate per-deck budget for adaptive skipping
        let frame_budget_us = if target_fps > 0 {
            1_000_000.0 / target_fps as f32
        } else {
            f32::MAX // uncapped
        };
        let per_deck_budget_us = if total_active_decks > 0 {
            frame_budget_us / total_active_decks as f32
        } else {
            frame_budget_us
        };

        // Render each deck to its texture (skip muted, non-solo'd, zero-opacity)
        // Done decks still render — they serve as visible background for transitioning decks above.
        // Adaptive skipping: expensive decks render less frequently, reusing stale textures.
        let t_deck_render = std::time::Instant::now();
        let mut cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();
        let mut active_count: u32 = 0;
        let mut per_deck_timings: Vec<(String, u128, bool)> = Vec::new(); // (name, us, skipped)
        for (deck_idx, slot) in self.decks.iter_mut().enumerate() {
            if !slot.mute && (!any_solo || slot.solo) && slot.opacity > 0.0 {
                active_count += 1;
                slot.skip_counter += 1;

                // Determine if this deck should render this frame
                let should_render = match slot.render_fps {
                    DeckRenderFps::Fixed(fps) if fps > 0 && target_fps > 0 => {
                        // Render every Nth frame: N = target_fps / deck_fps
                        let skip_interval = (target_fps / fps).max(1);
                        slot.skip_counter % skip_interval == 0
                    }
                    DeckRenderFps::Auto => {
                        // Auto: skip if estimated cost exceeds the budget share. Uses CPU cost ×
                        // gpu_load_ratio, because GPU timestamps miss pipeline setup, compositing,
                        // and submission overhead, which scale with deck count.
                        let estimated_cost = slot.render_cost_us * gpu_load_ratio;
                        if estimated_cost > 0.0
                            && per_deck_budget_us < f32::MAX
                            && estimated_cost > per_deck_budget_us
                        {
                            let skip_interval = (estimated_cost / per_deck_budget_us).ceil() as u32;
                            slot.skip_counter % skip_interval.max(1) == 0
                        } else {
                            true // under budget or no data yet
                        }
                    }
                    DeckRenderFps::Fixed(_) => true, // Fixed(0) or uncapped target = always render
                };

                if should_render {
                    // Set the fixed time step so the shader TIME uniform advances
                    // by consistent increments regardless of skip gaps.
                    let step = if target_fps > 0 {
                        1.0 / target_fps as f32
                    } else {
                        dt // uncapped: use actual frame delta
                    };
                    slot.deck.set_pacing(FramePacing { step, live });

                    // Allocate GPU timing queries for this deck
                    let gpu_timing = match (&mut timing, query_set) {
                        (Some(t), Some(qs)) => {
                            t.allocate(channel_idx, deck_idx).map(|(b, e)| (qs, b, e))
                        }
                        _ => None,
                    };

                    let param_prefix = slot.deck.param_prefix().to_owned();
                    let t_single = std::time::Instant::now();
                    slot.deck.render_with_prefix(
                        context,
                        audio_data,
                        modulation,
                        &param_prefix,
                        &mut cmd_buffers,
                        gpu_timing,
                    )?;
                    let elapsed_us = t_single.elapsed().as_micros() as f32;
                    // Update EMA render cost (α = 0.2 for responsive adaptation)
                    if slot.render_cost_us > 0.0 {
                        slot.render_cost_us = 0.2 * elapsed_us + 0.8 * slot.render_cost_us;
                    } else {
                        slot.render_cost_us = elapsed_us; // first sample
                    }
                    per_deck_timings.push((
                        slot.deck.source_name().to_owned(),
                        elapsed_us as u128,
                        false,
                    ));
                } else {
                    // Skipped — stale texture reuse, compositing still includes this deck
                    per_deck_timings.push((slot.deck.source_name().to_owned(), 0, true));
                }
            }
        }
        self.active_deck_count = active_count;
        let deck_render_us = t_deck_render.elapsed().as_micros();
        // Batch submit all deck renders. Prefix command buffers (e.g. video upload copies) go first
        // so render passes read the updated textures, all in one queue.submit().
        let t_deck_submit = std::time::Instant::now();
        if !prefix_cmds.is_empty() {
            let mut all_cmds: Vec<wgpu::CommandBuffer> = std::mem::take(prefix_cmds);
            all_cmds.extend(cmd_buffers);
            context.submit(all_cmds);
        } else if !cmd_buffers.is_empty() {
            context.submit(cmd_buffers);
        }
        let deck_submit_us = t_deck_submit.elapsed().as_micros();

        // Collect render info for visible decks, including transition phase
        let deck_composite_info: Vec<DeckCompositeInfo> = deck_indices
            .iter()
            .filter_map(|&idx| {
                let slot = &self.decks[idx];
                let phase = slot.transition_phase();
                if slot.mute || (any_solo && !slot.solo) || slot.opacity <= 0.0 {
                    return None;
                }
                // Inactive and Done auto-transition decks don't composite: Inactive decks wait
                // their turn, and Done decks have played (the next deck is re-activated when
                // needed).
                let has_at = slot.auto_transition.as_ref().is_some_and(|at| at.enabled);
                if has_at
                    && (phase == DeckTransitionPhase::Inactive
                        || phase == DeckTransitionPhase::Done)
                {
                    return None;
                }
                let transition_progress = match phase {
                    DeckTransitionPhase::Transitioning { progress } => Some(progress),
                    // Every other phase composites without a transition.
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

        // Composite non-transitioning decks first, then transitioning ones, so the composite
        // already holds all revealed content when a transitioning deck draws.
        let mut non_transitioning: Vec<&DeckCompositeInfo> = Vec::new();
        let mut transitioning: Vec<&DeckCompositeInfo> = Vec::new();
        for info in &deck_composite_info {
            if info.transition_progress.is_some() {
                transitioning.push(info);
            } else {
                non_transitioning.push(info);
            }
        }
        let ordered: Vec<&DeckCompositeInfo> =
            non_transitioning.into_iter().chain(transitioning).collect();

        // Composite all decks. Per-draw params go into a persistent ring buffer (one slot per deck)
        // so all command buffers batch into one queue.submit(). Each Metal command buffer commit is
        // expensive on Intel integrated GPUs.
        let t_composite = std::time::Instant::now();
        let width = self.composite_texture.width();
        let height = self.composite_texture.height();
        let mut composite_cmds: Vec<wgpu::CommandBuffer> = Vec::new();
        // One params slot per compositing deck. Sized up front, not per draw:
        // growing mid-loop would strand bind groups on the previous buffer.
        self.blit_pipeline
            .ensure_ring_slots(&context.device, ordered.len());
        self.composite_pipeline
            .ensure_ring_slots(&context.device, ordered.len());
        let pipelines = crate::renderer::layers::LayerPipelines {
            blit: &self.blit_pipeline,
            composite: &self.composite_pipeline,
        };
        let mut stack = crate::renderer::layers::LayerStack::new(
            (&self.composite_texture, &self.composite_view),
            (&self.effect_ping_texture, &self.effect_ping_view),
            ordered.len(),
            wgpu::Color::TRANSPARENT,
        );

        for info in &ordered {
            let slot = &mut self.decks[info.deck_idx];
            let mut opacity = info.opacity;

            if let Some(progress) = info.transition_progress {
                if let Some(effect) = slot.transition_effect.as_mut()
                    && let Some((below, target)) = stack.pass_over()
                {
                    // Transition shader: start = the outgoing deck, end = the
                    // composite below it, which it reveals.
                    let uniforms = ISFUniforms {
                        time,
                        time_delta: dt,
                        frame_index: self.frame_count,
                        pass_index: 0,
                        render_size: [width as f32, height as f32],
                        phase_times: [0.0; 4],
                        ..Default::default()
                    };
                    effect.params.set(
                        "progress",
                        crate::params::ParamValue::Float(progress as f32),
                    );
                    effect.params.build_buffer_data();
                    if let Some(buf) = effect.params.buffer() {
                        context.queue.write_buffer(buf, 0, effect.params.scratch());
                    }
                    composite_cmds.push(effect.pipeline.render_to_cmd(
                        context,
                        &slot.deck.texture_view,
                        below,
                        target,
                        &uniforms,
                        effect.params.buffer(),
                    ));
                    continue;
                }
                // No shader, or nothing below to transition to: fade out.
                opacity *= 1.0 - progress as f32;
            }

            stack.blend(
                context,
                &pipelines,
                &crate::renderer::layers::BlendLayer {
                    view: &slot.deck.texture_view,
                    opacity,
                    blend_mode: info.blend_mode.to_index(),
                    premultiplied: false,
                },
                &mut composite_cmds,
            );
        }
        let composite_encode_us = t_composite.elapsed().as_micros();

        // Batch submit all channel composite commands at once
        let t_composite_submit = std::time::Instant::now();
        if !composite_cmds.is_empty() {
            context.submit(composite_cmds);
        }
        let composite_submit_us = t_composite_submit.elapsed().as_micros();

        // If no decks, clear the composite texture to transparent
        if deck_composite_info.is_empty() {
            let mut encoder =
                context
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                    multiview_mask: None,
                });
            }
            context.submit(std::iter::once(encoder.finish()));
        }

        // Apply channel effect chain (if any)
        let t_effects = std::time::Instant::now();
        let mut effects_count: u32 = 0;
        if !self.effects.is_empty() {
            let width = self.composite_texture.width();
            let height = self.composite_texture.height();

            let uniforms = crate::generator::pass::uniforms(
                audio_data,
                time,
                1.0 / 60.0,
                self.frame_count,
                0,
                [width as f32, height as f32],
                [0.0; 4],
            );

            let mut read_from_composite = true;
            let mut fx_cmd_buffers: Vec<wgpu::CommandBuffer> = Vec::new();

            for (eff_idx, effect) in self.effects.iter_mut().enumerate() {
                if !effect.is_active() {
                    continue;
                }

                let (input_view, output_view) = if read_from_composite {
                    (&self.composite_view, &self.effect_ping_view)
                } else {
                    (&self.effect_ping_view, &self.composite_view)
                };

                if let Err(e) = effect.apply_with_modulation(
                    context,
                    input_view,
                    output_view,
                    &uniforms,
                    Some(modulation),
                    live,
                    &mut fx_cmd_buffers,
                ) {
                    log::warn!("Effect {eff_idx} failed, skipping: {e}");
                    continue;
                }
                effects_count += 1;
                read_from_composite = !read_from_composite;
            }

            // If result is in ping texture, copy back to composite
            if !read_from_composite {
                let mut encoder =
                    context
                        .device
                        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
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
                context.submit(fx_cmd_buffers);
            }
        }
        let effects_us = t_effects.elapsed().as_micros();

        self.frame_count += 1;

        // Update smoothed render time (EMA, α = 0.1)
        let raw_ms = render_start.elapsed().as_secs_f32() * 1000.0;
        self.render_time_ms = 0.1 * raw_ms + 0.9 * self.render_time_ms;

        // Log detailed timing every 127 frames (~2s at 60fps). A prime interval avoids always
        // sampling the same phase of the skip intervals.
        if self.frame_count.is_multiple_of(127) && active_count > 0 {
            let total_us = render_start.elapsed().as_micros();
            // Build per-deck breakdown string: "shader_name=123us" or "shader_name=SKIP"
            let deck_breakdown: String = per_deck_timings
                .iter()
                .map(|(name, us, skipped)| {
                    if *skipped {
                        format!("{name}=SKIP")
                    } else {
                        format!("{name}={us}us")
                    }
                })
                .collect::<Vec<_>>()
                .join(", ");
            log::debug!(
                "[PERF] ch={} decks={} | deck_render={}us [{}] deck_submit={}us | \
                 composite_encode={}us composite_submit={}us | \
                 effects={} effects={}us | total={}us ({:.1}ms)",
                self.name,
                active_count,
                deck_render_us,
                deck_breakdown,
                deck_submit_us,
                composite_encode_us,
                composite_submit_us,
                effects_count,
                effects_us,
                total_us,
                raw_ms,
            );
        }

        Ok(())
    }

    /// Resize the channel's textures
    pub fn resize(&mut self, context: &GpuContext, width: u32, height: u32) {
        self.composite_texture = context.create_compositing_texture(width, height);
        self.composite_view = self
            .composite_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        self.effect_ping_texture = context.create_compositing_texture(width, height);
        self.effect_ping_view = self
            .effect_ping_texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        // Dropped rather than resized; the per-frame sync reallocates it at the
        // new size, costing one black frame for any deck tapping this channel.
        self.tap_prev = None;

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
            slot.opacity = if opacity.is_finite() {
                opacity.clamp(0.0, 1.0)
            } else {
                1.0
            };
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
            let has_at = slot.auto_transition.as_ref().is_some_and(|at| at.enabled);
            let phase = slot.transition_phase();
            has_at
                && !slot.mute
                && !slot.arrangement_authority
                && phase != DeckTransitionPhase::Done
        });

        // Update phase for each deck, collecting indices that just started transitioning
        let mut just_started_transitioning: Vec<usize> = Vec::new();

        for i in 0..self.decks.len() {
            let is_active = active_idx == Some(i);
            let slot = &mut self.decks[i];
            // A deck the arrangement drives keeps its auto-transition config but stops advancing
            // it: auto-transition phase depends on when the deck became active, not transport
            // position, so it would fight the regions.
            if slot.arrangement_authority {
                continue;
            }
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
                            // A source that never ends falls back to the timer.
                            match slot.deck.source().reached_end() {
                                Some(ended) => ended,
                                None => *elapsed >= play_secs,
                            }
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
                let is_candidate = slot
                    .auto_transition
                    .as_ref()
                    .is_some_and(|at| at.enabled && at.phase == DeckTransitionPhase::Inactive);
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
                    let is_candidate = slot
                        .auto_transition
                        .as_ref()
                        .is_some_and(|at| at.enabled && at.phase == DeckTransitionPhase::Done);
                    if is_candidate && !slot.mute {
                        if let Some(at) = self.decks[j].auto_transition.as_mut() {
                            at.phase = DeckTransitionPhase::Playing { elapsed: 0.0 };
                        }
                        break;
                    }
                }
            }
        }

        // When every auto-transition deck is Done, loop. Runs after phase updates so the reset
        // happens in the same frame, avoiding a flash of stale content.
        let all_done = self.decks.iter().all(|slot| match &slot.auto_transition {
            Some(at) if at.enabled => at.phase == DeckTransitionPhase::Done,
            _ => true,
        });
        let any_at = self
            .decks
            .iter()
            .any(|slot| slot.auto_transition.as_ref().is_some_and(|at| at.enabled));

        if all_done && any_at {
            // Reset all AT decks to Inactive, then immediately activate the first one
            for slot in &mut self.decks {
                if let Some(at) = &mut slot.auto_transition
                    && at.enabled
                {
                    at.phase = DeckTransitionPhase::Inactive;
                }
            }
            for slot in &mut self.decks {
                let dominated = slot.mute;
                let is_inactive_at = slot
                    .auto_transition
                    .as_ref()
                    .is_some_and(|at| at.enabled && at.phase == DeckTransitionPhase::Inactive);
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
mod tests;
