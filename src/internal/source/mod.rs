//! Deck source providers.
//!
//! A deck's source draws its base image before the effect chain: a shader, a
//! clip, a camera, a stream. Each kind is a provider implementing
//! [`DeckSourceProvider`], registered once, which builds a
//! [`DeckSourceInstance`] per deck. The library panel, deck controls, API,
//! persistence and router all work through these traits.

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

pub use crate::engine::value::param::{Component, ComponentKind};
pub use crate::engine::value::provider::{
    ControlKind, ControlSpec, ControlStatus, ControlValue, LibraryCreate, LibraryEntry,
    LibraryNotice, LibrarySection, ProviderTypeSnapshot, WidgetHint,
};

pub use crate::engine::value::source::{DeckSourceSnapshot, ScalingMode, SourceConfig};

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
pub enum ControlError {
    /// This source has no control by that name or route.
    Unknown(String),
    /// The value has the wrong shape for the control.
    Invalid(String),
    /// The control exists but the source cannot take it right now.
    State(String),
}

impl std::fmt::Display for ControlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(name) => write!(f, "unknown source parameter '{name}'"),
            Self::Invalid(msg) | Self::State(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for ControlError {}

/// What a source re-enters from Varda's own output.
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
    /// Whether the deck keeps its source's alpha.
    pub transparent: bool,
    pub time: f32,
    pub time_delta: f32,
    pub frame_index: u32,
    /// The deck's phase accumulators.
    pub phase_times: [f32; 4],
    pub audio: &'a AudioData,
    pub modulation: &'a ModulationEngine,
    /// Modulation key prefix of the deck's generator parameters.
    pub param_prefix: &'a str,
    /// The deck's generator parameters, for sources that declare them.
    pub params: &'a mut ShaderParams,
    pub cmd_buffers: &'a mut Vec<wgpu::CommandBuffer>,
}

/// The clocks a source's controls may follow this frame.
#[derive(Debug, Clone, Copy, Default)]
pub struct SourceClock {
    pub transport: Option<crate::timebase::TransportSample>,
    /// The Beat timebase, when a tempo source is running.
    pub beat: Option<crate::timebase::TimeContext>,
    /// Free-running seconds since the previous frame.
    pub dt: f32,
}

/// Per-frame context for a source's own controls, handed to
/// [`DeckSourceInstance::control`] before anything renders.
pub struct SourceControl<'a> {
    pub deck_uuid: &'a str,
    pub modulation: &'a ModulationEngine,
    /// False while the arrangement has this deck asleep. A sleeping source
    /// holds its frame and must not be driven.
    pub awake: bool,
    pub transport: Option<crate::timebase::TransportSample>,
    /// The Beat timebase, when a tempo source is running.
    pub beat: Option<crate::timebase::TimeContext>,
    /// Free-running seconds since the previous frame.
    pub dt: f32,
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
        clock: SourceClock,
        target_fps: u32,
        scratch: &'a mut String,
    ) -> Self {
        Self {
            deck_uuid,
            modulation,
            awake,
            transport: clock.transport,
            beat: clock.beat,
            dt: clock.dt,
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
        self.resolve_key(route, None)
    }

    fn resolve_key(
        &mut self,
        route: &str,
        component: Option<Component>,
    ) -> Option<ResolvedModulation> {
        if !self.modulation.has_modulation_for_any() {
            return None;
        }
        self.scratch.clear();
        self.scratch.push_str("deck/");
        self.scratch.push_str(self.deck_uuid);
        self.scratch.push('/');
        self.scratch.push_str(route);
        if let Some(component) = component {
            component.push_suffix(self.scratch);
        }
        if !self.modulation.has_modulation(self.scratch) {
            return None;
        }
        Some(self.modulation.resolve(self.scratch))
    }

    /// A normalized float control at `route` with its modulation applied.
    pub fn modulated_norm(&mut self, route: &str, base: f32) -> f32 {
        self.resolve(route)
            .map_or(base, |resolved| apply_modulation(base, &resolved))
    }

    /// A color control at `route` with each channel's modulation applied.
    pub fn modulated_color(&mut self, route: &str, base: [f32; 4]) -> [f32; 4] {
        let mut out = base;
        for &component in ComponentKind::Color.components() {
            if let Some(resolved) = self.resolve_key(route, Some(component)) {
                let i = component.index();
                out[i] = apply_modulation(base[i], &resolved);
            }
        }
        out
    }

    /// A point control at `route`, ranging over `min..=max`, with each axis's
    /// modulation applied in the control's normalized space.
    pub fn modulated_point(&mut self, route: &str, base: [f32; 2], min: f32, max: f32) -> [f32; 2] {
        let span = max - min;
        if span <= 0.0 {
            return base;
        }
        let mut out = base;
        for &component in ComponentKind::Point.components() {
            if let Some(resolved) = self.resolve_key(route, Some(component)) {
                let i = component.index();
                let norm = ((base[i] - min) / span).clamp(0.0, 1.0);
                out[i] = min + apply_modulation(norm, &resolved) * span;
            }
        }
        out
    }
}

