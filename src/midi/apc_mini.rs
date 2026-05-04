//! Akai APC Mini mk1 controller profile.
//!
//! Provides LED feedback for mapped controls. No hardcoded mappings —
//! all control assignment is via MIDI learn.
//! Supports N APC Mini devices simultaneously — each gets its own LED state.

use std::collections::HashMap;
use super::{DeviceId, MidiKey, MidiDeviceManager, MidiMappingStore, ControllerProfile};
use crate::mixer::Mixer;

/// APC Mini mk1 LED velocities (sent as Note On velocity).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LedColor {
    Off = 0,
    Green = 1,
    GreenBlink = 2,
    Red = 3,
    RedBlink = 4,
    Yellow = 5,
    YellowBlink = 6,
}

/// APC Mini mk1 note ranges.
pub const GRID_NOTE_MIN: u8 = 0;
pub const GRID_NOTE_MAX: u8 = 63;
pub const BOTTOM_NOTE_MIN: u8 = 64;
pub const BOTTOM_NOTE_MAX: u8 = 71;
pub const SIDE_NOTE_MIN: u8 = 82;
pub const SIDE_NOTE_MAX: u8 = 89;
pub const SHIFT_NOTE: u8 = 98;

/// Fader CC range.
pub const FADER_CC_MIN: u8 = 48;
pub const FADER_CC_MAX: u8 = 56;

/// Returns true if a note number is an APC Mini button (grid, side, or bottom).
pub fn is_button_note(note: u8) -> bool {
    (GRID_NOTE_MIN..=GRID_NOTE_MAX).contains(&note)
        || (BOTTOM_NOTE_MIN..=BOTTOM_NOTE_MAX).contains(&note)
        || (SIDE_NOTE_MIN..=SIDE_NOTE_MAX).contains(&note)
        || note == SHIFT_NOTE
}

/// Per-device LED state tracker for one APC Mini.
struct ApcMiniDeviceLeds {
    /// Last-sent LED velocity for each note.
    leds: HashMap<u8, u8>,
}

impl ApcMiniDeviceLeds {
    fn new() -> Self {
        Self { leds: HashMap::new() }
    }

    /// Set an LED, only sending if the value changed.
    fn set_led(&mut self, mgr: &MidiDeviceManager, device_id: DeviceId, note: u8, color: LedColor) -> bool {
        let vel = color as u8;
        if self.leds.get(&note) == Some(&vel) {
            return false;
        }
        self.leds.insert(note, vel);
        mgr.send_note_on(device_id, 0, note, vel);
        true
    }

    /// Turn all LEDs off.
    fn all_off(&mut self, mgr: &MidiDeviceManager, device_id: DeviceId) {
        for note in GRID_NOTE_MIN..=GRID_NOTE_MAX {
            self.set_led(mgr, device_id, note, LedColor::Off);
        }
        for note in BOTTOM_NOTE_MIN..=BOTTOM_NOTE_MAX {
            self.set_led(mgr, device_id, note, LedColor::Off);
        }
        for note in SIDE_NOTE_MIN..=SIDE_NOTE_MAX {
            self.set_led(mgr, device_id, note, LedColor::Off);
        }
    }
}

/// Manages LED state for all connected APC Mini devices.
pub struct ApcMiniManager {
    /// Per-device LED state, keyed by DeviceId.
    device_leds: HashMap<DeviceId, ApcMiniDeviceLeds>,
}

impl ApcMiniManager {
    pub fn new() -> Self {
        Self { device_leds: HashMap::new() }
    }

    /// Sync tracked devices with the device manager. Call after rescan.
    pub fn sync_devices(&mut self, mgr: &MidiDeviceManager) {
        let apc_ids = mgr.devices_with_profile(ControllerProfile::ApcMini);

        // Add new devices
        for id in &apc_ids {
            self.device_leds.entry(*id).or_insert_with(|| {
                log::info!("APC Mini LED tracking started for device [{}]", id);
                ApcMiniDeviceLeds::new()
            });
        }

        // Remove stale devices
        self.device_leds.retain(|id, _| apc_ids.contains(id));
    }

    /// How many APC Minis are being tracked.
    pub fn device_count(&self) -> usize {
        self.device_leds.len()
    }

    /// Turn all LEDs off on all tracked devices.
    pub fn all_off(&mut self, mgr: &MidiDeviceManager) {
        for (device_id, leds) in &mut self.device_leds {
            leds.all_off(mgr, *device_id);
        }
    }

