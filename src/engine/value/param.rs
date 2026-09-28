//! The address of every parameter a control surface, the API, automation, or
//! modulation can reach.
//!
//! One type, one spelling: `Display` renders the canonical path and `FromStr`
//! parses it, along with the older spellings still found in saved bindings.
//! See /spec/parameter-routing.md (WS4).

use std::fmt;
use std::str::FromStr;

/// Separator between a parameter-bag prefix and a parameter name in a
/// modulation key: `deck/<u>/param` + `/` + `speed`.
pub const PARAM_KEY_SEPARATOR: char = '/';

/// Prefix of the modulation keys for a deck's shader parameters.
pub fn deck_param_prefix(deck: &str) -> String {
    format!("deck/{deck}/param")
}

/// Prefix of the modulation keys for an effect's parameters.
pub fn effect_param_prefix(effect: &str) -> String {
    format!("effect/{effect}/param")
}

/// Prefix every key a deck owns shares (`deck/<u>/`): its built-ins and its
/// shader parameters. For matching or removing them together.
pub fn deck_prefix(deck: &str) -> String {
    format!("deck/{deck}/")
}

/// Prefix every key a channel owns shares (`ch/<u>/`).
pub fn channel_prefix(channel: &str) -> String {
    format!("ch/{channel}/")
}

/// Prefix every key an effect owns shares (`effect/<u>/`).
pub fn effect_prefix(effect: &str) -> String {
    format!("effect/{effect}/")
}

/// Prefix every key a modulator owns shares (`mod/<u>/`).
pub fn modulator_prefix(source: &str) -> String {
    format!("mod/{source}/")
}

/// Whether a value with components is a color or a point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentKind {
    Color,
    Point,
}

impl ComponentKind {
    pub fn components(self) -> &'static [Component] {
        match self {
            Self::Color => &[Component::R, Component::G, Component::B, Component::A],
            Self::Point => &[Component::X, Component::Y],
        }
    }

    /// The component at `index`, for keys saved with an index before scene
    /// version 9.
    pub fn component(self, index: usize) -> Option<Component> {
        self.components().get(index).copied()
    }
}

/// One channel of a color or one axis of a point, addressed by a path suffix
/// (`.../color/r`, `.../param/offset/x`). Source controls and shader
/// parameters use the same suffixes. See /spec/deck-source-providers.md
/// § One component vocabulary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Component {
    R,
    G,
    B,
    A,
    X,
    Y,
}

impl Component {
    pub fn kind(self) -> ComponentKind {
        match self {
            Self::R | Self::G | Self::B | Self::A => ComponentKind::Color,
            Self::X | Self::Y => ComponentKind::Point,
        }
    }

    /// Position in the value's array: `[r, g, b, a]` or `[x, y]`.
    pub fn index(self) -> usize {
        match self {
            Self::R | Self::X => 0,
            Self::G | Self::Y => 1,
            Self::B => 2,
            Self::A => 3,
        }
    }

    pub fn suffix(self) -> &'static str {
        match self {
            Self::R => "r",
            Self::G => "g",
            Self::B => "b",
            Self::A => "a",
            Self::X => "x",
            Self::Y => "y",
        }
    }

    /// Split `base/<suffix>` into the base and the component. `None` when the
    /// last segment is not a component suffix. The caller decides whether the
    /// base names a color or point; only then is the suffix a component.
    pub fn split(path: &str) -> Option<(&str, Self)> {
        let (base, suffix) = path.rsplit_once(PARAM_KEY_SEPARATOR)?;
        let component = match suffix {
            "r" => Self::R,
            "g" => Self::G,
            "b" => Self::B,
            "a" => Self::A,
            "x" => Self::X,
            "y" => Self::Y,
            _ => return None,
        };
        (!base.is_empty()).then_some((base, component))
    }

    /// `base/<suffix>`.
    pub fn path(self, base: &str) -> String {
        format!("{base}{PARAM_KEY_SEPARATOR}{}", self.suffix())
    }

    /// Append `/<suffix>` to `key`, for building keys in a reused buffer.
    pub fn push_suffix(self, key: &mut String) {
        key.push(PARAM_KEY_SEPARATOR);
        key.push_str(self.suffix());
    }

    /// This component of `values`, when `values` is the kind it belongs to.
    pub fn read(self, values: &[f32]) -> Option<f32> {
        self.fits(values).then(|| values[self.index()])
    }

    /// Replace this component of `values`. False when `values` is the other
    /// kind of value.
    pub fn write(self, values: &mut [f32], value: f32) -> bool {
        if !self.fits(values) {
            return false;
        }
        values[self.index()] = value;
        true
    }

    fn fits(self, values: &[f32]) -> bool {
        values.len() == self.kind().components().len()
    }
}