/// A normalized base value with resolved modulation applied: an absolute
/// source replaces the base, additive sources add to it.
pub fn apply_modulation(base: f32, resolved: &ResolvedModulation) -> f32 {
    (resolved.absolute.unwrap_or(base) + resolved.additive).clamp(0.0, 1.0)
}

/// A kind of deck source.
///
/// Registered once for the process. It lists what can be created, builds one
/// [`DeckSourceInstance`] per deck, and services the device managers its
/// instances read from once per frame.
pub trait DeckSourceProvider: 'static {
    /// Stable id: the `type` tag in scene files and the API, in CamelCase
    /// (`Video`, `SolidColor`). Never rename it.
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;
    /// A short glyph shown next to the label.
    fn icon(&self) -> &'static str;

    /// The controls every deck of this type has.
    fn params(&self) -> &'static [ControlSpec] {
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

    /// Whether adding the same source (see [`Self::identity`]) as a deck
    /// already on the channel returns that deck instead of a second one. For
    /// sources an external controller re-subscribes to on every reconnect.
    fn one_per_channel(&self) -> bool {
        false
    }

    /// Rebuild a saved deck. Defaults to [`Self::create`]. Override to keep a
    /// deck whose device is missing (a closed window, an unpublished server).
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

    /// The part of a config that identifies the source, excluding settings. A
    /// scene diff patches a deck in place when this matches and rebuilds it
    /// otherwise. Defaults to the whole config.
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
    fn status(&self, instance: &dyn DeckSourceInstance, _query: &SourceQuery) -> ControlStatus {
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
    fn schema(&self) -> &'static [ControlSpec] {
        &[]
    }

    /// Current value of a control. Numeric controls read normalized.
    fn param(&self, _name: &str) -> Option<ControlValue> {
        None
    }

    /// Write a control. Numeric controls take normalized values.
    ///
    /// # Errors
    ///
    /// Fails for an unknown control or a value of the wrong shape.
    fn set_param(&mut self, name: &str, _value: &ControlValue) -> Result<(), ControlError> {
        Err(ControlError::Unknown(name.to_string()))
    }

    /// Fire an action control.
    ///
    /// # Errors
    ///
    /// Fails for an unknown action.
    fn trigger(&mut self, action: &str) -> Result<(), ControlError> {
        Err(ControlError::Unknown(action.to_string()))
    }

    /// State for snapshots. Defaults to the current value of every control.
    fn status(&self) -> ControlStatus {
        status_from_params(self)
    }

    /// Controls that currently have no effect, with the reason shown when
    /// the performer hovers one. Read when a snapshot is built, not per frame.
    fn inactive(&self) -> Vec<(&'static str, &'static str)> {
        Vec::new()
    }

    /// Whether the source writes its own alpha, so the deck's transparent
    /// flag has no effect on it and is not offered.
    fn owns_alpha(&self) -> bool {
        false
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
pub fn status_from_params<S: DeckSourceInstance + ?Sized>(source: &S) -> ControlStatus {
    let mut status = ControlStatus::default();
    for spec in source.schema() {
        if let Some(value) = source.param(&spec.name) {
            status.params.insert(spec.name.clone(), value);
        }
    }
    status
}

/// The control at router path `route`, and which component of it when the
/// route is one channel of a color or one axis of a point (`color/r`).
pub fn route_control<'s>(
    schema: &'s [ControlSpec],
    route: &str,
) -> Option<(&'s ControlSpec, Option<Component>)> {
    if let Some(spec) = schema.iter().find(|s| s.route.as_deref() == Some(route)) {
        return Some((spec, None));
    }
    let (base, component) = Component::split(route)?;
    let spec = schema.iter().find(|s| s.route.as_deref() == Some(base))?;
    (spec.kind.component_kind() == Some(component.kind())).then_some((spec, Some(component)))
}

/// Whether the control at `route` is a color or a point.
pub fn route_component_kind(
    source: &dyn DeckSourceInstance,
    route: &str,
) -> Option<crate::engine::value::param::ComponentKind> {
    match route_control(source.schema(), route)? {
        (spec, None) => spec.kind.component_kind(),
        (_, Some(_)) => None,
    }
}

/// A point axis's value as a fader position within the control's range.
fn point_norm(kind: &ControlKind, value: f32) -> f32 {
    match kind {
        ControlKind::Point {
            display_min,
            display_max,
        } if display_max > display_min => {
            ((value - display_min) / (display_max - display_min)).clamp(0.0, 1.0)
        }
        _ => value,
    }
}

/// A fader position as a point axis's value within the control's range.
fn point_value(kind: &ControlKind, normalized: f32) -> f32 {
    match kind {
        ControlKind::Point {
            display_min,
            display_max,
        } => display_min + normalized * (display_max - display_min),
        _ => normalized,
    }
}

