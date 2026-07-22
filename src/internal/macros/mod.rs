//! Macro controls: user-defined knobs, faders, and buttons that fan out to many
//! parameter targets at once. See `/spec/macro-controls.md`.
//!
//! A macro is a manually-driven, **absolute** control layered on the parameter
//! router: moving/pressing it writes to every configured target through that
//! target's own sub-range, curve, and polarity. The macro is itself addressable
//! as `macro/<uuid>/value`, so a single hardware knob mapped via MIDI learn
//! drives the macro, and the macro drives many parameters.
//!
//! This module is pure domain logic — no GPU, no UI, no engine coupling. Fan-out
//! is computed here and applied by the parameter router (see `param_router.rs`).

use crate::deck::generate_short_uuid;
use serde::{Deserialize, Serialize};

fn one() -> f32 {
    1.0
}

/// Kind of macro control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum MacroKind {
    #[default]
    Knob,
    Fader,
    Button,
}

/// Response curve applied to a macro's 0..1 value before it is mapped to a
/// target's `[min, max]` sub-range. All curves are pure `[0,1] -> [0,1]`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum MacroCurve {
    #[default]
    Linear,
    /// Ease-in (slow start): `v²`.
    Exponential,
    /// Ease-out (fast start): `sqrt(v)`.
    Logarithmic,
    /// Ease-in-out (smoothstep): `v²(3 - 2v)`.
    SCurve,
    /// Quantize into `n` discrete levels (n≥2).
    Stepped(u32),
}

impl MacroCurve {
    /// Shape a normalized 0..1 input into a normalized 0..1 output.
    pub fn apply(self, v: f32) -> f32 {
        let v = if v.is_finite() {
            v.clamp(0.0, 1.0)
        } else {
            0.0
        };
        match self {
            MacroCurve::Linear => v,
            MacroCurve::Exponential => v * v,
            MacroCurve::Logarithmic => v.sqrt(),
            MacroCurve::SCurve => v * v * (3.0 - 2.0 * v),
            MacroCurve::Stepped(n) => {
                let n = n.max(2);
                let level = (v * n as f32).floor().min((n - 1) as f32);
                level / (n - 1) as f32
            }
        }
    }
}

/// A single parameter target of a macro. `path` is a parameter-router path
/// (UUID-addressed); `min`/`max` select a sub-range in normalized 0..1 space
/// that the router scales to the parameter's native range downstream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MacroTarget {
    pub path: String,
    #[serde(default)]
    pub min: f32,
    #[serde(default = "one")]
    pub max: f32,
    #[serde(default)]
    pub curve: MacroCurve,
    #[serde(default)]
    pub invert: bool,
}

impl MacroTarget {
    /// A target on `path` spanning the full range with a linear curve.
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            min: 0.0,
            max: 1.0,
            curve: MacroCurve::Linear,
            invert: false,
        }
    }

    /// The normalized value to route to this target for the given macro value.
    pub fn routed(&self, macro_value: f32) -> f32 {
        let shaped = self.curve.apply(macro_value);
        let t = if self.invert { 1.0 - shaped } else { shaped };
        (self.min + (self.max - self.min) * t).clamp(0.0, 1.0)
    }

    /// A macro may not target another macro (loop prevention).
    pub fn is_macro_path(&self) -> bool {
        self.path == "macro" || self.path.starts_with("macro/")
    }
}

/// How a button macro responds to presses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema, Default)]
pub enum ButtonBehavior {
    /// Held = on (targets → max), released = off (targets → min).
    #[default]
    Momentary,
    /// Each press latches on/off, toggling targets between max/min.
    Toggle,
    /// Each press fires `trigger` actions once (no latched state).
    Trigger,
}

/// A global app action a Trigger button can fire. These require app-layer
/// context and reuse the same dispatch (and pending-flag drain) as the MIDI
/// `action/*` paths — see `app/inputs.rs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum GlobalAction {
    Undo,
    Redo,
    Save,
}

