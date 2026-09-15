//! Looks, groups, and attribute values: the content a lighting deck plays.
//!
//! A Look is the lighting analogue of a shader or media file in the library. Dragging one into
//! a channel creates a lighting deck whose source is that Look, exactly as dragging a shader
//! creates a deck. See /spec/lighting-routing.md § Nouns.

use super::role::Role;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A value in a look: either written out, or a reference to a palette.
///
/// The indirection ships from the first release deliberately. Adding it later would mean a
/// persistence migration plus a population of shows already built on literals, and without
/// position palettes every mover cue stores raw pan and tilt and is locked to the venue it was
/// authored in. See /spec/lighting-routing.md § Palettes.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AttrValue {
    Literal { value: f32 },
    Palette { id: Uuid },
}

impl AttrValue {
    #[must_use]
    pub fn literal(value: f32) -> Self {
        Self::Literal {
            value: value.clamp(0.0, 1.0),
        }
    }

    #[must_use]
    pub fn palette(id: Uuid) -> Self {
        Self::Palette { id }
    }

    /// The written-out value, or `None` when this follows a palette.
    ///
    /// A palette reference has no literal of its own by design: the whole point is that its
    /// value is read from the palette at merge time.
    #[must_use]
    pub fn as_literal(self) -> Option<f32> {
        match self {
            Self::Literal { value } => Some(value),
            Self::Palette { .. } => None,
        }
    }

    /// The palette this follows, if any.
    #[must_use]
    pub fn palette_id(self) -> Option<Uuid> {
        match self {
            Self::Palette { id } => Some(id),
            Self::Literal { .. } => None,
        }
    }

    /// True when this value follows a palette, so the editor can mark it visually.
    ///
    /// A performer needs to see at a glance which parts of a look will follow a palette edit
    /// and which will not.
    #[must_use]
    pub fn is_reference(self) -> bool {
        matches!(self, Self::Palette { .. })
    }
}

/// One role of one target, set to one value.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LookAssignment {
    pub role: Role,
    pub value: AttrValue,
}

/// One role of one target, driven by a modulation source.
///
/// Modulation already reaches fixture roles through the parameter router, so this exists for the
/// one thing the router cannot express: **spread**, fanning a modulator's phase across the
/// fixtures of a group so a sine chases along a truss rather than pulsing the whole rig in
/// unison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModulatedBinding {
    pub role: Role,
    /// Modulation source UUID.
    pub source: String,
    /// Value the modulation rides on, before depth is applied.
    #[serde(default)]
    pub base: f32,
    /// Depth, 0 to 1.
    #[serde(default = "unit_depth")]
    pub amount: f32,
    #[serde(default)]
    pub spread: super::spread::Spread,
}

fn unit_depth() -> f32 {
    1.0
}

/// What a lighting deck plays: which fixtures, held at what, and driven by what.
///
/// Static values and modulation bindings sit side by side rather than in an enum, because a deck
/// routinely wants both: a color that holds while the dimmer chases. Forcing a choice between
/// them would mean two decks for one idea.
///
/// A saved look is the lighting counterpart of a **deck preset**: content authored on a deck and
/// kept for reuse, not a source that exists independently of the show.
/// See /spec/lighting-routing.md § Nouns.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Look {
    #[serde(default = "Uuid::new_v4")]
    pub id: Uuid,
    pub name: String,
    /// Values held steady.
    #[serde(default)]
    pub assignments: Vec<LookAssignment>,
    /// Values driven by the modulation engine, optionally fanned across a group.
    ///
    /// `Sampled` content (pixel mapping) is a third list here rather than a competing variant,
    /// for the same reason these two are not an enum.
    #[serde(default)]
    pub bindings: Vec<ModulatedBinding>,
    /// Roles driven by sampling a video signal at each fixture's place on the stage.
    ///
    /// This is the edge that makes Varda one routing graph rather than two that resemble each
    /// other: a lighting deck whose source is video. Resolume's Lumiverse, `MadMapper`'s `MadLight`
    /// and Millumin's DMX layer all arrive here as one more list beside the other two, mixing
    /// through the same merge, at the same level, under the same master.
    /// See /spec/lighting-routing.md § Sampled.
    #[serde(default)]
    pub samples: Vec<SampledBinding>,
}

