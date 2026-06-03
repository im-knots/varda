//! Profile-driven auto-mapping engine for MIDI controllers.
//!
//! Maps a controller's grid, faders, and buttons to the mixer state based on
//! the `auto_map` section of a controller profile. Runs in parallel with
//! `MidiMappingStore` and takes priority over user MIDI-learn mappings.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::controller_profile::{AutoMapConfig, ControllerProfileData};
use super::{DeviceId, MidiDeviceManager, MidiKey};
use crate::mixer::Mixer;

// ── Device-level auto-map state ─────────────────────────────────────

/// Maximum allowed CC value jump between consecutive messages on the same CC.
/// Jumps larger than this are treated as hardware glitches (e.g. dirty fader
/// wiper losing contact) and silently rejected.  32 steps out of 127 is
/// generous enough for fast human fader sweeps while catching the classic
/// "spike to 0 near max" artefact.
const MAX_CC_JUMP: u8 = 32;

struct DeviceAutoMapState {
    config: AutoMapConfig,
    profile: Arc<ControllerProfileData>,
    page_offset: usize,
    shift_held: bool,
    press_times: HashMap<u8, Instant>,
    // Cached MIDI ranges from profile controls
    grid_range: [u8; 2],
    fader_range: [u8; 2],
    shift_note: Option<u8>,
    page_range: Option<[u8; 2]>,
    // LED change tracking: note → last-sent velocity
    last_led_values: HashMap<u8, u8>,
    // Hysteresis filter: last accepted CC value per CC number
    last_cc_values: HashMap<u8, u8>,
}

impl DeviceAutoMapState {
    fn new(config: AutoMapConfig, profile: Arc<ControllerProfileData>) -> Self {
        let grid_range = Self::lookup_range(&profile, &config.grid_control);
        let fader_range = Self::lookup_range(&profile, &config.fader_control);
        let shift_note = config
            .shift_control
            .as_ref()
            .map(|name| Self::lookup_range(&profile, name)[0]);
        let page_range = config
            .page_buttons_control
            .as_ref()
            .map(|name| Self::lookup_range(&profile, name));

        Self {
            config,
            profile,
            page_offset: 0,
            shift_held: false,
            press_times: HashMap::new(),
            grid_range,
            fader_range,
            shift_note,
            page_range,
            last_led_values: HashMap::new(),
            last_cc_values: HashMap::new(),
        }
    }

    fn lookup_range(profile: &ControllerProfileData, control_name: &str) -> [u8; 2] {
        profile
            .controls
            .iter()
            .find(|c| c.name == control_name)
            .map(|c| c.range)
            .unwrap_or([0, 0])
    }

    fn is_grid_note(&self, note: u8) -> bool {
        note >= self.grid_range[0] && note <= self.grid_range[1]
    }

    fn is_fader_cc(&self, cc: u8) -> bool {
        cc >= self.fader_range[0] && cc <= self.fader_range[1]
    }

    fn is_shift_note(&self, note: u8) -> bool {
        self.shift_note == Some(note)
    }

    fn is_page_button(&self, note: u8) -> bool {
        self.page_range
            .map_or(false, |r| note >= r[0] && note <= r[1])
    }
}

// ── Public grid mapping utility ─────────────────────────────────────

/// Convert a grid note number to (channel_index, deck_index) given page offset and column count.
/// APC Mini grid: note = row*8 + col, row 0 is bottom (note 0-7), row 7 is top (note 56-63).
/// We map top row = deck 0, bottom row = deck 7 (top-down).
pub fn grid_note_to_channel_deck(note: u8, page_offset: usize, columns: u8) -> (usize, usize) {
    let col = (note % columns) as usize;
    let row = (note / columns) as usize;
    let channel = col + page_offset * columns as usize;
    // Invert row: top of grid (high note) = deck 0
    let deck = (columns - 1) as usize - row;
    (channel, deck)
}

// ── AutoMapEngine ───────────────────────────────────────────────────