/// The canonical spelling of a router path, or `path` unchanged when it names
/// no parameter. Older saved bindings (`video/seek`, owner-qualified effect
/// paths) are rewritten this way as they load.
pub fn canonical_path(path: &str) -> String {
    path.parse::<ParamAddress>()
        .map_or_else(|_| path.to_string(), |address| address.to_string())
}

/// A parameter or action, named by the stable UUIDs of what owns it.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ParamAddress {
    /// `crossfader`
    Crossfader,
    /// `action/<name>`: a global action (undo, redo, save, record, ...).
    Action(String),
    /// `cue/<uuid>/fire`
    CueFire { cue: String },
    /// `ch/<uuid>/opacity`
    ChannelOpacity { channel: String },
    /// `deck/<uuid>/...`
    Deck { deck: String, target: DeckTarget },
    /// `effect/<uuid>/param/<name>`. Effects are addressed by UUID alone; the
    /// chain that owns one never changes.
    EffectParam { effect: String, param: String },
    /// `mod/<uuid>/<param>` or `mod/<uuid>/step/<n>`
    Modulator {
        source: String,
        target: ModulatorTarget,
    },
    /// `macro/<uuid>/value`
    MacroValue { macro_uuid: String },
    /// `output/<uuid>/...`. See /spec/output-sink-providers.md Decision 11.
    Output {
        output: String,
        target: OutputControl,
    },
    /// `surface/<uuid>/source`: what a surface shows, as text
    /// (`master`, `domemaster`, `ch/<uuid>`, `chs/<uuid>,<uuid>`,
    /// `deck/<uuid>`). See /spec/output-sink-providers.md Decision 12.
    SurfaceSource { surface: String },
}

/// What on an output an address names.
///
/// The output's own controls are listed here. Its sink's settings are
/// [`OutputControl::Sink`], a route the sink type declares: the address layer
/// names no sink type.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum OutputControl {
    /// `start`: begin delivering (a press).
    Start,
    /// `stop`: stop delivering (a press).
    Stop,
    /// `active`: delivering or not, as one toggle.
    Active,
    /// `calibration`: Off, Projector or Surfaces.
    Calibration,
    /// `rotation`: 0°, 90°, 180° or 270°.
    Rotation,
    /// `surface/<uuid>`: whether the surface is shown on this output.
    Surface(String),
    /// A setting of the output's sink, by the route its sink type declares.
    Sink(String),
}

/// What on a deck an address names.
///
/// The deck's own controls are listed here. Its source's controls are
/// [`DeckTarget::Source`], a route the source type declares (`video/speed`,
/// `capture/rate`, `scaling_mode`): the address layer names no source type.
/// See /spec/deck-source-providers.md.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DeckTarget {
    Opacity,
    Mute,
    Solo,
    Trigger,
    /// `at/play_duration`
    AutoTransitionPlay,
    /// `at/trans_duration`
    AutoTransitionFade,
    /// Toggles whether the deck keeps its source alpha.
    Transparent,
    /// `depth_prepro/<name>`: a shader deck's depth-sensor preprocessor.
    DepthPreprocess(String),
    /// `param/<name>`: a generator parameter.
    Param(String),
    /// A control of the deck's source, by the route its source type declares.
    Source(String),
}

impl DeckTarget {
    pub fn param(name: &str) -> Self {
        Self::Param(name.to_string())
    }

    pub fn source(route: &str) -> Self {
        Self::Source(route.to_string())
    }

    pub fn depth_preprocess(name: &str) -> Self {
        Self::DepthPreprocess(name.to_string())
    }
}

/// What on a modulator an address names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ModulatorTarget {
    Param(String),
    Step(usize),
}

impl ParamAddress {
    pub fn deck(deck: &str, target: DeckTarget) -> Self {
        Self::Deck {
            deck: deck.to_string(),
            target,
        }
    }

    pub fn deck_param(deck: &str, name: &str) -> Self {
        Self::deck(deck, DeckTarget::param(name))
    }

