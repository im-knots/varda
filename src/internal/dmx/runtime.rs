//! The lighting runtime: config in, DMX on the wire.
//!
//! Owns the resolved rig, the driver, the smoother, the guard, and the watchdog, and ticks them
//! in the order /spec/lighting-routing.md requires: guard, then smooth, then patch, then
//! transmit.
//!
//! Lives in the engine, never in `usecases/ui`, so `--headless` drives lights identically to a
//! windowed run. See /spec/lighting-routing.md § API Parity.

use super::config::LightingConfig;
use super::driver::{DmxDriver, DriverStatus};
use super::library::ProfileLibrary;
use super::merge::{LightingChannel, merge};
use super::palette::{self, PaletteSet};
use super::patch::{PatchMessage, ResolvedRig, Severity, validate};
use super::role::Role;
use super::show::LightingShow;
use super::smoother::RoleSmoother;
use super::snapshot::LightingSnapshot;
use super::universe::{SlotWrite, UniverseSet};
use super::values::RoleValues;
use super::watchdog::FixtureWatchdog;
use std::path::PathBuf;
use std::time::Instant;
use uuid::Uuid;

/// Ties the lighting subsystem together for one workspace.
pub struct LightingRuntime {
    config: LightingConfig,
    library: ProfileLibrary,
    rig: Option<ResolvedRig>,
    driver: Option<DmxDriver>,
    smoother: RoleSmoother,
    watchdog: FixtureWatchdog,
    /// Show-side lighting state: looks, palettes, decks per channel, master.
    show: LightingShow,
    /// Every palette from both storages, rebuilt whenever either side changes.
    palettes: PaletteSet,
    /// Live channel opacities, supplied by the mixer each tick. Shared control state: the mixer
    /// owns channel opacity once and both composite backends read it.
    channels: Vec<(String, f32)>,
    /// The latest downsampled video frame for each sampled source, supplied by the render path.
    ///
    /// Held rather than fetched so the merge stays a pure function of state: the render thread
    /// pushes frames in when a readback lands, and a tick that arrives between readbacks reuses
    /// the last one rather than stalling for a fresh one.
    /// See /spec/lighting-routing.md § Sampled.
    frames: super::sample::SampledFrames,
    /// Direct role writes from the parameter router, which outrank the merged look values.
    ///
    /// This is the console programmer: whatever the performer's hands are touching wins until
    /// released. See /spec/lighting-routing.md § The programmer.
    programmer: Vec<RoleValues>,
    /// Per fixture, the values handed to the output path this tick.
    values: Vec<RoleValues>,
    last_frames: UniverseSet,
    blackout: bool,
    /// Findings from the most recent patch resolution, fatal ones included.
    messages: Vec<PatchMessage>,
    scratch: Vec<SlotWrite>,
    /// Rebuilt on a divider rather than every frame. This is a monitoring surface read by the
    /// UI panel and `/api/state/lighting`, not part of the render path, and rebuilding it
    /// allocates a string per claimed slot. At 60fps a divider of 6 gives 10 Hz, which is
    /// faster than an operator can read and far cheaper than per-frame.
    snapshot: LightingSnapshot,
    snapshot_tick: u32,
    /// Mode names per profile reference, for the patch editor's mode picker.
    ///
    /// Built once at construction and shared by `Arc`: indexing the bundled library parses
    /// hundreds of files, which must not happen per frame or per patch edit.
    profile_modes: std::sync::Arc<std::collections::HashMap<String, Vec<String>>>,
}

/// Frames between snapshot rebuilds.
const SNAPSHOT_DIVIDER: u32 = 6;

impl LightingRuntime {
    /// Build a runtime from config, resolving the patch and starting the driver.
    ///
    /// Never fails: a rig that cannot be resolved reports its findings and runs inert. An
    /// unpatchable rig must not stop the rest of the application from starting, because the
    /// operator still has video to run.
    #[must_use]
    pub fn new(config: LightingConfig, bundled_profile_dirs: Vec<PathBuf>) -> Self {
        let mut dirs = config.profile_dirs.clone();
        dirs.extend(bundled_profile_dirs);
        let mut runtime = Self {
            config,
            library: ProfileLibrary::new(dirs),
            rig: None,
            driver: None,
            smoother: RoleSmoother::default(),
            watchdog: FixtureWatchdog::default(),
            show: LightingShow::default(),
            palettes: PaletteSet::new(),
            channels: Vec::new(),
            frames: super::sample::SampledFrames::new(),
            programmer: Vec::new(),
            values: Vec::new(),
            last_frames: UniverseSet::new(),
            blackout: false,
            messages: Vec::new(),
            scratch: Vec::new(),
            snapshot: LightingSnapshot::default(),
            snapshot_tick: 0,
            profile_modes: std::sync::Arc::default(),
        };
        runtime.rebuild();
        runtime.reindex_modes();
        runtime.refresh_snapshot();
        runtime
    }

