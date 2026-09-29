mod effect;
mod render;
mod source;

pub use crate::generator::{PassBuffer, get_current_date};
pub use crate::source::{PreprocessorSlot, ScalingMode, preprocessor_texture_format};
pub(crate) use render::analyzer_registry;

use crate::isf::{ISFPass, ISFShader};
use crate::params::ShaderParams;
use crate::renderer::UnifiedPipeline;
use crate::source::DeckSourceInstance;
use std::collections::HashMap;
use std::time::Instant;

/// A shader deck's depth-sensor preprocessor as saved under the source config's
/// `depth_prepro` key.
///
/// The sensor is matched by name on restore, since device ids change across replugs. Params
/// are in physical units, matching `DepthPreprocessParams`, and every field has a default so
/// older scenes load.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct DepthPreproConfig {
    pub sensor_name: String,
    #[serde(default)]
    pub near_mm: f32,
    #[serde(default)]
    pub far_mm: f32,
    #[serde(default)]
    pub smoothing: f32,
    #[serde(default)]
    pub hole_fill: f32,
    #[serde(default)]
    pub mask_feather: f32,
    #[serde(default)]
    pub motion_gain: f32,
    #[serde(default)]
    pub mirror: bool,
}

/// Live state for a deck's `depth_sensor` shader preprocessor.
///
/// The sensor is ref-counted on `DepthSensorManager`: acquired before the deck is built and
/// released on removal, so any number of decks can share one device.
pub struct DepthPreprocessState {
    /// The acquired sensor, released on deck teardown.
    pub sensor_id: crate::depth::DepthSensorId,
    /// Device name, captured at acquisition. Persistence matches sensors by name, and snapshots
    /// must not need the device manager.
    pub sensor_name: String,
    /// Router-exposed params (`deck/<uuid>/depth_prepro/*`).
    pub params: crate::depth::preprocess::DepthPreprocessParams,
    /// The conversion pipeline and its owned output textures.
    pub pipeline: crate::depth::preprocess::DepthPreprocessPipeline,
    /// Whether any consuming shader declared the `rgb` output. When false the color pass is
    /// skipped.
    pub wants_rgb: bool,
    /// Last sensor frame generation processed, so a 30 Hz sensor does not drive
    /// 60 Hz of redundant passes.
    pub last_generation: Option<u64>,
    /// Set while the sensor reports disconnected, so the warning fires once.
    pub warned_disconnected: bool,
    /// This frame's sensor inputs, pushed by the app render loop. `None` before the first tick or
    /// while the sensor is gone.
    pub input: Option<DepthPreprocessInput>,
}

/// Per-frame sensor inputs handed to a deck's depth preprocessor by the app layer.
pub struct DepthPreprocessInput {
    /// Shared `R16Uint` depth texture owned by `DepthSensorManager`.
    pub depth_view: wgpu::TextureView,
    /// Shared color texture owned by `DepthSensorManager`.
    pub rgb_view: wgpu::TextureView,
    /// Manager upload counter, used to skip redundant passes.
    pub generation: u64,
    /// Measured seconds between the last two sensor frames.
    pub frame_dt: f32,
    /// Whether the sensor is currently producing frames.
    pub connected: bool,
}

/// An effect in the deck's effect chain (ISF filter)
pub struct Effect {
    /// Stable UUID for this effect (8-char hex).
    ///
    /// Private so that [`Effect::set_uuid`] keeps `param_prefix`, which modulation assignments are
    /// keyed on, in step with it.
    uuid: String,
    /// Cached `effect/<uuid>/param` prefix for modulation key lookups, avoiding a per-frame
    /// `format!`.
    param_prefix: String,
    pub shader: ISFShader,
    pub pipeline: UnifiedPipeline,
    pub enabled: bool,
    pub params: ShaderParams,
    pub pass_buffers: HashMap<String, PassBuffer>,
    pub passes: Vec<ISFPass>,
    pub target_format: wgpu::TextureFormat,
    /// GPU textures loaded from ISF IMPORTED images (sorted by name for deterministic binding)
    pub imported_textures: Vec<(String, wgpu::Texture, wgpu::TextureView)>,
    /// Preprocessor textures from PREPROCESSORS declarations (placeholder until analyzer provides data)
    pub preprocessor_textures: Vec<PreprocessorSlot>,
    /// Phase accumulators for smooth speed transitions
    pub phase_accumulators: [f32; 4],
    /// Phase input config from shader metadata
    pub phase_inputs_config: Option<Vec<crate::isf::PhaseInput>>,
}