    pub fn effect_param(effect: &str, param: &str) -> Self {
        Self::EffectParam {
            effect: effect.to_string(),
            param: param.to_string(),
        }
    }

    pub fn channel_opacity(channel: &str) -> Self {
        Self::ChannelOpacity {
            channel: channel.to_string(),
        }
    }

    pub fn macro_value(macro_uuid: &str) -> Self {
        Self::MacroValue {
            macro_uuid: macro_uuid.to_string(),
        }
    }

    pub fn modulator_param(source: &str, param: &str) -> Self {
        Self::Modulator {
            source: source.to_string(),
            target: ModulatorTarget::Param(param.to_string()),
        }
    }

    pub fn modulator_step(source: &str, step: usize) -> Self {
        Self::Modulator {
            source: source.to_string(),
            target: ModulatorTarget::Step(step),
        }
    }

    pub fn output(output: &str, target: OutputControl) -> Self {
        Self::Output {
            output: output.to_string(),
            target,
        }
    }

    pub fn surface_source(surface: &str) -> Self {
        Self::SurfaceSource {
            surface: surface.to_string(),
        }
    }

    pub fn cue_fire(cue: &str) -> Self {
        Self::CueFire {
            cue: cue.to_string(),
        }
    }

    /// Whether modulation or automation can drive this address. Actions,
    /// triggers, and in/out points cannot: an in or out point is the reference
    /// a position offset is scaled against, so modulating it would feed back.
    pub fn is_modulatable(&self) -> bool {
        match self {
            Self::ChannelOpacity { .. }
            | Self::EffectParam { .. }
            | Self::MacroValue { .. }
            | Self::Modulator {
                target: ModulatorTarget::Param(_),
                ..
            } => true,
            // Whether a source control is modulatable is its source type's
            // call, which this layer cannot see; the engine checks the
            // schema before it accepts an assignment.
            Self::Deck { target, .. } => matches!(
                target,
                DeckTarget::Opacity | DeckTarget::Param(_) | DeckTarget::Source(_)
            ),
            // Output controls start encoders, rebuild sinks and re-route
            // surfaces: discrete changes no curve should sweep.
            Self::Output { .. }
            | Self::SurfaceSource { .. }
            | Self::Crossfader
            | Self::Action(_)
            | Self::CueFire { .. }
            | Self::Modulator {
                target: ModulatorTarget::Step(_),
                ..
            } => false,
        }
    }

    /// Read a modulation key written before scene version 8
    /// (`deck_<u>:<name>`, `fx_<u>:<name>`, `ch_<u>:opacity`, `macro_<u>:value`,
    /// `mod:<u>:<param>`). Only the scene and preset migrations need this.
    pub fn from_legacy_modulation_key(key: &str) -> Option<Self> {
        if let Some(rest) = key.strip_prefix("mod:") {
            let (source, param) = rest.split_once(':')?;
            return Some(Self::modulator_param(source, param));
        }
        let (owner, name) = key.split_once(':')?;
        if let Some(deck) = owner.strip_prefix("deck_") {
            // Before v8 a shader parameter named `opacity` shared this key with
            // the deck's own opacity. The deck built-in is what it meant.
            let target = match name {
                "opacity" => DeckTarget::Opacity,
                "video_speed" => DeckTarget::source("video/speed"),
                "video_position" => DeckTarget::source("video/position"),
                "video_play" => DeckTarget::source("video/play"),
                "video_loop_mode" => DeckTarget::source("video/loop_mode"),
                "scaling_mode" => DeckTarget::source("scaling_mode"),
                _ => DeckTarget::Param(name.to_string()),
            };
            return Some(Self::deck(deck, target));
        }
        if let Some(effect) = owner.strip_prefix("fx_") {
            return Some(Self::effect_param(effect, name));
        }
        if let Some(channel) = owner.strip_prefix("ch_") {
            return (name == "opacity").then(|| Self::channel_opacity(channel));
        }
        if let Some(macro_uuid) = owner.strip_prefix("macro_") {
            return (name == "value").then(|| Self::macro_value(macro_uuid));
        }
        None
    }
}

