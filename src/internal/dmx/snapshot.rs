//! The lighting state snapshot.
//!
//! One serializable view read by the UI panel and by `GET /api/state/lighting` alike. Neither
//! consumer computes anything the other lacks, which is the parity rule in
//! /spec/lighting-routing.md § API Parity stated as a data structure.
//!
//! Built from a `try_lock` on the driver status so a slow consumer can never back up the
//! transmit path. A missed snapshot is harmless; the next frame overwrites it.

use super::driver::DriverStatus;
use super::patch::{ResolvedRig, Severity};
use super::universe::{UNIVERSE_SIZE, UniverseId};
use super::watchdog::WatchdogFlag;
use serde::Serialize;

/// Who owns one DMX slot, flattened for transport.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct SlotOwnerView {
    /// 1-based DMX address, as printed on a fixture's display.
    pub address: u16,
    pub fixture: String,
    pub channel: String,
    pub role: Option<String>,
    /// Value most recently handed to the transport.
    pub value: u8,
}

/// One universe as last transmitted, annotated with who owns each slot.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct UniverseView {
    pub universe: UniverseId,
    /// Only slots claimed by a fixture. A 512-entry array per universe would dominate the
    /// payload for a rig using twenty of them, and an unclaimed slot is always zero.
    pub slots: Vec<SlotOwnerView>,
}

/// One fixture as patched, for the rig panel and the patch editor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FixtureView {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub model: String,
    pub mode: String,
    pub universe: UniverseId,
    pub address: u16,
    pub channel_count: usize,
    pub roles: Vec<String>,
    pub invert_pan: bool,
    pub invert_tilt: bool,
    pub swap_pan_tilt: bool,
    /// Normalized stage position, or `None` when unplaced. Lays out the stage plot and is the
    /// point a sampled role reads the video at.
    pub position: Option<[f32; 2]>,
}

/// One entry of the patch **as configured**, whether or not it resolved.
///
/// Distinct from [`FixtureView`], which describes a fixture that actually patched. A rig that
/// fails validation produces no `FixtureView`s at all, so without this the UI showed "No
/// fixtures patched" while the config still held the entries that caused the failure — leaving
/// a bad patch impossible to see and impossible to delete, and the rig permanently dark.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PatchEntryView {
    pub id: String,
    pub name: String,
    pub profile: String,
    pub mode: String,
    pub universe: UniverseId,
    pub address: u16,
    /// Why this entry did not patch. `None` means it resolved.
    pub error: Option<String>,
}

/// A palette, flattened for transport.
///
/// Both storages appear in one list because a consumer references a palette by UUID and does
/// not care which file it came from. `venue` says which, so the editor can show an operator
/// which of their palettes will survive a move to another room.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PaletteView {
    pub id: String,
    pub name: String,
    pub kind: String,
    /// True when the palette belongs to the venue rather than the show.
    pub venue: bool,
    /// Roles the role-space default covers.
    pub default_roles: Vec<String>,
    /// The values behind those roles, in the same order as `default_roles`.
    ///
    /// Carried so an editor can show a palette's actual color rather than only its name. A
    /// palette whose value is invisible is indistinguishable from one that does nothing, which
    /// is exactly how the first version read.
    pub default_values: Vec<f32>,
    pub override_count: usize,
}

/// A patch validation finding, carried so the API reports the same thing the UI shows.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PatchMessageView {
    pub severity: String,
    pub fixture: Option<String>,
    pub text: String,
}