    /// Re-resolve the patch and restart the driver.
    ///
    /// Called on construction and after any structural change. The old driver is dropped first,
    /// which releases every universe it was holding before the new one claims them.
    pub fn rebuild(&mut self) {
        self.driver = None; // release universes before the replacement binds
        self.rig = None;
        self.messages.clear();
        self.library.clear_cache();

        if !self.config.enabled {
            self.resize_state(0);
            return;
        }

        match validate(&self.config.fixtures, &mut self.library) {
            Ok(rig) => {
                self.messages.clone_from(&rig.warnings);
                self.resize_state(rig.fixtures.len());
                self.rig = Some(rig);
            }
            Err(messages) => {
                for m in &messages {
                    if m.severity == Severity::Error {
                        log::error!(
                            "DMX patch: {}{}",
                            m.fixture
                                .as_deref()
                                .map_or(String::new(), |f| format!("{f}: ")),
                            m.text
                        );
                    }
                }
                self.messages = messages;
                self.resize_state(0);
                return;
            }
        }

        if !self.config.is_live() {
            return;
        }

        let mut transports = Vec::new();
        for t in &self.config.transports {
            match t.build(
                &self.config.source_name,
                self.config.cid_bytes(),
                self.config.priority,
            ) {
                Ok(built) => transports.push(built),
                Err(e) => {
                    log::error!("DMX transport: {e}");
                    self.messages.push(PatchMessage {
                        severity: Severity::Error,
                        fixture: None,
                        text: e,
                    });
                }
            }
        }
        if transports.is_empty() {
            log::warn!("DMX: no usable transport, lighting is inert");
            return;
        }
        self.driver = Some(DmxDriver::start(transports, self.config.fps));
    }

    /// Force a snapshot rebuild, so a structural change is visible immediately rather than on
    /// the next divider tick.
    fn refresh_snapshot(&mut self) {
        self.snapshot = self.build_snapshot();
    }

    fn resize_state(&mut self, count: usize) {
        self.smoother.resize(count);
        self.watchdog.resize(count);
        self.values.resize(count, RoleValues::new());
        self.programmer.resize(count, RoleValues::new());
        self.rebuild_palettes();
    }

    /// Recombine the show's palettes with the venue's.
    fn rebuild_palettes(&mut self) {
        self.palettes = PaletteSet::from_parts(&self.show.palettes, &self.config.palettes);
    }

    /// Replace the show-side lighting state.
    /// Hand the merge a freshly read-back frame for one sampled source.
    ///
    /// Called from the render path when a readback completes. Keyed by channel UUID, or
    /// [`super::sample::PROGRAM_KEY`] for the program.
    pub fn set_sampled_frame(
        &mut self,
        key: impl Into<String>,
        frame: super::sample::SampledFrame,
    ) {
        self.frames.insert(key.into(), frame);
    }

    /// Drop frames for sources that are no longer sampled, so a deleted channel's last frame
    /// cannot keep driving a rig.
    pub fn retain_sampled_frames(&mut self, keep: &dyn Fn(&str) -> bool) {
        self.frames.retain(|k, _| keep(k));
    }

    /// Which sources any deck is currently sampling, so the render path knows what to read back.
    ///
    /// Empty almost always: a show with no sampled decks costs no readbacks at all.
    #[must_use]
    pub fn sampled_sources(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        // A sampled deck reads the channel its *listening group* is pointed at. The group already
        // knows which channel that is, so there is nothing to configure and nothing to ask.
        for group in &self.config.groups {
            let Some(source) = group.source.as_ref() else {
                continue;
            };
            // A Program group hears every channel, so every channel with a sampled deck needs a
            // frame; a Channel group needs only its own.
            let heard: Vec<String> = match source {
                super::look::GroupSource::Program => self
                    .show
                    .channel_decks
                    .iter()
                    .map(|c| c.channel.clone())
                    .collect(),
                super::look::GroupSource::Channel { uuid } => vec![uuid.clone()],
            };
            for key in heard {
                let samples = self
                    .show
                    .decks(&key)
                    .iter()
                    .any(|d| !d.content.samples().is_empty());
                if samples && !out.contains(&key) {
                    out.push(key);
                }
            }
        }
        out
    }

    /// The venue-side config: fixtures, groups, transports, venue palettes.
    #[must_use]
    pub fn config(&self) -> &LightingConfig {
        &self.config
    }

    /// Replace the config and re-resolve the patch. Structural, so the snapshot refreshes now
    /// rather than on the next divider tick.
    pub fn set_config(&mut self, config: LightingConfig) {
        self.config = config;
        self.rebuild();
        self.reindex_modes();
        self.refresh_snapshot();
    }

    /// Show-side state: saved looks, palettes, decks per channel, master.
    #[must_use]
    pub fn show(&self) -> &LightingShow {
        &self.show
    }

    pub fn set_show(&mut self, show: LightingShow) {
        self.show = show;
        self.rebuild_palettes();
        self.refresh_snapshot();
    }

    /// Channel opacities from the mixer. Shared control state: the mixer owns channel opacity
    /// once and both composite backends read the same numbers.
    pub fn set_channels(&mut self, channels: Vec<(String, f32)>) {
        self.channels = channels;
    }