/// One component of a color or point control, normalized.
fn read_component(spec: &ControlSpec, value: &ControlValue, component: Component) -> Option<f32> {
    match value {
        ControlValue::Color(c) => component.read(c),
        ControlValue::Point(p) => component.read(p).map(|v| point_norm(&spec.kind, v)),
        _ => None,
    }
}

/// Write the control at router path `route` with a normalized value: what a
/// MIDI fader, an OSC message or a macro does. A component route replaces one
/// channel or axis and keeps the rest.
///
/// # Errors
///
/// Fails when no control has that route, or the source refused the write.
pub fn write_route(
    source: &mut dyn DeckSourceInstance,
    route: &str,
    normalized: f32,
) -> Result<(), ControlError> {
    let (spec, component) = route_control(source.schema(), route)
        .ok_or_else(|| ControlError::Unknown(route.to_string()))?;
    let name = spec.name.clone();
    let normalized = normalized.clamp(0.0, 1.0);
    if let Some(component) = component {
        let value = match source.param(&name) {
            Some(ControlValue::Color(mut c)) => {
                component.write(&mut c, normalized);
                ControlValue::Color(c)
            }
            Some(ControlValue::Point(mut p)) => {
                component.write(&mut p, point_value(&spec.kind, normalized));
                ControlValue::Point(p)
            }
            _ => {
                return Err(ControlError::State(format!(
                    "'{route}' has no value to change"
                )));
            }
        };
        return source.set_param(&name, &value);
    }
    match spec.kind {
        ControlKind::Action => {
            if normalized > 0.5 {
                source.trigger(&name)
            } else {
                Ok(())
            }
        }
        ControlKind::Float { .. } | ControlKind::Choice { .. } => {
            source.set_param(&name, &ControlValue::Float(normalized))
        }
        ControlKind::Toggle => source.set_param(&name, &ControlValue::Bool(normalized > 0.5)),
        ControlKind::Color
        | ControlKind::Point { .. }
        | ControlKind::Text { .. }
        | ControlKind::File { .. }
        | ControlKind::Number { .. } => Err(ControlError::Invalid(format!(
            "'{route}' takes a typed value, not a fader"
        ))),
    }
}

/// The normalized value of the control at router path `route`: what a
/// controller's LEDs show.
pub fn read_route(source: &dyn DeckSourceInstance, route: &str) -> Option<f32> {
    let (spec, component) = route_control(source.schema(), route)?;
    let value = source.param(&spec.name)?;
    match component {
        Some(component) => read_component(spec, &value, component),
        None => value.as_f32(),
    }
}

/// Whether the control at `route` may be driven by modulation. A color or
/// point is driven through its component routes, not as a whole.
pub fn route_is_modulatable(source: &dyn DeckSourceInstance, route: &str) -> bool {
    route_control(source.schema(), route).is_some_and(|(spec, component)| {
        spec.modulatable && (component.is_some() || spec.kind.component_kind().is_none())
    })
}

/// The component routes and normalized values a typed write to a color or
/// point control changes, relative to `before`. Empty for other controls.
pub fn changed_components(
    spec: &ControlSpec,
    before: Option<&ControlValue>,
    after: &ControlValue,
) -> Vec<(Component, f32)> {
    let Some(kind) = spec.kind.component_kind() else {
        return Vec::new();
    };
    kind.components()
        .iter()
        .filter_map(|&component| {
            let new = read_component(spec, after, component)?;
            let old = before.and_then(|b| read_component(spec, b, component));
            (old != Some(new)).then_some((component, new))
        })
        .collect()
}

/// Read a normalized float out of a control write.
///
/// # Errors
///
/// Fails when the value is not a number or a boolean.
pub fn expect_norm(name: &str, value: &ControlValue) -> Result<f32, ControlError> {
    value
        .as_f32()
        .map(|v| v.clamp(0.0, 1.0))
        .ok_or_else(|| ControlError::Invalid(format!("'{name}' takes a number")))
}

/// Read text out of a control write.
///
/// # Errors
///
/// Fails when the value is not text.
pub fn expect_text<'v>(name: &str, value: &'v ControlValue) -> Result<&'v str, ControlError> {
    value
        .as_str()
        .ok_or_else(|| ControlError::Invalid(format!("'{name}' takes text")))
}

/// Read a color out of a control write.
///
/// # Errors
///
/// Fails when the value is not a color.
pub fn expect_color(name: &str, value: &ControlValue) -> Result<[f32; 4], ControlError> {
    match value {
        ControlValue::Color(c) => Ok(*c),
        _ => Err(ControlError::Invalid(format!("'{name}' takes a color"))),
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
        .map_err(|e| anyhow::anyhow!("invalid {} source config: {e}", config.type_id()))
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

/// The normalized value at the center of choice `index` of `n`.
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
