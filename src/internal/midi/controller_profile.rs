//! Data-driven controller profiles for MIDI LED feedback.
//!
//! Profiles describe a controller's physical layout and LED protocol so the
//! LED feedback system works with any hardware. Profiles are loaded from TOML
//! files in `.varda/controllers/` or compiled-in as built-in defaults.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use serde::Deserialize;

use super::{DeviceId, MidiKey, MidiDeviceManager, MidiMappingStore};
use crate::mixer::Mixer;

// ── Profile Data Model ────────────────────────────────────────────

/// A controller profile loaded from TOML or compiled-in.
#[derive(Debug, Clone, Deserialize)]
pub struct ControllerProfileData {
    pub profile: ProfileMeta,
    pub leds: Option<LedConfig>,
    #[serde(default)]
    pub controls: Vec<ControlDef>,
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

fn default_led_method() -> String { "note_velocity".to_string() }

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

impl ControllerProfileData {
    /// Check if a MIDI device name matches this profile.
    pub fn matches(&self, device_name: &str) -> bool {
        device_name.to_lowercase().contains(&self.profile.name_match.to_lowercase())
    }

    /// Get the MIDI value for a logical color name. Returns 0 (off) if not found.
    pub fn color_value(&self, color: &str) -> u8 {
        self.leds.as_ref()
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
        self.leds.as_ref().map(|l| l.method.as_str()).unwrap_or("note_velocity")
    }
}

// ── Built-in APC Mini Profile ─────────────────────────────────────

const APC_MINI_PROFILE_TOML: &str = include_str!("apc_mini_profile.toml");

/// Load the compiled-in APC Mini mk1 profile.
pub fn builtin_apc_mini() -> ControllerProfileData {
    toml::from_str(APC_MINI_PROFILE_TOML)
        .expect("Built-in APC Mini profile TOML is invalid")
}

// ── Profile Registry ──────────────────────────────────────────────

/// Holds all loaded controller profiles (built-in + user-supplied).
pub struct ProfileRegistry {
    profiles: Vec<Arc<ControllerProfileData>>,
}

impl ProfileRegistry {
    /// Create a registry with only the built-in profiles.
    pub fn new() -> Self {
        let mut profiles = Vec::new();
        profiles.push(Arc::new(builtin_apc_mini()));
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
                log::warn!("Failed to read controllers directory {}: {}", dir.display(), e);
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
            if path.extension().and_then(|e| e.to_str()) != Some("toml") {
                continue;
            }
            match std::fs::read_to_string(&path) {
                Ok(content) => match toml::from_str::<ControllerProfileData>(&content) {
                    Ok(profile) => {
                        log::info!("Loaded controller profile '{}' from {}",
                            profile.profile.name, path.display());
                        user_profiles.push(Arc::new(profile));
                    }
                    Err(e) => {
                        log::warn!("Failed to parse controller profile {}: {}", path.display(), e);
                    }
                },
                Err(e) => {
                    log::warn!("Failed to read controller profile {}: {}", path.display(), e);
                }
            }
        }

        // User profiles go first so they override built-ins with same name_match
        user_profiles.append(&mut self.profiles);
        self.profiles = user_profiles;
    }

    /// Find a profile matching a MIDI device name. First match wins.
    pub fn detect(&self, device_name: &str) -> Option<Arc<ControllerProfileData>> {
        self.profiles.iter()
            .find(|p| p.matches(device_name))
            .cloned()
    }

    /// Get all loaded profile names (for debugging/UI).
    pub fn profile_names(&self) -> Vec<&str> {
        self.profiles.iter().map(|p| p.profile.name.as_str()).collect()
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
        Self { profile, leds: HashMap::new() }
    }

