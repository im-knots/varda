//! Configurable keyboard shortcut system.
//!
//! Mirrors the MIDI mapping architecture: a data-driven keymap with learn mode,
//! persistence to `.varda/keymap.json`, and default bindings that can be overridden.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};

/// A key combination: a key + modifier state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyCombo {
    pub key: String,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}

impl KeyCombo {
    /// Create a KeyCombo from egui key + modifiers.
    pub fn from_egui(key: egui::Key, modifiers: &egui::Modifiers) -> Self {
        Self {
            key: egui_key_to_string(key),
            command: modifiers.command,
            shift: modifiers.shift,
            alt: modifiers.alt,
        }
    }

    /// Convert back to egui::Key (returns None if string doesn't map).
    pub fn to_egui_key(&self) -> Option<egui::Key> {
        string_to_egui_key(&self.key)
    }
}

/// What a key binding targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyTarget {
    /// A discrete application action.
    Action(ActionId),
    /// A param_path (same addressing as MIDI).
    ParamPath(String),
}

/// All discrete actions that can be keyboard-mapped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ActionId {
    Undo,
    Redo,
    Save,
    ToggleLibrary,
    ToggleStageEditor,
    ToolSelect,
    ToolRectangle,
    ToolPolygon,
    ToolCircle,
    DuplicateSurface,
    FlipHorizontal,
    FlipVertical,
    DeleteSurface,
    ClearDrawing,
    CombineSurfaces,
    ToggleMidiLearn,
    ToggleKeyboardLearn,
}

/// Persistent keymap store. Mirrors MidiMappingStore pattern.
#[derive(Debug, Clone)]
pub struct KeymapStore {
    pub bindings: HashMap<KeyCombo, KeyTarget>,
    pub learn_mode: bool,
    pub learn_target: Option<KeyTarget>,
}