impl fmt::Display for DeckTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Opacity => f.write_str("opacity"),
            Self::Mute => f.write_str("mute"),
            Self::Solo => f.write_str("solo"),
            Self::Trigger => f.write_str("trigger"),
            Self::AutoTransitionPlay => f.write_str("at/play_duration"),
            Self::AutoTransitionFade => f.write_str("at/trans_duration"),
            Self::Transparent => f.write_str("transparent"),
            Self::DepthPreprocess(name) => write!(f, "depth_prepro/{name}"),
            Self::Param(name) => write!(f, "param/{name}"),
            Self::Source(route) => f.write_str(route),
        }
    }
}

impl fmt::Display for OutputControl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start => f.write_str("start"),
            Self::Stop => f.write_str("stop"),
            Self::Active => f.write_str("active"),
            Self::Calibration => f.write_str("calibration"),
            Self::Rotation => f.write_str("rotation"),
            Self::Surface(surface) => write!(f, "surface/{surface}"),
            Self::Sink(route) => f.write_str(route),
        }
    }
}

impl fmt::Display for ParamAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Crossfader => f.write_str("crossfader"),
            Self::Action(name) => write!(f, "action/{name}"),
            Self::CueFire { cue } => write!(f, "cue/{cue}/fire"),
            Self::ChannelOpacity { channel } => write!(f, "ch/{channel}/opacity"),
            Self::Deck { deck, target } => write!(f, "deck/{deck}/{target}"),
            Self::EffectParam { effect, param } => write!(f, "effect/{effect}/param/{param}"),
            Self::Modulator {
                source,
                target: ModulatorTarget::Param(param),
            } => write!(f, "mod/{source}/{param}"),
            Self::Modulator {
                source,
                target: ModulatorTarget::Step(n),
            } => write!(f, "mod/{source}/step/{n}"),
            Self::MacroValue { macro_uuid } => write!(f, "macro/{macro_uuid}/value"),
            Self::Output { output, target } => write!(f, "output/{output}/{target}"),
            Self::SurfaceSource { surface } => write!(f, "surface/{surface}/source"),
        }
    }
}

/// A string that names no parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownParamPath(pub String);

impl fmt::Display for UnknownParamPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown parameter path '{}'", self.0)
    }
}

impl std::error::Error for UnknownParamPath {}

impl FromStr for ParamAddress {
    type Err = UnknownParamPath;

    fn from_str(path: &str) -> Result<Self, Self::Err> {
        let unknown = || UnknownParamPath(path.to_string());
        let parts: Vec<&str> = path.split('/').collect();
        let owned = |s: &str| s.to_string();
        let address = match parts.as_slice() {
            ["crossfader"] => Self::Crossfader,
            ["action", name] if !name.is_empty() => Self::Action(owned(name)),
            ["cue", cue, "fire"] => Self::CueFire { cue: owned(cue) },
            ["ch", channel, "opacity"] => Self::channel_opacity(channel),
            ["effect", effect, "param", param]
            | ["deck" | "ch", _, "effect", effect, "param", param]
            | ["master", "effect", effect, "param", param] => Self::effect_param(effect, param),
            ["mod", source, "step", n] => Self::Modulator {
                source: owned(source),
                target: ModulatorTarget::Step(n.parse().map_err(|_| unknown())?),
            },
            ["mod", source, param] => Self::modulator_param(source, param),
            ["macro", macro_uuid, "value"] => Self::macro_value(macro_uuid),
            ["surface", surface, "source"] => Self::surface_source(surface),
            ["output", output, rest @ ..] => {
                let target = match rest {
                    ["start"] => OutputControl::Start,
                    ["stop"] => OutputControl::Stop,
                    ["active"] => OutputControl::Active,
                    ["calibration"] => OutputControl::Calibration,
                    ["rotation"] => OutputControl::Rotation,
                    ["surface", surface] => OutputControl::Surface(owned(surface)),
                    [] => return Err(unknown()),
                    // Anything else is a sink setting; whether the output's
                    // sink has it is answered when the path is routed.
                    route => OutputControl::Sink(route.join("/")),
                };
                Self::output(output, target)
            }
            ["deck", deck, rest @ ..] => {
                let target = match rest {
                    ["opacity"] => DeckTarget::Opacity,
                    ["mute"] => DeckTarget::Mute,
                    ["solo"] => DeckTarget::Solo,
                    ["trigger"] => DeckTarget::Trigger,
                    ["at", "play_duration"] => DeckTarget::AutoTransitionPlay,
                    ["at", "trans_duration"] => DeckTarget::AutoTransitionFade,
                    ["transparent"] => DeckTarget::Transparent,
                    ["depth_prepro", name] => DeckTarget::DepthPreprocess(owned(name)),
                    ["param", name] => DeckTarget::Param(owned(name)),
                    // The playhead's older spelling, still in saved bindings.
                    ["video", "seek"] => DeckTarget::source("video/position"),
                    [] => return Err(unknown()),
                    // Anything else is a source control; whether the deck's
                    // source has it is answered when the path is routed.
                    route => DeckTarget::Source(route.join("/")),
                };
                Self::deck(deck, target)
            }
            _ => return Err(unknown()),
        };
        let has_empty_segment = parts.iter().any(|p| p.is_empty());
        if has_empty_segment {
            return Err(unknown());
        }
        Ok(address)
    }
}

