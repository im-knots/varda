//! Deck source providers.
//!
//! A deck's source is whatever draws its base image before the effect chain:
//! a shader, a clip, a camera, a stream. Each kind is a *provider*: one type
//! implementing [`DeckSourceProvider`], registered once, which builds a
//! [`DeckSourceInstance`] per deck. Everything that used to be wired by hand
//! for each kind (the library panel, deck controls, the API, persistence, the
//! router) reads the registry and the traits instead.
//!
//! See /spec/deck-source-providers.md.

mod blit;
mod feed;
mod preprocessor;
mod registry;
mod services;
mod share;
mod unavailable;

pub use blit::{AlphaPolicy, SCALING_MODE, ScaledBlit, clear_target, scaling_mode_spec};
pub use feed::Feed;
pub use preprocessor::{PreprocessorSlot, preprocessor_texture_format};
pub use registry::SourceRegistry;
pub use services::Services;
pub use share::{ShareProtocol, ShareProvider, ShareReceiver};
pub use unavailable::UnavailableSource;

pub use crate::engine::value::source::{
    DeckSourceSnapshot, LibraryCreate, LibraryEntry, LibraryNotice, LibrarySection, ScalingMode,
    SourceConfig, SourceParamKind, SourceParamSpec, SourceStatus, SourceTypeSnapshot, SourceValue,
    WidgetHint,
};

use crate::audio::AudioData;
use crate::isf::ISFShader;
use crate::modulation::{ModulationEngine, ResolvedModulation};
use crate::params::ShaderParams;
use crate::renderer::GpuContext;
use anyhow::Result;
use std::any::Any;
use std::collections::HashMap;

/// Why a control write was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceParamError {
    /// This source has no control by that name or route.
    Unknown(String),
    /// The value has the wrong shape for the control.
    Invalid(String),
    /// The control exists but the source cannot take it right now.
    State(String),
}

impl std::fmt::Display for SourceParamError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "unknown source parameter '{name}'"),
            Self::Invalid(msg) | Self::State(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for SourceParamError {}

/// What a source re-enters from Varda's own output. See spec/program-tap.md.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FeedbackSource {
    /// The master program, before tonemap and LUT.
    MasterProgram,
    /// A channel composite, by the channel's stable UUID.
    Channel(String),
}

/// Everything a provider may need when it builds, restores or services decks.
pub struct SourceEnv<'a> {
    pub gpu: &'a GpuContext,
    /// Deck render size.
    pub width: u32,
    pub height: u32,
    /// Device managers shared with the rest of the engine.
    pub services: &'a mut Services,
    /// The shader library.
    pub shaders: &'a crate::registry::ShaderRegistry,
    /// `(uuid, name)` of every channel, in mixer order.
    pub channels: &'a [(String, String)],
}

impl SourceEnv<'_> {
    /// The read-only view of this environment.
    pub fn query(&self) -> SourceQuery<'_> {
        SourceQuery {
            services: self.services,
            shaders: self.shaders,
            channels: self.channels,
        }
    }
}

/// The read-only part of [`SourceEnv`], for snapshots and the library.
#[derive(Clone, Copy)]
pub struct SourceQuery<'a> {
    pub services: &'a Services,
    pub shaders: &'a crate::registry::ShaderRegistry,
    pub channels: &'a [(String, String)],
}

/// Builds a deck's source off the render thread. Returned by
/// [`DeckSourceProvider::loader`] for sources whose construction is slow
/// (file decode, shader compile) and needs no device manager.
pub type SourceLoader =
    Box<dyn FnOnce(&GpuContext, u32, u32) -> Result<Box<dyn DeckSourceInstance>> + Send>;

