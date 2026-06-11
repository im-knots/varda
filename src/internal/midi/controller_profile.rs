//! Data-driven controller profiles for MIDI LED feedback.
//!
//! Profiles describe a controller's physical layout and LED protocol so the
//! LED feedback system works with any hardware. Profiles are loaded from JSON
//! files in `.varda/controller-profiles/` or compiled-in as built-in defaults.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use super::{DeviceId, MidiDeviceManager, MidiKey, MidiMappingStore};
use crate::mixer::Mixer;

// ── Profile Data Model ────────────────────────────────────────────

/// A controller profile loaded from JSON or compiled-in.
#[derive(Debug, Clone, Deserialize)]
pub struct ControllerProfileData {
    pub profile: ProfileMeta,
    pub leds: Option<LedConfig>,
    #[serde(default)]
    pub controls: Vec<ControlDef>,
    #[serde(default)]
    pub auto_map: Option<AutoMapConfig>,
}

/// Profile identity and detection.
#[derive(Debug, Clone, Deserialize)]
pub struct ProfileMeta {
    pub name: String,
    /// Case-insensitive substring match against MIDI device name.
    pub name_match: String,
}

/// LED feedback protocol configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct LedConfig {
    /// How LEDs are addressed: "note_velocity", "cc_value"
    #[serde(default = "default_led_method")]
    pub method: String,
    #[serde(default)]
    pub channel: u8,
    /// Logical color name → MIDI value (velocity or CC value).
    #[serde(default)]
    pub colors: HashMap<String, u8>,
}

fn default_led_method() -> String {
    "note_velocity".to_string()
}
fn default_tap_hold_threshold() -> u64 {
    300
}

/// Auto-mapping configuration: drives grid/fader/button behavior from profile JSON.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoMapConfig {
    pub strategy: String,
    pub grid_control: String,
    pub fader_control: String,
    pub shift_control: Option<String>,
    pub page_buttons_control: Option<String>,
    pub columns: u8,
    pub rows: u8,
    #[serde(default = "default_tap_hold_threshold")]
    pub tap_hold_threshold_ms: u64,
    pub tap_action: String,
    pub hold_action: String,
    pub fader_target: String,
    pub last_fader_target: Option<String>,
    pub led_rules: AutoMapLedRules,
}

/// LED color rules for auto-mapped grid positions.
#[derive(Debug, Clone, Deserialize)]
pub struct AutoMapLedRules {
    pub active: String,
    pub muted: String,
    pub zero_opacity: String,
    pub soloed: String,
    pub empty: String,
}

/// A physical control on the device.
#[derive(Debug, Clone, Deserialize)]
pub struct ControlDef {
    pub name: String,
    /// "button", "fader", "encoder"
    #[serde(rename = "type")]
    pub control_type: String,
    /// "note" or "cc"
    pub midi_type: String,
    #[serde(default)]
    pub channel: u8,
    /// [min, max] inclusive range of note/CC numbers.
    pub range: [u8; 2],
    #[serde(default)]
    pub has_led: bool,
}

const VALID_LED_METHODS: &[&str] = &["note_velocity", "cc_value"];
const VALID_CONTROL_TYPES: &[&str] = &["button", "fader", "encoder"];
const VALID_MIDI_TYPES: &[&str] = &["note", "cc"];

impl ControllerProfileData {
    /// Validate the profile for semantic correctness. Returns a list of errors.
    /// An empty list means the profile is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();

        if self.profile.name.trim().is_empty() {
            errors.push("profile.name is empty".into());
        }
        if self.profile.name_match.trim().is_empty() {
            errors.push("profile.name_match is empty".into());
        }

        if let Some(leds) = &self.leds {
            if !VALID_LED_METHODS.contains(&leds.method.as_str()) {
                errors.push(format!(
                    "leds.method '{}' is invalid (expected one of: {})",
                    leds.method,
                    VALID_LED_METHODS.join(", ")
                ));
            }
            if leds.channel > 15 {
                errors.push(format!(
                    "leds.channel {} exceeds MIDI range 0-15",
                    leds.channel
                ));
            }
        }

