//! Configurable keyboard shortcut system.
//!
//! Mirrors the MIDI mapping architecture: a data-driven keymap with learn mode,
//! persistence to `.varda/keymap.json`, and default bindings that can be overridden.
//! This module is framework-free. egui conversion lives in `usecases::ui::keyboard`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Key names persisted in `.varda/keymap.json` (`egui::Key` Debug spellings).
pub const SUPPORTED_KEY_NAMES: &[&str] = &[
    "A",
    "B",
    "C",
    "D",
    "E",
    "F",
    "G",
    "H",
    "I",
    "J",
    "K",
    "L",
    "M",
    "N",
    "O",
    "P",
    "Q",
    "R",
    "S",
    "T",
    "U",
    "V",
    "W",
    "X",
    "Y",
    "Z",
    "Num0",
    "Num1",
    "Num2",
    "Num3",
    "Num4",
    "Num5",
    "Num6",
    "Num7",
    "Num8",
    "Num9",
    "F1",
    "F2",
    "F3",
    "F4",
    "F5",
    "F6",
    "F7",
    "F8",
    "F9",
    "F10",
    "F11",
    "F12",
    "ArrowUp",
    "ArrowDown",
    "ArrowLeft",
    "ArrowRight",
    "Home",
    "End",
    "PageUp",
    "PageDown",
    "Insert",
    "Delete",
    "Backspace",
    "Enter",
    "Tab",
    "Space",
    "Escape",
    "Minus",
    "Plus",
];

/// Whether `name` is a persistable key identity.
pub fn is_supported_key_name(name: &str) -> bool {
    SUPPORTED_KEY_NAMES.contains(&name)
}

/// A key combination: a key + modifier state.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyCombo {
    pub key: String,
    pub command: bool,
    pub shift: bool,
    pub alt: bool,
}

/// What a key binding targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyTarget {
    /// A discrete application action.
    Action(ActionId),
    /// A `param_path` (same addressing as MIDI).
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
    /// Copy, paste, and duplicate the current selection: the deck, channel, or
    /// effect the bottom bar is already following. See /spec/clipboard.md.
    Copy,
    Paste,
    Duplicate,
}

/// Persistent keymap store. Mirrors `MidiMappingStore` pattern.
#[derive(Debug, Clone)]
pub struct KeymapStore {
    pub bindings: HashMap<KeyCombo, KeyTarget>,
    pub learn_mode: bool,
    pub learn_target: Option<KeyTarget>,
}

impl Default for KeymapStore {
    fn default() -> Self {
        Self::new()
    }
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
        m.insert(
            KeyCombo {
                key: "Z".into(),
                command: true,
                shift: false,
                alt: false,
            },
            action(ActionId::Undo),
        );
        m.insert(
            KeyCombo {
                key: "Z".into(),
                command: true,
                shift: true,
                alt: false,
            },
            action(ActionId::Redo),
        );
        m.insert(
            KeyCombo {
                key: "S".into(),
                command: true,
                shift: false,
                alt: false,
            },
            action(ActionId::Save),
        );
        m.insert(
            KeyCombo {
                key: "L".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::ToggleLibrary),
        );
        for (key, id) in [
            ("C", ActionId::Copy),
            ("V", ActionId::Paste),
            ("D", ActionId::Duplicate),
        ] {
            m.insert(
                KeyCombo {
                    key: key.into(),
                    command: true,
                    shift: false,
                    alt: false,
                },
                action(id),
            );
        }

        // Stage editor tools (context-checked at dispatch)
        m.insert(
            KeyCombo {
                key: "S".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::ToolSelect),
        );
        m.insert(
            KeyCombo {
                key: "R".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::ToolRectangle),
        );
        m.insert(
            KeyCombo {
                key: "P".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::ToolPolygon),
        );
        m.insert(
            KeyCombo {
                key: "C".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::ToolCircle),
        );
        m.insert(
            KeyCombo {
                key: "D".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::DuplicateSurface),
        );
        m.insert(
            KeyCombo {
                key: "H".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::FlipHorizontal),
        );
        m.insert(
            KeyCombo {
                key: "V".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::FlipVertical),
        );
        m.insert(
            KeyCombo {
                key: "Delete".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::DeleteSurface),
        );
        m.insert(
            KeyCombo {
                key: "Backspace".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::DeleteSurface),
        );
        m.insert(
            KeyCombo {
                key: "Escape".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::ClearDrawing),
        );
        m.insert(
            KeyCombo {
                key: "G".into(),
                command: false,
                shift: false,
                alt: false,
            },
            action(ActionId::CombineSurfaces),
        );