impl KeymapStore {
    /// Create an empty keymap store.
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
            learn_mode: false,
            learn_target: None,
        }
    }

    /// Create a keymap store populated with default bindings.
    pub fn with_defaults() -> Self {
        let mut store = Self::new();
        store.bindings = Self::defaults();
        store
    }

    /// Static default bindings.
    pub fn defaults() -> HashMap<KeyCombo, KeyTarget> {
        let mut m = HashMap::new();
        let action = |id: ActionId| KeyTarget::Action(id);

        // Global shortcuts
        m.insert(KeyCombo { key: "Z".into(), command: true, shift: false, alt: false }, action(ActionId::Undo));
        m.insert(KeyCombo { key: "Z".into(), command: true, shift: true, alt: false }, action(ActionId::Redo));
        m.insert(KeyCombo { key: "S".into(), command: true, shift: false, alt: false }, action(ActionId::Save));
        m.insert(KeyCombo { key: "L".into(), command: false, shift: false, alt: false }, action(ActionId::ToggleLibrary));

        // Stage editor tools (context-checked at dispatch)
        m.insert(KeyCombo { key: "S".into(), command: false, shift: false, alt: false }, action(ActionId::ToolSelect));
        m.insert(KeyCombo { key: "R".into(), command: false, shift: false, alt: false }, action(ActionId::ToolRectangle));
        m.insert(KeyCombo { key: "P".into(), command: false, shift: false, alt: false }, action(ActionId::ToolPolygon));
        m.insert(KeyCombo { key: "C".into(), command: false, shift: false, alt: false }, action(ActionId::ToolCircle));
        m.insert(KeyCombo { key: "D".into(), command: false, shift: false, alt: false }, action(ActionId::DuplicateSurface));
        m.insert(KeyCombo { key: "H".into(), command: false, shift: false, alt: false }, action(ActionId::FlipHorizontal));
        m.insert(KeyCombo { key: "V".into(), command: false, shift: false, alt: false }, action(ActionId::FlipVertical));
        m.insert(KeyCombo { key: "Delete".into(), command: false, shift: false, alt: false }, action(ActionId::DeleteSurface));
        m.insert(KeyCombo { key: "Backspace".into(), command: false, shift: false, alt: false }, action(ActionId::DeleteSurface));
        m.insert(KeyCombo { key: "Escape".into(), command: false, shift: false, alt: false }, action(ActionId::ClearDrawing));
        m.insert(KeyCombo { key: "G".into(), command: false, shift: false, alt: false }, action(ActionId::CombineSurfaces));

        m
    }

    /// Add or replace a binding.
    pub fn set(&mut self, combo: KeyCombo, target: KeyTarget) {
        log::info!("Keyboard mapped {:?} → {:?}", combo, target);
        self.bindings.insert(combo, target);
    }

    /// Remove a binding.
    pub fn remove(&mut self, combo: &KeyCombo) {
        self.bindings.remove(combo);
    }

    /// Look up a binding.
    pub fn get(&self, combo: &KeyCombo) -> Option<&KeyTarget> {
        self.bindings.get(combo)
    }

    /// Convenience: look up from egui types.
    pub fn lookup_egui(&self, key: egui::Key, modifiers: &egui::Modifiers) -> Option<&KeyTarget> {
        let combo = KeyCombo::from_egui(key, modifiers);
        self.bindings.get(&combo)
    }

    /// Toggle learn mode on/off.
    pub fn toggle_learn(&mut self) {
        self.learn_mode = !self.learn_mode;
        if !self.learn_mode {
            self.learn_target = None;
        }
        log::info!("Keyboard learn mode: {}", if self.learn_mode { "ON" } else { "OFF" });
    }


    /// Select a learn target (must be in learn mode).
    pub fn select_learn_target(&mut self, target: KeyTarget) {
        if self.learn_mode {
            log::info!("Keyboard learn target: {:?}", target);
            self.learn_target = Some(target);
        }
    }

    /// Cancel learn mode.
    pub fn cancel_learn(&mut self) {
        self.learn_mode = false;
        self.learn_target = None;
    }

    /// Process a key press in learn mode. Binds combo to current target.
    /// Returns true if a mapping was created. Stays in learn mode.
    pub fn process_learn(&mut self, combo: KeyCombo) -> bool {
        if let Some(target) = self.learn_target.take() {
            self.set(combo, target);
            true
        } else {
            false
        }
    }

    /// Serialize to a persistable config.
    pub fn to_config(&self) -> KeymapConfig {
        let bindings = self.bindings.iter().map(|(combo, target)| {
            KeyBinding {
                key: combo.key.clone(),
                command: combo.command,
                shift: combo.shift,
                alt: combo.alt,
                target: target.clone(),
            }
        }).collect();
        KeymapConfig { version: 1, bindings }
    }

    /// Load bindings from config, merging over defaults.
    pub fn load_config(&mut self, config: &KeymapConfig) {
        // Start from defaults, then overlay custom bindings
        self.bindings = Self::defaults();
        for binding in &config.bindings {
            let combo = KeyCombo {
                key: binding.key.clone(),
                command: binding.command,
                shift: binding.shift,
                alt: binding.alt,
            };
            // Only load if the key string is valid
            if combo.to_egui_key().is_some() || binding.key == "Delete" || binding.key == "Backspace" || binding.key == "Escape" {
                self.bindings.insert(combo, binding.target.clone());
            } else {
                log::warn!("Keymap: skipping unknown key '{}'", binding.key);
            }
        }
    }

    /// Reset to default bindings.
    pub fn reset_to_defaults(&mut self) {
        self.bindings = Self::defaults();
        log::info!("Keyboard shortcuts reset to defaults");
    }

    /// Sorted bindings for UI display.
    pub fn sorted_bindings(&self) -> Vec<(&KeyCombo, &KeyTarget)> {
        let mut list: Vec<_> = self.bindings.iter().collect();
        list.sort_by(|a, b| a.0.key.cmp(&b.0.key));
        list
    }
}

/// Serializable keymap config for `.varda/keymap.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeymapConfig {
    #[serde(default = "default_keymap_version")]
    pub version: u32,
    #[serde(default)]
    pub bindings: Vec<KeyBinding>,
}

fn default_keymap_version() -> u32 { 1 }

/// A single key binding entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyBinding {
    pub key: String,
    #[serde(default)]
    pub command: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    pub target: KeyTarget,
}

impl KeyBinding {
    /// Validate a single key binding. Returns a list of errors (empty = valid).
    pub fn validate(&self, prefix: &str) -> Vec<String> {
        let mut errors = Vec::new();
        if self.key.trim().is_empty() {
            errors.push(format!("{}: key is empty", prefix));
        }
        errors
    }
}

impl KeymapConfig {
    /// Validate the keymap config for semantic correctness. Returns a list of errors.
    /// An empty list means the config is valid.
    pub fn validate(&self) -> Vec<String> {
        let mut errors = Vec::new();
        for (i, binding) in self.bindings.iter().enumerate() {
            errors.extend(binding.validate(&format!("bindings[{}]", i)));
        }
        errors
    }