/// One frame of drawing, handed to [`DeckSourceInstance::render`].
pub struct SourceFrame<'a> {
    pub gpu: &'a GpuContext,
    /// Where the source draws: one of the deck's two ping-pong targets.
    pub target: &'a wgpu::TextureView,
    /// The texture behind `target`, for sources that copy rather than draw.
    pub target_texture: &'a wgpu::Texture,
    pub width: u32,
    pub height: u32,
    /// Whether the deck keeps its source's alpha. See /spec/html-source.md §2.
    pub transparent: bool,
    pub time: f32,
    pub time_delta: f32,
    pub frame_index: u32,
    /// The deck's phase accumulators. See /spec/phase-accumulators.md.
    pub phase_times: [f32; 4],
    pub audio: &'a AudioData,
    pub modulation: &'a ModulationEngine,
    /// Modulation key prefix of the deck's generator parameters.
    pub param_prefix: &'a str,
    /// The deck's generator parameters, for sources that declare them.
    pub params: &'a mut ShaderParams,
    pub cmd_buffers: &'a mut Vec<wgpu::CommandBuffer>,
}

/// Per-frame context for a source's own controls, handed to
/// [`DeckSourceInstance::control`] before anything renders.
pub struct SourceControl<'a> {
    pub deck_uuid: &'a str,
    pub modulation: &'a ModulationEngine,
    /// False while the arrangement has this deck asleep. A sleeping source
    /// holds its frame and must not be driven. See /spec/deck-residency.md.
    pub awake: bool,
    pub transport: Option<crate::timebase::TransportSample>,
    /// The rate the renderer presents at (0 = uncapped).
    pub target_fps: u32,
    /// Reused across every deck in a frame, so resolving a key allocates
    /// nothing.
    scratch: &'a mut String,
}

impl<'a> SourceControl<'a> {
    pub fn new(
        deck_uuid: &'a str,
        modulation: &'a ModulationEngine,
        awake: bool,
        transport: Option<crate::timebase::TransportSample>,
        target_fps: u32,
        scratch: &'a mut String,
    ) -> Self {
        Self {
            deck_uuid,
            modulation,
            awake,
            transport,
            target_fps,
            scratch,
        }
    }

    /// Whether the transport is running.
    pub fn transport_running(&self) -> bool {
        self.transport.is_some_and(|t| t.running)
    }

    /// The modulation assigned to this deck's control at `route`, if any.
    pub fn resolve(&mut self, route: &str) -> Option<ResolvedModulation> {
        if !self.modulation.has_modulation_for_any() {
            return None;
        }
        self.scratch.clear();
        self.scratch.push_str("deck/");
        self.scratch.push_str(self.deck_uuid);
        self.scratch.push('/');
        self.scratch.push_str(route);
        if !self.modulation.has_modulation(self.scratch) {
            return None;
        }
        Some(self.modulation.resolve(self.scratch, None))
    }
}

