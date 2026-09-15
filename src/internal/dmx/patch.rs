//! The patch: fixtures at addresses, and the fail-closed validation that guards it.
//!
//! A patch that cannot be resolved correctly refuses to run and says why. It never partially
//! resolves, because a partially resolved patch writes one fixture's values onto another
//! fixture's channels and reports success. See /spec/dmx-output.md § C5.

use super::profile::{FixtureProfile, ProfileError};
use super::role::Role;
use super::universe::{Resolution, SlotWrite, UNIVERSE_SIZE, UniverseId};
use std::collections::HashMap;
use std::sync::Arc;
use uuid::Uuid;

/// Key identifying a profile in the library, e.g. `"generic/rgbw-par"`.
pub type ProfileRef = String;

/// One physical light in the rig, as authored. Venue state, persisted to `stage.json`.
///
/// Identity is a [`Uuid`], never the row position. Positional identity would rebind every value
/// to a different physical light whenever the patch is reordered, which is why the reviewed
/// prior art had to refuse live edits and demand a restart.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Fixture {
    pub id: Uuid,
    pub name: String,
    pub profile: ProfileRef,
    pub mode: String,
    pub universe: UniverseId,
    /// 1-based DMX start address, as printed on the fixture's own display.
    pub address: u16,
    /// Per-channel overrides for slots with no role, keyed by profile channel name. This is how
    /// a macro or mode channel gets pinned to a working value.
    pub defaults: HashMap<String, u8>,
    pub invert_pan: bool,
    pub invert_tilt: bool,
    pub swap_pan_tilt: bool,
    /// Where this fixture stands, in normalized stage coordinates: `[0,0]` upstage left,
    /// `[1,1]` downstage right.
    ///
    /// Venue state, like the address: it describes the room, not the show. It does two jobs at
    /// once — it lays out the stage plot in the deck detail, and it is the point a sampled
    /// (pixel-mapped) role reads the video at. Those are the same fact, so they are one field.
    ///
    /// `None` means unplaced, and the plot falls back to a grid derived from patch order. A rig
    /// is patched in the order it is rigged, so that is usually close to right, and it means a
    /// performer is never made to lay out a plot before they can use a light.
    /// See /spec/lighting-routing.md § Sampled.
    #[serde(default)]
    pub position: Option<[f32; 2]>,
}

impl Fixture {
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        profile: impl Into<String>,
        mode: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            profile: profile.into(),
            mode: mode.into(),
            universe: 1,
            address: 1,
            defaults: HashMap::new(),
            invert_pan: false,
            invert_tilt: false,
            swap_pan_tilt: false,
            position: None,
        }
    }
}

/// Severity of a patch message. Only [`Severity::Error`] prevents the rig from running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

/// One validation finding, always naming the fixture it concerns.
#[derive(Debug, Clone, PartialEq)]
pub struct PatchMessage {
    pub severity: Severity,
    pub fixture: Option<String>,
    pub text: String,
}

impl PatchMessage {
    fn error(fixture: &str, text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            fixture: Some(fixture.to_owned()),
            text: text.into(),
        }
    }
    fn warning(fixture: &str, text: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            fixture: Some(fixture.to_owned()),
            text: text.into(),
        }
    }
}

/// A fixture that passed validation, pre-resolved for the output path.
#[derive(Debug, Clone)]
pub struct PatchedFixture {
    pub id: Uuid,
    pub name: String,
    pub universe: UniverseId,
    /// Zero-based slot offset of the fixture's first channel.
    pub base: u16,
    pub profile: Arc<FixtureProfile>,
    pub mode_index: usize,
    /// Per channel of the mode: the value to write when no role drives it.
    pub channel_defaults: Vec<u8>,
    pub invert_pan: bool,
    pub invert_tilt: bool,
    pub swap_pan_tilt: bool,
    /// Normalized stage position, carried through from the patch so the merge can sample video
    /// at the point this fixture occupies without consulting the config again.
    pub position: Option<[f32; 2]>,
}

impl PatchedFixture {
    #[must_use]
    fn mode(&self) -> &super::profile::ProfileMode {
        &self.profile.modes[self.mode_index]
    }