    /// Update LEDs on all tracked APC Minis.
    pub fn update_leds(
        &mut self,
        mgr: &MidiDeviceManager,
        mappings: &MidiMappingStore,
        mixer: &Mixer,
        midi_learn_active: bool,
        midi_learn_target: Option<&str>,
    ) {
        for (&device_id, leds) in &mut self.device_leds {
            // Only process mappings that belong to this device
            for (key, path) in &mappings.mappings {
                if let MidiKey::Note(dev, _ch, note) = key {
                    if *dev != device_id || !is_button_note(*note) {
                        continue;
                    }

                    // MIDI learn target blinks red
                    if midi_learn_active {
                        if let Some(target) = midi_learn_target {
                            if path == target {
                                leds.set_led(mgr, device_id, *note, LedColor::RedBlink);
                                continue;
                            }
                        }
                    }

                    let value = read_param_value(mixer, path);
                    let color = param_value_to_color(path, value);
                    leds.set_led(mgr, device_id, *note, color);
                }
            }
        }
    }
}

/// Read a parameter value from the mixer. Returns 0.0–1.0 or -1.0 if not found.
fn read_param_value(mixer: &Mixer, path: &str) -> f32 {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        ["crossfader"] => mixer.crossfader,
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
        // ch/<n>/deck/<m>/trigger — deck trigger (opacity > 0 = playing)
        ["ch", ch_s, "deck", dk_s, "trigger"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                mixer.channel(ch)
                    .and_then(|c| c.decks.get(dk))
                    .map(|d| d.opacity)
                    .unwrap_or(-1.0)
            } else { -1.0 }
        }
        // ch/<n>/effect/<k>/param/<name> — channel effect param
        ["ch", ch_s, "effect", ek_s, "param", name] => {
            if let (Ok(ch), Ok(ek)) = (ch_s.parse::<usize>(), ek_s.parse::<usize>()) {
                mixer.channel(ch)
                    .and_then(|c| c.effects.get(ek))
                    .and_then(|e| e.params.get_float(name))
                    .unwrap_or(-1.0)
            } else { -1.0 }
        }
        // For generic float params, return -1.0 (we can't easily read them without
        // coupling to ShaderParams here — treat as "always on" if mapped)
        _ => -1.0,
    }
}

/// Determine LED color from parameter path and current value.
fn param_value_to_color(path: &str, value: f32) -> LedColor {
    if value < 0.0 {
        // Unmapped or unreadable — show green to indicate "mapped"
        return LedColor::Green;
    }

    // Deck trigger: green when playing (opacity > 0), yellow when mapped but off
    if path.ends_with("/trigger") {
        return if value > 0.01 { LedColor::Green } else { LedColor::Yellow };
    }

    // Boolean-like paths (mute, solo, effect toggle)
    if path.ends_with("/mute") || path.ends_with("/solo") {
        return if value > 0.5 { LedColor::Red } else { LedColor::Off };
    }

    // Continuous params (opacity, crossfader, shader params)
    if value < 0.01 {
        LedColor::Off
    } else if value > 0.99 {
        LedColor::Green
    } else {
        LedColor::Yellow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_button_note() {
        assert!(is_button_note(0));    // grid min
        assert!(is_button_note(63));   // grid max
        assert!(is_button_note(64));   // bottom min
        assert!(is_button_note(71));   // bottom max
        assert!(is_button_note(82));   // side min
        assert!(is_button_note(89));   // side max
        assert!(is_button_note(98));   // shift
        assert!(!is_button_note(72));  // gap
        assert!(!is_button_note(81));  // gap
        assert!(!is_button_note(99));  // beyond shift
    }

    #[test]
    fn test_led_color_values() {
        assert_eq!(LedColor::Off as u8, 0);
        assert_eq!(LedColor::Green as u8, 1);
        assert_eq!(LedColor::GreenBlink as u8, 2);
        assert_eq!(LedColor::Red as u8, 3);
        assert_eq!(LedColor::RedBlink as u8, 4);
        assert_eq!(LedColor::Yellow as u8, 5);
        assert_eq!(LedColor::YellowBlink as u8, 6);
    }

    #[test]
    fn test_param_value_to_color_mute() {
        assert_eq!(param_value_to_color("ch/0/deck/0/mute", 1.0), LedColor::Red);
        assert_eq!(param_value_to_color("ch/0/deck/0/mute", 0.0), LedColor::Off);
    }

    #[test]
    fn test_param_value_to_color_continuous() {
        assert_eq!(param_value_to_color("ch/0/opacity", 0.0), LedColor::Off);
        assert_eq!(param_value_to_color("ch/0/opacity", 0.5), LedColor::Yellow);
        assert_eq!(param_value_to_color("ch/0/opacity", 1.0), LedColor::Green);
    }

    #[test]
    fn test_param_value_to_color_unknown() {
        // -1.0 = unreadable → green (mapped indicator)
        assert_eq!(param_value_to_color("some/unknown/param", -1.0), LedColor::Green);
    }
}
