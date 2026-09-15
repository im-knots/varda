//! The lighting merge engine.
//!
//! Lighting decks live in the same channel as video decks and carry the same controls, but they
//! composite through this rather than through the GPU. One control model, two backends.
//! See /spec/lighting-routing.md § Mixing.

use super::look::{AttrValue, Group, GroupSource, Look};
use super::role::{Role, RoleGroup};
use super::values::RoleValues;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// How a lighting deck combines with what is already there.
///
/// Named after Varda's existing blend modes rather than HTP and LTP, so a VJ reads the control
/// without learning lighting jargon and a lighting operator recognizes the behaviour instantly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LightingBlend {
    /// Role-derived: HTP for intensity, LTP for everything else.
    ///
    /// The default, and the one almost every deck should use. Consoles do not ask an operator
    /// to choose per playback because the right answer is derivable: two looks lighting the
    /// same fixture should not sum to brighter than either, and averaging two pan values aims
    /// the head between two targets, which is never what anyone wants.
    #[default]
    Auto,
    /// Forced HTP: the highest value wins.
    Lighten,
    /// Forced LTP: the latest active deck takes over.
    Normal,
    /// Sum and clamp. The additive submaster.
    Add,
    /// Scale down, never up. The inhibitive submaster, a limiter.
    Multiply,
}

impl LightingBlend {
    /// Every mode, for a UI dropdown, `Auto` first.
    pub const ALL: &'static [LightingBlend] = &[
        LightingBlend::Auto,
        LightingBlend::Lighten,
        LightingBlend::Normal,
        LightingBlend::Add,
        LightingBlend::Multiply,
    ];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto",
            Self::Lighten => "Lighten",
            Self::Normal => "Normal",
            Self::Add => "Add",
            Self::Multiply => "Multiply",
        }
    }

    /// The mode actually applied to `role`, resolving `Auto`.
    #[must_use]
    fn resolve(self, role: Role) -> LightingBlend {
        match self {
            Self::Auto => {
                if role.group() == RoleGroup::Intensity {
                    Self::Lighten
                } else {
                    Self::Normal
                }
            }
            other => other,
        }
    }
}

/// A look instantiated in a channel, with the same controls a video deck carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LightingDeck {
    pub id: Uuid,
    /// The content this deck plays, owned outright.
    ///
    /// Not a reference into the saved-look library. A deck made from a saved Look holds a *copy*
    /// of it, so two decks made from the same Look are edited independently — exactly as two
    /// video decks made from the same preset are.
    /// See /spec/lighting-routing.md § A deck owns its content.
    #[serde(default)]
    pub content: Look,
    /// Deck opacity under a different name. Scales intensity directly and acts as the crossfade
    /// weight for LTP roles, so pulling a deck down fades its contribution rather than snapping.
    pub level: f32,
    #[serde(default)]
    pub blend: LightingBlend,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub solo: bool,
    /// Ignore channel opacity and the crossfader, holding at this deck's own level.
    ///
    /// For house lights, blinders, and audience wash that must survive a transition. ETC Eos
    /// has exactly this as a submaster property, under the same name.
    #[serde(default)]
    pub independent: bool,
    /// How LTP roles behave as this deck's weight crosses in.
    ///
    /// Crossfading between two looks that aim the same head at different positions sweeps it
    /// through the intermediate angles. Sometimes that is beautiful and sometimes it is a head
    /// swinging across the audience, so it is the operator's call rather than a fixed rule.
    #[serde(default)]
    pub ltp_transition: LtpTransition,
}

/// What an LTP role does while a deck fades in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum LtpTransition {
    /// Sweep through the intermediate values. A head travels to its new position.
    #[default]
    Fade,
    /// Take the new value the moment the deck has any weight at all. A head cuts.
    Snap,
}

impl LtpTransition {
    pub const ALL: &'static [LtpTransition] = &[LtpTransition::Fade, LtpTransition::Snap];

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Fade => "Fade",
            Self::Snap => "Snap",
        }
    }
}

impl LightingDeck {
    #[must_use]
    pub fn new(content: Look) -> Self {
        Self {
            id: Uuid::new_v4(),
            content,
            level: 1.0,
            blend: LightingBlend::Auto,
            mute: false,
            solo: false,
            independent: false,
            ltp_transition: LtpTransition::Fade,
        }
    }
}