    /// The profile library, for the patch editor's profile picker.
    #[must_use]
    pub fn library(&self) -> &ProfileLibrary {
        &self.library
    }

    /// Mode names per profile reference, for the mode picker.
    #[must_use]
    pub fn profile_modes(&self) -> std::sync::Arc<std::collections::HashMap<String, Vec<String>>> {
        self.profile_modes.clone()
    }

    /// Rebuild the profile→modes index. Indexing the bundled library parses hundreds of files, so
    /// this runs on a structural change and never per frame.
    fn reindex_modes(&mut self) {
        self.profile_modes = std::sync::Arc::new(self.library.mode_index());
    }

    /// Fatal patch findings. Empty when the rig resolved.
    ///
    /// Returned whole rather than pre-formatted so a caller can render them per fixture — the
    /// patch editor puts each error on the row it belongs to rather than in one rig-wide list.
    #[must_use]
    pub fn errors(&self) -> Vec<PatchMessage> {
        self.messages
            .iter()
            .filter(|m| m.severity == Severity::Error)
            .cloned()
            .collect()
    }

    /// Every patched fixture's UUID, in patch order.
    #[must_use]
    pub fn fixture_ids(&self) -> Vec<Uuid> {
        self.rig
            .as_ref()
            .map(|r| r.fixtures.iter().map(|f| f.id).collect())
            .unwrap_or_default()
    }

    fn slot_of(&self, fixture: Uuid) -> Option<usize> {
        self.rig
            .as_ref()?
            .fixtures
            .iter()
            .position(|f| f.id == fixture)
    }

    /// Hold one role of one fixture by hand. Returns false when the fixture is not patched.
    ///
    /// This is the console programmer: whatever the performer's hands are touching outranks every
    /// look until released. See /spec/lighting-routing.md § The programmer.
    pub fn set_role(&mut self, fixture: Uuid, role: Role, value: f32) -> bool {
        let Some(slot) = self.slot_of(fixture) else {
            return false;
        };
        if let Some(values) = self.programmer.get_mut(slot) {
            values.set(role, value.clamp(0.0, 1.0));
            return true;
        }
        false
    }

    /// Hold one role across every member of a group. Returns how many fixtures were reached, so
    /// a caller can tell an empty group from a missing one.
    pub fn set_group_role(&mut self, group: Uuid, role: Role, value: f32) -> usize {
        let Some(members) = self
            .config
            .groups
            .iter()
            .find(|g| g.id == group)
            .map(|g| g.members.clone())
        else {
            return 0;
        };
        members
            .into_iter()
            .filter(|id| self.set_role(*id, role, value))
            .count()
    }

    /// Hand one role back to the looks. False when the fixture is not patched.
    pub fn release_role(&mut self, fixture: Uuid, role: Role) -> bool {
        if let Some(slot) = self.slot_of(fixture)
            && let Some(values) = self.programmer.get_mut(slot)
        {
            values.clear(role);
            return true;
        }
        false
    }

    /// Release everything the hands are holding.
    pub fn release_all(&mut self) {
        for values in &mut self.programmer {
            *values = RoleValues::new();
        }
    }

    /// Fixtures the programmer is holding values for, with those values, in patch order.
    ///
    /// Pairs rather than ids: every caller needs the values too, and looking them up again by id
    /// would re-walk the rig once per fixture.
    #[must_use]
    pub fn programmer_fixtures(&self) -> Vec<(Uuid, RoleValues)> {
        let Some(rig) = self.rig.as_ref() else {
            return Vec::new();
        };
        rig.fixtures
            .iter()
            .enumerate()
            .filter_map(|(i, f)| {
                let values = self.programmer.get(i)?;
                (!values.is_empty()).then_some((f.id, *values))
            })
            .collect()
    }

    /// What the programmer holds for one fixture.
    #[must_use]
    pub fn programmer_for(&self, fixture: Uuid) -> Option<RoleValues> {
        let values = *self.slot_of(fixture).and_then(|i| self.programmer.get(i))?;
        // Empty reads as `None`: "the hands are not on this light" and "the hands are on it but
        // holding nothing" are the same fact, and a caller made to check both will check one.
        (!values.is_empty()).then_some(values)
    }

    /// Global intensity scalar, `lighting/master`.
    #[must_use]
    pub fn master(&self) -> f32 {
        self.show.master
    }

    pub fn set_master(&mut self, value: f32) {
        self.show.master = value.clamp(0.0, 1.0);
    }

    /// Latching blackout. Only an explicit release clears it, so a stray modulator cannot
    /// un-blackout a rig mid-show.
    pub fn set_blackout(&mut self, on: bool) {
        self.blackout = on;
        // Structural enough to be visible at once: an operator hitting blackout must see it in
        // the snapshot now, not on the next divider tick.
        self.refresh_snapshot();
    }

    #[must_use]
    pub fn blackout(&self) -> bool {
        self.blackout
    }