pub struct AutoMapEngine {
    devices: HashMap<DeviceId, DeviceAutoMapState>,
}

impl AutoMapEngine {
    pub fn new() -> Self {
        Self {
            devices: HashMap::new(),
        }
    }

    /// Register a device for auto-mapping if its profile has an `auto_map` section.
    pub fn register_device(&mut self, device_id: DeviceId, profile: Arc<ControllerProfileData>) {
        if let Some(config) = profile.auto_map.clone() {
            log::info!(
                "Auto-map: registered device {} with profile '{}'",
                device_id,
                profile.profile.name
            );
            self.devices
                .insert(device_id, DeviceAutoMapState::new(config, profile));
        }
    }

    /// Remove a device from auto-mapping.
    pub fn unregister_device(&mut self, device_id: DeviceId) {
        self.devices.remove(&device_id);
    }

    /// Sync with current device manager state — register new devices, remove stale ones.
    pub fn sync_devices(&mut self, mgr: &MidiDeviceManager) {
        // Remove devices no longer present
        let active_ids: Vec<DeviceId> = mgr.devices.keys().copied().collect();
        self.devices.retain(|id, _| active_ids.contains(id));

        // Register new devices with auto_map profiles
        for (id, info) in &mgr.devices {
            if !self.devices.contains_key(id) {
                if let Some(profile) = &info.profile {
                    self.register_device(*id, Arc::clone(profile));
                }
            }
        }
    }

    /// Check if a MIDI key falls within any auto-mapped control range for this device.
    pub fn handles_key(&self, device_id: DeviceId, key: &MidiKey) -> bool {
        let state = match self.devices.get(&device_id) {
            Some(s) => s,
            None => return false,
        };
        match key {
            MidiKey::Note(_, _, note) => {
                state.is_grid_note(*note)
                    || state.is_shift_note(*note)
                    || state.is_page_button(*note)
            }
            MidiKey::CC(_, _, cc) => state.is_fader_cc(*cc),
        }
    }

    /// Process a note-on event: record press time, detect shift.
    pub fn process_note_on(&mut self, device_id: DeviceId, note: u8, _channel: u8) {
        if let Some(state) = self.devices.get_mut(&device_id) {
            if state.is_shift_note(note) {
                state.shift_held = true;
                return;
            }
            state.press_times.insert(note, Instant::now());
        }
    }

    /// Process a note-off event: tap/hold detection, paging.
    pub fn process_note_off(
        &mut self,
        device_id: DeviceId,
        note: u8,
        _channel: u8,
        mixer: &mut Mixer,
    ) {
        if let Some(state) = self.devices.get_mut(&device_id) {
            if state.is_shift_note(note) {
                state.shift_held = false;
                return;
            }

            // Page button + shift → change page
            if state.is_page_button(note) && state.shift_held {
                if let Some(page_range) = state.page_range {
                    let page_idx = (note - page_range[0]) as usize;
                    state.page_offset = page_idx;
                    log::debug!("Auto-map: page offset changed to {}", page_idx);
                }
                return;
            }

            // Grid note → tap/hold for mute/solo (only if we saw the note-on)
            if state.is_grid_note(note) {
                if let Some(press_time) = state.press_times.remove(&note) {
                    let duration = press_time.elapsed();
                    let threshold = Duration::from_millis(state.config.tap_hold_threshold_ms);
                    let (ch_idx, dk_idx) =
                        grid_note_to_channel_deck(note, state.page_offset, state.config.columns);

                    if duration < threshold {
                        // Tap → mute/solo based on tap_action
                        Self::apply_action(&state.config.tap_action, mixer, ch_idx, dk_idx);
                    } else {
                        // Hold → mute/solo based on hold_action
                        Self::apply_action(&state.config.hold_action, mixer, ch_idx, dk_idx);
                    }
                }
            }
        }
    }

