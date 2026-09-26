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
}

/// What on a deck an address names.
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
    VideoPlay,
    VideoSpeed,
    /// The playhead. Parsed from `video/seek` as well, which writes it.
    VideoPosition,
    VideoInPoint,
    VideoOutPoint,
    VideoClearInOut,
    VideoLoopMode,
    ScalingMode,
    /// Toggles whether the deck keeps its source alpha.
    Transparent,
    /// Reloads an HTML deck's page.
    HtmlReload,
    /// Opens the interactive window on an HTML deck, or closes it if open.
    HtmlInteractive,
    /// `capture/<name>`
    Capture(String),
    /// `depth/<name>`
    Depth(String),
    /// `depth_prepro/<name>`
    DepthPreprocess(String),
    /// `param/<name>`: a shader parameter.
    Param(String),
}

impl DeckTarget {
    pub fn param(name: &str) -> Self {
        Self::Param(name.to_string())
    }

    pub fn capture(name: &str) -> Self {
        Self::Capture(name.to_string())
    }

    pub fn depth(name: &str) -> Self {
        Self::Depth(name.to_string())
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
            Self::Deck { target, .. } => matches!(
                target,
                DeckTarget::Opacity
                    | DeckTarget::Param(_)
                    | DeckTarget::VideoSpeed
                    | DeckTarget::VideoPosition
                    | DeckTarget::VideoPlay
                    | DeckTarget::VideoLoopMode
                    | DeckTarget::ScalingMode
            ),
            Self::Crossfader
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
                "video_speed" => DeckTarget::VideoSpeed,
                "video_position" => DeckTarget::VideoPosition,
                "video_play" => DeckTarget::VideoPlay,
                "video_loop_mode" => DeckTarget::VideoLoopMode,
                "scaling_mode" => DeckTarget::ScalingMode,
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
            Self::VideoPlay => f.write_str("video/play"),
            Self::VideoSpeed => f.write_str("video/speed"),
            Self::VideoPosition => f.write_str("video/position"),
            Self::VideoInPoint => f.write_str("video/in_point"),
            Self::VideoOutPoint => f.write_str("video/out_point"),
            Self::VideoClearInOut => f.write_str("video/clear"),
            Self::VideoLoopMode => f.write_str("video/loop_mode"),
            Self::ScalingMode => f.write_str("scaling_mode"),
            Self::Transparent => f.write_str("transparent"),
            Self::HtmlReload => f.write_str("html/reload"),
            Self::HtmlInteractive => f.write_str("html/interactive"),
            Self::Capture(name) => write!(f, "capture/{name}"),
            Self::Depth(name) => write!(f, "depth/{name}"),
            Self::DepthPreprocess(name) => write!(f, "depth_prepro/{name}"),
            Self::Param(name) => write!(f, "param/{name}"),
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
            ["deck", deck, rest @ ..] => {
                let target = match rest {
                    ["opacity"] => DeckTarget::Opacity,
                    ["mute"] => DeckTarget::Mute,
                    ["solo"] => DeckTarget::Solo,
                    ["trigger"] => DeckTarget::Trigger,
                    ["at", "play_duration"] => DeckTarget::AutoTransitionPlay,
                    ["at", "trans_duration"] => DeckTarget::AutoTransitionFade,
                    ["video", "play"] => DeckTarget::VideoPlay,
                    ["video", "speed"] => DeckTarget::VideoSpeed,
                    ["video", "position" | "seek"] => DeckTarget::VideoPosition,
                    ["video", "in_point"] => DeckTarget::VideoInPoint,
                    ["video", "out_point"] => DeckTarget::VideoOutPoint,
                    ["video", "clear"] => DeckTarget::VideoClearInOut,
                    ["video", "loop_mode"] => DeckTarget::VideoLoopMode,
                    ["scaling_mode"] => DeckTarget::ScalingMode,
                    ["transparent"] => DeckTarget::Transparent,
                    ["html", "reload"] => DeckTarget::HtmlReload,
                    ["html", "interactive"] => DeckTarget::HtmlInteractive,
                    ["capture", name] => DeckTarget::Capture(owned(name)),
                    ["depth", name] => DeckTarget::Depth(owned(name)),
                    ["depth_prepro", name] => DeckTarget::DepthPreprocess(owned(name)),
                    ["param", name] => DeckTarget::Param(owned(name)),
                    _ => return Err(unknown()),
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
            "deck/d1/nope",
            "deck//opacity",
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
            "deck/d1/video/in_point",
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
    fn serializes_as_its_path() {
        let address = ParamAddress::effect_param("f1", "warp");
        let json = serde_json::to_string(&address).unwrap();
        assert_eq!(json, "\"effect/f1/param/warp\"");
        let back: ParamAddress = serde_json::from_str("\"master/effect/f1/param/warp\"").unwrap();
        assert_eq!(back, address);
    }
}
