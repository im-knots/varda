//! egui adapter for the domain keymap.
//!
//! `internal/keymap` stores string key names and modifier flags. This module
//! is the only place that converts those names to and from `egui::Key`.

use crate::keymap::KeyCombo;

impl KeyCombo {
    /// Build a domain combo from an egui key event.
    pub fn from_egui(key: egui::Key, modifiers: &egui::Modifiers) -> Self {
        Self {
            key: format!("{key:?}"),
            command: modifiers.command,
            shift: modifiers.shift,
            alt: modifiers.alt,
        }
    }
}

/// Collect non-repeat key-down events this frame as domain combos.
pub fn collect_pressed_keys(ctx: &egui::Context) -> Vec<KeyCombo> {
    ctx.input(|i| {
        let mods = i.modifiers;
        i.events
            .iter()
            .filter_map(|event| {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat: false,
                    ..
                } = event
                {
                    Some(KeyCombo::from_egui(*key, &mods))
                } else {
                    None
                }
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keymap::{SUPPORTED_KEY_NAMES, is_supported_key_name};

    #[test]
    fn from_egui_uses_debug_names_and_modifiers() {
        let mods = egui::Modifiers {
            command: true,
            shift: true,
            alt: false,
            ..egui::Modifiers::default()
        };
        let combo = KeyCombo::from_egui(egui::Key::Z, &mods);
        assert_eq!(combo.key, "Z");
        assert!(combo.command);
        assert!(combo.shift);
        assert!(!combo.alt);
        assert!(is_supported_key_name(&combo.key));
    }

    #[test]
    fn supported_names_cover_mapped_egui_keys() {
        let keys = [
            egui::Key::A,
            egui::Key::Z,
            egui::Key::Num0,
            egui::Key::F12,
            egui::Key::ArrowLeft,
            egui::Key::Delete,
            egui::Key::Backspace,
            egui::Key::Escape,
            egui::Key::Space,
            egui::Key::Minus,
            egui::Key::Plus,
        ];
        for key in keys {
            let name = format!("{key:?}");
            assert!(
                SUPPORTED_KEY_NAMES.contains(&name.as_str()),
                "{name} must stay in SUPPORTED_KEY_NAMES"
            );
        }
    }
}