        if self.controls.is_empty() {
            errors.push("controls array is empty — profile defines no controls".into());
        }

        for (i, ctrl) in self.controls.iter().enumerate() {
            let prefix = format!("controls[{}] '{}'", i, ctrl.name);
            if ctrl.name.trim().is_empty() {
                errors.push(format!("{}: name is empty", prefix));
            }
            if !VALID_CONTROL_TYPES.contains(&ctrl.control_type.as_str()) {
                errors.push(format!(
                    "{}: type '{}' is invalid (expected one of: {})",
                    prefix,
                    ctrl.control_type,
                    VALID_CONTROL_TYPES.join(", ")
                ));
            }
            if !VALID_MIDI_TYPES.contains(&ctrl.midi_type.as_str()) {
                errors.push(format!(
                    "{}: midi_type '{}' is invalid (expected one of: {})",
                    prefix,
                    ctrl.midi_type,
                    VALID_MIDI_TYPES.join(", ")
                ));
            }
            if ctrl.channel > 15 {
                errors.push(format!(
                    "{}: channel {} exceeds MIDI range 0-15",
                    prefix, ctrl.channel
                ));
            }
            if ctrl.range[0] > ctrl.range[1] {
                errors.push(format!(
                    "{}: range [{}, {}] is inverted (min > max)",
                    prefix, ctrl.range[0], ctrl.range[1]
                ));
            }
            if ctrl.range[1] > 127 {
                errors.push(format!(
                    "{}: range max {} exceeds MIDI range 0-127",
                    prefix, ctrl.range[1]
                ));
            }
        }

        // Validate auto_map if present
        if let Some(am) = &self.auto_map {
            const VALID_STRATEGIES: &[&str] = &["channel_grid"];
            const VALID_ACTIONS: &[&str] = &["mute", "solo"];

            if !VALID_STRATEGIES.contains(&am.strategy.as_str()) {
                errors.push(format!(
                    "auto_map.strategy '{}' is invalid (expected one of: {})",
                    am.strategy,
                    VALID_STRATEGIES.join(", ")
                ));
            }

            let control_names: Vec<&str> = self.controls.iter().map(|c| c.name.as_str()).collect();

            if !control_names.contains(&am.grid_control.as_str()) {
                errors.push(format!(
                    "auto_map.grid_control '{}' does not match any control name",
                    am.grid_control
                ));
            }
            if !control_names.contains(&am.fader_control.as_str()) {
                errors.push(format!(
                    "auto_map.fader_control '{}' does not match any control name",
                    am.fader_control
                ));
            }
            if let Some(ref sc) = am.shift_control {
                if !control_names.contains(&sc.as_str()) {
                    errors.push(format!(
                        "auto_map.shift_control '{}' does not match any control name",
                        sc
                    ));
                }
            }
            if let Some(ref pb) = am.page_buttons_control {
                if !control_names.contains(&pb.as_str()) {
                    errors.push(format!(
                        "auto_map.page_buttons_control '{}' does not match any control name",
                        pb
                    ));
                }
            }
            if !VALID_ACTIONS.contains(&am.tap_action.as_str()) {
                errors.push(format!(
                    "auto_map.tap_action '{}' is invalid (expected one of: {})",
                    am.tap_action,
                    VALID_ACTIONS.join(", ")
                ));
            }
            if !VALID_ACTIONS.contains(&am.hold_action.as_str()) {
                errors.push(format!(
                    "auto_map.hold_action '{}' is invalid (expected one of: {})",
                    am.hold_action,
                    VALID_ACTIONS.join(", ")
                ));
            }
            if am.columns == 0 {
                errors.push("auto_map.columns must be > 0".into());
            }
            if am.rows == 0 {
                errors.push("auto_map.rows must be > 0".into());
            }
        }