    /// The patch as configured, each entry carrying the error that stopped it patching.
    ///
    /// Built from the config rather than the resolved rig, because a rig that fails validation
    /// resolves to nothing at all: without this the UI reported "No lights yet" over a config
    /// holding the very entries that caused the failure, so the bad patch could neither be seen
    /// nor deleted and the rig stayed dark for good.
    fn patch_entries(&self) -> Vec<super::snapshot::PatchEntryView> {
        self.config
            .fixtures
            .iter()
            .map(|f| {
                let error = self
                    .messages
                    .iter()
                    .find(|m| {
                        m.severity == Severity::Error && m.fixture.as_deref() == Some(&f.name)
                    })
                    .map(|m| m.text.clone());
                super::snapshot::PatchEntryView {
                    id: f.id.to_string(),
                    name: f.name.clone(),
                    profile: f.profile.clone(),
                    mode: f.mode.clone(),
                    universe: f.universe,
                    address: f.address,
                    error,
                }
            })
            .collect()
    }

    fn build_snapshot(&mut self) -> LightingSnapshot {
        let status = self
            .driver
            .as_ref()
            .map_or_else(DriverStatus::default, DmxDriver::status_snapshot);

        let flags = match &self.rig {
            Some(rig) => {
                let pairs: Vec<(String, RoleValues)> = rig
                    .fixtures
                    .iter()
                    .enumerate()
                    .map(|(i, f)| (f.name.clone(), self.values[i]))
                    .collect();
                self.watchdog.tick(&pairs, Instant::now(), self.blackout)
            }
            None => Vec::new(),
        };

        let patch = self.patch_entries();
        // Findings ride along even when the rig failed to resolve — that is exactly when an
        // operator needs them, and dropping them is how a bad patch became invisible before.
        let messages = self.messages.clone();
        let programmer_roles = self.programmer.iter().map(|v| v.iter().count()).sum();
        LightingSnapshot::build(super::snapshot::SnapshotInput {
            rig: self.rig.as_ref(),
            status: &status,
            last_frames: &self.last_frames,
            watchdog: flags,
            blackout: self.blackout,
            palettes: &self.palettes,
            master: self.show.master,
            programmer_roles,
            patch,
            messages,
        })
    }

    /// The monitoring snapshot, rebuilt on a divider rather than every frame.
    #[must_use]
    pub fn snapshot(&self) -> &LightingSnapshot {
        &self.snapshot
    }

    /// One frame: merge the looks, let the programmer overwrite, smooth, guard, emit, transmit.
    ///
    /// `bypass_smoothing` is for tests and for a hard cut; the modulation sampler is supplied by
    /// the caller so the merge reads the same engine the GPU path does.
    pub fn tick(
        &mut self,
        dt: f32,
        bypass_smoothing: bool,
        modulation: &super::merge::ModulationSampler<'_>,
    ) {
        let Some(rig) = self.rig.as_ref() else {
            return;
        };
        self.scratch.clear();

        // Merge the looks, then let the programmer overwrite. Whatever the performer's hands
        // are touching outranks every deck until released.
        let fixtures: Vec<Uuid> = rig.fixtures.iter().map(|f| f.id).collect();
        let channels: Vec<LightingChannel> = self
            .channels
            .iter()
            .map(|(id, opacity)| LightingChannel {
                key: id.clone(),
                opacity: *opacity,
                decks: self.show.decks(id).to_vec(),
            })
            .collect();
        // No look map: each deck carries its own content, so there is nothing to resolve.
        let positions: std::collections::HashMap<Uuid, Option<[f32; 2]>> =
            rig.fixtures.iter().map(|f| (f.id, f.position)).collect();
        let palettes = &self.palettes;
        let merged = merge(
            &channels,
            &self.config.groups,
            &fixtures,
            &|value, fixture, role| palette::resolve(value, fixture, role, palettes, rig),
            modulation,
            &self.frames,
            &positions,
        );

        let master = self.show.master;
        let blackout = self.blackout;
        let guard = self.config.white_guard;

        for (i, fixture) in rig.fixtures.iter().enumerate() {
            let mut values = merged.get(i).copied().unwrap_or_else(RoleValues::new);

            // The hands win.
            if let Some(held) = self.programmer.get(i) {
                for (role, value) in held.iter() {
                    values.set(role, value);
                }
            }

            // Grand master scales intensity only: scaling pan would aim every head at the floor.
            if master < 1.0 {
                for role in [
                    Role::Dimmer,
                    Role::Red,
                    Role::Green,
                    Role::Blue,
                    Role::White,
                    Role::Amber,
                    Role::Uv,
                    Role::Lime,
                    Role::Cyan,
                    Role::Magenta,
                    Role::Yellow,
                ] {
                    if let Some(v) = values.get(role) {
                        values.set(role, v * master);
                    }
                }
            }

            if blackout {
                values.zero_all();
            }

            super::guard::apply(guard, &mut values, blackout);
            self.smoother.tick(i, &mut values, dt, bypass_smoothing);
            self.values[i] = values;

            let held = self.values[i];
            fixture.emit(&|role| held.get(role), &mut self.scratch);
        }

        let mut frames = UniverseSet::new();
        for universe in rig.slot_owners.keys() {
            frames.ensure(*universe);
        }
        frames.apply_all(self.scratch.drain(..));
        self.last_frames = frames.clone();
        if let Some(driver) = self.driver.as_mut() {
            driver.submit(frames, false);
        }

        // The snapshot is a monitoring surface, not part of the output path, and rebuilding it
        // allocates a string per claimed slot. A divider keeps it at ~10Hz.
        self.snapshot_tick = self.snapshot_tick.wrapping_add(1);
        if self.snapshot_tick.is_multiple_of(SNAPSHOT_DIVIDER) {
            self.snapshot = self.build_snapshot();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::TransportConfig;
    use super::super::patch::Fixture;
    use super::*;
    use std::fs;
    use std::path::Path;

    const RGBW: &str = r#"{
      "name": "RGBW Par",
      "availableChannels": {
        "Red":   { "capability": { "type": "ColorIntensity", "color": "Red" } },
        "Green": { "capability": { "type": "ColorIntensity", "color": "Green" } },
        "Blue":  { "capability": { "type": "ColorIntensity", "color": "Blue" } },
        "White": { "capability": { "type": "ColorIntensity", "color": "White" } }
      },
      "modes": [ { "name": "4ch", "channels": ["Red","Green","Blue","White"] } ]
    }"#;

    fn profile_dir() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let p: &Path = d.path();
        fs::create_dir_all(p.join("generic")).unwrap();
        fs::write(p.join("generic/rgbw.json"), RGBW).unwrap();
        d
    }