    /// Apply this fixture's rig geometry to a role lookup.
    ///
    /// Invert and swap are physical facts about how the light is hung, not show data, so they
    /// are applied here rather than being baked into any look. A head rigged upside down on the
    /// upstage truss is ordinary and the show file must not have to know.
    fn geometry_adjust(&self, role: Role, values: &dyn Fn(Role) -> Option<f32>) -> Option<f32> {
        let source = match (role, self.swap_pan_tilt) {
            (Role::Pan, true) => Role::Tilt,
            (Role::Tilt, true) => Role::Pan,
            (r, _) => r,
        };
        let v = values(source)?;
        let inverted = match role {
            Role::Pan if self.invert_pan => 1.0 - v,
            Role::Tilt if self.invert_tilt => 1.0 - v,
            _ => v,
        };
        Some(inverted)
    }

    /// Emit the slot writes for this fixture.
    ///
    /// `values` returns the current normalized value for a role, or `None` when nothing drives
    /// it. A role nothing drives falls back to the channel default rather than to zero, so a
    /// mode or macro channel keeps the value the operator pinned.
    pub fn emit(&self, values: &dyn Fn(Role) -> Option<f32>, out: &mut Vec<SlotWrite>) {
        for (idx, ch) in self.mode().channels.iter().enumerate() {
            // A fine byte is written by its coarse partner; emitting it separately would
            // overwrite the low byte with a default.
            if ch.role.is_none() && self.is_fine_byte(idx) {
                continue;
            }
            let Ok(offset) = u16::try_from(idx) else {
                continue;
            };
            let slot = self.base + offset;
            let value = match ch.role {
                Some(role) => self
                    .geometry_adjust(role, values)
                    .unwrap_or_else(|| f32::from(self.channel_defaults[idx]) / 255.0),
                None => f32::from(self.channel_defaults[idx]) / 255.0,
            };
            out.push(SlotWrite {
                universe: self.universe,
                slot,
                resolution: ch.resolution,
                value,
            });
        }
    }

    /// True when the channel at `idx` is the fine half of a 16-bit pair declared elsewhere in
    /// this mode.
    fn is_fine_byte(&self, idx: usize) -> bool {
        let Ok(target) = isize::try_from(idx) else {
            return false;
        };
        self.mode()
            .channels
            .iter()
            .enumerate()
            .any(|(i, c)| match c.resolution {
                Resolution::SixteenCoarse { fine_offset } => isize::try_from(i)
                    .is_ok_and(|coarse| coarse + isize::from(fine_offset) == target),
                Resolution::Eight => false,
            })
    }

    /// Slot span this fixture occupies, as zero-based indices.
    #[must_use]
    pub fn span(&self) -> std::ops::Range<usize> {
        let base = self.base as usize;
        base..base + self.mode().channels.len()
    }
}

/// Which fixture and channel owns one slot. Built once at resolution, read by the universe view.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlotOwner {
    /// Empty when no fixture claims this slot.
    pub fixture: String,
    pub channel: String,
    pub role: Option<Role>,
}

/// A validated rig, ready to drive.
#[derive(Debug, Clone)]
pub struct ResolvedRig {
    pub fixtures: Vec<PatchedFixture>,
    /// Non-fatal findings worth showing the operator.
    pub warnings: Vec<PatchMessage>,
    /// Per universe, who owns each of the 512 slots.
    pub slot_owners: HashMap<UniverseId, Vec<SlotOwner>>,
}

impl ResolvedRig {
    /// Every universe the rig touches, so the driver can pre-create and keep transmitting them.
    #[must_use]
    pub fn universes(&self) -> Vec<UniverseId> {
        let mut u: Vec<UniverseId> = self.fixtures.iter().map(|f| f.universe).collect();
        u.sort_unstable();
        u.dedup();
        u
    }
}

/// Supplies fixture profiles by reference. Injected so validation is testable without disk.
pub trait ProfileSource {
    /// Resolve a profile reference to a loaded profile.
    ///
    /// # Errors
    ///
    /// Returns whatever the underlying loader reports; the caller turns it into a fatal
    /// [`PatchMessage`] naming the fixture.
    fn load(&mut self, reference: &str) -> Result<Arc<FixtureProfile>, ProfileError>;
}