impl serde::Serialize for ParamAddress {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ParamAddress {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let path = String::deserialize(deserializer)?;
        path.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn component_suffixes_split_and_rebuild() {
        for kind in [ComponentKind::Color, ComponentKind::Point] {
            for (index, &component) in kind.components().iter().enumerate() {
                assert_eq!(component.kind(), kind);
                assert_eq!(component.index(), index);
                assert_eq!(kind.component(index), Some(component));
                let path = component.path("deck/d1/color");
                assert_eq!(Component::split(&path), Some(("deck/d1/color", component)));
            }
        }
        assert_eq!(ComponentKind::Point.component(2), None);
    }

    #[test]
    fn a_path_without_a_component_suffix_does_not_split() {
        assert_eq!(Component::split("video/speed"), None);
        assert_eq!(Component::split("capture/crop_x"), None);
        assert_eq!(Component::split("r"), None);
        assert_eq!(Component::split("/r"), None);
    }

    #[test]
    fn a_component_reads_and_writes_only_its_own_kind_of_value() {
        let mut color = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(Component::B.read(&color), Some(0.3));
        assert!(Component::G.write(&mut color, 0.9));
        assert_eq!(color, [0.1, 0.9, 0.3, 0.4]);

        let mut point = [0.5, 0.6];
        assert_eq!(Component::Y.read(&point), Some(0.6));
        assert_eq!(Component::R.read(&point), None);
        assert!(!Component::A.write(&mut point, 1.0));
        assert!(!Component::X.write(&mut color, 1.0));
        assert_eq!(point, [0.5, 0.6]);
    }

    /// Outputs and surfaces are addressed like every other entity, by UUID.
    /// See /spec/output-sink-providers.md Decisions 11 and 12.
    #[test]
    fn output_and_surface_paths_round_trip() {
        let cases = [
            (
                "output/o1/start",
                ParamAddress::output("o1", OutputControl::Start),
            ),
            (
                "output/o1/stop",
                ParamAddress::output("o1", OutputControl::Stop),
            ),
            (
                "output/o1/active",
                ParamAddress::output("o1", OutputControl::Active),
            ),
            (
                "output/o1/calibration",
                ParamAddress::output("o1", OutputControl::Calibration),
            ),
            (
                "output/o1/rotation",
                ParamAddress::output("o1", OutputControl::Rotation),
            ),
            (
                "output/o1/surface/s1",
                ParamAddress::output("o1", OutputControl::Surface("s1".into())),
            ),
            (
                "output/o1/stream/name",
                ParamAddress::output("o1", OutputControl::Sink("stream/name".into())),
            ),
            ("surface/s1/source", ParamAddress::surface_source("s1")),
        ];
        for (path, address) in cases {
            assert_eq!(path.parse::<ParamAddress>(), Ok(address.clone()), "{path}");
            assert_eq!(address.to_string(), path);
            assert!(!address.is_modulatable(), "{path}");
        }
        assert!("output/o1".parse::<ParamAddress>().is_err());
        assert!("surface/s1/warp".parse::<ParamAddress>().is_err());
    }

    /// Every canonical form, as written today and after v8.
    const CANONICAL: &[&str] = &[
        "crossfader",
        "action/undo",
        "cue/c1/fire",
        "ch/c1/opacity",
        "deck/d1/opacity",
        "deck/d1/mute",
        "deck/d1/solo",
        "deck/d1/trigger",
        "deck/d1/at/play_duration",
        "deck/d1/at/trans_duration",
        "deck/d1/video/play",
        "deck/d1/video/speed",
        "deck/d1/video/position",
        "deck/d1/video/in_point",
        "deck/d1/video/out_point",
        "deck/d1/video/clear",
        "deck/d1/video/loop_mode",
        "deck/d1/scaling_mode",
        "deck/d1/transparent",
        "deck/d1/html/reload",
        "deck/d1/html/interactive",
        "deck/d1/capture/rate",
        "deck/d1/depth/near",
        "deck/d1/depth_prepro/mirror",
        "deck/d1/param/speed",
        "effect/f1/param/warp",
        "mod/m1/frequency",
        "mod/m1/step/3",
        "macro/k1/value",
    ];