/// One channel's lighting half: its decks and the opacity they inherit.
#[derive(Debug, Clone, PartialEq)]
pub struct LightingChannel {
    /// The channel's UUID, so a sampled deck can read the video of the channel it lives in
    /// without being told which one that is.
    pub key: String,
    /// Effective channel opacity, crossfader included. Shared with the video half, because
    /// channel opacity is control state the mixer owns once for both backends.
    pub opacity: f32,
    pub decks: Vec<LightingDeck>,
}

/// Resolves an [`AttrValue`] to a number for one fixture.
///
/// A closure rather than a concrete type so the palette resolver slots in without the merge
/// engine learning about palettes at all.
pub type ValueResolver<'a> = dyn Fn(AttrValue, Uuid, Role) -> Option<f32> + 'a;

/// Samples a modulation source at a phase offset.
///
/// `None` when the source does not exist, or is not periodic and therefore has no meaningful
/// phase. A caller passing offset 0 gets the source's current value.
pub type ModulationSampler<'a> = dyn Fn(&str, f32) -> Option<f32> + 'a;

/// Merge every channel's lighting decks into per-fixture role values.
///
/// `fixtures` is the rig in patch order; the returned vector matches it index for index.
///
/// Decks are applied in channel order, then deck order within a channel, which is the z-order
/// the UI shows top to bottom.
#[must_use]
pub fn merge(
    channels: &[LightingChannel],
    groups: &[Group],
    fixtures: &[Uuid],
    resolver: &ValueResolver<'_>,
    modulation: &ModulationSampler<'_>,
    frames: &super::sample::SampledFrames,
    positions: &HashMap<Uuid, Option<[f32; 2]>, impl std::hash::BuildHasher>,
) -> Vec<RoleValues> {
    let mut out = vec![RoleValues::new(); fixtures.len()];
    let index: HashMap<Uuid, usize> = fixtures
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();

    // Solo is scoped to the lighting band. A rig going black because the performer soloed a
    // shader to check it is a failure mode with no upside.
    let soloing = channels
        .iter()
        .flat_map(|c| &c.decks)
        .any(|d| d.solo && !d.mute);

    // The group pulls. Each group listens to one channel, and applies what that channel's decks
    // merge to, to its own members in its own authored order. A deck never names a target — the
    // same way a video deck never names a surface.
    // See /spec/lighting-routing.md § A group is the lighting surface.
    for group in groups {
        let Some(source) = group.source.as_ref() else {
            continue;
        };
        if group.members.is_empty() {
            continue;
        }
        // `Program` hears every channel in order, each already carrying its crossfader-weighted
        // opacity — which is what keeps lighting on the crossfader. A `Channel` group hears one.
        let heard: Vec<&LightingChannel> = match source {
            GroupSource::Program => channels.iter().collect(),
            GroupSource::Channel { uuid } => channels.iter().filter(|c| &c.key == uuid).collect(),
        };

        for channel in heard {
            for deck in &channel.decks {
                let weight = deck_weight(deck, channel.opacity, soloing);
                if weight <= 0.0 {
                    continue;
                }
                let look = &deck.content;

                for (position, fixture) in group.members.iter().copied().enumerate() {
                    let Some(&slot) = index.get(&fixture) else {
                        continue;
                    };

                    // Held values: the same at every index, which is what makes this a wash.
                    for assignment in look.assignments() {
                        let Some(value) = resolver(assignment.value, fixture, assignment.role)
                        else {
                            continue;
                        };
                        apply(
                            &mut out[slot],
                            assignment.role,
                            value,
                            weight,
                            deck.blend.resolve(assignment.role),
                            deck.ltp_transition,
                        );
                    }

                    // Modulated values, fanned across the group's *authored* order. Not patch order
                    // and not physical position: a shuffled order is a deliberate technique, and
                    // deriving phase from anything else would delete it.
                    for binding in look.bindings() {
                        let offset = binding.spread.offset_for(position, group.members.len());
                        // A non-periodic source has no meaningful phase, so it fans uniformly:
                        // sampling at offset 0 gives every member the same current value.
                        let sampled = modulation(&binding.source, offset)
                            .or_else(|| modulation(&binding.source, 0.0));
                        let Some(sampled) = sampled else { continue };
                        let value = (binding.base + sampled * binding.amount).clamp(0.0, 1.0);
                        apply(
                            &mut out[slot],
                            binding.role,
                            value,
                            weight,
                            deck.blend.resolve(binding.role),
                            deck.ltp_transition,
                        );
                    }

                    // Sampled values: the one case where the space is *physical* rather than
                    // authored. The group already knows which channel it listens to, so there is
                    // nothing to configure — the frame is that channel's.
                    if !look.samples().is_empty()
                    // The frame of the channel actually being heard: a Program group samples
                    // whichever channel this deck lives in, which is what a pixel map means.
                    && let Some(frame) = frames.get(&channel.key)
                    // An unplaced fixture has no point to read, so it is skipped rather than
                    // defaulted to the middle of the frame, where every unplaced light in the
                    // rig would show one colour and look deliberate.
                    && let Some(point) = positions.get(&fixture).copied().flatten()
                    && let Some(rgb) = frame.sample(point)
                    {
                        for binding in look.samples() {
                            let Some(value) = super::sample::component(binding.role, rgb) else {
                                continue;
                            };
                            apply(
                                &mut out[slot],
                                binding.role,
                                (value * binding.gain).clamp(0.0, 1.0),
                                weight,
                                deck.blend.resolve(binding.role),
                                deck.ltp_transition,
                            );
                        }
                    }
                }
            }
        }
    }
    out
}