// Effect impl is in effect.rs

/// A deck renders its source, then its effect chain, into a texture.
pub struct Deck {
    /// Stable UUID for this deck (8-char hex, persists across moves/saves)
    uuid: String,

    /// Cached `deck/<uuid>/param` prefix for modulation key lookups, avoiding a per-frame
    /// `format!`.
    param_prefix: String,

    /// Display name, from the source unless the user renamed the deck.
    source_name: String,

    /// What draws the base image.
    source: Box<dyn DeckSourceInstance>,

    /// Generator parameters, from the source's ISF `INPUTS` when it declares them. Held by the
    /// deck so every ISF-style source gets MIDI, OSC, modulation, presets and exploration.
    pub generator_params: ShaderParams,

    /// Render target texture (primary)
    pub texture: wgpu::Texture,

    pub texture_view: wgpu::TextureView,

    /// Secondary texture for ping-pong rendering in effect chain
    texture_b: wgpu::Texture,
    texture_b_view: wgpu::TextureView,

    /// Effect chain (ISF filters applied to generator output)
    pub effects: Vec<Effect>,

    /// Deck opacity (0.0 - 1.0)
    pub opacity: f32,

    /// When true, the base texture preserves source alpha (transparent letterbox and HTML
    /// regions). When false (default), the source is composited over opaque black.
    transparent: bool,

    /// Accumulated render time for the TIME uniform, advanced by a fixed dt per render so skipped
    /// frames don't cause jumps.
    render_time: f32,

    /// Fixed time step per render (`1/target_fps`), set by the channel.
    render_dt: f32,

    frame_count: u32,

    /// Last wall-clock render instant (for FPS measurement only, not for TIME uniform)
    last_frame_time: Instant,

    /// Depth-sensor shader preprocessor, present when this deck's shader or one of its effects
    /// declared a `depth_sensor` PREPROCESSOR and the device was acquired.
    pub depth_prepro: Option<DepthPreprocessState>,

    /// Smoothed FPS derived from actual render pipeline timing (EMA of `1/time_delta`)
    fps_smoothed: f32,

    /// Phase accumulators for smooth speed transitions (generator shader)
    phase_accumulators: [f32; 4],

    /// Phase input config from generator shader metadata
    generator_phase_inputs: Option<Vec<crate::isf::PhaseInput>>,

    /// Per-deck analyzer instances (brightness, beat detection, etc.)
    pub(crate) analyzers: crate::analyzer::DeckAnalyzers,

    /// Set when this deck raised a GPU error. A quarantined deck stops rendering and holds its
    /// last good frame. Cleared by [`Deck::clear_gpu_error`] when the shader is reloaded.
    gpu_error: Option<String>,
}

/// Accessors. Constructors are in source.rs, rendering in render.rs.
impl Deck {
    /// Get the stable UUID for this deck
    pub fn uuid(&self) -> &str {
        &self.uuid
    }

    /// The cached `deck/<uuid>/param` prefix its parameters' modulation keys share.
    pub fn param_prefix(&self) -> &str {
        &self.param_prefix
    }

    /// Set the UUID (used during scene restore to preserve identity)
    pub fn set_uuid(&mut self, uuid: String) {
        self.param_prefix = crate::engine::value::param::deck_param_prefix(&uuid);
        self.uuid = uuid;
    }

    /// The deck's display name.
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Override the display name (e.g. when loading a preset with a custom name).
    pub fn set_source_name(&mut self, name: String) {
        self.source_name = name;
    }

    /// The deck's source.
    pub fn source(&self) -> &dyn DeckSourceInstance {
        self.source.as_ref()
    }

    /// The deck's source, for writing its controls.
    pub fn source_mut(&mut self) -> &mut dyn DeckSourceInstance {
        self.source.as_mut()
    }