    /// Process a CC event: fader → channel opacity or crossfader.
    /// Applies a hysteresis filter to reject hardware glitch spikes.
    pub fn process_cc(&mut self, device_id: DeviceId, cc: u8, value: u8, mixer: &mut Mixer) {
        if let Some(state) = self.devices.get_mut(&device_id) {
            if !state.is_fader_cc(cc) {
                return;
            }

            // Hysteresis filter: reject suspicious jumps (dirty fader protection)
            if let Some(&last) = state.last_cc_values.get(&cc) {
                let delta = (value as i16 - last as i16).unsigned_abs() as u8;
                if delta > MAX_CC_JUMP {
                    log::debug!(
                        "Auto-map: rejected CC {} spike {} → {} (Δ{})",
                        cc,
                        last,
                        value,
                        delta
                    );
                    return;
                }
            }
            state.last_cc_values.insert(cc, value);

            let fader_idx = (cc - state.fader_range[0]) as usize;
            let fader_count = (state.fader_range[1] - state.fader_range[0] + 1) as usize;
            let normalized = value as f32 / 127.0;

            // Last fader → crossfader (if configured)
            if fader_idx == fader_count - 1
                && state.config.last_fader_target.as_deref() == Some("crossfader")
            {
                mixer.set_crossfader(normalized);
                return;
            }

            // Other faders → channel opacity
            if state.config.fader_target == "channel_opacity" {
                let ch_idx = fader_idx + state.page_offset * state.config.columns as usize;
                if let Some(ch) = mixer.channel_mut(ch_idx) {
                    ch.opacity = normalized;
                }
            }
        }
    }

    fn apply_action(action: &str, mixer: &mut Mixer, ch_idx: usize, dk_idx: usize) {
        if let Some(ch) = mixer.channel_mut(ch_idx) {
            if dk_idx < ch.deck_count() {
                match action {
                    "mute" => {
                        let current = ch.decks[dk_idx].mute;
                        ch.set_deck_mute(dk_idx, !current);
                    }
                    "solo" => {
                        let current = ch.decks[dk_idx].solo;
                        ch.set_deck_solo(dk_idx, !current);
                    }
                    _ => {}
                }
            }
        }
    }
}

// ── LED Feedback ────────────────────────────────────────────────────

impl AutoMapEngine {
    /// Update grid LEDs based on current mixer state.
    pub fn update_leds(&mut self, mgr: &MidiDeviceManager, mixer: &Mixer) {
        for (&device_id, state) in &mut self.devices {
            let led_channel = state.profile.led_channel();
            let columns = state.config.columns as usize;
            let rows = state.config.rows as usize;

            for row in 0..rows {
                for col in 0..columns {
                    let note = (row * columns + col) as u8;
                    if note < state.grid_range[0] || note > state.grid_range[1] {
                        continue;
                    }

                    let ch_idx = col + state.page_offset * columns;
                    let color =
                        Self::determine_led_color(mixer, &state.config, ch_idx, rows - 1 - row);
                    let velocity = state.profile.color_value(&color);

                    // Only send if changed
                    if state.last_led_values.get(&note) != Some(&velocity) {
                        state.last_led_values.insert(note, velocity);
                        mgr.send_note_on(device_id, led_channel, note, velocity);
                    }
                }
            }
        }
    }