/// Everything a consumer needs to render or inspect the lighting subsystem.
// Serialized DTO: these flags mirror independent engine conditions, not a state enum. A rig can
// be enabled but not running, running but unhealthy, and healthy but wedged, all at once.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct LightingSnapshot {
    /// False when no rig is patched, which is the default and must stay cheap.
    pub enabled: bool,
    pub running: bool,
    pub healthy: bool,
    pub transports: Vec<String>,
    pub packets_sent: u64,
    pub consecutive_errors: u64,
    pub frames_dropped: u64,
    /// Overwrites persisted with no drain at all, which is a stuck consumer rather than
    /// ordinary backpressure and deserves a distinct signal.
    pub consumer_wedged: bool,
    pub last_error: Option<String>,
    pub fixtures: Vec<FixtureView>,
    /// The patch as configured, including entries that failed to resolve. Always populated, so
    /// a broken patch can be seen and repaired.
    pub patch: Vec<PatchEntryView>,
    pub universes: Vec<UniverseView>,
    pub warnings: Vec<PatchMessageView>,
    pub watchdog: Vec<WatchdogFlag>,
    pub blackout: bool,
    /// Every palette from both storages.
    pub palettes: Vec<PaletteView>,
    /// Global intensity scalar.
    pub master: f32,
    /// How many role values the programmer is holding.
    ///
    /// Non-zero means the performer's hands are overriding the looks somewhere, which the UI
    /// surfaces as a Release control rather than leaving as an invisible mode.
    pub programmer_roles: usize,
}

impl LightingSnapshot {
    /// Build a snapshot from the resolved rig, the driver status, and the last transmitted
    /// frames.
    #[must_use]
    pub fn build(input: SnapshotInput<'_>) -> Self {
        let SnapshotInput {
            rig,
            status,
            last_frames,
            watchdog,
            blackout,
            palettes,
            master,
            programmer_roles,
            patch,
            messages,
        } = input;

        let to_view = |m: &super::patch::PatchMessage| PatchMessageView {
            severity: match m.severity {
                Severity::Error => "error".into(),
                Severity::Warning => "warning".into(),
            },
            fixture: m.fixture.clone(),
            text: m.text.clone(),
        };
        let palette_views = palette_views(palettes);

        let Some(rig) = rig else {
            // A rig that failed to validate still has a patch, and that patch is the only thing
            // an operator can act on to fix it.
            return Self {
                blackout,
                palettes: palette_views,
                master,
                programmer_roles,
                patch,
                warnings: messages.iter().map(to_view).collect(),
                ..Self::default()
            };
        };

        let fixtures = rig
            .fixtures
            .iter()
            .map(|f| {
                let mode = &f.profile.modes[f.mode_index];
                FixtureView {
                    id: f.id.to_string(),
                    name: f.name.clone(),
                    vendor: f.profile.vendor.clone(),
                    model: f.profile.model.clone(),
                    mode: mode.name.clone(),
                    universe: f.universe,
                    address: f.base + 1,
                    channel_count: mode.channels.len(),
                    roles: mode
                        .roles()
                        .into_iter()
                        .map(|r| r.as_str().to_owned())
                        .collect(),
                    invert_pan: f.invert_pan,
                    invert_tilt: f.invert_tilt,
                    swap_pan_tilt: f.swap_pan_tilt,
                    position: f.position,
                }
            })
            .collect();

        let mut universes: Vec<UniverseView> = rig
            .slot_owners
            .iter()
            .map(|(universe, owners)| {
                let frame = last_frames.get(*universe);
                let slots = owners
                    .iter()
                    .enumerate()
                    .filter(|(_, o)| !o.fixture.is_empty())
                    .map(|(idx, o)| SlotOwnerView {
                        address: u16::try_from(idx + 1).unwrap_or(u16::MAX),
                        fixture: o.fixture.clone(),
                        channel: o.channel.clone(),
                        role: o.role.map(|r| r.as_str().to_owned()),
                        value: frame.and_then(|f| f.get(idx).copied()).unwrap_or(0),
                    })
                    .collect();
                UniverseView {
                    universe: *universe,
                    slots,
                }
            })
            .collect();
        // Deterministic order: a map iteration would reshuffle the payload every poll and make
        // the WebSocket JSON Patch stream emit spurious deltas.
        universes.sort_by_key(|u| u.universe);

        Self {
            enabled: !rig.fixtures.is_empty(),
            running: status.running,
            healthy: status.healthy,
            transports: status.transports.clone(),
            packets_sent: status.packets_sent,
            consecutive_errors: status.consecutive_errors,
            frames_dropped: status.frames_dropped,
            consumer_wedged: status.consumer_wedged,
            last_error: status.last_error.clone(),
            fixtures,
            patch,
            universes,
            // `messages`, not `rig.warnings`: the runtime copies the rig's findings into it and
            // then appends transport failures, so a rig that resolved but cannot transmit still
            // reports why.
            warnings: messages.iter().map(to_view).collect(),
            watchdog,
            blackout,
            palettes: palette_views,
            master,
            programmer_roles,
        }
    }
}