    #[test]
    fn every_canonical_path_round_trips() {
        for path in CANONICAL {
            let address: ParamAddress = path.parse().unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(address.to_string(), *path);
        }
    }

    #[test]
    fn older_spellings_parse_to_the_canonical_address() {
        for (old, canonical) in [
            ("deck/d1/video/seek", "deck/d1/video/position"),
            ("deck/d1/effect/f1/param/warp", "effect/f1/param/warp"),
            ("ch/c1/effect/f1/param/warp", "effect/f1/param/warp"),
            ("master/effect/f1/param/warp", "effect/f1/param/warp"),
        ] {
            let address: ParamAddress = old.parse().unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(address.to_string(), canonical, "{old}");
        }
    }

    #[test]
    fn malformed_paths_are_refused() {
        for path in [
            "",
            "deck",
            "deck/d1",
            "deck//opacity",
            "deck/d1/video/",
            "mod/m1/step/x",
            "action/",
            "effect/f1/warp",
        ] {
            assert!(path.parse::<ParamAddress>().is_err(), "{path:?}");
        }
    }

    #[test]
    fn legacy_modulation_keys_map_to_what_they_drove() {
        for (key, path) in [
            ("deck_d1:opacity", "deck/d1/opacity"),
            ("deck_d1:speed", "deck/d1/param/speed"),
            ("deck_d1:video_speed", "deck/d1/video/speed"),
            ("deck_d1:video_position", "deck/d1/video/position"),
            ("deck_d1:video_play", "deck/d1/video/play"),
            ("deck_d1:video_loop_mode", "deck/d1/video/loop_mode"),
            ("deck_d1:scaling_mode", "deck/d1/scaling_mode"),
            ("fx_f1:warp", "effect/f1/param/warp"),
            ("ch_c1:opacity", "ch/c1/opacity"),
            ("macro_k1:value", "macro/k1/value"),
            ("mod:m1:frequency", "mod/m1/frequency"),
        ] {
            let address = ParamAddress::from_legacy_modulation_key(key)
                .unwrap_or_else(|| panic!("{key} did not map"));
            assert_eq!(address.to_string(), path, "{key}");
        }
        assert_eq!(ParamAddress::from_legacy_modulation_key("nonsense"), None);
        assert_eq!(ParamAddress::from_legacy_modulation_key("ch_c1:mute"), None);
    }

    #[test]
    fn only_what_modulation_can_drive_is_modulatable() {
        let yes = [
            "deck/d1/opacity",
            "deck/d1/param/speed",
            "deck/d1/video/position",
            "effect/f1/param/warp",
            "ch/c1/opacity",
            "macro/k1/value",
            "mod/m1/frequency",
        ];
        let no = [
            "crossfader",
            "action/undo",
            "cue/c1/fire",
            "deck/d1/trigger",
            "mod/m1/step/0",
        ];
        for path in yes {
            assert!(
                path.parse::<ParamAddress>().unwrap().is_modulatable(),
                "{path}"
            );
        }
        for path in no {
            assert!(
                !path.parse::<ParamAddress>().unwrap().is_modulatable(),
                "{path}"
            );
        }
    }

    #[test]
    fn any_other_deck_route_names_a_source_control() {
        let address: ParamAddress = "deck/d1/capture/crop_x".parse().unwrap();
        assert_eq!(
            address,
            ParamAddress::deck("d1", DeckTarget::source("capture/crop_x"))
        );
        assert_eq!(address.to_string(), "deck/d1/capture/crop_x");
    }

    #[test]
    fn serializes_as_its_path() {
        let address = ParamAddress::effect_param("f1", "warp");
        let json = serde_json::to_string(&address).unwrap();
        assert_eq!(json, "\"effect/f1/param/warp\"");
        let back: ParamAddress = serde_json::from_str("\"master/effect/f1/param/warp\"").unwrap();
        assert_eq!(back, address);
    }
}