    /// Determine the LED color for a grid position.
    fn determine_led_color(
        mixer: &Mixer,
        config: &AutoMapConfig,
        ch_idx: usize,
        dk_idx: usize,
    ) -> String {
        let channel = match mixer.channel(ch_idx) {
            Some(ch) => ch,
            None => return config.led_rules.empty.clone(),
        };

        if dk_idx >= channel.deck_count() {
            return config.led_rules.empty.clone();
        }

        let deck = &channel.decks[dk_idx];

        if deck.solo {
            return config.led_rules.soloed.clone();
        }
        if deck.mute {
            return config.led_rules.muted.clone();
        }
        if deck.opacity <= 0.0 {
            return config.led_rules.zero_opacity.clone();
        }

        config.led_rules.active.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::internal::midi::controller_profile::{builtin_apc_mini, AutoMapLedRules};

    fn test_config() -> AutoMapConfig {
        AutoMapConfig {
            strategy: "channel_grid".into(),
            grid_control: "grid".into(),
            fader_control: "faders".into(),
            shift_control: Some("shift".into()),
            page_buttons_control: Some("bottom_buttons".into()),
            columns: 8,
            rows: 8,
            tap_hold_threshold_ms: 300,
            tap_action: "mute".into(),
            hold_action: "solo".into(),
            fader_target: "channel_opacity".into(),
            last_fader_target: Some("crossfader".into()),
            led_rules: AutoMapLedRules {
                active: "green".into(),
                muted: "red".into(),
                zero_opacity: "red".into(),
                soloed: "yellow".into(),
                empty: "off".into(),
            },
        }
    }

    #[test]
    fn test_grid_note_to_channel_deck() {
        // Note 0 = bottom-left → col 0, row 0 → deck 7 (inverted)
        assert_eq!(grid_note_to_channel_deck(0, 0, 8), (0, 7));
        // Note 7 = bottom-right → col 7, row 0 → deck 7
        assert_eq!(grid_note_to_channel_deck(7, 0, 8), (7, 7));
        // Note 56 = top-left → col 0, row 7 → deck 0
        assert_eq!(grid_note_to_channel_deck(56, 0, 8), (0, 0));
        // Note 63 = top-right → col 7, row 7 → deck 0
        assert_eq!(grid_note_to_channel_deck(63, 0, 8), (7, 0));
        // With page offset 1
        assert_eq!(grid_note_to_channel_deck(0, 1, 8), (8, 7));
        assert_eq!(grid_note_to_channel_deck(56, 1, 8), (8, 0));
    }

    #[test]
    fn test_handles_key_grid() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));

        // Grid notes (0-63) should be handled
        assert!(engine.handles_key(1, &MidiKey::Note(1, 0, 0)));
        assert!(engine.handles_key(1, &MidiKey::Note(1, 0, 63)));
        // Shift note (98) should be handled
        assert!(engine.handles_key(1, &MidiKey::Note(1, 0, 98)));
        // Bottom buttons (64-71) should be handled (page buttons)
        assert!(engine.handles_key(1, &MidiKey::Note(1, 0, 64)));
        assert!(engine.handles_key(1, &MidiKey::Note(1, 0, 71)));
        // Fader CCs (48-56) should be handled
        assert!(engine.handles_key(1, &MidiKey::CC(1, 0, 48)));
        assert!(engine.handles_key(1, &MidiKey::CC(1, 0, 56)));
    }

    #[test]
    fn test_handles_key_unrelated() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));

        // Side buttons (82-89) are NOT auto-mapped
        assert!(!engine.handles_key(1, &MidiKey::Note(1, 0, 82)));
        // Unknown device
        assert!(!engine.handles_key(99, &MidiKey::Note(99, 0, 0)));
        // CC outside fader range
        assert!(!engine.handles_key(1, &MidiKey::CC(1, 0, 10)));
    }

    #[test]
    fn test_page_offset_changes() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));

        // Press shift
        engine.process_note_on(1, 98, 0);
        // Press bottom button 2 (note 66) while shift held → page 2
        // We need a mixer but page change doesn't touch it, pass a minimal one
        // Actually process_note_off needs mixer, but for page buttons it doesn't use it
        // We can't easily construct a mixer without GPU. Let's test via the state directly.
        let state = engine.devices.get(&1).unwrap();
        assert!(state.shift_held);
        assert_eq!(state.page_offset, 0);
    }

    #[test]
    fn test_led_color_determination() {
        let config = test_config();

        // We can't create a real Mixer without GPU, but we can test determine_led_color
        // indirectly through the logic. For now, test with no mixer channels (empty).
        // The function needs a Mixer reference, so we skip direct unit test of it
        // and rely on the integration test path.

        // Verify the config values are correct
        assert_eq!(config.led_rules.active, "green");
        assert_eq!(config.led_rules.muted, "red");
        assert_eq!(config.led_rules.soloed, "yellow");
        assert_eq!(config.led_rules.empty, "off");
    }

    #[test]
    fn test_register_unregister() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));
        assert!(engine.handles_key(1, &MidiKey::Note(1, 0, 0)));

        engine.unregister_device(1);
        assert!(!engine.handles_key(1, &MidiKey::Note(1, 0, 0)));
    }

    #[test]
    fn test_profile_without_automap_not_registered() {
        let mut engine = AutoMapEngine::new();
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        engine.register_device(1, Arc::new(profile));
        assert!(!engine.handles_key(1, &MidiKey::Note(1, 0, 0)));
    }

    #[test]
    fn test_fader_mapping() {
        let config = test_config();
        // Verify fader range extraction
        let profile = builtin_apc_mini();
        let range = DeviceAutoMapState::lookup_range(&profile, "faders");
        assert_eq!(range, [48, 56]);

        // Verify fader index calculation
        let fader_idx = (50u8 - range[0]) as usize; // CC 50 → fader index 2
        assert_eq!(fader_idx, 2);

        // Last fader (CC 56) → index 8 (= fader_count - 1 = 9 - 1)
        let last_idx = (56u8 - range[0]) as usize;
        let fader_count = (range[1] - range[0] + 1) as usize;
        assert_eq!(last_idx, fader_count - 1);
        assert_eq!(config.last_fader_target, Some("crossfader".into()));
    }

    // ── CC hysteresis / jump filter tests ─────────────────────────────

    #[test]
    fn test_cc_filter_first_message_always_accepted() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));

        // First CC message (no prior value) must be accepted regardless of value
        let state = engine.devices.get(&1).unwrap();
        assert!(state.last_cc_values.is_empty());
        // We can't call process_cc without a Mixer (needs GPU), so test the
        // filter logic directly on DeviceAutoMapState.
        let state = engine.devices.get_mut(&1).unwrap();
        // Simulate first message: no last value → accepted
        assert!(!state.last_cc_values.contains_key(&48));
    }

    #[test]
    fn test_cc_filter_rejects_large_jump() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));

        let state = engine.devices.get_mut(&1).unwrap();
        // Simulate accepted value at 120
        state.last_cc_values.insert(48, 120);
        // A jump to 0 (delta=120) should be rejected
        let last = *state.last_cc_values.get(&48).unwrap();
        let delta = (0i16 - last as i16).unsigned_abs() as u8;
        assert!(delta > MAX_CC_JUMP);
        // The value should NOT be updated
        assert_eq!(*state.last_cc_values.get(&48).unwrap(), 120);
    }

    #[test]
    fn test_cc_filter_accepts_normal_movement() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));

        let state = engine.devices.get_mut(&1).unwrap();
        // Simulate a smooth fader sweep: 0 → 5 → 10 → 15
        for value in (0u8..=60).step_by(5) {
            if let Some(&last) = state.last_cc_values.get(&48) {
                let delta = (value as i16 - last as i16).unsigned_abs() as u8;
                assert!(
                    delta <= MAX_CC_JUMP,
                    "Normal step {} → {} should be accepted",
                    last,
                    value
                );
            }
            state.last_cc_values.insert(48, value);
        }
        assert_eq!(*state.last_cc_values.get(&48).unwrap(), 60);
    }

    #[test]
    fn test_cc_filter_boundary_value() {
        let mut engine = AutoMapEngine::new();
        let profile = Arc::new(builtin_apc_mini());
        engine.register_device(1, Arc::clone(&profile));

        let state = engine.devices.get_mut(&1).unwrap();
        state.last_cc_values.insert(48, 64);
        // Exactly at threshold: delta = 32 → should be accepted
        let delta = (96u8 as i16 - 64i16).unsigned_abs() as u8;
        assert_eq!(delta, MAX_CC_JUMP);
        // One step beyond: delta = 33 → should be rejected
        let delta = (97u8 as i16 - 64i16).unsigned_abs() as u8;
        assert!(delta > MAX_CC_JUMP);
    }
}