        m
    }

    /// Add or replace a binding.
    pub fn set(&mut self, combo: KeyCombo, target: KeyTarget) {
        log::info!("Keyboard mapped {combo:?} → {target:?}");
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

    /// Toggle learn mode on/off.
    pub fn toggle_learn(&mut self) {
        self.learn_mode = !self.learn_mode;
        if !self.learn_mode {
            self.learn_target = None;
        }
        log::info!(
            "Keyboard learn mode: {}",
            if self.learn_mode { "ON" } else { "OFF" }
        );
    }

    /// Select a learn target (must be in learn mode).
    pub fn select_learn_target(&mut self, target: KeyTarget) {
        if self.learn_mode {
            log::info!("Keyboard learn target: {target:?}");
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
        let bindings = self
            .bindings
            .iter()
            .map(|(combo, target)| KeyBinding {
                key: combo.key.clone(),
                command: combo.command,
                shift: combo.shift,
                alt: combo.alt,
                target: target.clone(),
            })
            .collect();
        KeymapConfig {
            version: 1,
            bindings,
        }
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
            if is_supported_key_name(&binding.key) {
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
}

/// Serializable keymap config for `.varda/keymap.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeymapConfig {
    #[serde(default = "default_keymap_version")]
    pub version: u32,
    #[serde(default)]
    pub bindings: Vec<KeyBinding>,
}

fn default_keymap_version() -> u32 {
    1
}

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
            errors.push(format!("{prefix}: key is empty"));
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
            errors.extend(binding.validate(&format!("bindings[{i}]")));
        }
        errors
    }

    /// Load from a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or if its contents are not
    /// valid keymap JSON. Semantic validation issues are logged as warnings and
    /// do not fail the load.
    pub fn load<P: AsRef<std::path::Path>>(path: P) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .map_err(|e| anyhow::anyhow!("Failed to read keymap config: {e}"))?;
        let config: KeymapConfig = serde_json::from_str(&content)
            .map_err(|e| anyhow::anyhow!("Failed to parse keymap config: {e}"))?;
        let warnings = config.validate();
        for w in &warnings {
            log::warn!("Keymap config {}: {}", path.as_ref().display(), w);
        }
        Ok(config)
    }

    /// Save to a JSON file.
    ///
    /// # Errors
    ///
    /// Returns an error if the config cannot be serialized to JSON or if the
    /// atomic write to `path` fails (missing directory, permissions, disk full).
    pub fn save<P: AsRef<std::path::Path>>(&self, path: P) -> anyhow::Result<()> {
        let errors = self.validate();
        for e in &errors {
            log::error!("Keymap config save: {e}");
        }
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| anyhow::anyhow!("Failed to serialize keymap config: {e}"))?;
        crate::persistence::atomic_write(path.as_ref(), &content)?;
        Ok(())
    }
}

/// Display a `KeyCombo` as a user-friendly string (e.g. "Cmd+Shift+Z").
impl std::fmt::Display for KeyCombo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.command {
            write!(f, "Cmd+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        write!(f, "{}", self.key)
    }
}

/// Display a `KeyTarget` as a user-friendly string.
impl std::fmt::Display for KeyTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyTarget::Action(id) => write!(f, "{id:?}"),
            KeyTarget::ParamPath(path) => write!(f, "{path}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(key: &str, command: bool, shift: bool, alt: bool) -> KeyCombo {
        KeyCombo {
            key: key.into(),
            command,
            shift,
            alt,
        }
    }

    #[test]
    fn test_default_bindings_complete() {
        let store = KeymapStore::with_defaults();
        assert!(store
            .bindings
            .values()
            .any(|t| *t == KeyTarget::Action(ActionId::Undo)));
        assert!(store
            .bindings
            .values()
            .any(|t| *t == KeyTarget::Action(ActionId::Save)));
        assert!(store
            .bindings
            .values()
            .any(|t| *t == KeyTarget::Action(ActionId::ToggleLibrary)));
        assert!(store
            .bindings
            .values()
            .any(|t| *t == KeyTarget::Action(ActionId::ToolSelect)));
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
        assert_eq!(
            store.get(&c),
            Some(&KeyTarget::ParamPath("ch/0/opacity".into()))
        );
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
        assert_eq!(
            store.get(&c),
            Some(&KeyTarget::Action(ActionId::ToggleLibrary))
        );
    }

    #[test]
    fn test_key_combo_display() {
        let c = combo("Z", true, true, false);
        assert_eq!(format!("{c}"), "Cmd+Shift+Z");
    }

    #[test]
    fn default_bindings_use_supported_key_names() {
        for combo in KeymapStore::defaults().keys() {
            assert!(
                is_supported_key_name(&combo.key),
                "default binding uses unsupported key {}",
                combo.key
            );
        }
        assert!(is_supported_key_name("Z"));
        assert!(is_supported_key_name("Delete"));
        assert!(is_supported_key_name("F1"));
        assert!(!is_supported_key_name("NotAKey"));
        assert!(!is_supported_key_name("z"));
    }

    #[test]
    fn load_config_skips_unknown_key_names() {
        let mut store = KeymapStore::with_defaults();
        let before = store.bindings.len();
        store.load_config(&KeymapConfig {
            version: 1,
            bindings: vec![KeyBinding {
                key: "NotAKey".into(),
                command: false,
                shift: false,
                alt: false,
                target: KeyTarget::Action(ActionId::Save),
            }],
        });
        assert_eq!(store.bindings.len(), before);
        assert!(store.get(&combo("NotAKey", false, false, false)).is_none());
    }

    #[test]
    fn test_load_config_merges_over_defaults() {
        let mut store = KeymapStore::with_defaults();
        let config = KeymapConfig {
            version: 1,
            bindings: vec![KeyBinding {
                key: "L".into(),
                command: false,
                shift: false,
                alt: false,
                target: KeyTarget::ParamPath("ch/0/opacity".into()),
            }],
        };
        store.load_config(&config);
        // Custom binding overrides default
        let c = combo("L", false, false, false);
        assert_eq!(
            store.get(&c),
            Some(&KeyTarget::ParamPath("ch/0/opacity".into()))
        );
        // Other defaults remain
        let undo = combo("Z", true, false, false);
        assert_eq!(store.get(&undo), Some(&KeyTarget::Action(ActionId::Undo)));
    }

    #[test]
    fn test_keymap_config_validate_valid() {
        let config = KeymapConfig {
            version: 1,
            bindings: vec![KeyBinding {
                key: "Z".into(),
                command: true,
                shift: false,
                alt: false,
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
                key: String::new(),
                command: false,
                shift: false,
                alt: false,
                target: KeyTarget::Action(ActionId::Save),
            }],
        };
        let errors = config.validate();
        assert!(errors.iter().any(|e| e.contains("key is empty")));
    }
}