    fn config_with(fixtures: Vec<Fixture>, enabled: bool) -> LightingConfig {
        LightingConfig {
            enabled,
            fixtures,
            transports: vec![TransportConfig::ArtNet {
                target: "127.0.0.1:0".into(),
                broadcast: false,
            }],
            ..LightingConfig::default()
        }
    }

    /// Tick far enough for the snapshot divider to refresh.
    fn tick_to_snapshot(rt: &mut LightingRuntime) {
        for _ in 0..SNAPSHOT_DIVIDER {
            rt.tick(1.0 / 60.0, true, &|_, _| None);
        }
    }

    fn par(name: &str, address: u16) -> Fixture {
        let mut f = Fixture::new(name, "generic/rgbw", "4ch");
        f.address = address;
        f
    }

    #[test]
    fn a_disabled_config_runs_inert() {
        let d = profile_dir();
        let mut rt = LightingRuntime::new(
            config_with(vec![par("a", 1)], false),
            vec![d.path().to_path_buf()],
        );
        rt.tick(1.0 / 60.0, true, &|_, _| None);
        let s = rt.snapshot();
        assert!(!s.enabled);
        assert!(s.fixtures.is_empty());
    }

    #[test]
    fn an_enabled_rig_resolves_and_reports_its_fixtures() {
        let d = profile_dir();
        let rt = LightingRuntime::new(
            config_with(vec![par("a", 1), par("b", 5)], true),
            vec![d.path().to_path_buf()],
        );
        let s = rt.snapshot();
        assert!(s.enabled, "warnings: {:?}", s.warnings);
        assert_eq!(s.fixtures.len(), 2);
        assert!(s.running, "the driver must be up");
    }

    /// An unpatchable rig must not stop the application: the operator still has video to run.
    #[test]
    fn an_overlapping_patch_runs_inert_and_explains_itself() {
        let d = profile_dir();
        let rt = LightingRuntime::new(
            config_with(vec![par("a", 1), par("b", 3)], true),
            vec![d.path().to_path_buf()],
        );
        let s = rt.snapshot();
        assert!(!s.enabled, "a fatal patch must not drive anything");
        assert!(
            s.warnings.iter().any(|w| w.severity == "error"),
            "the failure must be reported, got {:?}",
            s.warnings
        );
        assert!(!rt.errors().is_empty());
    }

    #[test]
    fn setting_a_role_reaches_the_wire() {
        let d = profile_dir();
        let fixture = par("a", 1);
        let id = fixture.id;
        let mut rt = LightingRuntime::new(
            config_with(vec![fixture], true),
            vec![d.path().to_path_buf()],
        );
        assert!(rt.set_role(id, Role::Red, 1.0));
        tick_to_snapshot(&mut rt);
        let s = rt.snapshot();
        let red = s.universes[0]
            .slots
            .iter()
            .find(|slot| slot.channel == "Red")
            .expect("red slot");
        assert_eq!(red.value, 255);
    }

    #[test]
    fn setting_an_unknown_fixture_reports_failure() {
        let d = profile_dir();
        let mut rt = LightingRuntime::new(
            config_with(vec![par("a", 1)], true),
            vec![d.path().to_path_buf()],
        );
        assert!(!rt.set_role(Uuid::new_v4(), Role::Red, 1.0));
    }

    #[test]
    fn blackout_drives_every_slot_dark() {
        let d = profile_dir();
        let fixture = par("a", 1);
        let id = fixture.id;
        let mut rt = LightingRuntime::new(
            config_with(vec![fixture], true),
            vec![d.path().to_path_buf()],
        );
        rt.set_role(id, Role::Red, 1.0);
        tick_to_snapshot(&mut rt);
        rt.set_blackout(true);
        tick_to_snapshot(&mut rt);
        let s = rt.snapshot();
        assert!(s.blackout);
        assert!(
            s.universes[0].slots.iter().all(|slot| slot.value == 0),
            "blackout must be instant: {:?}",
            s.universes[0].slots
        );
    }