/// Everything [`LightingSnapshot::build`] needs.
///
/// A struct rather than eight positional parameters: the two `f32`-ish scalars and the two
/// `bool`-ish flags at the end were easy to transpose silently.
pub struct SnapshotInput<'a> {
    pub rig: Option<&'a ResolvedRig>,
    pub status: &'a DriverStatus,
    pub last_frames: &'a super::universe::UniverseSet,
    pub watchdog: Vec<WatchdogFlag>,
    pub blackout: bool,
    pub palettes: &'a super::palette::PaletteSet,
    pub master: f32,
    pub programmer_roles: usize,
    /// The patch as configured. Passed separately from `rig` because it must survive a
    /// validation failure, which is exactly when an operator needs to see it.
    pub patch: Vec<PatchEntryView>,
    /// Patch findings, carried for the same reason: a rig that failed to resolve reports nothing
    /// of its own, and those findings are the only thing saying why.
    pub messages: Vec<super::patch::PatchMessage>,
}

/// Flatten every palette for transport, in a stable order.
fn palette_views(palettes: &super::palette::PaletteSet) -> Vec<PaletteView> {
    palettes
        .all_sorted()
        .into_iter()
        .map(|p| PaletteView {
            id: p.id.to_string(),
            name: p.name.clone(),
            kind: match p.kind {
                super::palette::PaletteKind::Color => "color".into(),
                super::palette::PaletteKind::Beam => "beam".into(),
                super::palette::PaletteKind::Position => "position".into(),
            },
            venue: !p.kind.is_show_state(),
            default_roles: {
                // Sorted so the payload is stable and the JSON Patch stream does not emit
                // deltas for a map that merely re-ordered itself.
                let mut pairs: Vec<(String, f32)> = p
                    .default
                    .iter()
                    .map(|(r, v)| (r.as_str().to_owned(), *v))
                    .collect();
                pairs.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                pairs.into_iter().map(|(r, _)| r).collect()
            },
            default_values: {
                let mut pairs: Vec<(String, f32)> = p
                    .default
                    .iter()
                    .map(|(r, v)| (r.as_str().to_owned(), *v))
                    .collect();
                pairs.sort_unstable_by(|a, b| a.0.cmp(&b.0));
                pairs.into_iter().map(|(_, v)| v).collect()
            },
            override_count: p.overrides.len(),
        })
        .collect()
}