        errors
    }

    /// Check if a MIDI device name matches this profile.
    pub fn matches(&self, device_name: &str) -> bool {
        device_name
            .to_lowercase()
            .contains(&self.profile.name_match.to_lowercase())
    }

    /// Get the MIDI value for a logical color name. Returns 0 (off) if not found.
    pub fn color_value(&self, color: &str) -> u8 {
        self.leds
            .as_ref()
            .and_then(|l| l.colors.get(color))
            .copied()
            .unwrap_or(0)
    }

    /// Check if a given control number has an LED on this controller.
    /// `midi_type` should be "note" or "cc".
    pub fn control_has_led(&self, midi_type: &str, channel: u8, number: u8) -> bool {
        self.controls.iter().any(|c| {
            c.has_led
                && c.midi_type == midi_type
                && c.channel == channel
                && number >= c.range[0]
                && number <= c.range[1]
        })
    }

    /// Get the LED send channel (from leds config, default 0).
    pub fn led_channel(&self) -> u8 {
        self.leds.as_ref().map(|l| l.channel).unwrap_or(0)
    }

    /// Get the LED method (default "note_velocity").
    pub fn led_method(&self) -> &str {
        self.leds
            .as_ref()
            .map(|l| l.method.as_str())
            .unwrap_or("note_velocity")
    }
}

// ── Built-in APC Mini Profile ─────────────────────────────────────

const APC_MINI_PROFILE_JSON: &str = include_str!("apc_mini_profile.json");

/// Load the compiled-in APC Mini mk1 profile.
pub fn builtin_apc_mini() -> ControllerProfileData {
    let profile: ControllerProfileData = serde_json::from_str(APC_MINI_PROFILE_JSON)
        .expect("Built-in APC Mini profile JSON is invalid");
    let errors = profile.validate();
    assert!(
        errors.is_empty(),
        "Built-in APC Mini profile validation failed: {:?}",
        errors
    );
    profile
}

// ── Profile Registry ──────────────────────────────────────────────

/// Holds all loaded controller profiles (built-in + user-supplied).
pub struct ProfileRegistry {
    profiles: Vec<Arc<ControllerProfileData>>,
}

impl Default for ProfileRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl ProfileRegistry {
    /// Create a registry with only the built-in profiles.
    pub fn new() -> Self {
        let profiles = vec![Arc::new(builtin_apc_mini())];
        Self { profiles }
    }

    /// Load user profiles from a directory, adding them before built-ins
    /// (so user profiles take precedence in matching).
    pub fn load_user_profiles(&mut self, dir: &Path) {
        if !dir.is_dir() {
            return;
        }
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                log::warn!(
                    "Failed to read controller-profiles directory {}: {}",
                    dir.display(),
                    e
                );
                return;
            }
        };

        // Remove previous user profiles (keep only built-ins at the end).
        // Built-ins are always loaded in new(), user profiles are prepended.
        let builtin_count = 1; // just APC Mini for now
        let user_start = self.profiles.len().saturating_sub(builtin_count);
        self.profiles.drain(..user_start);

        let mut user_profiles = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => match serde_json::from_str::<ControllerProfileData>(&content) {
                    Ok(profile) => {
                        let errors = profile.validate();
                        if errors.is_empty() {
                            log::info!(
                                "Loaded controller profile '{}' from {}",
                                profile.profile.name,
                                path.display()
                            );
                            user_profiles.push(Arc::new(profile));
                        } else {
                            log::warn!(
                                "Controller profile {} has validation errors, skipping:",
                                path.display()
                            );
                            for err in &errors {
                                log::warn!("  - {}", err);
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to parse controller profile {}: {}",
                            path.display(),
                            e
                        );
                    }
                },
                Err(e) => {
                    log::warn!(
                        "Failed to read controller profile {}: {}",
                        path.display(),
                        e
                    );
                }
            }
        }

        // User profiles go first so they override built-ins with same name_match
        user_profiles.append(&mut self.profiles);
        self.profiles = user_profiles;
    }

    /// Find a profile matching a MIDI device name. First match wins.
    pub fn detect(&self, device_name: &str) -> Option<Arc<ControllerProfileData>> {
        self.profiles
            .iter()
            .find(|p| p.matches(device_name))
            .cloned()
    }

    /// Get all loaded profile names (for debugging/UI).
    pub fn profile_names(&self) -> Vec<&str> {
        self.profiles
            .iter()
            .map(|p| p.profile.name.as_str())
            .collect()
    }
}