    #[test]
    fn blackout_is_latching() {
        let d = profile_dir();
        let mut rt = LightingRuntime::new(
            config_with(vec![par("a", 1)], true),
            vec![d.path().to_path_buf()],
        );
        rt.set_blackout(true);
        for _ in 0..10 {
            rt.tick(1.0 / 60.0, false, &|_, _| None);
        }
        assert!(rt.blackout(), "only an explicit release may clear blackout");
    }

    /// A universe whose fixtures are all dark must keep transmitting, or receivers hold their
    /// last value until the data-loss timeout expires.
    #[test]
    fn a_fully_dark_universe_is_still_present() {
        let d = profile_dir();
        let mut rt = LightingRuntime::new(
            config_with(vec![par("a", 1)], true),
            vec![d.path().to_path_buf()],
        );
        tick_to_snapshot(&mut rt);
        let s = rt.snapshot();
        assert_eq!(s.universes.len(), 1, "the universe must still be reported");
    }

    #[test]
    fn set_config_repatches_live() {
        let d = profile_dir();
        let mut rt = LightingRuntime::new(
            config_with(vec![par("a", 1)], true),
            vec![d.path().to_path_buf()],
        );
        assert_eq!(rt.snapshot().fixtures.len(), 1);
        rt.set_config(config_with(vec![par("a", 1), par("b", 5)], true));
        assert_eq!(rt.snapshot().fixtures.len(), 2);
    }

    #[test]
    fn a_missing_profile_reports_rather_than_panicking() {
        let d = profile_dir();
        let mut f = par("a", 1);
        f.profile = "nope/missing".into();
        let rt = LightingRuntime::new(config_with(vec![f], true), vec![d.path().to_path_buf()]);
        let s = rt.snapshot();
        assert!(!s.enabled);
        assert!(s.warnings.iter().any(|w| w.text.contains("failed to load")));
    }

    /// The bug this exists to prevent: a rig that fails validation resolves to no fixtures at
    /// all, so a UI listing resolved fixtures showed "No fixtures patched" over a config holding
    /// the broken entry. The entry that needed deleting was the one that could not be seen, and
    /// the rig stayed dark permanently.
    #[test]
    fn a_broken_patch_is_still_listed_so_it_can_be_repaired() {
        let d = profile_dir();
        let mut bad = par("bad", 1);
        bad.profile = "nope/missing".into();
        let rt = LightingRuntime::new(
            config_with(vec![par("good", 10), bad], true),
            vec![d.path().to_path_buf()],
        );
        let s = rt.snapshot();

        assert!(s.fixtures.is_empty(), "one bad entry fails the whole rig");
        assert_eq!(
            s.patch.len(),
            2,
            "every configured entry must still be listed: {:?}",
            s.patch
        );
        let broken = s.patch.iter().find(|e| e.name == "bad").unwrap();
        assert!(
            broken
                .error
                .as_ref()
                .is_some_and(|e| e.contains("failed to load")),
            "the broken entry must carry its reason: {:?}",
            broken.error
        );
        assert!(
            s.patch
                .iter()
                .find(|e| e.name == "good")
                .unwrap()
                .error
                .is_none(),
            "an entry that is fine must not be blamed for its neighbour"
        );
    }

    /// And removing it recovers the rig, which is the whole point of listing it.
    #[test]
    fn removing_the_broken_entry_brings_the_rig_back() {
        let d = profile_dir();
        let mut bad = par("bad", 1);
        bad.profile = "nope/missing".into();
        let mut rt = LightingRuntime::new(
            config_with(vec![par("good", 10), bad], true),
            vec![d.path().to_path_buf()],
        );
        assert!(rt.snapshot().fixtures.is_empty());

        rt.set_config(config_with(vec![par("good", 10)], true));
        let s = rt.snapshot();
        assert_eq!(s.fixtures.len(), 1, "the rig must light again");
        assert_eq!(s.patch.len(), 1);
        assert!(s.patch[0].error.is_none());
    }

    /// A patch that resolves cleanly still reports itself, so the list is one code path rather
    /// than a healthy view plus an error view.
    #[test]
    fn a_healthy_patch_is_listed_with_no_errors() {
        let d = profile_dir();
        let rt = LightingRuntime::new(
            config_with(vec![par("a", 1), par("b", 10)], true),
            vec![d.path().to_path_buf()],
        );
        let s = rt.snapshot();
        assert_eq!(s.patch.len(), 2);
        assert!(s.patch.iter().all(|e| e.error.is_none()));
    }

    #[test]
    fn a_bad_transport_address_leaves_the_rig_resolved_but_inert() {
        let d = profile_dir();
        let mut cfg = config_with(vec![par("a", 1)], true);
        cfg.transports = vec![TransportConfig::ArtNet {
            target: "not an address".into(),
            broadcast: false,
        }];
        let rt = LightingRuntime::new(cfg, vec![d.path().to_path_buf()]);
        let s = rt.snapshot();
        assert!(s.enabled, "the patch itself is fine");
        assert!(!s.running, "but nothing can transmit");
        assert!(s.warnings.iter().any(|w| w.severity == "error"));
    }