/// A discrete action fired by a Trigger button on its rising edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
pub enum TriggerAction {
    /// Apply a router path at a fixed value (e.g. `deck/<uuid>/trigger` → 1.0).
    Param { path: String, value: f32 },
    /// A global app action (undo/redo/save/tap-tempo).
    Global(GlobalAction),
}

/// Button-specific configuration (present only when `kind == Button`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct ButtonSpec {
    #[serde(default)]
    pub behavior: ButtonBehavior,
    /// Only used when `behavior == Trigger`. Momentary/Toggle drive `Macro.targets`.
    #[serde(default)]
    pub trigger: Vec<TriggerAction>,
}

/// The result of feeding an input into a macro: parameter writes to apply
/// through the router, plus any global app actions for the app layer to dispatch.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MacroFanout {
    /// `(router_path, normalized_value)` pairs to apply via `apply_param_by_path`.
    pub params: Vec<(String, f32)>,
    /// Global app actions (undo/redo/save/tap) to dispatch at the app layer.
    pub actions: Vec<GlobalAction>,
}

/// A user-defined macro control.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    #[serde(default = "generate_short_uuid")]
    pub uuid: String,
    pub name: String,
    #[serde(default)]
    pub kind: MacroKind,
    /// Current output state 0..1 (knob/fader position, or button on/off state).
    #[serde(default)]
    pub value: f32,
    #[serde(default)]
    pub targets: Vec<MacroTarget>,
    /// Present only for `MacroKind::Button`.
    #[serde(default)]
    pub button: Option<ButtonSpec>,
    /// Runtime-only: last raw input, for button rising-edge detection.
    #[serde(skip)]
    last_input: f32,
}

impl Macro {
    /// Create a new macro of the given kind with an empty target list.
    pub fn new(kind: MacroKind, name: impl Into<String>) -> Self {
        Self {
            uuid: generate_short_uuid(),
            name: name.into(),
            kind,
            value: 0.0,
            targets: Vec::new(),
            button: if matches!(kind, MacroKind::Button) {
                Some(ButtonSpec::default())
            } else {
                None
            },
            last_input: 0.0,
        }
    }

    /// Change the macro kind, adding/removing the button spec as appropriate.
    pub fn set_kind(&mut self, kind: MacroKind) {
        self.kind = kind;
        match kind {
            MacroKind::Button if self.button.is_none() => {
                self.button = Some(ButtonSpec::default());
            }
            MacroKind::Knob | MacroKind::Fader => self.button = None,
            _ => {}
        }
    }