    /// Set an LED, only sending if the value changed.
    fn set_led(&mut self, mgr: &MidiDeviceManager, device_id: DeviceId, note: u8, color: &str) -> bool {
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
                mgr.send_raw(device_id, &[
                    0xB0 | (self.profile.led_channel() & 0x0F),
                    note,
                    val,
                ]);
            }
            other => {
                log::warn!("Unknown LED method '{}' in profile '{}'", other, self.profile.profile.name);
            }
        }
        true
    }

    /// Turn all LEDs off on this device.
    fn all_off(&mut self, mgr: &MidiDeviceManager, device_id: DeviceId) {
        // Collect ranges first to avoid borrow conflict with self.set_led
        let led_ranges: Vec<(u8, u8)> = self.profile.controls.iter()
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

impl ControllerLedManager {
    pub fn new() -> Self {
        Self { device_leds: HashMap::new() }
    }

    /// Sync tracked devices with the device manager. Call after rescan.
    pub fn sync_devices(&mut self, mgr: &MidiDeviceManager) {
        // Add new devices that have profiles
        for (id, info) in &mgr.devices {
            if let Some(profile) = &info.profile {
                self.device_leds.entry(*id).or_insert_with(|| {
                    log::info!("LED tracking started for '{}' device [{}]", profile.profile.name, id);
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
        ["ch", ch_s, "opacity"] => {
            ch_s.parse::<usize>().ok()
                .and_then(|ch| mixer.channel(ch))
                .map(|c| c.opacity)
                .unwrap_or(-1.0)
        }
        ["ch", ch_s, "deck", dk_s, "opacity"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer.channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| d.opacity)
                    .unwrap_or(-1.0)
            } else { -1.0 }
        }
        ["ch", ch_s, "deck", dk_s, "mute"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer.channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| if d.mute { 1.0 } else { 0.0 })
                    .unwrap_or(-1.0)
            } else { -1.0 }
        }
        ["ch", ch_s, "deck", dk_s, "solo"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer.channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| if d.solo { 1.0 } else { 0.0 })
                    .unwrap_or(-1.0)
            } else { -1.0 }
        }
        ["ch", ch_s, "deck", dk_s, "trigger"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer.channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| d.opacity)
                    .unwrap_or(-1.0)
            } else { -1.0 }
        }
        ["ch", ch_s, "effect", ek_s, "param", name] => {
            if let (Ok(ch), Ok(ek)) = (ch_s.parse::<usize>(), ek_s.parse::<usize>()) {
                mixer.channel(ch)
                    .and_then(|c| c.effects.get(ek))
                    .and_then(|e| e.params.get_float(name))
                    .unwrap_or(-1.0)
            } else { -1.0 }
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
        assert!(profile.control_has_led("note", 0, 0));    // grid min
        assert!(profile.control_has_led("note", 0, 63));   // grid max
        assert!(profile.control_has_led("note", 0, 64));   // bottom min
        assert!(profile.control_has_led("note", 0, 71));   // bottom max
        assert!(profile.control_has_led("note", 0, 82));   // side min
        assert!(profile.control_has_led("note", 0, 89));   // side max
        assert!(profile.control_has_led("note", 0, 98));   // shift
        assert!(!profile.control_has_led("note", 0, 72));  // gap
        assert!(!profile.control_has_led("note", 0, 81));  // gap
        assert!(!profile.control_has_led("note", 0, 99));  // beyond shift
        assert!(!profile.control_has_led("cc", 0, 48));    // faders have no LED
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
    fn test_toml_roundtrip() {
        // Verify a minimal profile parses correctly
        let toml_str = r#"
[profile]
name = "Test Controller"
name_match = "test ctrl"

[leds]
method = "cc_value"
channel = 1

[leds.colors]
off = 0
on = 127

[[controls]]
name = "buttons"
type = "button"
midi_type = "cc"
channel = 1
range = [0, 7]
has_led = true
"#;
        let profile: ControllerProfileData = toml::from_str(toml_str).unwrap();
        assert_eq!(profile.profile.name, "Test Controller");
        assert_eq!(profile.led_method(), "cc_value");
        assert_eq!(profile.led_channel(), 1);
        assert_eq!(profile.color_value("on"), 127);
        assert!(profile.control_has_led("cc", 1, 3));
        assert!(!profile.control_has_led("cc", 0, 3)); // wrong channel
    }
}