/// Validate a patch, resolving every fixture or failing with the complete list of findings.
///
/// All findings are collected before returning rather than short-circuiting on the first, so an
/// operator fixing a patch at load-in sees everything wrong at once instead of one error per
/// restart.
///
/// # Errors
///
/// Returns every [`PatchMessage`] produced, warnings included, when any finding is fatal or
/// when no fixture resolved at all.
pub fn validate(
    fixtures: &[Fixture],
    profiles: &mut dyn ProfileSource,
) -> Result<ResolvedRig, Vec<PatchMessage>> {
    let mut ctx = Resolver::default();

    for f in fixtures {
        match ctx.resolve_one(f, profiles) {
            Ok(pf) => ctx.resolved.push(pf),
            Err(msgs) => {
                ctx.messages.extend(msgs);
                ctx.fatal = true;
            }
        }
    }

    if ctx.fatal || (ctx.resolved.is_empty() && !fixtures.is_empty()) {
        return Err(ctx.messages);
    }

    Ok(ResolvedRig {
        fixtures: ctx.resolved,
        warnings: ctx.messages,
        slot_owners: ctx.owners,
    })
}

/// Accumulates state across a whole patch so overlap detection can see earlier fixtures.
#[derive(Default)]
struct Resolver {
    messages: Vec<PatchMessage>,
    resolved: Vec<PatchedFixture>,
    occupied: HashMap<UniverseId, Vec<bool>>,
    owners: HashMap<UniverseId, Vec<SlotOwner>>,
    fatal: bool,
}