    /// Feed a raw input value (0..1, e.g. from a controller or the UI). Updates
    /// internal state and returns the fan-out to apply. The only mutation is to
    /// `self` (value / latch / edge state).
    pub fn apply_input(&mut self, raw: f32) -> MacroFanout {
        let raw = if raw.is_finite() {
            raw.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let rising = self.last_input <= 0.5 && raw > 0.5;
        let mut out = MacroFanout::default();

        match self.kind {
            MacroKind::Knob | MacroKind::Fader => {
                self.value = raw;
                self.fan_targets(&mut out);
            }
            MacroKind::Button => {
                let behavior = self.button.as_ref().map(|b| b.behavior).unwrap_or_default();
                match behavior {
                    ButtonBehavior::Momentary => {
                        self.value = if raw > 0.5 { 1.0 } else { 0.0 };
                        self.fan_targets(&mut out);
                    }
                    ButtonBehavior::Toggle => {
                        if rising {
                            self.value = if self.value > 0.5 { 0.0 } else { 1.0 };
                            self.fan_targets(&mut out);
                        }
                    }
                    ButtonBehavior::Trigger => {
                        if rising {
                            if let Some(spec) = &self.button {
                                for act in &spec.trigger {
                                    match act {
                                        TriggerAction::Param { path, value } => {
                                            if !Self::is_macro_str(path) {
                                                out.params.push((path.clone(), *value));
                                            }
                                        }
                                        TriggerAction::Global(g) => out.actions.push(*g),
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        self.last_input = raw;
        out
    }

    fn fan_targets(&self, out: &mut MacroFanout) {
        for t in &self.targets {
            if t.is_macro_path() {
                continue; // loop prevention
            }
            out.params.push((t.path.clone(), t.routed(self.value)));
        }
    }

    /// The modulation target key for this macro's value (`macro_<uuid>:value`),
    /// matching the `{prefix}:{name}` convention used by the modulation engine.
    pub fn value_mod_key(uuid: &str) -> String {
        format!("macro_{}:value", uuid)
    }

    /// Compute the fan-out for `clamp(base + offset, 0, 1)` **without** mutating
    /// the macro, where `base` is the current (manual) value. Used to drive the
    /// macro from a modulation source each frame while preserving the manual set
    /// point. Returns target writes (macro-path targets filtered); empty for
    /// button macros, which are not modulatable.
    pub fn modulated_fanout(&self, offset: f32) -> Vec<(String, f32)> {
        if !matches!(self.kind, MacroKind::Knob | MacroKind::Fader) {
            return Vec::new();
        }
        let effective = (self.value + offset).clamp(0.0, 1.0);
        self.targets
            .iter()
            .filter(|t| !t.is_macro_path())
            .map(|t| (t.path.clone(), t.routed(effective)))
            .collect()
    }

    fn is_macro_str(path: &str) -> bool {
        path == "macro" || path.starts_with("macro/")
    }
}

/// Collection of user-defined macros. Owned by `Mixer`, serialized per-scene.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MacroBank {
    #[serde(default)]
    macros: Vec<Macro>,
    /// Runtime-only queue of global actions produced by trigger buttons; the app
    /// layer drains this each frame (see `param_router` + `app/inputs.rs`).
    #[serde(skip)]
    pending_actions: Vec<GlobalAction>,
}

impl MacroBank {
    pub fn new() -> Self {
        Self::default()
    }

    /// Read-only access to all macros.
    pub fn macros(&self) -> &[Macro] {
        &self.macros
    }

    /// Mutable access to all macros (used by config commands / persistence).
    pub fn macros_mut(&mut self) -> &mut Vec<Macro> {
        &mut self.macros
    }

    /// Replace all macros (used by persistence restore).
    pub fn set_macros(&mut self, macros: Vec<Macro>) {
        self.macros = macros;
    }

    /// Add a new macro of the given kind with an auto-generated name. Returns its UUID.
    pub fn add_macro(&mut self, kind: MacroKind) -> String {
        let name = self.next_macro_name();
        let m = Macro::new(kind, name);
        let uuid = m.uuid.clone();
        self.macros.push(m);
        uuid
    }

    /// Remove a macro by UUID. Returns true if one was removed.
    pub fn remove_macro(&mut self, uuid: &str) -> bool {
        let before = self.macros.len();
        self.macros.retain(|m| m.uuid != uuid);
        self.macros.len() != before
    }

    pub fn find(&self, uuid: &str) -> Option<&Macro> {
        self.macros.iter().find(|m| m.uuid == uuid)
    }

    pub fn find_mut(&mut self, uuid: &str) -> Option<&mut Macro> {
        self.macros.iter_mut().find(|m| m.uuid == uuid)
    }

    /// Feed a raw input into the macro identified by `uuid`. Returns the list of
    /// `(path, value)` parameter writes to apply, or `None` if the macro does not
    /// exist. Any global actions produced are queued in `pending_actions`.
    pub fn apply_input(&mut self, uuid: &str, raw: f32) -> Option<Vec<(String, f32)>> {
        let m = self.macros.iter_mut().find(|m| m.uuid == uuid)?;
        let fanout = m.apply_input(raw);
        self.pending_actions.extend(fanout.actions);
        Some(fanout.params)
    }

    /// Drain queued global actions (undo/redo/save/tap) for the app layer.
    pub fn take_pending_actions(&mut self) -> Vec<GlobalAction> {
        std::mem::take(&mut self.pending_actions)
    }

    /// Generate the next default macro name ("Macro 1", "Macro 2", …), avoiding
    /// collisions with existing "Macro N" names.
    fn next_macro_name(&self) -> String {
        let max = self
            .macros
            .iter()
            .filter_map(|m| {
                m.name
                    .strip_prefix("Macro ")
                    .and_then(|s| s.parse::<usize>().ok())
            })
            .max()
            .unwrap_or(0);
        format!("Macro {}", max + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Curves ───────────────────────────────────────────────────────

    #[test]
    fn curve_linear_is_identity() {
        for v in [0.0, 0.25, 0.5, 0.75, 1.0] {
            assert!((MacroCurve::Linear.apply(v) - v).abs() < 1e-6);
        }
    }

    #[test]
    fn curve_shapes_match_definitions() {
        assert!((MacroCurve::Exponential.apply(0.5) - 0.25).abs() < 1e-6);
        assert!((MacroCurve::Logarithmic.apply(0.25) - 0.5).abs() < 1e-6);
        // smoothstep midpoint = 0.5, endpoints exact
        assert!((MacroCurve::SCurve.apply(0.5) - 0.5).abs() < 1e-6);
        assert!(MacroCurve::SCurve.apply(0.25) < 0.25); // slow start
        assert!((MacroCurve::SCurve.apply(0.0)).abs() < 1e-6);
        assert!((MacroCurve::SCurve.apply(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn curve_stepped_quantizes_evenly() {
        let c = MacroCurve::Stepped(4);
        assert!((c.apply(0.0) - 0.0).abs() < 1e-6);
        assert!((c.apply(0.1) - 0.0).abs() < 1e-6);
        assert!((c.apply(0.25) - 1.0 / 3.0).abs() < 1e-6);
        assert!((c.apply(0.5) - 2.0 / 3.0).abs() < 1e-6);
        assert!((c.apply(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn curve_clamps_and_handles_nan() {
        assert!((MacroCurve::Linear.apply(-1.0) - 0.0).abs() < 1e-6);
        assert!((MacroCurve::Linear.apply(2.0) - 1.0).abs() < 1e-6);
        assert!((MacroCurve::Linear.apply(f32::NAN) - 0.0).abs() < 1e-6);
        // Stepped with n<2 is treated as 2
        assert!((MacroCurve::Stepped(0).apply(1.0) - 1.0).abs() < 1e-6);
    }

    // ── Target mapping ────────────────────────────────────────────────

    #[test]
    fn target_full_range_linear() {
        let t = MacroTarget::new("deck/aaa/opacity");
        assert!((t.routed(0.0) - 0.0).abs() < 1e-6);
        assert!((t.routed(0.5) - 0.5).abs() < 1e-6);
        assert!((t.routed(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn target_subrange_maps_within_bounds() {
        let mut t = MacroTarget::new("ch/bbb/opacity");
        t.min = 0.2;
        t.max = 0.8;
        assert!((t.routed(0.0) - 0.2).abs() < 1e-6);
        assert!((t.routed(1.0) - 0.8).abs() < 1e-6);
        assert!((t.routed(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn target_invert_flips_response() {
        let mut t = MacroTarget::new("master/effect/ccc/param/mix");
        t.invert = true;
        assert!((t.routed(0.0) - 1.0).abs() < 1e-6);
        assert!((t.routed(1.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn target_min_greater_than_max_also_inverts() {
        let mut t = MacroTarget::new("deck/ddd/param/warp");
        t.min = 1.0;
        t.max = 0.0;
        assert!((t.routed(0.0) - 1.0).abs() < 1e-6);
        assert!((t.routed(1.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn target_rejects_macro_path() {
        assert!(MacroTarget::new("macro/xyz/value").is_macro_path());
        assert!(!MacroTarget::new("deck/xyz/opacity").is_macro_path());
    }

    // ── Knob/fader fan-out (the motivating example) ───────────────────

    #[test]
    fn knob_fans_out_to_multiple_targets() {
        // One knob → two effect params on two different decks; second inverted.
        let mut m = Macro::new(MacroKind::Knob, "Sweep");
        m.targets
            .push(MacroTarget::new("deck/AAAA/effect/fx1/param/scale"));
        let mut inv = MacroTarget::new("deck/BBBB/effect/fx2/param/warp");
        inv.invert = true;
        m.targets.push(inv);

        let out = m.apply_input(0.75);
        assert_eq!(out.params.len(), 2);
        assert_eq!(out.params[0].0, "deck/AAAA/effect/fx1/param/scale");
        assert!((out.params[0].1 - 0.75).abs() < 1e-6);
        assert_eq!(out.params[1].0, "deck/BBBB/effect/fx2/param/warp");
        assert!((out.params[1].1 - 0.25).abs() < 1e-6); // inverted
        assert!(out.actions.is_empty());
        assert!((m.value - 0.75).abs() < 1e-6);
    }

    #[test]
    fn modulated_fanout_rides_on_base_without_mutating() {
        let mut m = Macro::new(MacroKind::Knob, "Sweep");
        m.value = 0.4;
        m.targets.push(MacroTarget::new("crossfader"));

        // base 0.4 + offset 0.2 → effective 0.6 fanned out; base untouched.
        let writes = m.modulated_fanout(0.2);
        assert_eq!(writes.len(), 1);
        assert_eq!(writes[0].0, "crossfader");
        assert!((writes[0].1 - 0.6).abs() < 1e-6);
        assert!((m.value - 0.4).abs() < 1e-6, "base must not be mutated");
    }

    #[test]
    fn modulated_fanout_clamps_and_respects_target_mapping() {
        let mut m = Macro::new(MacroKind::Fader, "Blend");
        m.value = 0.9;
        let mut t = MacroTarget::new("ch/AAAA/effect/fx/param/mix");
        t.min = 0.0;
        t.max = 0.5; // half-range target
        m.targets.push(t);

        // effective clamps to 1.0; target maps 1.0 → max 0.5.
        let writes = m.modulated_fanout(0.5);
        assert!((writes[0].1 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn modulated_fanout_empty_for_buttons() {
        let mut m = Macro::new(MacroKind::Button, "Drop");
        m.targets.push(MacroTarget::new("crossfader"));
        assert!(m.modulated_fanout(0.5).is_empty());
    }

    #[test]
    fn value_mod_key_matches_prefix_convention() {
        assert_eq!(Macro::value_mod_key("abcd1234"), "macro_abcd1234:value");
    }

    #[test]
    fn fanout_skips_macro_targets() {
        let mut m = Macro::new(MacroKind::Fader, "Bad");
        m.targets.push(MacroTarget::new("macro/other/value")); // illegal, skipped
        m.targets.push(MacroTarget::new("crossfader"));
        let out = m.apply_input(0.5);
        assert_eq!(out.params.len(), 1);
        assert_eq!(out.params[0].0, "crossfader");
    }

    // ── Button behaviors ──────────────────────────────────────────────

    #[test]
    fn button_momentary_drives_on_off() {
        let mut m = Macro::new(MacroKind::Button, "Drop");
        m.targets.push(MacroTarget::new("ch/AAAA/opacity"));
        m.button = Some(ButtonSpec {
            behavior: ButtonBehavior::Momentary,
            trigger: vec![],
        });

        let press = m.apply_input(1.0);
        assert!((press.params[0].1 - 1.0).abs() < 1e-6);
        let release = m.apply_input(0.0);
        assert!((release.params[0].1 - 0.0).abs() < 1e-6);
    }

    #[test]
    fn button_toggle_latches_across_presses() {
        let mut m = Macro::new(MacroKind::Button, "Kill");
        m.targets.push(MacroTarget::new("ch/AAAA/opacity"));
        m.button = Some(ButtonSpec {
            behavior: ButtonBehavior::Toggle,
            trigger: vec![],
        });

        // First press → on (1.0)
        let p1 = m.apply_input(1.0);
        assert!((p1.params[0].1 - 1.0).abs() < 1e-6);
        // Release → no change (toggle ignores falling edge)
        let r1 = m.apply_input(0.0);
        assert!(r1.params.is_empty());
        // Second press → off (0.0)
        let p2 = m.apply_input(1.0);
        assert!((p2.params[0].1 - 0.0).abs() < 1e-6);
    }

    #[test]
    fn button_trigger_fires_once_on_rising_edge() {
        let mut m = Macro::new(MacroKind::Button, "Panic");
        m.button = Some(ButtonSpec {
            behavior: ButtonBehavior::Trigger,
            trigger: vec![
                TriggerAction::Param {
                    path: "deck/AAAA/trigger".to_string(),
                    value: 1.0,
                },
                TriggerAction::Global(GlobalAction::Undo),
            ],
        });

        let press = m.apply_input(1.0);
        assert_eq!(press.params.len(), 1);
        assert_eq!(press.params[0].0, "deck/AAAA/trigger");
        assert_eq!(press.actions, vec![GlobalAction::Undo]);

        // Holding (no new rising edge) fires nothing.
        let hold = m.apply_input(1.0);
        assert!(hold.params.is_empty() && hold.actions.is_empty());

        // Release then press again fires again.
        m.apply_input(0.0);
        let press2 = m.apply_input(1.0);
        assert_eq!(press2.params.len(), 1);
    }

    #[test]
    fn trigger_skips_macro_param_paths() {
        let mut m = Macro::new(MacroKind::Button, "Loop");
        m.button = Some(ButtonSpec {
            behavior: ButtonBehavior::Trigger,
            trigger: vec![TriggerAction::Param {
                path: "macro/self/value".to_string(),
                value: 1.0,
            }],
        });
        let press = m.apply_input(1.0);
        assert!(press.params.is_empty());
    }

    // ── MacroBank ─────────────────────────────────────────────────────

    #[test]
    fn bank_add_remove_and_name() {
        let mut bank = MacroBank::new();
        let a = bank.add_macro(MacroKind::Knob);
        let b = bank.add_macro(MacroKind::Fader);
        assert_eq!(bank.macros().len(), 2);
        assert_eq!(bank.find(&a).unwrap().name, "Macro 1");
        assert_eq!(bank.find(&b).unwrap().name, "Macro 2");
        assert!(bank.remove_macro(&a));
        assert!(!bank.remove_macro(&a));
        assert_eq!(bank.macros().len(), 1);
    }

    #[test]
    fn bank_apply_input_unknown_returns_none() {
        let mut bank = MacroBank::new();
        assert!(bank.apply_input("nope", 0.5).is_none());
    }

    #[test]
    fn bank_apply_input_queues_global_actions() {
        let mut bank = MacroBank::new();
        let uuid = bank.add_macro(MacroKind::Button);
        {
            let m = bank.find_mut(&uuid).unwrap();
            m.button = Some(ButtonSpec {
                behavior: ButtonBehavior::Trigger,
                trigger: vec![TriggerAction::Global(GlobalAction::Save)],
            });
        }
        let params = bank.apply_input(&uuid, 1.0).unwrap();
        assert!(params.is_empty());
        assert_eq!(bank.take_pending_actions(), vec![GlobalAction::Save]);
        // Drained.
        assert!(bank.take_pending_actions().is_empty());
    }

    #[test]
    fn set_kind_toggles_button_spec() {
        let mut m = Macro::new(MacroKind::Knob, "M");
        assert!(m.button.is_none());
        m.set_kind(MacroKind::Button);
        assert!(m.button.is_some());
        m.set_kind(MacroKind::Fader);
        assert!(m.button.is_none());
    }

    // ── Persistence round-trip ────────────────────────────────────────

    #[test]
    fn bank_serde_roundtrip() {
        let mut bank = MacroBank::new();
        let uuid = bank.add_macro(MacroKind::Knob);
        bank.find_mut(&uuid)
            .unwrap()
            .targets
            .push(MacroTarget::new("crossfader"));
        let json = serde_json::to_string(&bank).unwrap();
        let restored: MacroBank = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.macros().len(), 1);
        assert_eq!(restored.macros()[0].targets[0].path, "crossfader");
    }

    #[test]
    fn empty_bank_deserializes_from_missing_fields() {
        // Graceful default: an object with no macros array yields an empty bank.
        let bank: MacroBank = serde_json::from_str("{}").unwrap();
        assert!(bank.macros().is_empty());
    }
}