// ── Generic LED Manager ───────────────────────────────────────────

/// Per-device LED state tracker.
struct DeviceLeds {
    profile: Arc<ControllerProfileData>,
    /// Last-sent LED value for each note/CC.
    leds: HashMap<u8, u8>,
}

impl DeviceLeds {
    fn new(profile: Arc<ControllerProfileData>) -> Self {
        Self {
            profile,
            leds: HashMap::new(),
        }
    }

    /// Set an LED, only sending if the value changed.
    fn set_led(
        &mut self,
        mgr: &MidiDeviceManager,
        device_id: DeviceId,
        note: u8,
        color: &str,
    ) -> bool {
        let val = self.profile.color_value(color);
        if self.leds.get(&note) == Some(&val) {
            return false;
        }
        self.leds.insert(note, val);

        match self.profile.led_method() {
            "note_velocity" => {
                mgr.send_note_on(device_id, self.profile.led_channel(), note, val);
            }
            "cc_value" => {
                mgr.send_raw(
                    device_id,
                    &[0xB0 | (self.profile.led_channel() & 0x0F), note, val],
                );
            }
            other => {
                log::warn!(
                    "Unknown LED method '{}' in profile '{}'",
                    other,
                    self.profile.profile.name
                );
            }
        }
        true
    }

    /// Turn all LEDs off on this device.
    fn all_off(&mut self, mgr: &MidiDeviceManager, device_id: DeviceId) {
        // Collect ranges first to avoid borrow conflict with self.set_led
        let led_ranges: Vec<(u8, u8)> = self
            .profile
            .controls
            .iter()
            .filter(|c| c.has_led)
            .map(|c| (c.range[0], c.range[1]))
            .collect();
        for (min, max) in led_ranges {
            for num in min..=max {
                self.set_led(mgr, device_id, num, "off");
            }
        }
    }
}

/// Manages LED state for all connected devices that have a controller profile.
pub struct ControllerLedManager {
    device_leds: HashMap<DeviceId, DeviceLeds>,
}

impl Default for ControllerLedManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ControllerLedManager {
    pub fn new() -> Self {
        Self {
            device_leds: HashMap::new(),
        }
    }

    /// Sync tracked devices with the device manager. Call after rescan.
    pub fn sync_devices(&mut self, mgr: &MidiDeviceManager) {
        // Add new devices that have profiles
        for (id, info) in &mgr.devices {
            if let Some(profile) = &info.profile {
                self.device_leds.entry(*id).or_insert_with(|| {
                    log::info!(
                        "LED tracking started for '{}' device [{}]",
                        profile.profile.name,
                        id
                    );
                    DeviceLeds::new(Arc::clone(profile))
                });
            }
        }

        // Remove stale devices
        let active_ids: Vec<DeviceId> = mgr.devices.keys().copied().collect();
        self.device_leds.retain(|id, _| active_ids.contains(id));
    }

    /// How many devices are being tracked for LED feedback.
    pub fn device_count(&self) -> usize {
        self.device_leds.len()
    }

    /// Turn all LEDs off on all tracked devices.
    pub fn all_off(&mut self, mgr: &MidiDeviceManager) {
        for (device_id, leds) in &mut self.device_leds {
            leds.all_off(mgr, *device_id);
        }
    }