impl Resolver {
    /// Validate one fixture against the rig built so far.
    ///
    /// Returns the fatal findings for this fixture, or the resolved fixture. Non-fatal warnings
    /// are pushed onto `self.messages` directly, because they accompany a success.
    fn resolve_one(
        &mut self,
        f: &Fixture,
        profiles: &mut dyn ProfileSource,
    ) -> Result<PatchedFixture, Vec<PatchMessage>> {
        let bail = |text: String| vec![PatchMessage::error(&f.name, text)];

        if f.address < 1 {
            return Err(bail(format!(
                "address must be at least 1, got {}",
                f.address
            )));
        }
        if f.universe < super::universe::UNIVERSE_MIN || f.universe > super::universe::UNIVERSE_MAX
        {
            return Err(bail(format!(
                "universe {} is outside the legal range {}..={}",
                f.universe,
                super::universe::UNIVERSE_MIN,
                super::universe::UNIVERSE_MAX
            )));
        }

        let profile = profiles
            .load(&f.profile)
            .map_err(|e| bail(format!("profile {:?} failed to load: {e}", f.profile)))?;

        let Some(mode_index) = profile.modes.iter().position(|m| m.name == f.mode) else {
            return Err(bail(format!(
                "profile {:?} has no mode {:?} (available: {})",
                f.profile,
                f.mode,
                profile
                    .modes
                    .iter()
                    .map(|m| m.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )));
        };

        let mode = &profile.modes[mode_index];
        let base = f.address - 1;
        let end = base as usize + mode.channels.len();
        if end > UNIVERSE_SIZE {
            return Err(bail(format!(
                "spans DMX {}..{} in universe {}, which runs past slot {UNIVERSE_SIZE}",
                f.address,
                f.address as usize + mode.channels.len() - 1,
                f.universe
            )));
        }

        // Overlap matters more than it looks: an overlapping patch resolves "successfully" and
        // then writes one fixture's values onto another fixture's channels, which is far harder
        // to diagnose in a venue than a refusal at load.
        let taken = self
            .occupied
            .entry(f.universe)
            .or_insert_with(|| vec![false; UNIVERSE_SIZE]);
        if let Some(slot) = (base as usize..end).find(|&s| taken[s]) {
            let owner_name = self
                .owners
                .get(&f.universe)
                .and_then(|o| o.get(slot))
                .map(|o| o.fixture.clone())
                .unwrap_or_default();
            return Err(bail(format!(
                "DMX {} in universe {} is already claimed by {:?}",
                slot + 1,
                f.universe,
                owner_name
            )));
        }
        taken[base as usize..end].fill(true);

        // Loader decisions an operator should know about, such as a switching channel resolved
        // at its dependency's default. Reported rather than silent, per
        // /spec/dmx-output.md § Switching channels.
        for note in &profile.notes {
            self.messages
                .push(PatchMessage::warning(&f.name, note.clone()));
        }

        let channel_defaults = self.claim_slots(f, mode, base);

        Ok(PatchedFixture {
            id: f.id,
            name: f.name.clone(),
            universe: f.universe,
            base,
            profile,
            mode_index,
            channel_defaults,
            invert_pan: f.invert_pan,
            invert_tilt: f.invert_tilt,
            swap_pan_tilt: f.swap_pan_tilt,
            position: f.position,
        })
    }

    /// Record slot ownership for the universe view and resolve each channel's fallback value.
    ///
    /// Warns about a slot with no role and no default: that is usually a mode or macro channel
    /// the operator forgot, and holding it at 0 often means the fixture never lights.
    fn claim_slots(
        &mut self,
        f: &Fixture,
        mode: &super::profile::ProfileMode,
        base: u16,
    ) -> Vec<u8> {
        let owner_table = self
            .owners
            .entry(f.universe)
            .or_insert_with(|| vec![SlotOwner::default(); UNIVERSE_SIZE]);
        let mut channel_defaults = Vec::with_capacity(mode.channels.len());
        let mut missing_defaults = Vec::new();

        for (i, ch) in mode.channels.iter().enumerate() {
            channel_defaults.push(f.defaults.get(&ch.name).copied().unwrap_or(ch.default));
            owner_table[base as usize + i] = SlotOwner {
                fixture: f.name.clone(),
                channel: ch.name.clone(),
                role: ch.role,
            };
            if ch.role.is_none()
                && !f.defaults.contains_key(&ch.name)
                && ch.default == 0
                && !ch.name.starts_with("unused ")
            {
                missing_defaults.push(ch.name.clone());
            }
        }

        for name in missing_defaults {
            self.messages.push(PatchMessage::warning(
                &f.name,
                format!(
                    "channel {name:?} has no role and no default, so it will be held at 0. \
                     Set a default if this is a mode or macro channel."
                ),
            ));
        }
        channel_defaults
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmx::profile::{ProfileChannel, ProfileMode};

    fn ch(name: &str, role: Option<Role>, res: Resolution) -> ProfileChannel {
        ProfileChannel {
            name: name.into(),
            role,
            resolution: res,
            default: 0,
            ranges: Vec::new(),
        }
    }

    fn rgbw() -> FixtureProfile {
        FixtureProfile {
            vendor: "Generic".into(),
            notes: Vec::new(),
            model: "RGBW".into(),
            modes: vec![ProfileMode {
                name: "4ch".into(),
                channels: vec![
                    ch("Red", Some(Role::Red), Resolution::Eight),
                    ch("Green", Some(Role::Green), Resolution::Eight),
                    ch("Blue", Some(Role::Blue), Resolution::Eight),
                    ch("White", Some(Role::White), Resolution::Eight),
                ],
            }],
        }
    }

    fn mover() -> FixtureProfile {
        FixtureProfile {
            vendor: "Acme".into(),
            notes: Vec::new(),
            model: "Mover".into(),
            modes: vec![ProfileMode {
                name: "5ch".into(),
                channels: vec![
                    ch(
                        "Pan",
                        Some(Role::Pan),
                        Resolution::SixteenCoarse { fine_offset: 1 },
                    ),
                    ch("Pan fine", None, Resolution::Eight),
                    ch(
                        "Tilt",
                        Some(Role::Tilt),
                        Resolution::SixteenCoarse { fine_offset: 1 },
                    ),
                    ch("Tilt fine", None, Resolution::Eight),
                    ch("Dimmer", Some(Role::Dimmer), Resolution::Eight),
                ],
            }],
        }
    }

    struct Lib(HashMap<String, Arc<FixtureProfile>>);
    impl Lib {
        fn new() -> Self {
            let mut m = HashMap::new();
            m.insert("generic/rgbw".to_string(), Arc::new(rgbw()));
            m.insert("acme/mover".to_string(), Arc::new(mover()));
            Self(m)
        }
    }
    impl ProfileSource for Lib {
        fn load(&mut self, r: &str) -> Result<Arc<FixtureProfile>, ProfileError> {
            self.0
                .get(r)
                .cloned()
                .ok_or_else(|| ProfileError::Io(format!("no such profile {r}")))
        }
    }

    fn par(name: &str, universe: UniverseId, address: u16) -> Fixture {
        let mut f = Fixture::new(name, "generic/rgbw", "4ch");
        f.universe = universe;
        f.address = address;
        f
    }

    fn errs(r: Result<ResolvedRig, Vec<PatchMessage>>) -> Vec<PatchMessage> {
        match r {
            Err(m) => m
                .into_iter()
                .filter(|m| m.severity == Severity::Error)
                .collect(),
            Ok(_) => panic!("expected the patch to be refused"),
        }
    }

    #[test]
    fn a_clean_patch_resolves() {
        let rig = validate(&[par("a", 1, 1), par("b", 1, 5)], &mut Lib::new()).unwrap();
        assert_eq!(rig.fixtures.len(), 2);
        assert_eq!(rig.fixtures[0].base, 0);
        assert_eq!(rig.fixtures[1].base, 4);
        assert_eq!(rig.universes(), vec![1]);
    }

    #[test]
    fn fixtures_may_share_an_address_in_different_universes() {
        let rig = validate(&[par("a", 1, 1), par("b", 2, 1)], &mut Lib::new()).unwrap();
        assert_eq!(rig.fixtures.len(), 2);
        assert_eq!(rig.universes(), vec![1, 2]);
    }

    /// Contract C5. The failure this prevents resolves "successfully" and then writes one
    /// fixture's values onto another's channels.
    #[test]
    fn overlapping_addresses_are_fatal_and_name_both_fixtures() {
        let e = errs(validate(&[par("a", 1, 1), par("b", 1, 3)], &mut Lib::new()));
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].fixture.as_deref(), Some("b"));
        assert!(e[0].text.contains("DMX 3"), "{}", e[0].text);
        assert!(
            e[0].text.contains("\"a\""),
            "must name the owner: {}",
            e[0].text
        );
    }

