//! Referenced palettes.
//!
//! A look attribute holds either a literal or a reference to a palette. Editing a palette
//! updates every look referencing it: the difference between a show you can retune in one place
//! and a show where a color change means opening forty looks.
//!
//! **The two palette kinds are genuinely different, and treating them alike is the easy mistake.**
//!
//! | | Color and beam | Position |
//! |---|---|---|
//! | Example | "Deep Blue", "Warm Wash" | "Downstage Centre", "Drum Riser" |
//! | Nature | An artistic choice | A physical fact about a room |
//! | Stored in | `scene.json` | `stage.json` |
//! | Keyed by | Profile | Fixture |
//!
//! "Deep Blue" is one decision for every LED par in the rig, so a per-profile entry covers them
//! all at once. "Downstage Centre" is a *different pan and tilt for every mover*, because two
//! heads at opposite ends of a truss need different angles to hit the same spot.
//!
//! The two axes align rather than coincidentally agreeing: position is per-fixture **and** venue
//! state because it is a physical-world fact, and color is per-type **and** show state because
//! it is an artistic one. See /spec/lighting-routing.md § Palettes.

use super::look::AttrValue;
use super::patch::ResolvedRig;
use super::role::{Role, RoleGroup};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Which storage a palette belongs to, derived from what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteKind {
    /// Artistic. Keyed by profile, saved with the show.
    Color,
    /// Artistic. Keyed by profile, saved with the show.
    Beam,
    /// Physical. Keyed by fixture, saved with the venue.
    Position,
}

impl PaletteKind {
    /// The kind a role belongs to, so the editor files a stored value automatically.
    #[must_use]
    pub fn for_role(role: Role) -> Self {
        match role.group() {
            RoleGroup::Position => Self::Position,
            RoleGroup::Color | RoleGroup::Intensity => Self::Color,
            RoleGroup::Beam | RoleGroup::Control => Self::Beam,
        }
    }

    /// True when this kind travels with the show rather than the venue.
    #[must_use]
    pub fn is_show_state(self) -> bool {
        matches!(self, Self::Color | Self::Beam)
    }

    /// True when entries are keyed per fixture rather than per profile.
    #[must_use]
    pub fn is_per_fixture(self) -> bool {
        matches!(self, Self::Position)
    }
}

/// What an override is keyed by.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OverrideKey {
    /// A profile reference plus mode, for color and beam.
    Profile { profile: String, mode: String },
    /// One physical light, for position.
    Fixture { id: Uuid },
}

/// A named, referenced set of role values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Palette {
    pub id: Uuid,
    pub name: String,
    pub kind: PaletteKind,
    /// Role-space values applying to any fixture carrying those roles.
    ///
    /// This is what keeps per-type resolution cheap in practice: a normalized red/green/blue
    /// triple already produces a sensible result on an RGB par, an RGBW par, and an RGBAW par
    /// without anyone authoring three entries.
    #[serde(default)]
    pub default: HashMap<Role, f32>,
    /// Values for fixtures the role-space default cannot express, such as a color-wheel mover
    /// with no RGB channels that needs `color_wheel = slot 4` to make the same blue.
    #[serde(default)]
    pub overrides: Vec<PaletteOverride>,
}

/// One keyed override within a palette.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PaletteOverride {
    pub key: OverrideKey,
    #[serde(default)]
    pub values: HashMap<Role, f32>,
}

impl Palette {
    #[must_use]
    pub fn new(name: impl Into<String>, kind: PaletteKind) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            kind,
            default: HashMap::new(),
            overrides: Vec::new(),
        }
    }

    /// Set a role-space default.
    pub fn set_default(&mut self, role: Role, value: f32) {
        self.default.insert(role, value.clamp(0.0, 1.0));
    }

    /// Set an override for one key.
    pub fn set_override(&mut self, key: OverrideKey, role: Role, value: f32) {
        let value = value.clamp(0.0, 1.0);
        if let Some(existing) = self.overrides.iter_mut().find(|o| o.key == key) {
            existing.values.insert(role, value);
        } else {
            let mut values = HashMap::new();
            values.insert(role, value);
            self.overrides.push(PaletteOverride { key, values });
        }
    }
}

/// Every palette available, from both storages.
///
/// Held as one lookup because a look references a palette by UUID and neither knows nor cares
/// which file it came from. The split exists so a show file carries no venue-specific data, not
/// so consumers have to think about it.
#[derive(Debug, Clone, Default)]
pub struct PaletteSet {
    palettes: HashMap<Uuid, Palette>,
}