/// A kind of deck source.
///
/// Registered once for the process. It lists what can be created, builds one
/// [`DeckSourceInstance`] per deck, and services the device managers its
/// instances read from once per frame.
pub trait DeckSourceProvider: 'static {
    /// Stable id: the `type` tag in scene files and the API, in the CamelCase
    /// every scene has used (`Video`, `SolidColor`). Never renamed.
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// A short glyph shown next to the label.
    fn icon(&self) -> &'static str;

    /// The controls every deck of this type has.
    fn params(&self) -> &'static [SourceParamSpec] {
        &[]
    }

    /// `Err(reason)` when this build or host cannot create this type.
    ///
    /// # Errors
    ///
    /// Returns the reason the type cannot be created.
    fn availability(&self, _query: &SourceQuery) -> std::result::Result<(), String> {
        Ok(())
    }

    /// Whether the library panel lists the type.
    fn listed(&self, _query: &SourceQuery) -> bool {
        true
    }

    /// What the library offers for creating decks of this type.
    fn library(&self, _query: &SourceQuery) -> LibrarySection {
        LibrarySection::default()
    }

    /// Run a library action: `rescan`, or one a [`LibraryNotice`] offered.
    ///
    /// # Errors
    ///
    /// Fails for an action this type does not offer, or one that failed.
    fn library_action(&mut self, action: &str, _env: &mut SourceEnv) -> Result<()> {
        anyhow::bail!("{} has no library action '{action}'", self.label())
    }

    /// Add a user-entered library entry (a stream URL).
    ///
    /// # Errors
    ///
    /// Fails when this type keeps no user entries or the entry is malformed.
    fn add_library_entry(&mut self, _entry: SourceConfig) -> Result<()> {
        anyhow::bail!("{} keeps no library entries", self.label())
    }

    /// Remove a user-entered library entry.
    ///
    /// # Errors
    ///
    /// Fails when this type keeps no user entries.
    fn remove_library_entry(&mut self, _entry: &SourceConfig) -> Result<()> {
        anyhow::bail!("{} keeps no library entries", self.label())
    }

    /// A builder that runs off the render thread, for slow constructions that
    /// need no device manager. `None` builds through [`Self::create`] instead.
    fn loader(&self, _config: &SourceConfig, _query: &SourceQuery) -> Option<Result<SourceLoader>> {
        None
    }

    /// Build a deck's source from its config, on the render thread.
    ///
    /// # Errors
    ///
    /// Fails when the config is malformed or what it names cannot be opened.
    fn create(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>>;

    /// Whether adding a deck that is the same source (see [`Self::identity`])
    /// as one already on the channel returns that deck instead of a second
    /// one. For sources an external controller re-subscribes to on every
    /// reconnect, which must converge on a single deck.
    fn one_per_channel(&self) -> bool {
        false
    }

    /// Rebuild a saved deck. Defaults to [`Self::create`]; a provider that can
    /// hold a deck whose device is missing (a closed window, an unpublished
    /// server) overrides it to keep the deck, where an explicit add would fail.
    ///
    /// # Errors
    ///
    /// Fails when the deck cannot be rebuilt at all; the registry then keeps a
    /// placeholder holding the config.
    fn restore(
        &mut self,
        config: &SourceConfig,
        env: &mut SourceEnv,
    ) -> Result<Box<dyn DeckSourceInstance>> {
        self.create(config, env)
    }

    /// What makes two configs the same source, as opposed to the same source
    /// with different settings. A scene diff patches a deck in place when this
    /// matches and rebuilds it when it does not. Defaults to the whole config.
    fn identity(&self, config: &SourceConfig) -> serde_json::Value {
        serde_json::Value::Object(config.fields().clone())
    }

    /// Called for every deck of this type before [`Self::tick`], with whether
    /// the deck is wanted this frame (visible, cued, or scheduled soon).
    fn observe(&mut self, _instance: &dyn DeckSourceInstance, _wanted: bool) {}

    /// Once per frame: poll devices and upload their frames. Command buffers
    /// pushed onto `submit` are submitted before any deck renders.
    fn tick(&mut self, _env: &mut SourceEnv, _submit: &mut Vec<wgpu::CommandBuffer>) {}

    /// Once per frame per deck, after [`Self::tick`]: hand the instance what
    /// it reads this frame.
    fn prepare(&mut self, _instance: &mut dyn DeckSourceInstance, _env: &mut SourceEnv) {}

    /// Release what a removed deck held on a device manager.
    fn release(&mut self, _instance: &mut dyn DeckSourceInstance, _env: &mut SourceEnv) {}

    /// The instance's state for snapshots, with anything only the provider
    /// knows (a device's connection state) folded in.
    fn status(&self, instance: &dyn DeckSourceInstance, _query: &SourceQuery) -> SourceStatus {
        instance.status()
    }
}