    #[test]
    fn address_zero_is_fatal() {
        let e = errs(validate(&[par("a", 1, 0)], &mut Lib::new()));
        assert!(e[0].text.contains("at least 1"), "{}", e[0].text);
    }

    #[test]
    fn running_past_the_universe_end_is_fatal() {
        let e = errs(validate(&[par("a", 1, 510)], &mut Lib::new()));
        assert!(e[0].text.contains("past slot 512"), "{}", e[0].text);
    }

    #[test]
    fn a_fixture_ending_exactly_at_slot_512_is_legal() {
        let rig = validate(&[par("a", 1, 509)], &mut Lib::new()).unwrap();
        assert_eq!(rig.fixtures[0].span(), 508..512);
    }

    #[test]
    fn missing_profile_is_fatal_and_names_the_fixture() {
        let mut f = par("a", 1, 1);
        f.profile = "nope/missing".into();
        let e = errs(validate(&[f], &mut Lib::new()));
        assert_eq!(e[0].fixture.as_deref(), Some("a"));
        assert!(e[0].text.contains("failed to load"), "{}", e[0].text);
    }

    #[test]
    fn unknown_mode_is_fatal_and_lists_what_is_available() {
        let mut f = par("a", 1, 1);
        f.mode = "99ch".into();
        let e = errs(validate(&[f], &mut Lib::new()));
        assert!(e[0].text.contains("no mode"), "{}", e[0].text);
        assert!(
            e[0].text.contains("4ch"),
            "must list alternatives: {}",
            e[0].text
        );
    }

    #[test]
    fn out_of_range_universe_is_fatal() {
        let e = errs(validate(&[par("a", 0, 1)], &mut Lib::new()));
        assert!(
            e[0].text.contains("outside the legal range"),
            "{}",
            e[0].text
        );
    }

    /// An operator fixing a patch at load-in should see everything wrong at once, not one
    /// error per restart.
    #[test]
    fn all_errors_are_collected_not_short_circuited() {
        let mut bad_mode = par("c", 1, 20);
        bad_mode.mode = "nope".into();
        let e = errs(validate(
            &[par("a", 1, 0), par("b", 1, 511), bad_mode],
            &mut Lib::new(),
        ));
        assert_eq!(e.len(), 3, "every fixture's problem must be reported");
    }