/// One role of one target, driven by sampling video.
///
/// Sampling is per *fixture*, not per target: a group's members each read the point they occupy
/// on stage, which is what makes a pixel map a map rather than one color fanned across a truss.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SampledBinding {
    pub role: Role,
    /// Scales the sampled value. Above 1.0 it clips, which is usually what a performer wants
    /// from a dim video source driving a rig.
    #[serde(default = "unit_gain")]
    pub gain: f32,
}

fn unit_gain() -> f32 {
    1.0
}

impl Look {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            assignments: Vec::new(),
            bindings: Vec::new(),
            samples: Vec::new(),
        }
    }

    /// Roles driven by sampling video.
    #[must_use]
    pub fn samples(&self) -> &[SampledBinding] {
        &self.samples
    }

    /// Drive one role from a video sample, replacing whatever held it.
    ///
    /// A role is held, driven by a modulator, or sampled — never more than one, or two writers
    /// would fight over the same slot every frame.
    pub fn sample(&mut self, role: Role, gain: f32) {
        self.assignments.retain(|a| a.role != role);
        self.bindings.retain(|b| b.role != role);
        if let Some(existing) = self.samples.iter_mut().find(|s| s.role == role) {
            existing.gain = gain;
            return;
        }
        self.samples.push(SampledBinding { role, gain });
    }

    /// Values held steady.
    #[must_use]
    pub fn assignments(&self) -> &[LookAssignment] {
        &self.assignments
    }

    /// Values driven by a modulation source.
    #[must_use]
    pub fn bindings(&self) -> &[ModulatedBinding] {
        &self.bindings
    }

    /// Every role this look drives — held, modulated or sampled — in first-seen order.
    #[must_use]
    pub fn roles(&self) -> Vec<Role> {
        let mut out: Vec<Role> = Vec::new();
        for role in self
            .assignments
            .iter()
            .map(|a| a.role)
            .chain(self.bindings.iter().map(|b| b.role))
            .chain(self.samples.iter().map(|s| s.role))
        {
            if !out.contains(&role) {
                out.push(role);
            }
        }
        out
    }

    /// Add or replace the value for one role.
    pub fn set(&mut self, role: Role, value: AttrValue) {
        // A role is held, driven or sampled — never more than one. Setting a static value
        // releases both of the others, or two writers fight over one slot every frame.
        self.bindings.retain(|b| b.role != role);
        self.samples.retain(|s| s.role != role);
        let assignments = &mut self.assignments;
        if let Some(existing) = assignments.iter_mut().find(|a| a.role == role) {
            existing.value = value;
        } else {
            assignments.push(LookAssignment { role, value });
        }
    }

    /// Remove the value for one role, however it was driven.
    pub fn clear(&mut self, role: Role) {
        self.assignments.retain(|a| a.role != role);
        self.bindings.retain(|b| b.role != role);
        self.samples.retain(|s| s.role != role);
    }

    /// Drive one role from a modulation source.
    ///
    /// Replaces any static or sampled value for the same role: a role is held, driven or
    /// sampled, never more than one.
    pub fn bind(&mut self, binding: ModulatedBinding) {
        self.assignments.retain(|a| a.role != binding.role);
        self.samples.retain(|s| s.role != binding.role);
        if let Some(existing) = self.bindings.iter_mut().find(|b| b.role == binding.role) {
            *existing = binding;
        } else {
            self.bindings.push(binding);
        }
    }
}

/// A named set of fixtures.
///
/// Whether groups are ultimately the same mechanism as macro controls is Open Question 3 in
/// /spec/lighting-routing.md. Kept separate for now because a group also carries the ordering
/// that modulation spread fans across, which a macro does not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub id: Uuid,
    pub name: String,
    /// Fixture UUIDs, in the order spread fans across them.
    ///
    /// The order is **authored**, not derived. Patch order is an accident of how the rig was
    /// wired and physical position cannot express the deliberately shuffled sequences LDs use —
    /// "randomized but even" is a named technique, and grandMA3 lets an operator shuffle a
    /// selection and store the order into the group for later recall. This list is that order.
    /// See /spec/lighting-routing.md § What a source produces.
    #[serde(default)]
    pub members: Vec<Uuid>,
    /// What this group listens to, or `None` while it is unrouted.
    ///
    /// **The group pulls.** A video deck does not name a surface — the surface declares which
    /// channel it shows — and a group is the lighting surface, so it declares its feed the same
    /// way. A lighting deck therefore produces role values with no opinion about who receives
    /// them, exactly as a shader produces pixels with no opinion about which wall they land on.
    /// See /spec/lighting-routing.md § A group is the lighting surface.
    #[serde(default)]
    pub source: Option<GroupSource>,
}