/// One deck's source.
pub trait DeckSourceInstance: Send + 'static {
    /// The id of the provider that built it.
    fn source_type(&self) -> &str;

    /// Display name for the deck.
    fn label(&self) -> String;

    /// The config that rebuilds this source as it is now.
    fn config(&self) -> SourceConfig;

    /// Draw this frame's image into `frame.target`.
    ///
    /// # Errors
    ///
    /// Fails when encoding fails. GPU validation errors are caught by the deck.
    fn render(&mut self, frame: &mut SourceFrame) -> Result<()>;

    /// The deck's render size changed.
    fn resize(&mut self, _gpu: &GpuContext, _width: u32, _height: u32) {}

    /// Take the settings in `config`, which names this same source (see
    /// [`DeckSourceProvider::identity`]). Used when a scene diff patches a
    /// deck in place rather than rebuilding it.
    fn patch(&mut self, _config: &SourceConfig) {}

    /// Apply modulation, the transport and residency to the source's own
    /// controls. Runs every frame for every deck, visible or not.
    fn control(&mut self, _ctx: &mut SourceControl) {}

    /// Record CPU-to-GPU uploads for this frame into `encoder`. Runs for every
    /// awake deck, visible or not, so a clip faded out stays in step.
    fn upload(&mut self, _encoder: &mut wgpu::CommandEncoder) {}

    /// Called after the frame's uploads were submitted.
    fn after_submit(&mut self) {}

    /// The controls this source has. The same for every deck of a type.
    fn schema(&self) -> &'static [SourceParamSpec] {
        &[]
    }

    /// Current value of a control. Numeric controls read normalized.
    fn param(&self, _name: &str) -> Option<SourceValue> {
        None
    }

    /// Write a control. Numeric controls take normalized values.
    ///
    /// # Errors
    ///
    /// Fails for an unknown control or a value of the wrong shape.
    fn set_param(&mut self, name: &str, _value: &SourceValue) -> Result<(), SourceParamError> {
        Err(SourceParamError::Unknown(name.to_string()))
    }

    /// Fire an action control.
    ///
    /// # Errors
    ///
    /// Fails for an unknown action.
    fn trigger(&mut self, action: &str) -> Result<(), SourceParamError> {
        Err(SourceParamError::Unknown(action.to_string()))
    }

    /// State for snapshots. Defaults to the current value of every control.
    fn status(&self) -> SourceStatus {
        status_from_params(self)
    }

    /// Something the performer should be told once about this deck (a clip
    /// whose reverse cache ran out). Checked every frame; keep it cheap.
    fn warning(&self) -> Option<String> {
        None
    }

    /// For a clip: whether it has played to its end. `None` for sources that
    /// never end, which fall back to a timer. Drives clip-end auto-transitions.
    fn reached_end(&self) -> Option<bool> {
        None
    }

    /// Whether the source is holding its frame for residency.
    fn is_suspended(&self) -> bool {
        false
    }

    /// Which of Varda's own outputs this source re-enters, if any. The mixer
    /// resolves every request before decks render, one frame behind.
    fn feedback_request(&self) -> Option<&FeedbackSource> {
        None
    }

    /// The view resolved for [`Self::feedback_request`], and the label of what
    /// it shows (a channel's current name).
    fn bind_feedback(&mut self, _view: Option<wgpu::TextureView>, _label: &str) {}

    /// The ISF shader this source runs, for sources that are one. Its inputs
    /// become the deck's generator parameters.
    fn shader(&self) -> Option<&ISFShader> {
        None
    }

    /// Analyzer-driven texture slots this source binds.
    fn preprocessor_slots(&self) -> &[PreprocessorSlot] {
        &[]
    }

    fn preprocessor_slots_mut(&mut self) -> Option<&mut Vec<PreprocessorSlot>> {
        None
    }

    /// Rewrite UUIDs this source refers to (a channel) after a paste or import
    /// gave them new ones.
    fn remap_ids(&mut self, _map: &HashMap<String, String>) {}

    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

/// Every declared control's current value.
pub fn status_from_params<S: DeckSourceInstance + ?Sized>(source: &S) -> SourceStatus {
    let mut status = SourceStatus::default();
    for spec in source.schema() {
        if let Some(value) = source.param(&spec.name) {
            status.params.insert(spec.name.clone(), value);
        }
    }
    status
}

/// Write the control at router path `route` with a normalized value: what a
/// MIDI fader, an OSC message or a macro does.
///
/// # Errors
///
/// Fails when no control has that route, or the source refused the write.
pub fn write_route(
    source: &mut dyn DeckSourceInstance,
    route: &str,
    normalized: f32,
) -> Result<(), SourceParamError> {
    let spec = source
        .schema()
        .iter()
        .find(|s| s.route.as_deref() == Some(route))
        .ok_or_else(|| SourceParamError::Unknown(route.to_string()))?;
    let name = spec.name.clone();
    match spec.kind {
        SourceParamKind::Action => {
            if normalized > 0.5 {
                source.trigger(&name)
            } else {
                Ok(())
            }
        }
        SourceParamKind::Float { .. } | SourceParamKind::Choice { .. } => {
            source.set_param(&name, &SourceValue::Float(normalized.clamp(0.0, 1.0)))
        }
        SourceParamKind::Toggle => source.set_param(&name, &SourceValue::Bool(normalized > 0.5)),
        SourceParamKind::Color | SourceParamKind::Text | SourceParamKind::Number { .. } => Err(
            SourceParamError::Invalid(format!("'{route}' takes a typed value, not a fader")),
        ),
    }
}