    /// The divider is real behaviour, not an artefact: a caller reading after one tick sees the
    /// previous snapshot. Structural changes refresh immediately so that is never surprising for
    /// anything an operator does by hand.
    #[test]
    fn structural_changes_refresh_the_snapshot_immediately() {
        let d = profile_dir();
        let mut rt = LightingRuntime::new(
            config_with(vec![par("a", 1)], true),
            vec![d.path().to_path_buf()],
        );
        assert_eq!(rt.snapshot().fixtures.len(), 1, "visible without any tick");
        rt.set_blackout(true);
        assert!(rt.snapshot().blackout, "blackout is visible at once");
        rt.set_config(config_with(vec![par("a", 1), par("b", 5)], true));
        assert_eq!(
            rt.snapshot().fixtures.len(),
            2,
            "repatch is visible at once"
        );
    }

    /// The whole pipeline in one test: a look in a lighting deck, merged through a channel at
    /// full opacity, palette-resolved, smoothed, patched, and reaching the universe.
    #[test]
    fn a_look_in_a_deck_reaches_the_wire() {
        use crate::dmx::look::{AttrValue, Group, Look};
        use crate::dmx::merge::LightingDeck;
        use crate::dmx::show::LightingShow;

        let d = profile_dir();
        let fixture = par("a", 1);
        let fixture_id = fixture.id;
        let mut group = Group::new("all");
        group.members = vec![fixture_id];
        // The group is what routes: a deck names nobody, so without a source nothing is heard.
        group.source = Some(crate::dmx::GroupSource::Channel {
            uuid: "abc12345".to_string(),
        });

        let mut config = config_with(vec![fixture], true);
        config.groups = vec![group];
        let mut rt = LightingRuntime::new(config, vec![d.path().to_path_buf()]);

        let mut look = Look::new("red wash");
        look.set(Role::Red, AttrValue::literal(1.0));
        let channel = "abc12345".to_string();
        let mut show = LightingShow::default();
        show.decks_mut(&channel).push(LightingDeck::new(look));
        rt.set_show(show);
        rt.set_channels(vec![(channel.clone(), 1.0)]);

        tick_to_snapshot(&mut rt);
        let s = rt.snapshot();
        let red = s.universes[0]
            .slots
            .iter()
            .find(|slot| slot.channel == "Red")
            .expect("red slot");
        assert_eq!(red.value, 255, "the look must reach the wire");
    }

    /// Channel opacity is shared control state: crossfading a channel away takes its lighting
    /// with it, which is what makes a transition take the room with it.
    #[test]
    fn channel_opacity_fades_the_lighting_deck() {
        use crate::dmx::look::{AttrValue, Group, Look};
        use crate::dmx::merge::LightingDeck;
        use crate::dmx::show::LightingShow;

        let d = profile_dir();
        let fixture = par("a", 1);
        let fixture_id = fixture.id;
        let mut group = Group::new("all");
        group.members = vec![fixture_id];
        // The group is what routes: a deck names nobody, so without a source nothing is heard.
        group.source = Some(crate::dmx::GroupSource::Channel {
            uuid: "abc12345".to_string(),
        });
        let mut config = config_with(vec![fixture], true);
        config.groups = vec![group];
        let mut rt = LightingRuntime::new(config, vec![d.path().to_path_buf()]);

        let mut look = Look::new("red");
        look.set(Role::Red, AttrValue::literal(1.0));
        let channel = "abc12345".to_string();
        let mut show = LightingShow::default();
        show.decks_mut(&channel).push(LightingDeck::new(look));
        rt.set_show(show);

        rt.set_channels(vec![(channel.clone(), 0.0)]);
        for _ in 0..120 {
            rt.tick(1.0 / 60.0, true, &|_, _| None);
        }
        let s = rt.snapshot();
        let red = s.universes[0]
            .slots
            .iter()
            .find(|slot| slot.channel == "Red")
            .expect("red slot");
        assert_eq!(
            red.value, 0,
            "a crossfaded-away channel takes its lights with it"
        );
    }