    /// Update LEDs on all tracked devices based on current mixer state.
    pub fn update_leds(
        &mut self,
        mgr: &MidiDeviceManager,
        mappings: &MidiMappingStore,
        mixer: &Mixer,
        midi_learn_active: bool,
        midi_learn_target: Option<&str>,
    ) {
        for (&device_id, leds) in &mut self.device_leds {
            for (key, path) in &mappings.mappings {
                let (dev, midi_type, ch, number) = match key {
                    MidiKey::Note(dev, ch, note) => (*dev, "note", *ch, *note),
                    MidiKey::CC(dev, ch, cc) => (*dev, "cc", *ch, *cc),
                };
                if dev != device_id {
                    continue;
                }
                if !leds.profile.control_has_led(midi_type, ch, number) {
                    continue;
                }

                // MIDI learn target blinks red
                if midi_learn_active {
                    if let Some(target) = midi_learn_target {
                        if path == target {
                            leds.set_led(mgr, device_id, number, "red_blink");
                            continue;
                        }
                    }
                }

                let value = read_param_value(mixer, path);
                let color = param_value_to_color(path, value);
                leds.set_led(mgr, device_id, number, color);
            }
        }
    }
}

// ── Parameter → Color Logic ───────────────────────────────────────
// This is parameter semantics, not device knowledge. It maps parameter
// state to logical color names that profiles resolve to MIDI values.

/// Read a parameter value from the mixer. Returns 0.0–1.0 or -1.0 if not found.
fn read_param_value(mixer: &Mixer, path: &str) -> f32 {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["crossfader"] => mixer.crossfader(),
        ["ch", ch_s, "opacity"] => ch_s
            .parse::<usize>()
            .ok()
            .and_then(|ch| mixer.channel(ch))
            .map(|c| c.opacity)
            .unwrap_or(-1.0),
        ["ch", ch_s, "deck", dk_s, "opacity"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer
                    .channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| d.opacity)
                    .unwrap_or(-1.0)
            } else {
                -1.0
            }
        }
        ["ch", ch_s, "deck", dk_s, "mute"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer
                    .channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| if d.mute { 1.0 } else { 0.0 })
                    .unwrap_or(-1.0)
            } else {
                -1.0
            }
        }
        ["ch", ch_s, "deck", dk_s, "solo"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer
                    .channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| if d.solo { 1.0 } else { 0.0 })
                    .unwrap_or(-1.0)
            } else {
                -1.0
            }
        }
        ["ch", ch_s, "deck", dk_s, "trigger"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer
                    .channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| d.opacity)
                    .unwrap_or(-1.0)
            } else {
                -1.0
            }
        }
        ["ch", ch_s, "effect", ek_s, "param", name] => {
            if let (Ok(ch), Ok(ek)) = (ch_s.parse::<usize>(), ek_s.parse::<usize>()) {
                mixer
                    .channel(ch)
                    .and_then(|c| c.effects.get(ek))
                    .and_then(|e| e.params.get_float(name))
                    .unwrap_or(-1.0)
            } else {
                -1.0
            }
        }
        _ => -1.0,
    }
}