/// What a group listens to.
///
/// The lighting counterpart of `OutputSource`, and it exists for the same reason a surface can
/// show either one channel or the master: a group pinned to a single channel would be **deaf to
/// the crossfader**, so moving from Ch A to Ch B would swap the visuals and leave the lights on
/// the old song. `Program` is the crossfaded result and is what most groups want.
/// See /spec/lighting-routing.md § A group is the lighting surface.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GroupSource {
    /// Every channel, in order, each at its crossfader-weighted opacity. The default.
    #[default]
    Program,
    /// One channel, whatever the crossfader is doing. For a group that must stay on one song's
    /// lighting while the visuals move on — house lights, blinders, an audience wash.
    Channel { uuid: String },
}

impl Group {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            members: Vec::new(),
            source: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_look_is_empty() {
        let look = Look::new("Deep Blue");
        assert_eq!(look.name, "Deep Blue");
        assert!(look.assignments().is_empty());
    }

    #[test]
    fn set_adds_then_replaces() {
        let mut look = Look::new("wash");
        look.set(Role::Red, AttrValue::literal(1.0));
        assert_eq!(look.assignments().len(), 1);
        look.set(Role::Red, AttrValue::literal(0.5));
        assert_eq!(look.assignments().len(), 1, "must replace, not duplicate");
        assert_eq!(look.assignments()[0].value, AttrValue::literal(0.5));
    }

    /// A look is keyed by role alone now that it names no target, so two roles are two
    /// assignments and the same role twice is one.
    #[test]
    fn set_distinguishes_roles() {
        let mut look = Look::new("wash");
        look.set(Role::Red, AttrValue::literal(1.0));
        look.set(Role::Blue, AttrValue::literal(1.0));
        look.set(Role::Red, AttrValue::literal(0.5));
        assert_eq!(look.assignments().len(), 2);
    }

    #[test]
    fn clear_removes_only_the_named_assignment() {
        let mut look = Look::new("wash");
        look.set(Role::Red, AttrValue::literal(1.0));
        look.set(Role::Blue, AttrValue::literal(1.0));
        look.clear(Role::Red);
        assert_eq!(look.assignments().len(), 1);
        assert_eq!(look.assignments()[0].role, Role::Blue);
    }

    #[test]
    fn literal_values_are_clamped() {
        assert_eq!(AttrValue::literal(5.0), AttrValue::Literal { value: 1.0 });
        assert_eq!(AttrValue::literal(-1.0), AttrValue::Literal { value: 0.0 });
    }

    #[test]
    fn a_palette_value_is_marked_as_a_reference() {
        assert!(AttrValue::palette(Uuid::new_v4()).is_reference());
        assert!(!AttrValue::literal(0.5).is_reference());
    }

    /// The order is the axis spread fans across, and it is authored rather than derived — patch
    /// order is an accident of wiring and physical position cannot express a deliberate shuffle.
    /// See /spec/lighting-routing.md § What a source produces.
    #[test]
    fn a_groups_member_order_is_preserved() {
        let mut group = Group::new("front truss");
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        group.members = vec![c, a, b];
        assert_eq!(group.members, vec![c, a, b]);
    }

    /// A group starts unrouted: it hears nothing until someone points it at a channel, exactly
    /// as a new surface shows nothing until it is given a source.
    #[test]
    fn a_new_group_listens_to_nothing() {
        assert_eq!(Group::new("front truss").source, None);
    }

    #[test]
    fn a_look_round_trips_through_json() {
        let mut look = Look::new("Deep Blue");
        look.set(Role::Blue, AttrValue::literal(0.8));
        look.set(Role::Pan, AttrValue::palette(Uuid::new_v4()));
        let text = serde_json::to_string(&look).unwrap();
        let back: Look = serde_json::from_str(&text).unwrap();
        assert_eq!(look, back);
        assert!(back.assignments()[1].value.is_reference());
    }
}