/// The normalized value of the control at router path `route`: what a
/// controller's LEDs show.
pub fn read_route(source: &dyn DeckSourceInstance, route: &str) -> Option<f32> {
    let spec = source
        .schema()
        .iter()
        .find(|s| s.route.as_deref() == Some(route))?;
    source.param(&spec.name)?.as_f32()
}

/// Whether the control at `route` may be driven by modulation.
pub fn route_is_modulatable(source: &dyn DeckSourceInstance, route: &str) -> bool {
    source
        .schema()
        .iter()
        .any(|s| s.route.as_deref() == Some(route) && s.modulatable)
}

/// Read a normalized float out of a control write.
///
/// # Errors
///
/// Fails when the value is not a number or a boolean.
pub fn expect_norm(name: &str, value: &SourceValue) -> Result<f32, SourceParamError> {
    value
        .as_f32()
        .map(|v| v.clamp(0.0, 1.0))
        .ok_or_else(|| SourceParamError::Invalid(format!("'{name}' takes a number")))
}

/// Read text out of a control write.
///
/// # Errors
///
/// Fails when the value is not text.
pub fn expect_text<'v>(name: &str, value: &'v SourceValue) -> Result<&'v str, SourceParamError> {
    value
        .as_str()
        .ok_or_else(|| SourceParamError::Invalid(format!("'{name}' takes text")))
}

/// Read a color out of a control write.
///
/// # Errors
///
/// Fails when the value is not a color.
pub fn expect_color(name: &str, value: &SourceValue) -> Result<[f32; 4], SourceParamError> {
    match value {
        SourceValue::Color(c) => Ok(*c),
        _ => Err(SourceParamError::Invalid(format!("'{name}' takes a color"))),
    }
}

/// Encode a provider's config struct, which is always a plain object.
pub fn encode_config<T: serde::Serialize>(source_type: &str, value: &T) -> SourceConfig {
    SourceConfig::encode(source_type, value).unwrap_or_else(|e| {
        log::error!("source config for '{source_type}' did not encode: {e}");
        SourceConfig::new(source_type)
    })
}

/// Decode a provider's config struct, naming the source type on failure.
///
/// # Errors
///
/// Fails when a field is missing or malformed.
pub fn decode_config<T: serde::de::DeserializeOwned>(config: &SourceConfig) -> Result<T> {
    config
        .decode()
        .map_err(|e| anyhow::anyhow!("invalid {} source config: {e}", config.source_type()))
}

/// The normalized value a discrete control's modulation points at, ready for
/// the control's own bucketing.
pub fn discrete_value(resolved: &ResolvedModulation) -> f32 {
    resolved
        .absolute
        .unwrap_or(resolved.additive)
        .clamp(0.0, 1.0)
}

/// A normalized fader bucketed into `n` equal choices.
pub fn choice_index(normalized: f32, n: usize) -> usize {
    crate::params::bucket_index(normalized, n)
}

/// The normalized value at the centre of choice `index` of `n`.
pub fn choice_value(index: usize, n: usize) -> f32 {
    crate::params::bucket_center(index, n)
}

/// Downcast helper for providers reaching their own instance type.
pub fn downcast_mut<T: 'static>(instance: &mut dyn DeckSourceInstance) -> Option<&mut T> {
    instance.as_any_mut().downcast_mut::<T>()
}

/// Downcast helper for providers reaching their own instance type.
pub fn downcast_ref<T: 'static>(instance: &dyn DeckSourceInstance) -> Option<&T> {
    instance.as_any().downcast_ref::<T>()
}