impl PaletteSet {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Build from the show's palettes and the venue's, in that order.
    #[must_use]
    pub fn from_parts(show: &[Palette], venue: &[Palette]) -> Self {
        let mut palettes = HashMap::new();
        for p in show.iter().chain(venue.iter()) {
            palettes.insert(p.id, p.clone());
        }
        Self { palettes }
    }

    pub fn insert(&mut self, palette: Palette) {
        self.palettes.insert(palette.id, palette);
    }

    pub fn remove(&mut self, id: Uuid) -> Option<Palette> {
        self.palettes.remove(&id)
    }

    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&Palette> {
        self.palettes.get(&id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.palettes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.palettes.is_empty()
    }

    /// Palettes belonging to the show, for `scene.json`.
    #[must_use]
    pub fn show_palettes(&self) -> Vec<Palette> {
        self.sorted_where(PaletteKind::is_show_state)
    }

    /// Palettes belonging to the venue, for `stage.json`.
    #[must_use]
    pub fn venue_palettes(&self) -> Vec<Palette> {
        self.sorted_where(|k| !k.is_show_state())
    }

    fn sorted_where(&self, predicate: impl Fn(PaletteKind) -> bool) -> Vec<Palette> {
        let mut out: Vec<Palette> = self
            .palettes
            .values()
            .filter(|p| predicate(p.kind))
            .cloned()
            .collect();
        // Stable order so a save produces a stable file rather than reshuffling on every write.
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        out
    }

    /// Every palette, sorted, for a picker.
    #[must_use]
    pub fn all_sorted(&self) -> Vec<&Palette> {
        let mut out: Vec<&Palette> = self.palettes.values().collect();
        out.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        out
    }
}

/// Resolve an attribute value for one fixture and role.
///
/// Resolution order is **override, then role-space default, then nothing**. A palette that does
/// not cover a fixture yields `None`, which the merge engine treats as inert rather than dark.
#[must_use]
pub fn resolve(
    value: AttrValue,
    fixture: Uuid,
    role: Role,
    palettes: &PaletteSet,
    rig: &ResolvedRig,
) -> Option<f32> {
    match value {
        AttrValue::Literal { value } => Some(value),
        AttrValue::Palette { id } => {
            let palette = palettes.get(id)?;
            let key = override_key(palette.kind, fixture, rig)?;
            if let Some(found) = palette
                .overrides
                .iter()
                .find(|o| o.key == key)
                .and_then(|o| o.values.get(&role))
            {
                return Some(*found);
            }
            palette.default.get(&role).copied()
        }
    }
}

/// The key a palette of this kind uses for a given fixture.
fn override_key(kind: PaletteKind, fixture: Uuid, rig: &ResolvedRig) -> Option<OverrideKey> {
    if kind.is_per_fixture() {
        return Some(OverrideKey::Fixture { id: fixture });
    }
    let patched = rig.fixtures.iter().find(|f| f.id == fixture)?;
    Some(OverrideKey::Profile {
        profile: format!("{}/{}", patched.profile.vendor, patched.profile.model),
        mode: patched.profile.modes[patched.mode_index].name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmx::patch::{Fixture, ProfileSource, validate};
    use crate::dmx::profile::{FixtureProfile, ProfileChannel, ProfileError, ProfileMode};
    use crate::dmx::universe::Resolution;
    use std::sync::Arc;

    fn channels(specs: &[(&str, Option<Role>)]) -> Vec<ProfileChannel> {
        specs
            .iter()
            .map(|(n, r)| ProfileChannel {
                name: (*n).to_string(),
                role: *r,
                resolution: Resolution::Eight,
                default: 0,
                ranges: Vec::new(),
            })
            .collect()
    }

    struct Lib;
    impl ProfileSource for Lib {
        fn load(&mut self, reference: &str) -> Result<Arc<FixtureProfile>, ProfileError> {
            let profile = match reference {
                "generic/rgb-par" => FixtureProfile {
                    vendor: "Generic".into(),
                    notes: Vec::new(),
                    model: "RGB Par".into(),
                    modes: vec![ProfileMode {
                        name: "3ch".into(),
                        channels: channels(&[
                            ("Red", Some(Role::Red)),
                            ("Green", Some(Role::Green)),
                            ("Blue", Some(Role::Blue)),
                        ]),
                    }],
                },
                "acme/wheel-mover" => FixtureProfile {
                    vendor: "Acme".into(),
                    notes: Vec::new(),
                    model: "Wheel Mover".into(),
                    modes: vec![ProfileMode {
                        name: "3ch".into(),
                        channels: channels(&[
                            ("Pan", Some(Role::Pan)),
                            ("Tilt", Some(Role::Tilt)),
                            ("Color", Some(Role::ColorWheel)),
                        ]),
                    }],
                },
                other => return Err(ProfileError::Io(format!("no profile {other}"))),
            };
            Ok(Arc::new(profile))
        }
    }

    /// A rig with one RGB par and two wheel movers.
    fn test_rig() -> (ResolvedRig, Uuid, Uuid, Uuid) {
        let mut par = Fixture::new("par", "generic/rgb-par", "3ch");
        par.address = 1;
        let mut left = Fixture::new("mover-l", "acme/wheel-mover", "3ch");
        left.address = 10;
        let mut right = Fixture::new("mover-r", "acme/wheel-mover", "3ch");
        right.address = 20;
        let (p, l, r) = (par.id, left.id, right.id);
        let rig = validate(&[par, left, right], &mut Lib).expect("valid patch");
        (rig, p, l, r)
    }

    #[test]
    fn role_kinds_file_themselves_correctly() {
        assert_eq!(PaletteKind::for_role(Role::Pan), PaletteKind::Position);
        assert_eq!(PaletteKind::for_role(Role::Tilt), PaletteKind::Position);
        assert_eq!(PaletteKind::for_role(Role::Red), PaletteKind::Color);
        assert_eq!(PaletteKind::for_role(Role::Zoom), PaletteKind::Beam);
    }

    /// The storage split is the point: a show file must carry no venue-specific data.
    #[test]
    fn color_travels_with_the_show_and_position_with_the_venue() {
        assert!(PaletteKind::Color.is_show_state());
        assert!(PaletteKind::Beam.is_show_state());
        assert!(
            !PaletteKind::Position.is_show_state(),
            "Downstage Centre is a different angle in every room"
        );
    }

    #[test]
    fn only_position_is_keyed_per_fixture() {
        assert!(PaletteKind::Position.is_per_fixture());
        assert!(!PaletteKind::Color.is_per_fixture());
    }

    #[test]
    fn a_literal_resolves_to_itself_with_no_palettes() {
        let (rig, par, _, _) = test_rig();
        let set = PaletteSet::new();
        assert_eq!(
            resolve(AttrValue::literal(0.5), par, Role::Red, &set, &rig),
            Some(0.5)
        );
    }

    /// The role-space default is what keeps per-type resolution cheap: one entry serves an RGB
    /// par, an RGBW par, and an RGBAW par.
    #[test]
    fn a_role_space_default_reaches_any_fixture_with_that_role() {
        let (rig, par, left, _) = test_rig();
        let mut palette = Palette::new("Deep Blue", PaletteKind::Color);
        palette.set_default(Role::Blue, 0.8);
        let id = palette.id;
        let mut set = PaletteSet::new();
        set.insert(palette);
        assert_eq!(
            resolve(AttrValue::palette(id), par, Role::Blue, &set, &rig),
            Some(0.8)
        );
        assert_eq!(
            resolve(AttrValue::palette(id), left, Role::Blue, &set, &rig),
            Some(0.8),
            "the default applies regardless of fixture type"
        );
    }

    /// A color-wheel mover has no RGB channels at all and needs its own slot to make the same
    /// blue. That is what the per-profile override is for.
    #[test]
    fn a_profile_override_wins_over_the_default() {
        let (rig, par, left, right) = test_rig();
        let mut palette = Palette::new("Deep Blue", PaletteKind::Color);
        palette.set_default(Role::Blue, 0.8);
        palette.set_override(
            OverrideKey::Profile {
                profile: "Acme/Wheel Mover".into(),
                mode: "3ch".into(),
            },
            Role::ColorWheel,
            0.25,
        );
        let id = palette.id;
        let mut set = PaletteSet::new();
        set.insert(palette);

        assert_eq!(
            resolve(AttrValue::palette(id), par, Role::Blue, &set, &rig),
            Some(0.8),
            "the par still takes the default"
        );
        // One override entry covers every mover of that profile at once.
        for mover in [left, right] {
            assert_eq!(
                resolve(AttrValue::palette(id), mover, Role::ColorWheel, &set, &rig),
                Some(0.25)
            );
        }
    }

    /// Two heads at opposite ends of a truss need *different* angles to hit the same spot, so a
    /// position palette must be keyed per fixture, not per profile.
    #[test]
    fn a_position_palette_holds_a_different_angle_per_fixture() {
        let (rig, _, left, right) = test_rig();
        let mut palette = Palette::new("Downstage Centre", PaletteKind::Position);
        palette.set_override(OverrideKey::Fixture { id: left }, Role::Pan, 0.30);
        palette.set_override(OverrideKey::Fixture { id: right }, Role::Pan, 0.70);
        let id = palette.id;
        let mut set = PaletteSet::new();
        set.insert(palette);

        assert_eq!(
            resolve(AttrValue::palette(id), left, Role::Pan, &set, &rig),
            Some(0.30)
        );
        assert_eq!(
            resolve(AttrValue::palette(id), right, Role::Pan, &set, &rig),
            Some(0.70),
            "the same palette must aim each head differently"
        );
    }

    #[test]
    fn a_palette_not_covering_a_fixture_resolves_to_nothing() {
        let (rig, par, _, _) = test_rig();
        let palette = Palette::new("empty", PaletteKind::Color);
        let id = palette.id;
        let mut set = PaletteSet::new();
        set.insert(palette);
        assert_eq!(
            resolve(AttrValue::palette(id), par, Role::Red, &set, &rig),
            None
        );
    }

    #[test]
    fn a_dangling_palette_reference_resolves_to_nothing() {
        let (rig, par, _, _) = test_rig();
        let set = PaletteSet::new();
        assert_eq!(
            resolve(
                AttrValue::palette(Uuid::new_v4()),
                par,
                Role::Red,
                &set,
                &rig
            ),
            None
        );
    }

    #[test]
    fn a_fixture_not_in_the_rig_resolves_to_nothing_for_a_profile_palette() {
        let (rig, _, _, _) = test_rig();
        let mut palette = Palette::new("Deep Blue", PaletteKind::Color);
        palette.set_default(Role::Blue, 0.8);
        let id = palette.id;
        let mut set = PaletteSet::new();
        set.insert(palette);
        assert_eq!(
            resolve(
                AttrValue::palette(id),
                Uuid::new_v4(),
                Role::Blue,
                &set,
                &rig
            ),
            None
        );
    }

    #[test]
    fn palettes_partition_by_storage() {
        let mut set = PaletteSet::new();
        set.insert(Palette::new("Deep Blue", PaletteKind::Color));
        set.insert(Palette::new("Tight", PaletteKind::Beam));
        set.insert(Palette::new("Downstage", PaletteKind::Position));
        assert_eq!(set.show_palettes().len(), 2);
        assert_eq!(set.venue_palettes().len(), 1);
        assert_eq!(set.venue_palettes()[0].name, "Downstage");
    }

    /// A save must produce a stable file rather than reshuffling on every write.
    #[test]
    fn partitioned_palettes_come_out_in_a_stable_order() {
        let mut set = PaletteSet::new();
        set.insert(Palette::new("Zeta", PaletteKind::Color));
        set.insert(Palette::new("Alpha", PaletteKind::Color));
        for _ in 0..10 {
            let names: Vec<String> = set.show_palettes().iter().map(|p| p.name.clone()).collect();
            assert_eq!(names, vec!["Alpha".to_string(), "Zeta".to_string()]);
        }
    }

    #[test]
    fn from_parts_combines_both_storages() {
        let show = vec![Palette::new("Deep Blue", PaletteKind::Color)];
        let venue = vec![Palette::new("Downstage", PaletteKind::Position)];
        let set = PaletteSet::from_parts(&show, &venue);
        assert_eq!(set.len(), 2);
        assert!(set.get(show[0].id).is_some());
        assert!(set.get(venue[0].id).is_some());
    }

    #[test]
    fn a_palette_round_trips_through_json() {
        let mut palette = Palette::new("Deep Blue", PaletteKind::Color);
        palette.set_default(Role::Blue, 0.8);
        palette.set_override(
            OverrideKey::Profile {
                profile: "Acme/Wheel Mover".into(),
                mode: "3ch".into(),
            },
            Role::ColorWheel,
            0.25,
        );
        let text = serde_json::to_string(&palette).unwrap();
        let back: Palette = serde_json::from_str(&text).unwrap();
        assert_eq!(palette, back);
    }

    #[test]
    fn values_are_clamped_on_the_way_in() {
        let mut palette = Palette::new("p", PaletteKind::Color);
        palette.set_default(Role::Red, 5.0);
        assert_eq!(palette.default[&Role::Red], 1.0);
    }
}