    /// The programmer outranks the looks: whatever the performer's hands are touching wins.
    #[test]
    fn the_programmer_outranks_a_look() {
        use crate::dmx::look::{AttrValue, Group, Look};
        use crate::dmx::merge::LightingDeck;
        use crate::dmx::show::LightingShow;

        let d = profile_dir();
        let fixture = par("a", 1);
        let fixture_id = fixture.id;
        let mut group = Group::new("all");
        group.members = vec![fixture_id];
        // The group is what routes: a deck names nobody, so without a source nothing is heard.
        group.source = Some(crate::dmx::GroupSource::Channel {
            uuid: "abc12345".to_string(),
        });
        let mut config = config_with(vec![fixture], true);
        config.groups = vec![group];
        let mut rt = LightingRuntime::new(config, vec![d.path().to_path_buf()]);

        let mut look = Look::new("dim red");
        look.set(Role::Red, AttrValue::literal(0.2));
        let channel = "abc12345".to_string();
        let mut show = LightingShow::default();
        show.decks_mut(&channel).push(LightingDeck::new(look));
        rt.set_show(show);
        rt.set_channels(vec![(channel.clone(), 1.0)]);

        rt.set_role(fixture_id, Role::Red, 1.0);
        for _ in 0..120 {
            rt.tick(1.0 / 60.0, true, &|_, _| None);
        }
        let full = rt.snapshot().universes[0]
            .slots
            .iter()
            .find(|s| s.channel == "Red")
            .unwrap()
            .value;
        assert_eq!(full, 255, "the hand wins");

        // Releasing hands the role back to the look.
        rt.release_role(fixture_id, Role::Red);
        for _ in 0..120 {
            rt.tick(1.0 / 60.0, true, &|_, _| None);
        }
        let released = rt.snapshot().universes[0]
            .slots
            .iter()
            .find(|s| s.channel == "Red")
            .unwrap()
            .value;
        assert!(
            released < 100,
            "released should fall back to the look's 0.2, got {released}"
        );
    }

    /// The grand master scales intensity, never position: scaling pan would aim every head at
    /// its zero point as the master came down.
    #[test]
    fn master_scales_intensity_but_not_position() {
        let d = profile_dir();
        let fixture = par("a", 1);
        let id = fixture.id;
        let mut rt = LightingRuntime::new(
            config_with(vec![fixture], true),
            vec![d.path().to_path_buf()],
        );
        rt.set_role(id, Role::Red, 1.0);
        rt.set_master(0.5);
        for _ in 0..120 {
            rt.tick(1.0 / 60.0, true, &|_, _| None);
        }
        let red = rt.snapshot().universes[0]
            .slots
            .iter()
            .find(|s| s.channel == "Red")
            .unwrap()
            .value;
        assert!(
            (120..=136).contains(&red),
            "master should halve the intensity, got {red}"
        );
    }

    // ── Select, set, store ───────────────────────────────────────────

    #[test]
    fn the_programmer_reports_what_the_hands_are_holding() {
        let d = profile_dir();
        let a = par("a", 1);
        let b = par("b", 5);
        let (a_id, b_id) = (a.id, b.id);
        let mut rt =
            LightingRuntime::new(config_with(vec![a, b], true), vec![d.path().to_path_buf()]);

        assert!(rt.programmer_fixtures().is_empty(), "nothing held yet");
        assert!(rt.programmer_for(a_id).is_none());

        rt.set_role(a_id, Role::Red, 1.0);
        rt.set_role(a_id, Role::Blue, 0.5);
        let held = rt.programmer_fixtures();
        assert_eq!(held.len(), 1, "only the touched fixture is held");
        assert_eq!(held[0].0, a_id);
        assert_eq!(held[0].1.iter().count(), 2);
        assert!(rt.programmer_for(b_id).is_none());
    }

    #[test]
    fn releasing_empties_the_programmer() {
        let d = profile_dir();
        let fixture = par("a", 1);
        let id = fixture.id;
        let mut rt = LightingRuntime::new(
            config_with(vec![fixture], true),
            vec![d.path().to_path_buf()],
        );
        rt.set_role(id, Role::Red, 1.0);
        assert_eq!(rt.programmer_fixtures().len(), 1);
        rt.release_all();
        assert!(rt.programmer_fixtures().is_empty());
    }

    #[test]
    fn releasing_one_role_leaves_the_others_held() {
        let d = profile_dir();
        let fixture = par("a", 1);
        let id = fixture.id;
        let mut rt = LightingRuntime::new(
            config_with(vec![fixture], true),
            vec![d.path().to_path_buf()],
        );
        rt.set_role(id, Role::Red, 1.0);
        rt.set_role(id, Role::Blue, 1.0);
        assert!(rt.release_role(id, Role::Red));
        let held = rt.programmer_for(id).expect("still holding blue");
        assert_eq!(held.get(Role::Red), None);
        assert_eq!(held.get(Role::Blue), Some(1.0));
    }

    #[test]
    fn the_snapshot_reports_the_programmer_count() {
        let d = profile_dir();
        let fixture = par("a", 1);
        let id = fixture.id;
        let mut rt = LightingRuntime::new(
            config_with(vec![fixture], true),
            vec![d.path().to_path_buf()],
        );
        assert_eq!(rt.snapshot().programmer_roles, 0);
        rt.set_role(id, Role::Red, 1.0);
        rt.set_role(id, Role::Green, 1.0);
        tick_to_snapshot(&mut rt);
        assert_eq!(
            rt.snapshot().programmer_roles,
            2,
            "the UI surfaces this as a Release control rather than an invisible mode"
        );
    }

    #[test]
    fn ticking_with_no_rig_is_harmless() {
        let mut rt = LightingRuntime::new(LightingConfig::default(), Vec::new());
        rt.tick(1.0 / 60.0, true, &|_, _| None);
        assert!(!rt.snapshot().enabled);
    }
}