/// Determine logical LED color from parameter path and current value.
fn param_value_to_color(path: &str, value: f32) -> &'static str {
    if value < 0.0 {
        return "green";
    }

    if path.ends_with("/trigger") {
        return if value > 0.01 { "green" } else { "yellow" };
    }

    if path.ends_with("/mute") || path.ends_with("/solo") {
        return if value > 0.5 { "red" } else { "off" };
    }

    if value < 0.01 {
        "off"
    } else if value > 0.99 {
        "green"
    } else {
        "yellow"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_builtin_apc_mini_loads() {
        let profile = builtin_apc_mini();
        assert_eq!(profile.profile.name, "Akai APC Mini mk1");
        assert_eq!(profile.profile.name_match, "apc mini");
        assert!(profile.leds.is_some());
        let leds = profile.leds.as_ref().unwrap();
        assert_eq!(leds.method, "note_velocity");
        assert_eq!(leds.channel, 0);
        assert_eq!(*leds.colors.get("green").unwrap(), 1);
        assert_eq!(*leds.colors.get("red_blink").unwrap(), 4);
        assert_eq!(profile.controls.len(), 5);
        // Auto-map is present
        assert!(profile.auto_map.is_some());
        let am = profile.auto_map.as_ref().unwrap();
        assert_eq!(am.strategy, "channel_grid");
        assert_eq!(am.grid_control, "grid");
        assert_eq!(am.fader_control, "faders");
        assert_eq!(am.columns, 8);
        assert_eq!(am.rows, 8);
        assert_eq!(am.tap_hold_threshold_ms, 300);
        assert_eq!(am.tap_action, "mute");
        assert_eq!(am.hold_action, "solo");
        assert_eq!(am.led_rules.active, "green");
        assert_eq!(am.led_rules.soloed, "yellow");
    }

    #[test]
    fn test_profile_matches() {
        let profile = builtin_apc_mini();
        assert!(profile.matches("APC MINI"));
        assert!(profile.matches("Apc Mini mk2"));
        assert!(!profile.matches("Novation Launchpad"));
    }

    #[test]
    fn test_control_has_led() {
        let profile = builtin_apc_mini();
        assert!(profile.control_has_led("note", 0, 0)); // grid min
        assert!(profile.control_has_led("note", 0, 63)); // grid max
        assert!(profile.control_has_led("note", 0, 64)); // bottom min
        assert!(profile.control_has_led("note", 0, 71)); // bottom max
        assert!(profile.control_has_led("note", 0, 82)); // side min
        assert!(profile.control_has_led("note", 0, 89)); // side max
        assert!(profile.control_has_led("note", 0, 98)); // shift
        assert!(!profile.control_has_led("note", 0, 72)); // gap
        assert!(!profile.control_has_led("note", 0, 81)); // gap
        assert!(!profile.control_has_led("note", 0, 99)); // beyond shift
        assert!(!profile.control_has_led("cc", 0, 48)); // faders have no LED
    }

    #[test]
    fn test_color_value() {
        let profile = builtin_apc_mini();
        assert_eq!(profile.color_value("off"), 0);
        assert_eq!(profile.color_value("green"), 1);
        assert_eq!(profile.color_value("red"), 3);
        assert_eq!(profile.color_value("yellow"), 5);
        assert_eq!(profile.color_value("nonexistent"), 0);
    }

    #[test]
    fn test_param_value_to_color_mute() {
        assert_eq!(param_value_to_color("ch/0/deck/0/mute", 1.0), "red");
        assert_eq!(param_value_to_color("ch/0/deck/0/mute", 0.0), "off");
    }

    #[test]
    fn test_param_value_to_color_continuous() {
        assert_eq!(param_value_to_color("ch/0/opacity", 0.0), "off");
        assert_eq!(param_value_to_color("ch/0/opacity", 0.5), "yellow");
        assert_eq!(param_value_to_color("ch/0/opacity", 1.0), "green");
    }

    #[test]
    fn test_param_value_to_color_unknown() {
        assert_eq!(param_value_to_color("some/unknown/param", -1.0), "green");
    }

    #[test]
    fn test_profile_registry_detect() {
        let registry = ProfileRegistry::new();
        assert!(registry.detect("APC MINI").is_some());
        assert!(registry.detect("My APC Mini mk2").is_some());
        assert!(registry.detect("Novation Launchpad").is_none());
    }

    #[test]
    fn test_json_roundtrip() {
        let json_str = r#"{
  "profile": { "name": "Test Controller", "name_match": "test ctrl" },
  "leds": { "method": "cc_value", "channel": 1, "colors": { "off": 0, "on": 127 } },
  "controls": [
    { "name": "buttons", "type": "button", "midi_type": "cc", "channel": 1, "range": [0, 7], "has_led": true }
  ]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json_str).unwrap();
        assert!(profile.validate().is_empty());
        assert_eq!(profile.profile.name, "Test Controller");
        assert_eq!(profile.led_method(), "cc_value");
        assert_eq!(profile.led_channel(), 1);
        assert_eq!(profile.color_value("on"), 127);
        assert!(profile.control_has_led("cc", 1, 3));
        assert!(!profile.control_has_led("cc", 0, 3));
    }

    #[test]
    fn test_validate_valid_profile() {
        let profile = builtin_apc_mini();
        assert!(profile.validate().is_empty());
    }

    #[test]
    fn test_validate_empty_name() {
        let json = r#"{
  "profile": { "name": "", "name_match": "test" },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("profile.name is empty")));
    }

    #[test]
    fn test_validate_empty_name_match() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "" },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("name_match is empty")));
    }

    #[test]
    fn test_validate_bad_led_method() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "leds": { "method": "banana", "channel": 0, "colors": {} },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("leds.method")));
    }

    #[test]
    fn test_validate_inverted_range() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "range": [63, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("inverted")));
    }

    #[test]
    fn test_validate_bad_control_type() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": [{ "name": "btn", "type": "knob", "midi_type": "note", "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("type 'knob'")));
    }

    #[test]
    fn test_validate_bad_midi_type() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "sysex", "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("midi_type 'sysex'")));
    }

    #[test]
    fn test_validate_empty_controls() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": []
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("controls array is empty")));
    }

    #[test]
    fn test_validate_channel_out_of_range() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "leds": { "method": "note_velocity", "channel": 16, "colors": {} },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "channel": 20, "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("leds.channel 16")));
        assert!(errors.iter().any(|e| e.contains("channel 20")));
    }

    #[test]
    fn test_malformed_json_does_not_panic() {
        let bad_json = r#"{ "profile": { "name": 123 } }"#;
        assert!(serde_json::from_str::<ControllerProfileData>(bad_json).is_err());

        let not_json = "this is not json at all";
        assert!(serde_json::from_str::<ControllerProfileData>(not_json).is_err());

        let empty = "";
        assert!(serde_json::from_str::<ControllerProfileData>(empty).is_err());
    }

    #[test]
    fn test_auto_map_optional() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "range": [0, 0] }]
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        assert!(profile.auto_map.is_none());
        assert!(profile.validate().is_empty());
    }

    #[test]
    fn test_auto_map_validation_bad_control_ref() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": [{ "name": "btn", "type": "button", "midi_type": "note", "range": [0, 0] }],
  "auto_map": {
    "strategy": "channel_grid",
    "grid_control": "nonexistent",
    "fader_control": "also_bad",
    "columns": 8, "rows": 8,
    "tap_action": "mute", "hold_action": "solo",
    "fader_target": "channel_opacity",
    "led_rules": { "active": "green", "muted": "red", "zero_opacity": "red", "soloed": "yellow", "empty": "off" }
  }
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("grid_control")));
        assert!(errors.iter().any(|e| e.contains("fader_control")));
    }

    #[test]
    fn test_auto_map_validation_bad_strategy() {
        let json = r#"{
  "profile": { "name": "Test", "name_match": "test" },
  "controls": [{ "name": "grid", "type": "button", "midi_type": "note", "range": [0, 63] },
               { "name": "faders", "type": "fader", "midi_type": "cc", "range": [48, 56] }],
  "auto_map": {
    "strategy": "banana",
    "grid_control": "grid",
    "fader_control": "faders",
    "columns": 8, "rows": 8,
    "tap_action": "invalid_action", "hold_action": "solo",
    "fader_target": "channel_opacity",
    "led_rules": { "active": "green", "muted": "red", "zero_opacity": "red", "soloed": "yellow", "empty": "off" }
  }
}"#;
        let profile: ControllerProfileData = serde_json::from_str(json).unwrap();
        let errors = profile.validate();
        assert!(errors.iter().any(|e| e.contains("strategy 'banana'")));
        assert!(errors.iter().any(|e| e.contains("tap_action")));
    }
}