    /// The deck's source, boxed, for the registry's per-frame servicing.
    pub fn source_box_mut(&mut self) -> &mut Box<dyn DeckSourceInstance> {
        &mut self.source
    }

    /// The source type id (`Shader`, `Video`, ...).
    pub fn source_type(&self) -> &str {
        self.source.source_type()
    }

    /// Swap in a different source, keeping the deck's identity, effects,
    /// opacity and modulation. Returns the old source so its device references
    /// can be released. The generator parameters follow the new source.
    pub fn replace_source(
        &mut self,
        source: Box<dyn DeckSourceInstance>,
    ) -> Box<dyn DeckSourceInstance> {
        let (params, phase_inputs) = generator_params_for(source.as_ref());
        self.generator_params = params;
        self.generator_phase_inputs = phase_inputs;
        self.phase_accumulators = [0.0; 4];
        self.source_name = source.label();
        std::mem::replace(&mut self.source, source)
    }

    /// The config that rebuilds this deck's source as it is now, plus the deck-held parts all
    /// source types share: generator parameter values under `params` and a shader deck's depth
    /// preprocessor under `depth_prepro`.
    pub fn source_config(&self) -> crate::source::SourceConfig {
        let mut config = self.source.config();
        if !self.generator_params.values.is_empty() {
            config.set("params", &self.generator_params.values);
        }
        if let Some(state) = &self.depth_prepro {
            let p = &state.params;
            config.set(
                "depth_prepro",
                DepthPreproConfig {
                    sensor_name: state.sensor_name.clone(),
                    near_mm: p.near_mm,
                    far_mm: p.far_mm,
                    smoothing: p.smoothing,
                    hole_fill: p.hole_fill,
                    mask_feather: p.mask_feather,
                    motion_gain: p.motion_gain,
                    mirror: p.mirror,
                },
            );
        }
        config
    }

    /// The ISF shader this deck runs, when its source is one.
    pub fn shader(&self) -> Option<&ISFShader> {
        self.source.shader()
    }

    /// Set a depth-preprocessor parameter from a normalized value (0.0–1.0).
    /// Returns `false` if this deck has no `depth_sensor` preprocessor.
    pub fn set_depth_prepro_param(&mut self, name: &str, value: f32) -> bool {
        self.depth_prepro
            .as_mut()
            .is_some_and(|s| s.params.set_normalized_param(name, value))
    }

    /// Normalized (`0..1`) value of a depth-preprocessor parameter, for snapshots.
    pub fn depth_prepro_param(&self, name: &str) -> Option<f32> {
        self.depth_prepro
            .as_ref()
            .and_then(|s| s.params.normalized_param(name))
    }

    /// Attach an acquired depth sensor's preprocessor to this deck.
    ///
    /// Rebinds every `depth_sensor` preprocessor slot, on the source shader and every effect, to
    /// the pipeline's output textures (`Arc`-backed handles, not copies). Called by the app layer
    /// after `open_depth_sensor` succeeds, since device managers live above `internal::deck`.
    pub fn attach_depth_preprocessor(
        &mut self,
        sensor_id: crate::depth::DepthSensorId,
        sensor_name: String,
        pipeline: crate::depth::preprocess::DepthPreprocessPipeline,
        params: crate::depth::preprocess::DepthPreprocessParams,
    ) {
        self.depth_prepro = Some(DepthPreprocessState {
            sensor_id,
            sensor_name,
            params,
            pipeline,
            wants_rgb: false,
            last_generation: None,
            warned_disconnected: false,
            input: None,
        });
        self.rebind_depth_preprocessor_slots();
    }