    #[test]
    fn slot_owner_table_annotates_each_claimed_slot() {
        let rig = validate(&[par("a", 1, 1)], &mut Lib::new()).unwrap();
        let owners = &rig.slot_owners[&1];
        assert_eq!(owners[0].fixture, "a");
        assert_eq!(owners[0].channel, "Red");
        assert_eq!(owners[0].role, Some(Role::Red));
        assert_eq!(owners[4].fixture, "", "unclaimed slots stay empty");
    }

    #[test]
    fn emit_writes_every_channel_of_the_mode() {
        let rig = validate(&[par("a", 1, 1)], &mut Lib::new()).unwrap();
        let mut out = Vec::new();
        rig.fixtures[0].emit(&|r| (r == Role::Red).then_some(1.0), &mut out);
        assert_eq!(out.len(), 4);
        assert_eq!(out[0].slot, 0);
        assert!((out[0].value - 1.0).abs() < f32::EPSILON);
        assert!(
            (out[1].value - 0.0).abs() < f32::EPSILON,
            "undriven role falls to default"
        );
    }

    /// The fine byte is written by its coarse partner. Emitting it separately would overwrite
    /// the low byte with the channel default and destroy 16-bit precision.
    #[test]
    fn emit_skips_the_fine_byte_of_a_sixteen_bit_pair() {
        let mut f = Fixture::new("m", "acme/mover", "5ch");
        f.universe = 1;
        f.address = 1;
        let rig = validate(&[f], &mut Lib::new()).unwrap();
        let mut out = Vec::new();
        rig.fixtures[0].emit(&|_| Some(0.5), &mut out);
        let slots: Vec<u16> = out.iter().map(|w| w.slot).collect();
        assert_eq!(slots, vec![0, 2, 4], "fine bytes 1 and 3 are not emitted");
        assert_eq!(
            out[0].resolution,
            Resolution::SixteenCoarse { fine_offset: 1 }
        );
    }

    #[test]
    fn invert_pan_flips_the_value() {
        let mut f = Fixture::new("m", "acme/mover", "5ch");
        f.invert_pan = true;
        let rig = validate(&[f], &mut Lib::new()).unwrap();
        let mut out = Vec::new();
        rig.fixtures[0].emit(&|r| (r == Role::Pan).then_some(0.25), &mut out);
        assert!((out[0].value - 0.75).abs() < 1e-6, "got {}", out[0].value);
    }

    #[test]
    fn swap_pan_tilt_exchanges_the_two_sources() {
        let mut f = Fixture::new("m", "acme/mover", "5ch");
        f.swap_pan_tilt = true;
        let rig = validate(&[f], &mut Lib::new()).unwrap();
        let mut out = Vec::new();
        rig.fixtures[0].emit(
            &|r| match r {
                Role::Pan => Some(0.1),
                Role::Tilt => Some(0.9),
                _ => None,
            },
            &mut out,
        );
        // Slot 0 is the Pan channel, which must now carry the Tilt value.
        assert!(
            (out[0].value - 0.9).abs() < 1e-6,
            "pan slot got {}",
            out[0].value
        );
        assert!(
            (out[1].value - 0.1).abs() < 1e-6,
            "tilt slot got {}",
            out[1].value
        );
    }

    #[test]
    fn invert_and_swap_compose() {
        let mut f = Fixture::new("m", "acme/mover", "5ch");
        f.swap_pan_tilt = true;
        f.invert_pan = true;
        let rig = validate(&[f], &mut Lib::new()).unwrap();
        let mut out = Vec::new();
        rig.fixtures[0].emit(
            &|r| match r {
                Role::Pan => Some(0.1),
                Role::Tilt => Some(0.9),
                _ => None,
            },
            &mut out,
        );
        // Swap takes Tilt's 0.9 into the pan slot, then invert makes it 0.1.
        assert!((out[0].value - 0.1).abs() < 1e-6, "got {}", out[0].value);
    }

    #[test]
    fn a_pinned_default_overrides_the_profile_default() {
        let mut f = par("a", 1, 1);
        f.defaults.insert("White".into(), 200);
        let rig = validate(&[f], &mut Lib::new()).unwrap();
        assert_eq!(rig.fixtures[0].channel_defaults[3], 200);
    }

    #[test]
    fn an_empty_patch_resolves_to_an_empty_rig() {
        let rig = validate(&[], &mut Lib::new()).unwrap();
        assert!(rig.fixtures.is_empty());
        assert!(rig.universes().is_empty());
    }
}