/// A deck's contribution weight: level, channel opacity, mute and solo folded together.
fn deck_weight(deck: &LightingDeck, channel_opacity: f32, soloing: bool) -> f32 {
    if deck.mute || (soloing && !deck.solo) {
        return 0.0;
    }
    let inherited = if deck.independent {
        1.0
    } else {
        channel_opacity
    };
    (deck.level * inherited).clamp(0.0, 1.0)
}

/// Combine one value into the accumulator for one role.
fn apply(
    acc: &mut RoleValues,
    role: Role,
    value: f32,
    weight: f32,
    blend: LightingBlend,
    ltp: LtpTransition,
) {
    let value = value.clamp(0.0, 1.0);
    let previous = acc.get(role);
    let merged = match blend {
        // HTP. Two looks lighting the same fixture must not sum to brighter than either.
        LightingBlend::Lighten => previous.map_or(value * weight, |p| p.max(value * weight)),
        // LTP. `Fade` uses the level as a crossfade weight, so pulling a deck down eases its
        // contribution out. `Snap` takes over outright the moment the deck has any weight,
        // which is what an operator wants when a sweep would drag a beam across the audience.
        LightingBlend::Normal => match ltp {
            LtpTransition::Fade => previous.map_or(value * weight, |p| p + (value - p) * weight),
            LtpTransition::Snap => value,
        },
        LightingBlend::Add => previous.map_or(value * weight, |p| (p + value * weight).min(1.0)),
        // Inhibitive: at weight 0 it does not limit, at weight 1 it limits to `value`.
        LightingBlend::Multiply => {
            let limit = 1.0 + (value - 1.0) * weight;
            previous.map_or(limit, |p| p * limit)
        }
        LightingBlend::Auto => unreachable!("resolved before reaching apply"),
    };
    acc.set(role, merged.clamp(0.0, 1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn literal_resolver(value: AttrValue, _f: Uuid, _r: Role) -> Option<f32> {
        match value {
            AttrValue::Literal { value } => Some(value),
            AttrValue::Palette { .. } => None,
        }
    }

    struct Rig {
        fixtures: Vec<Uuid>,
        groups: Vec<Group>,
        frames: super::super::sample::SampledFrames,
        positions: HashMap<Uuid, Option<[f32; 2]>>,
    }

    /// A rig whose one group listens to `ch-1`, which is the channel `channel()` builds.
    ///
    /// The group is what routes: a deck names nobody, so a harness with no listening group would
    /// merge to nothing no matter what the decks hold.
    fn rig(n: usize) -> Rig {
        let fixtures: Vec<Uuid> = (0..n).map(|_| Uuid::new_v4()).collect();
        let mut all = Group::new("all");
        all.members.clone_from(&fixtures);
        all.source = Some(GroupSource::Channel {
            uuid: "ch-1".to_string(),
        });
        // Placed left to right across the plot, which is how a truss is rigged and what makes a
        // horizontal gradient readable as one.
        #[allow(clippy::cast_precision_loss)]
        let positions = fixtures
            .iter()
            .enumerate()
            .map(|(i, id)| {
                let x = if n > 1 {
                    i as f32 / (n - 1) as f32
                } else {
                    0.0
                };
                (*id, Some([x, 0.5]))
            })
            .collect();
        Rig {
            fixtures,
            groups: vec![all],
            frames: super::super::sample::SampledFrames::new(),
            positions,
        }
    }

    impl Rig {
        /// A look setting one role across the whole rig.
        ///
        /// Returned by value: a deck owns its content, so handing the same look to two decks is
        /// two independent copies, which is exactly the property under test.
        fn look(role: Role, value: f32) -> Look {
            let mut look = Look::new("l");
            look.set(role, AttrValue::literal(value));
            look
        }

        fn merge(&self, channels: &[LightingChannel]) -> Vec<RoleValues> {
            self.merge_with(channels, &|_, _| None)
        }

        fn merge_with(
            &self,
            channels: &[LightingChannel],
            modulation: &ModulationSampler<'_>,
        ) -> Vec<RoleValues> {
            merge(
                channels,
                &self.groups,
                &self.fixtures,
                &literal_resolver,
                modulation,
                &self.frames,
                &self.positions,
            )
        }
    }

    fn channel(opacity: f32, decks: Vec<LightingDeck>) -> LightingChannel {
        LightingChannel {
            key: "ch-1".to_string(),
            opacity,
            decks,
        }
    }

    // ── Auto resolution ──────────────────────────────────────────────

    #[test]
    fn auto_is_htp_for_intensity_and_ltp_for_everything_else() {
        assert_eq!(
            LightingBlend::Auto.resolve(Role::Dimmer),
            LightingBlend::Lighten
        );
        assert_eq!(
            LightingBlend::Auto.resolve(Role::Pan),
            LightingBlend::Normal
        );
        assert_eq!(
            LightingBlend::Auto.resolve(Role::Red),
            LightingBlend::Normal
        );
    }

    /// Two looks lighting the same fixture must not sum to brighter than either.
    #[test]
    fn two_intensity_decks_take_the_highest_not_the_sum() {
        let r = rig(1);
        let dim = Rig::look(Role::Dimmer, 0.4);
        let bright = Rig::look(Role::Dimmer, 0.7);
        let out = r.merge(&[channel(
            1.0,
            vec![LightingDeck::new(dim), LightingDeck::new(bright)],
        )]);
        let got = out[0].get(Role::Dimmer).unwrap();
        assert!((got - 0.7).abs() < 1e-5, "expected HTP 0.7, got {got}");
    }

    /// Averaging two pan values aims the head between two targets, which is never wanted.
    #[test]
    fn two_position_decks_let_the_later_one_take_over() {
        let r = rig(1);
        let left = Rig::look(Role::Pan, 0.0);
        let right = Rig::look(Role::Pan, 1.0);
        let out = r.merge(&[channel(
            1.0,
            vec![LightingDeck::new(left), LightingDeck::new(right)],
        )]);
        let got = out[0].get(Role::Pan).unwrap();
        assert!((got - 1.0).abs() < 1e-5, "later deck should win, got {got}");
    }

    #[test]
    fn a_half_level_ltp_deck_crossfades_rather_than_snapping() {
        let r = rig(1);
        let left = Rig::look(Role::Pan, 0.0);
        let right = Rig::look(Role::Pan, 1.0);
        let mut top = LightingDeck::new(right);
        top.level = 0.5;
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(left), top])]);
        let got = out[0].get(Role::Pan).unwrap();
        assert!(
            (got - 0.5).abs() < 1e-5,
            "expected a half crossfade, got {got}"
        );
    }

    // ── Forced modes ─────────────────────────────────────────────────

    #[test]
    fn add_sums_and_clamps() {
        let r = rig(1);
        let a = Rig::look(Role::Red, 0.6);
        let b = Rig::look(Role::Red, 0.7);
        let mut second = LightingDeck::new(b);
        second.blend = LightingBlend::Add;
        let mut first = LightingDeck::new(a);
        first.blend = LightingBlend::Add;
        let out = r.merge(&[channel(1.0, vec![first, second])]);
        assert!(
            (out[0].get(Role::Red).unwrap() - 1.0).abs() < 1e-5,
            "clamped"
        );
    }

    #[test]
    fn multiply_limits_and_never_raises() {
        let r = rig(1);
        let base = Rig::look(Role::Dimmer, 1.0);
        let limit = Rig::look(Role::Dimmer, 0.25);
        let mut inhibit = LightingDeck::new(limit);
        inhibit.blend = LightingBlend::Multiply;
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(base), inhibit])]);
        let got = out[0].get(Role::Dimmer).unwrap();
        assert!(
            (got - 0.25).abs() < 1e-5,
            "expected a limit to 0.25, got {got}"
        );
    }

    #[test]
    fn a_multiply_deck_at_zero_level_does_not_inhibit() {
        let r = rig(1);
        let base = Rig::look(Role::Dimmer, 1.0);
        let limit = Rig::look(Role::Dimmer, 0.0);
        let mut inhibit = LightingDeck::new(limit);
        inhibit.blend = LightingBlend::Multiply;
        inhibit.level = 0.0;
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(base), inhibit])]);
        assert!((out[0].get(Role::Dimmer).unwrap() - 1.0).abs() < 1e-5);
    }

    // ── Level, mute, solo ────────────────────────────────────────────

    #[test]
    fn level_scales_an_intensity_contribution() {
        let r = rig(1);
        let look = Rig::look(Role::Dimmer, 1.0);
        let mut deck = LightingDeck::new(look);
        deck.level = 0.25;
        let out = r.merge(&[channel(1.0, vec![deck])]);
        assert!((out[0].get(Role::Dimmer).unwrap() - 0.25).abs() < 1e-5);
    }

    #[test]
    fn a_muted_deck_contributes_nothing() {
        let r = rig(1);
        let look = Rig::look(Role::Dimmer, 1.0);
        let mut deck = LightingDeck::new(look);
        deck.mute = true;
        let out = r.merge(&[channel(1.0, vec![deck])]);
        assert_eq!(out[0].get(Role::Dimmer), None);
    }

    #[test]
    fn solo_restricts_contribution_to_soloed_decks() {
        let r = rig(1);
        let quiet = Rig::look(Role::Dimmer, 0.2);
        let loud = Rig::look(Role::Red, 0.9);
        let mut soloed = LightingDeck::new(loud);
        soloed.solo = true;
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(quiet), soloed])]);
        assert_eq!(out[0].get(Role::Dimmer), None, "unsoloed deck is silenced");
        assert!(out[0].get(Role::Red).is_some());
    }

    #[test]
    fn a_muted_solo_deck_does_not_engage_solo_mode() {
        let r = rig(1);
        let normal = Rig::look(Role::Dimmer, 0.5);
        let broken = Rig::look(Role::Red, 1.0);
        let mut muted_solo = LightingDeck::new(broken);
        muted_solo.solo = true;
        muted_solo.mute = true;
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(normal), muted_solo])]);
        assert!(
            out[0].get(Role::Dimmer).is_some(),
            "a muted solo must not silence the rig"
        );
    }

    // ── Channel opacity and independence ─────────────────────────────

    #[test]
    fn channel_opacity_scales_a_normal_deck() {
        let r = rig(1);
        let look = Rig::look(Role::Dimmer, 1.0);
        let out = r.merge(&[channel(0.5, vec![LightingDeck::new(look)])]);
        assert!((out[0].get(Role::Dimmer).unwrap() - 0.5).abs() < 1e-5);
    }

    /// House lights and blinders must survive a transition.
    #[test]
    fn an_independent_deck_ignores_channel_opacity() {
        let r = rig(1);
        let look = Rig::look(Role::Dimmer, 1.0);
        let mut deck = LightingDeck::new(look);
        deck.independent = true;
        let out = r.merge(&[channel(0.0, vec![deck])]);
        assert!(
            (out[0].get(Role::Dimmer).unwrap() - 1.0).abs() < 1e-5,
            "an independent deck holds through a full crossfade away"
        );
    }

    #[test]
    fn a_channel_at_zero_opacity_contributes_nothing() {
        let r = rig(1);
        let look = Rig::look(Role::Dimmer, 1.0);
        let out = r.merge(&[channel(0.0, vec![LightingDeck::new(look)])]);
        assert_eq!(out[0].get(Role::Dimmer), None);
    }

    // ── Targeting ────────────────────────────────────────────────────

    #[test]
    fn a_group_look_reaches_every_member() {
        let r = rig(3);
        let look = Rig::look(Role::Dimmer, 1.0);
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert!(out.iter().all(|v| v.get(Role::Dimmer).is_some()));
    }

    /// A deck reaches exactly the members of the groups listening to its channel, and nobody
    /// else. A deck names no target, so this is the only routing there is.
    /// See /spec/lighting-routing.md § A group is the lighting surface.
    #[test]
    fn only_a_listening_groups_members_hear_a_deck() {
        let mut r = rig(3);
        // Narrow the listening group to the middle lamp; the other two hear nothing.
        r.groups[0].members = vec![r.fixtures[1]];
        let mut look = Look::new("one");
        look.set(Role::Dimmer, AttrValue::literal(1.0));
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert_eq!(out[0].get(Role::Dimmer), None);
        assert!(out[1].get(Role::Dimmer).is_some());
        assert_eq!(out[2].get(Role::Dimmer), None);
    }

    /// An unrouted group hears nothing at all, exactly as a surface with no source shows nothing.
    #[test]
    fn an_unrouted_group_hears_nothing() {
        let mut r = rig(2);
        r.groups[0].source = None;
        let mut look = Look::new("wash");
        look.set(Role::Dimmer, AttrValue::literal(1.0));
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert!(out.iter().all(|v| v.get(Role::Dimmer).is_none()));
    }

    /// A group listening to a different channel does not hear this one.
    #[test]
    fn a_group_hears_only_the_channel_it_points_at() {
        let mut r = rig(2);
        r.groups[0].source = Some(GroupSource::Channel {
            uuid: "ch-9".to_string(),
        });
        let mut look = Look::new("wash");
        look.set(Role::Dimmer, AttrValue::literal(1.0));
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert!(out.iter().all(|v| v.get(Role::Dimmer).is_none()));
    }

    #[test]
    fn an_unresolvable_value_is_skipped_not_zeroed() {
        let r = rig(1);
        let mut look = Look::new("palette");
        look.set(Role::Red, AttrValue::palette(Uuid::new_v4()));
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert_eq!(
            out[0].get(Role::Red),
            None,
            "a dangling palette reference is inert, not dark"
        );
    }

    /// A `Program` group hears every channel in order, which is what keeps lighting on the
    /// crossfader: a group pinned to one channel would be deaf to it and would leave the lights
    /// on the old song while the visuals moved on.
    /// See /spec/lighting-routing.md § A group is the lighting surface.
    #[test]
    fn channels_apply_in_order() {
        let mut r = rig(1);
        // Program: every channel, in crossfader order.
        r.groups[0].source = Some(GroupSource::Program);
        let first = Rig::look(Role::Pan, 0.0);
        let second = Rig::look(Role::Pan, 1.0);
        let out = r.merge(&[
            channel(1.0, vec![LightingDeck::new(first)]),
            channel(1.0, vec![LightingDeck::new(second)]),
        ]);
        assert!((out[0].get(Role::Pan).unwrap() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn an_empty_rig_merges_to_nothing() {
        let r = Rig {
            fixtures: Vec::new(),
            groups: Vec::new(),
            frames: super::super::sample::SampledFrames::new(),
            positions: HashMap::new(),
        };
        assert!(r.merge(&[]).is_empty());
    }

    // ── LTP transition ───────────────────────────────────────────────

    /// Sometimes a sweep is beautiful and sometimes it is a beam dragging across the audience.
    #[test]
    fn snap_takes_over_outright_where_fade_crossfades() {
        let r = rig(1);
        let left = Rig::look(Role::Pan, 0.0);
        let right = Rig::look(Role::Pan, 1.0);

        let mut fading = LightingDeck::new(right.clone());
        fading.level = 0.25;
        let faded = r.merge(&[channel(1.0, vec![LightingDeck::new(left.clone()), fading])]);
        assert!((faded[0].get(Role::Pan).unwrap() - 0.25).abs() < 1e-5);

        let mut snapping = LightingDeck::new(right);
        snapping.level = 0.25;
        snapping.ltp_transition = LtpTransition::Snap;
        let snapped = r.merge(&[channel(1.0, vec![LightingDeck::new(left), snapping])]);
        assert!(
            (snapped[0].get(Role::Pan).unwrap() - 1.0).abs() < 1e-5,
            "snap must take the new value outright"
        );
    }

    #[test]
    fn snap_still_respects_mute_and_zero_weight() {
        let r = rig(1);
        let left = Rig::look(Role::Pan, 0.0);
        let right = Rig::look(Role::Pan, 1.0);
        let mut snapping = LightingDeck::new(right);
        snapping.ltp_transition = LtpTransition::Snap;
        snapping.level = 0.0;
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(left), snapping])]);
        assert!(
            (out[0].get(Role::Pan).unwrap() - 0.0).abs() < 1e-5,
            "a deck with no weight contributes nothing regardless of transition"
        );
    }

    #[test]
    fn snap_does_not_affect_intensity_which_is_htp() {
        let r = rig(1);
        let dim = Rig::look(Role::Dimmer, 0.8);
        let low = Rig::look(Role::Dimmer, 0.2);
        let mut snapping = LightingDeck::new(low);
        snapping.ltp_transition = LtpTransition::Snap;
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(dim), snapping])]);
        assert!(
            (out[0].get(Role::Dimmer).unwrap() - 0.8).abs() < 1e-5,
            "intensity is HTP; the transition setting must not touch it"
        );
    }

    #[test]
    fn fade_is_the_default() {
        assert_eq!(LtpTransition::default(), LtpTransition::Fade);
        assert_eq!(
            LightingDeck::new(Look::new("l")).ltp_transition,
            LtpTransition::Fade
        );
        assert_eq!(LtpTransition::ALL.len(), 2);
    }

    // ── Modulated looks and spread ───────────────────────────────────

    fn modulated_look(role: Role, spread: crate::dmx::Spread) -> Look {
        use crate::dmx::look::ModulatedBinding;
        let mut look = Look::new("chase");
        look.bindings = vec![ModulatedBinding {
            role,
            source: "lfo-1".into(),
            base: 0.0,
            amount: 1.0,
            spread,
        }];
        look
    }

    #[test]
    fn a_modulated_look_drives_every_member() {
        let r = rig(3);
        let look = modulated_look(Role::Dimmer, crate::dmx::Spread::default());
        let out = r.merge_with(&[channel(1.0, vec![LightingDeck::new(look)])], &|_, _| {
            Some(0.6)
        });
        for values in &out {
            assert!((values.get(Role::Dimmer).unwrap() - 0.6).abs() < 1e-5);
        }
    }

    /// The point of spread: without it a modulator pulses the whole rig in unison.
    ///
    /// Uses a partial turn so the ramp stays monotonic. A full turn deliberately wraps the last
    /// member back into phase with the first, which `spread::tests` covers.
    #[test]
    fn spread_gives_each_member_a_different_phase() {
        let r = rig(4);
        let look = modulated_look(
            Role::Dimmer,
            crate::dmx::Spread {
                amount: 0.75,
                mode: crate::dmx::SpreadMode::Linear,
            },
        );
        // Report the offset itself, so the test observes what each member was sampled at.
        let out = r.merge_with(
            &[channel(1.0, vec![LightingDeck::new(look)])],
            &|_, offset| Some(offset),
        );
        let sampled: Vec<f32> = out.iter().map(|v| v.get(Role::Dimmer).unwrap()).collect();
        assert!(
            sampled.windows(2).all(|w| w[1] > w[0]),
            "each member must be sampled at a later phase: {sampled:?}"
        );
    }

    #[test]
    fn no_spread_samples_every_member_identically() {
        let r = rig(4);
        let look = modulated_look(Role::Dimmer, crate::dmx::Spread::default());
        let out = r.merge_with(
            &[channel(1.0, vec![LightingDeck::new(look)])],
            &|_, offset| Some(offset),
        );
        for values in &out {
            assert!((values.get(Role::Dimmer).unwrap() - 0.0).abs() < 1e-6);
        }
    }

    /// An audio band or ADSR is not a periodic function of time, so "the same signal a quarter
    /// turn later" is undefined. Those fan uniformly rather than failing.
    #[test]
    fn a_non_periodic_source_falls_back_to_its_current_value() {
        let r = rig(3);
        let look = modulated_look(
            Role::Dimmer,
            crate::dmx::Spread {
                amount: 1.0,
                mode: crate::dmx::SpreadMode::Linear,
            },
        );
        // Refuse any offset sample, as a non-periodic source does; offset 0 still answers.
        let out = r.merge_with(
            &[channel(1.0, vec![LightingDeck::new(look)])],
            &|_, offset| {
                if offset == 0.0 { Some(0.42) } else { None }
            },
        );
        for values in &out {
            assert!((values.get(Role::Dimmer).unwrap() - 0.42).abs() < 1e-5);
        }
    }

    #[test]
    fn a_missing_modulation_source_leaves_the_role_inert() {
        let r = rig(2);
        let look = modulated_look(Role::Dimmer, crate::dmx::Spread::default());
        let out = r.merge_with(&[channel(1.0, vec![LightingDeck::new(look)])], &|_, _| None);
        assert!(
            out.iter().all(|v| v.get(Role::Dimmer).is_none()),
            "a dangling modulator is inert, not dark"
        );
    }

    #[test]
    fn base_and_amount_scale_the_modulator() {
        use crate::dmx::look::ModulatedBinding;
        let r = rig(1);
        let mut look = Look::new("shallow");
        look.bindings = vec![ModulatedBinding {
            role: Role::Dimmer,
            source: "lfo-1".into(),
            base: 0.5,
            amount: 0.25,
            spread: crate::dmx::Spread::default(),
        }];
        let out = r.merge_with(&[channel(1.0, vec![LightingDeck::new(look)])], &|_, _| {
            Some(1.0)
        });
        assert!((out[0].get(Role::Dimmer).unwrap() - 0.75).abs() < 1e-5);
    }

    // ── Sampled looks: the video-into-lighting edge ──────────────────

    /// A horizontal gradient across the frame must arrive as a gradient across the truss. This
    /// is the whole feature: each fixture reads the point *it* occupies, not one color fanned
    /// across the group.
    #[test]
    fn a_sampled_look_maps_the_image_across_the_rig() {
        let mut r = rig(3);
        // Black on the left, full red on the right.
        r.frames.insert(
            "ch-1".to_string(),
            super::super::sample::SampledFrame {
                width: 3,
                height: 1,
                rgba: vec![0, 0, 0, 255, 128, 0, 0, 255, 255, 0, 0, 255],
            },
        );
        let mut look = Look::new("map");
        look.sample(Role::Red, 1.0);
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        let reds: Vec<f32> = out.iter().map(|v| v.get(Role::Red).unwrap()).collect();
        assert!(
            reds[0] < reds[1] && reds[1] < reds[2],
            "the gradient must run across the rig: {reds:?}"
        );
    }

    /// No frame yet — before the first readback, or a source that is not rendering — must leave
    /// the role inert rather than dark. A rig blacking out waiting for a readback is the worst
    /// failure available on stage, and it is the same rule a dangling palette follows.
    #[test]
    fn a_sampled_look_with_no_frame_is_inert_not_dark() {
        let r = rig(2);
        let mut look = Look::new("map");
        look.sample(Role::Red, 1.0);
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert!(out.iter().all(|v| v.get(Role::Red).is_none()));
    }

    /// An unplaced fixture is skipped rather than defaulted to the middle of the frame, where
    /// every unplaced light in the rig would show one color and look deliberate.
    #[test]
    fn an_unplaced_fixture_is_skipped_by_a_sampled_look() {
        let mut r = rig(2);
        r.positions.insert(r.fixtures[1], None);
        r.frames.insert(
            "ch-1".to_string(),
            super::super::sample::SampledFrame {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            },
        );
        let mut look = Look::new("map");
        look.sample(Role::Red, 1.0);
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert!(out[0].get(Role::Red).is_some(), "a placed fixture samples");
        assert!(
            out[1].get(Role::Red).is_none(),
            "an unplaced fixture must not be given a position it does not have"
        );
    }

    /// The point of folding sampling into `apply` rather than beside it: pixel mapping inherits
    /// level, and through it channel opacity, the crossfader, mute and solo.
    #[test]
    fn a_sampled_look_obeys_deck_level() {
        let mut r = rig(1);
        r.frames.insert(
            "ch-1".to_string(),
            super::super::sample::SampledFrame {
                width: 1,
                height: 1,
                rgba: vec![255, 0, 0, 255],
            },
        );
        let mut look = Look::new("map");
        look.sample(Role::Red, 1.0);
        let mut deck = LightingDeck::new(look);
        deck.level = 0.5;
        let out = r.merge(&[channel(1.0, vec![deck])]);
        assert!((out[0].get(Role::Red).unwrap() - 0.5).abs() < 1e-5);
    }

    /// Gain scales the sample and clips, which is what a performer wants from a dim video source
    /// driving a rig.
    #[test]
    fn gain_scales_a_sample_and_clips() {
        let mut r = rig(1);
        r.frames.insert(
            "ch-1".to_string(),
            super::super::sample::SampledFrame {
                width: 1,
                height: 1,
                rgba: vec![128, 0, 0, 255],
            },
        );
        let mut look = Look::new("map");
        look.sample(Role::Red, 4.0);
        let out = r.merge(&[channel(1.0, vec![LightingDeck::new(look)])]);
        assert!(
            (out[0].get(Role::Red).unwrap() - 1.0).abs() < 1e-5,
            "must clip"
        );
    }

    #[test]
    fn blend_modes_all_have_labels_and_auto_is_first() {
        assert_eq!(LightingBlend::ALL[0], LightingBlend::Auto);
        assert_eq!(LightingBlend::ALL.len(), 5);
        for b in LightingBlend::ALL {
            assert!(!b.label().is_empty());
        }
    }
}