    /// Point every `depth_sensor` preprocessor slot at the attached pipeline's outputs, and
    /// recompute whether the color pass is needed.
    ///
    /// Idempotent. Re-run after adding an effect that declares the preprocessor to a deck that
    /// already has one attached.
    pub fn rebind_depth_preprocessor_slots(&mut self) {
        use crate::depth::preprocess::{Output, PREPROCESSOR_TYPE};

        // Take the state so the pipeline can be read while the source and
        // `self.effects` are mutably borrowed.
        let Some(mut state) = self.depth_prepro.take() else {
            return;
        };
        let mut wants_rgb = false;
        let mut rebind = |slots: &mut Vec<PreprocessorSlot>| {
            for slot in slots {
                if slot.analyzer_type != PREPROCESSOR_TYPE {
                    continue;
                }
                let Some(output) = Output::from_name(&slot.name) else {
                    log::warn!(
                        "Shader declared unknown depth_sensor output '{}'; leaving it blank",
                        slot.name
                    );
                    continue;
                };
                if output == Output::Rgb {
                    wants_rgb = true;
                }
                if let Some((texture, view)) = state.pipeline.output(output) {
                    slot.texture = texture;
                    slot.view = view;
                }
            }
        };

        if let Some(slots) = self.source.preprocessor_slots_mut() {
            rebind(slots);
        }
        for effect in &mut self.effects {
            rebind(&mut effect.preprocessor_textures);
        }

        state.wants_rgb = wants_rgb;
        self.depth_prepro = Some(state);
    }

    /// Whether any slot on this deck still consumes the `depth_sensor`
    /// preprocessor. Used after removing an effect to decide whether the sensor
    /// reference is still needed.
    fn wants_depth_preprocessor(&self) -> bool {
        let ty = crate::depth::preprocess::PREPROCESSOR_TYPE;
        self.source
            .preprocessor_slots()
            .iter()
            .any(|s| s.analyzer_type == ty)
            || self.effects.iter().any(|e| {
                e.preprocessor_textures
                    .iter()
                    .any(|s| s.analyzer_type == ty)
            })
    }

    /// Drop the depth preprocessor if nothing on this deck consumes it, returning the sensor ID the
    /// caller must release.
    ///
    /// Called after removing an effect, so an unused device does not keep a capture thread and
    /// three GPU passes running.
    pub fn detach_depth_preprocessor_if_unused(&mut self) -> Option<crate::depth::DepthSensorId> {
        if self.depth_prepro.is_none() || self.wants_depth_preprocessor() {
            return None;
        }
        self.depth_prepro.take().map(|s| s.sensor_id)
    }

    /// The depth sensor this deck's preprocessor holds, for release on removal.
    pub fn held_depth_prepro_sensor(&self) -> Option<crate::depth::DepthSensorId> {
        self.depth_prepro.as_ref().map(|s| s.sensor_id)
    }

    /// The GPU error that quarantined this deck, if any.
    pub fn gpu_error(&self) -> Option<&str> {
        self.gpu_error.as_deref()
    }

    /// Lift the quarantine and let the deck render again. Called on shader hot-reload, so one bad
    /// save does not black out the deck until restart.
    pub fn clear_gpu_error(&mut self) {
        if self.gpu_error.take().is_some() {
            log::info!("Deck '{}': GPU quarantine lifted", self.uuid);
        }
    }

    /// Whether this deck preserves source alpha (transparent compositing).
    pub fn transparent(&self) -> bool {
        self.transparent
    }

    /// Set whether this deck preserves source alpha (transparent compositing).
    pub fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
    }

    /// Set the fixed time step used for the TIME uniform.
    /// Called by the channel to keep `render_dt` in sync with the target FPS.
    pub fn set_render_dt(&mut self, dt: f32) {
        self.render_dt = dt;
    }

    /// Get the smoothed FPS derived from actual render pipeline timing
    pub fn fps(&self) -> f32 {
        self.fps_smoothed
    }
}

/// The generator parameters and phase inputs a source's ISF `INPUTS` declare.
fn generator_params_for(
    source: &dyn DeckSourceInstance,
) -> (ShaderParams, Option<Vec<crate::isf::PhaseInput>>) {
    match source.shader() {
        Some(shader) => (
            ShaderParams::from_inputs(shader.metadata.inputs.as_deref().unwrap_or(&[])),
            shader.metadata.phase_inputs.clone(),
        ),
        None => (ShaderParams::from_inputs(&[]), None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_analyzer_registry_includes_the_depth_preprocessor() {
        let ty = crate::depth::preprocess::PREPROCESSOR_TYPE;
        assert!(analyzer_registry().schema_for(ty).is_some());
        assert!(
            crate::analyzer::default_registry().schema_for(ty).is_none(),
            "the analyzer module registers only its own analyzers"
        );
    }
}