/// Convenience: the universe view's slot count, for tests and the UI's density decision.
#[must_use]
pub fn claimed_slot_count(view: &UniverseView) -> usize {
    view.slots.len().min(UNIVERSE_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmx::palette::PaletteSet;
    use crate::dmx::patch::{Fixture, ProfileSource, validate};
    use crate::dmx::profile::{FixtureProfile, ProfileChannel, ProfileError, ProfileMode};
    use crate::dmx::role::Role;
    use crate::dmx::universe::{Resolution, SlotWrite, UniverseSet};
    use std::sync::Arc;

    fn rgbw() -> FixtureProfile {
        FixtureProfile {
            vendor: "Generic".into(),
            notes: Vec::new(),
            model: "RGBW".into(),
            modes: vec![ProfileMode {
                name: "4ch".into(),
                channels: ["Red", "Green", "Blue", "White"]
                    .iter()
                    .zip([Role::Red, Role::Green, Role::Blue, Role::White])
                    .map(|(n, r)| ProfileChannel {
                        name: (*n).to_string(),
                        role: Some(r),
                        resolution: Resolution::Eight,
                        default: 0,
                        ranges: Vec::new(),
                    })
                    .collect(),
            }],
        }
    }

    struct Lib;
    impl ProfileSource for Lib {
        fn load(&mut self, _r: &str) -> Result<Arc<FixtureProfile>, ProfileError> {
            Ok(Arc::new(rgbw()))
        }
    }

    fn rig_of(specs: &[(&str, UniverseId, u16)]) -> crate::dmx::patch::ResolvedRig {
        let fixtures: Vec<Fixture> = specs
            .iter()
            .map(|(name, u, a)| {
                let mut f = Fixture::new(*name, "generic/rgbw", "4ch");
                f.universe = *u;
                f.address = *a;
                f
            })
            .collect();
        validate(&fixtures, &mut Lib).expect("valid patch")
    }

    fn status() -> DriverStatus {
        DriverStatus {
            running: true,
            healthy: true,
            transports: vec!["art-net 10.0.0.5:6454".into()],
            packets_sent: 42,
            ..DriverStatus::default()
        }
    }

    #[test]
    fn no_rig_produces_a_disabled_snapshot() {
        let s = LightingSnapshot::build(SnapshotInput {
            rig: None,
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert!(!s.enabled);
        assert!(s.fixtures.is_empty());
        assert!(s.universes.is_empty());
    }

    #[test]
    fn blackout_is_reported_even_with_no_rig() {
        let s = LightingSnapshot::build(SnapshotInput {
            rig: None,
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: true,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert!(s.blackout);
    }

    #[test]
    fn fixtures_are_reported_with_their_patch() {
        let rig = rig_of(&[("par-1", 1, 1), ("par-2", 1, 5)]);
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert!(s.enabled);
        assert_eq!(s.fixtures.len(), 2);
        assert_eq!(s.fixtures[0].name, "par-1");
        assert_eq!(
            s.fixtures[0].address, 1,
            "1-based, as printed on the fixture"
        );
        assert_eq!(s.fixtures[1].address, 5);
        assert_eq!(s.fixtures[0].channel_count, 4);
        assert_eq!(s.fixtures[0].roles, vec!["red", "green", "blue", "white"]);
    }

    #[test]
    fn driver_status_is_carried_through() {
        let rig = rig_of(&[("par-1", 1, 1)]);
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert!(s.running && s.healthy);
        assert_eq!(s.packets_sent, 42);
        assert_eq!(s.transports, vec!["art-net 10.0.0.5:6454".to_string()]);
    }

    #[test]
    fn only_claimed_slots_are_reported() {
        let rig = rig_of(&[("par-1", 1, 1)]);
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert_eq!(s.universes.len(), 1);
        assert_eq!(
            s.universes[0].slots.len(),
            4,
            "a 512-entry array would dominate the payload for no information"
        );
        assert_eq!(s.universes[0].slots[0].address, 1);
        assert_eq!(s.universes[0].slots[0].channel, "Red");
        assert_eq!(s.universes[0].slots[0].role.as_deref(), Some("red"));
    }

    #[test]
    fn slot_values_come_from_the_last_transmitted_frame() {
        let rig = rig_of(&[("par-1", 1, 1)]);
        let mut frames = UniverseSet::new();
        frames.apply(SlotWrite {
            universe: 1,
            slot: 0,
            resolution: Resolution::Eight,
            value: 1.0,
        });
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &frames,
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert_eq!(s.universes[0].slots[0].value, 255);
        assert_eq!(s.universes[0].slots[1].value, 0);
    }

    /// A map iteration order would reshuffle the payload every poll and make the WebSocket
    /// JSON Patch stream emit deltas for data that did not change.
    #[test]
    fn universes_are_reported_in_a_stable_order() {
        let rig = rig_of(&[("a", 7, 1), ("b", 2, 1), ("c", 40, 1)]);
        for _ in 0..20 {
            let s = LightingSnapshot::build(SnapshotInput {
                rig: Some(&rig),
                status: &status(),
                last_frames: &UniverseSet::new(),
                watchdog: Vec::new(),
                blackout: false,
                palettes: &PaletteSet::new(),
                master: 1.0,
                programmer_roles: 0,
                patch: Vec::new(),
                messages: Vec::new(),
            });
            let ids: Vec<UniverseId> = s.universes.iter().map(|u| u.universe).collect();
            assert_eq!(ids, vec![2, 7, 40]);
        }
    }

    #[test]
    fn watchdog_flags_ride_the_snapshot() {
        let rig = rig_of(&[("par-1", 1, 1)]);
        let flags = vec![WatchdogFlag {
            fixture: 0,
            name: "par-1".into(),
            kind: crate::dmx::watchdog::WatchdogKind::Dark,
            age_secs: 9.0,
        }];
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: flags,
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert_eq!(s.watchdog.len(), 1);
        assert_eq!(s.watchdog[0].name, "par-1");
    }

    #[test]
    fn the_snapshot_serializes_to_json() {
        let rig = rig_of(&[("par-1", 1, 1)]);
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        let v = serde_json::to_value(&s).expect("serializable");
        assert_eq!(v["enabled"], true);
        assert_eq!(v["fixtures"][0]["name"], "par-1");
        assert_eq!(v["universes"][0]["slots"][0]["channel"], "Red");
    }

    #[test]
    fn helper_counts_claimed_slots() {
        let rig = rig_of(&[("par-1", 1, 1)]);
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert_eq!(claimed_slot_count(&s.universes[0]), 4);
    }

    #[test]
    fn palettes_from_both_storages_are_reported_with_their_home() {
        use crate::dmx::palette::{Palette, PaletteKind};
        let rig = rig_of(&[("par-1", 1, 1)]);
        let show = vec![Palette::new("Deep Blue", PaletteKind::Color)];
        let venue = vec![Palette::new("Downstage", PaletteKind::Position)];
        let set = PaletteSet::from_parts(&show, &venue);
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &set,
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert_eq!(s.palettes.len(), 2);
        let blue = s.palettes.iter().find(|p| p.name == "Deep Blue").unwrap();
        assert!(!blue.venue, "a color palette travels with the show");
        let downstage = s.palettes.iter().find(|p| p.name == "Downstage").unwrap();
        assert!(downstage.venue, "a position palette belongs to the room");
    }

    #[test]
    fn palette_default_roles_are_sorted_for_a_stable_payload() {
        use crate::dmx::palette::{Palette, PaletteKind};
        let rig = rig_of(&[("par-1", 1, 1)]);
        let mut palette = Palette::new("p", PaletteKind::Color);
        palette.set_default(Role::Red, 1.0);
        palette.set_default(Role::Blue, 1.0);
        palette.set_default(Role::Green, 1.0);
        let set = PaletteSet::from_parts(&[palette], &[]);
        for _ in 0..10 {
            let s = LightingSnapshot::build(SnapshotInput {
                rig: Some(&rig),
                status: &status(),
                last_frames: &UniverseSet::new(),
                watchdog: Vec::new(),
                blackout: false,
                palettes: &set,
                master: 1.0,
                programmer_roles: 0,
                patch: Vec::new(),
                messages: Vec::new(),
            });
            assert_eq!(s.palettes[0].default_roles, vec!["blue", "green", "red"]);
        }
    }

    #[test]
    fn an_empty_rig_reports_disabled_but_still_running() {
        let rig = validate(&[], &mut Lib).unwrap();
        let s = LightingSnapshot::build(SnapshotInput {
            rig: Some(&rig),
            status: &status(),
            last_frames: &UniverseSet::new(),
            watchdog: Vec::new(),
            blackout: false,
            palettes: &PaletteSet::new(),
            master: 1.0,
            programmer_roles: 0,
            patch: Vec::new(),
            messages: Vec::new(),
        });
        assert!(!s.enabled, "no fixtures means nothing to light");
        assert!(s.running, "the driver may still be up");
    }
}