    /// Load from a JSON file.
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| anyhow::anyhow!("Failed to read keymap config: {}", e))?;
        let config: KeymapConfig = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse keymap config: {}", e))?;
        let warnings = config.validate();
        for w in &warnings {
            log::warn!("Keymap config {}: {}", path.as_ref().display(), w);
        }
        Ok(config)
    }

    /// Save to a JSON file.
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Keymap config save: {}", e);
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("Failed to serialize keymap config: {}", e))?;
        std::fs::write(path.as_ref(), content)
            .map_err(|e| anyhow::anyhow!("Failed to write keymap config: {}", e))?;
        Ok(())
    }
}

// ── egui::Key ↔ String conversion ──────────────────────────────────

/// Convert an egui::Key to a stable string name for serialization.
pub fn egui_key_to_string(key: egui::Key) -> String {
    format!("{:?}", key)
}

/// Convert a string name back to egui::Key.
pub fn string_to_egui_key(s: &str) -> Option<egui::Key> {
    match s {
        "A" => Some(egui::Key::A), "B" => Some(egui::Key::B), "C" => Some(egui::Key::C),
        "D" => Some(egui::Key::D), "E" => Some(egui::Key::E), "F" => Some(egui::Key::F),
        "G" => Some(egui::Key::G), "H" => Some(egui::Key::H), "I" => Some(egui::Key::I),
        "J" => Some(egui::Key::J), "K" => Some(egui::Key::K), "L" => Some(egui::Key::L),
        "M" => Some(egui::Key::M), "N" => Some(egui::Key::N), "O" => Some(egui::Key::O),
        "P" => Some(egui::Key::P), "Q" => Some(egui::Key::Q), "R" => Some(egui::Key::R),
        "S" => Some(egui::Key::S), "T" => Some(egui::Key::T), "U" => Some(egui::Key::U),
        "V" => Some(egui::Key::V), "W" => Some(egui::Key::W), "X" => Some(egui::Key::X),
        "Y" => Some(egui::Key::Y), "Z" => Some(egui::Key::Z),
        "Num0" => Some(egui::Key::Num0), "Num1" => Some(egui::Key::Num1),
        "Num2" => Some(egui::Key::Num2), "Num3" => Some(egui::Key::Num3),
        "Num4" => Some(egui::Key::Num4), "Num5" => Some(egui::Key::Num5),
        "Num6" => Some(egui::Key::Num6), "Num7" => Some(egui::Key::Num7),
        "Num8" => Some(egui::Key::Num8), "Num9" => Some(egui::Key::Num9),
        "F1" => Some(egui::Key::F1), "F2" => Some(egui::Key::F2),
        "F3" => Some(egui::Key::F3), "F4" => Some(egui::Key::F4),
        "F5" => Some(egui::Key::F5), "F6" => Some(egui::Key::F6),
        "F7" => Some(egui::Key::F7), "F8" => Some(egui::Key::F8),
        "F9" => Some(egui::Key::F9), "F10" => Some(egui::Key::F10),
        "F11" => Some(egui::Key::F11), "F12" => Some(egui::Key::F12),
        "ArrowUp" => Some(egui::Key::ArrowUp), "ArrowDown" => Some(egui::Key::ArrowDown),
        "ArrowLeft" => Some(egui::Key::ArrowLeft), "ArrowRight" => Some(egui::Key::ArrowRight),
        "Home" => Some(egui::Key::Home), "End" => Some(egui::Key::End),
        "PageUp" => Some(egui::Key::PageUp), "PageDown" => Some(egui::Key::PageDown),
        "Insert" => Some(egui::Key::Insert), "Delete" => Some(egui::Key::Delete),
        "Backspace" => Some(egui::Key::Backspace), "Enter" => Some(egui::Key::Enter),
        "Tab" => Some(egui::Key::Tab), "Space" => Some(egui::Key::Space),
        "Escape" => Some(egui::Key::Escape),
        "Minus" => Some(egui::Key::Minus), "Plus" => Some(egui::Key::Plus),
        _ => None,
    }
}

/// Collect all keys pressed this frame from egui input.
pub fn collect_pressed_keys(ctx: &egui::Context) -> Vec<(egui::Key, egui::Modifiers)> {
    ctx.input(|i| {
        let mut pressed = Vec::new();
        let mods = i.modifiers;
        for event in &i.events {
            if let egui::Event::Key { key, pressed: true, repeat: false, .. } = event {
                pressed.push((*key, mods));
            }
        }
        pressed
    })
}

/// Display a KeyCombo as a user-friendly string (e.g. "Cmd+Shift+Z").
impl std::fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.command { write!(f, "Cmd+")?; }
        if self.shift { write!(f, "Shift+")?; }
        if self.alt { write!(f, "Alt+")?; }
        write!(f, "{}", self.key)
    }
}

/// Display a KeyTarget as a user-friendly string.
impl std::fmt::Display for KeyTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyTarget::Action(id) => write!(f, "{:?}", id),
            KeyTarget::ParamPath(path) => write!(f, "{}", path),
        }
    }
}


// ── Keyboard param toggle ───────────────────────────────────────────

/// Toggle a parameter via keyboard shortcut.
/// Float params: toggle between current value and 0.0.
/// Bool params: toggle true/false.
pub fn apply_keyboard_toggle_param(mixer: &mut crate::mixer::Mixer, path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        // crossfader — toggle between 0.0 and 1.0
        ["crossfader"] => {
            let current = mixer.crossfader();
            mixer.snap_crossfader(if current > 0.5 { 0.0 } else { 1.0 });
            true
        }
        // ch/<n>/opacity
        ["ch", ch_s, "opacity"] => {
            if let Ok(ch) = ch_s.parse::<usize>() {
                if let Some(channel) = mixer.channel_mut(ch) {
                    channel.opacity = if channel.opacity > 0.01 { 0.0 } else { 1.0 };
                    return true;
                }
            }
            false
        }
        // ch/<n>/deck/<m>/opacity
        ["ch", ch_s, "deck", dk_s, "opacity"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if let Some(slot) = channel.decks.get_mut(dk) {
                        slot.opacity = if slot.opacity > 0.01 { 0.0 } else { 1.0 };
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/mute — toggle mute
        ["ch", ch_s, "deck", dk_s, "mute"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if let Some(slot) = channel.decks.get_mut(dk) {
                        slot.mute = !slot.mute;
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/solo — toggle solo
        ["ch", ch_s, "deck", dk_s, "solo"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if let Some(slot) = channel.decks.get_mut(dk) {
                        slot.solo = !slot.solo;
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/trigger — set deck opacity to 1.0
        ["ch", ch_s, "deck", dk_s, "trigger"] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if let Some(slot) = channel.decks.get_mut(dk) {
                        slot.opacity = 1.0;
                        return true;
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/param/<name>
        ["ch", ch_s, "deck", dk_s, "param", name] => {
            if let (Ok(ch), Ok(dk)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if let Some(slot) = channel.decks.get_mut(dk) {
                        if let Some(val) = slot.deck.generator_params.values.get_mut(*name) {
                            toggle_param_value(val);
                            return true;
                        }
                    }
                }
            }
            false
        }
        // ch/<n>/deck/<m>/effect/<k>/param/<name>
        ["ch", ch_s, "deck", dk_s, "effect", ek_s, "param", name] => {
            if let (Ok(ch), Ok(dk), Ok(ek)) = (ch_s.parse::<usize>(), dk_s.parse::<usize>(), ek_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if let Some(slot) = channel.decks.get_mut(dk) {
                        if ek < slot.deck.effects.len() {
                            if let Some(val) = slot.deck.effects[ek].params.values.get_mut(*name) {
                                toggle_param_value(val);
                                return true;
                            }
                        }
                    }
                }
            }
            false
        }
        // ch/<n>/effect/<k>/param/<name>
        ["ch", ch_s, "effect", ek_s, "param", name] => {
            if let (Ok(ch), Ok(ek)) = (ch_s.parse::<usize>(), ek_s.parse::<usize>()) {
                if let Some(channel) = mixer.channel_mut(ch) {
                    if ek < channel.effects.len() {
                        if let Some(val) = channel.effects[ek].params.values.get_mut(*name) {
                            toggle_param_value(val);
                            return true;
                        }
                    }
                }
            }
            false
        }
        // master/effect/<k>/param/<name>
        ["master", "effect", ek_s, "param", name] => {
            if let Ok(ek) = ek_s.parse::<usize>() {
                let effects = mixer.master_effects_mut();
                if ek < effects.len() {
                    if let Some(val) = effects[ek].params.values.get_mut(*name) {
                        toggle_param_value(val);
                        return true;
                    }
                }
            }
            false
        }
        // mod/<idx>/<param> — modulation source params (toggle float between 0 and 1)
        ["mod", _idx_s, _param] => {
            // Modulation params are continuous values; keyboard toggle doesn't apply well.
            // Fall through to default.
            false
        }
        _ => false,
    }
}

/// Toggle a param value: float toggles between 0.0 and 1.0, bool inverts.
fn toggle_param_value(val: &mut crate::params::ParamValue) {
    use crate::params::ParamValue;
    match val {
        ParamValue::Float(v) => *v = if v.abs() > 0.01 { 0.0 } else { 1.0 },
        ParamValue::Bool(b) => *b = !*b,
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(key: &str, command: bool, shift: bool, alt: bool) -> KeyCombo {
        KeyCombo { key: key.into(), command, shift, alt }
    }

    #[test]
    fn test_default_bindings_complete() {
        let store = KeymapStore::with_defaults();
        assert!(store.bindings.values().any(|t| *t == KeyTarget::Action(ActionId::Undo)));
        assert!(store.bindings.values().any(|t| *t == KeyTarget::Action(ActionId::Save)));
        assert!(store.bindings.values().any(|t| *t == KeyTarget::Action(ActionId::ToggleLibrary)));
        assert!(store.bindings.values().any(|t| *t == KeyTarget::Action(ActionId::ToolSelect)));
    }

    #[test]
    fn test_serialization_roundtrip() {
        let store = KeymapStore::with_defaults();
        let config = store.to_config();
        let json = serde_json::to_string(&config).unwrap();
        let loaded: KeymapConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(config.bindings.len(), loaded.bindings.len());
    }

    #[test]
    fn test_custom_binding_overrides_default() {
        let mut store = KeymapStore::with_defaults();
        let c = combo("L", false, false, false);
        store.set(c.clone(), KeyTarget::ParamPath("ch/0/opacity".into()));
        assert_eq!(store.get(&c), Some(&KeyTarget::ParamPath("ch/0/opacity".into())));
    }

    #[test]
    fn test_conflict_replaces() {
        let mut store = KeymapStore::with_defaults();
        let c = combo("Z", true, false, false);
        store.set(c.clone(), KeyTarget::Action(ActionId::Save));
        assert_eq!(store.get(&c), Some(&KeyTarget::Action(ActionId::Save)));
    }

    #[test]
    fn test_learn_mode_state_machine() {
        let mut store = KeymapStore::with_defaults();
        assert!(!store.learn_mode);

        store.toggle_learn();
        assert!(store.learn_mode);
        assert!(store.learn_target.is_none());

        store.select_learn_target(KeyTarget::Action(ActionId::Save));
        assert!(store.learn_target.is_some());

        let c = combo("F1", false, false, false);
        let created = store.process_learn(c.clone());
        assert!(created);
        assert!(store.learn_mode); // stays in learn mode
        assert!(store.learn_target.is_none()); // target consumed
        assert_eq!(store.get(&c), Some(&KeyTarget::Action(ActionId::Save)));
    }

    #[test]
    fn test_reset_to_defaults() {
        let mut store = KeymapStore::with_defaults();
        let c = combo("L", false, false, false);
        store.set(c.clone(), KeyTarget::ParamPath("custom".into()));
        store.reset_to_defaults();
        assert_eq!(store.get(&c), Some(&KeyTarget::Action(ActionId::ToggleLibrary)));
    }

    #[test]
    fn test_key_combo_display() {
        let c = combo("Z", true, true, false);
        assert_eq!(format!("{}", c), "Cmd+Shift+Z");
    }

    #[test]
    fn test_egui_key_roundtrip() {
        let keys = [egui::Key::Z, egui::Key::S, egui::Key::Delete, egui::Key::F1, egui::Key::Space];
        for key in keys {
            let s = egui_key_to_string(key);
            let back = string_to_egui_key(&s);
            assert_eq!(back, Some(key), "Roundtrip failed for {:?} -> {}", key, s);
        }
    }

    #[test]
    fn test_load_config_merges_over_defaults() {
        let mut store = KeymapStore::with_defaults();
        let config = KeymapConfig {
            version: 1,
            bindings: vec![
                KeyBinding {
                    key: "L".into(), command: false, shift: false, alt: false,
                    target: KeyTarget::ParamPath("ch/0/opacity".into()),
                },
            ],
        };
        store.load_config(&config);
        // Custom binding overrides default
        let c = combo("L", false, false, false);
        assert_eq!(store.get(&c), Some(&KeyTarget::ParamPath("ch/0/opacity".into())));
        // Other defaults remain
        let undo = combo("Z", true, false, false);
        assert_eq!(store.get(&undo), Some(&KeyTarget::Action(ActionId::Undo)));
    }

    #[test]
    fn test_keymap_config_validate_valid() {
        let config = KeymapConfig {
            version: 1,
            bindings: vec![KeyBinding {
                key: "Z".into(), command: true, shift: false, alt: false,
                target: KeyTarget::Action(ActionId::Undo),
            }],
        };
        assert!(config.validate().is_empty());
    }

    #[test]
    fn test_keymap_config_validate_empty_key() {
        let config = KeymapConfig {
            version: 1,
            bindings: vec![KeyBinding {
                key: "".into(), command: false, shift: false, alt: false,
                target: KeyTarget::Action(ActionId::Save),
            }],
        };
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("key is empty")));
    }
}