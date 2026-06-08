//! Preset persistence — save/load deck and channel presets from `.varda/presets/`.

use super::Workspace;
use crate::scene::{ChannelConfig, DeckConfig};
use anyhow::{Context, Result};

/// A loaded deck preset with its name and config.
#[derive(Debug, Clone)]
pub struct DeckPreset {
    pub name: String,
    pub config: DeckConfig,
}

/// A loaded channel preset with its name and config.
#[derive(Debug, Clone)]
pub struct ChannelPreset {
    pub name: String,
    pub config: ChannelConfig,
}

/// In-memory collection of loaded presets from disk.
pub struct PresetLibrary {
    pub deck_presets: Vec<DeckPreset>,
    pub channel_presets: Vec<ChannelPreset>,
}

impl PresetLibrary {
    /// Scan preset directories and load all valid JSON files.
    pub fn load(workspace: &Workspace) -> Self {
        let mut lib = Self {
            deck_presets: Vec::new(),
            channel_presets: Vec::new(),
        };
        lib.scan_dir(&workspace.deck_presets_dir(), true);
        lib.scan_dir(&workspace.channel_presets_dir(), false);
        lib
    }

    /// Save a deck preset to disk.
    pub fn save_deck_preset(workspace: &Workspace, name: &str, config: &DeckConfig) -> Result<()> {
        let errors = config.validate("deck_preset");
        for e in &errors {
            log::error!("Deck preset '{}' save: {}", name, e);
        }
        workspace.ensure_preset_dirs()?;
        let filename = sanitize_filename(name);
        let path = workspace.deck_presets_dir().join(&filename);
        let json =
            serde_json::to_string_pretty(config).context("Failed to serialize deck preset")?;
        super::atomic_write(&path, &json)?;
        log::info!("Saved deck preset '{}' to {}", name, path.display());
        Ok(())
    }

    /// Save a channel preset to disk.
    pub fn save_channel_preset(
        workspace: &Workspace,
        name: &str,
        config: &ChannelConfig,
    ) -> Result<()> {
        let errors = config.validate("channel_preset");
        for e in &errors {
            log::error!("Channel preset '{}' save: {}", name, e);
        }
        workspace.ensure_preset_dirs()?;
        let filename = sanitize_filename(name);
        let path = workspace.channel_presets_dir().join(&filename);
        let json =
            serde_json::to_string_pretty(config).context("Failed to serialize channel preset")?;
        super::atomic_write(&path, &json)?;
        log::info!("Saved channel preset '{}' to {}", name, path.display());
        Ok(())
    }

    /// Rescan directories to pick up new/removed presets.
    pub fn refresh(&mut self, workspace: &Workspace) {
        self.deck_presets.clear();
        self.channel_presets.clear();
        self.scan_dir(&workspace.deck_presets_dir(), true);
        self.scan_dir(&workspace.channel_presets_dir(), false);
    }

    fn scan_dir(&mut self, dir: &std::path::Path, is_deck: bool) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return, // Directory doesn't exist yet
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("Failed to read preset file {}: {}", path.display(), e);
                    continue;
                }
            };
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();

            if is_deck {
                match serde_json::from_str::<DeckConfig>(&content) {
                    Ok(config) => {
                        let warnings = config.validate(&format!("deck_preset '{}'", stem));
                        for w in &warnings {
                            log::warn!("Preset {}: {}", path.display(), w);
                        }
                        self.deck_presets.push(DeckPreset { name: stem, config });
                    }
                    Err(e) => log::warn!("Failed to parse deck preset {}: {}", path.display(), e),
                }
            } else {
                match serde_json::from_str::<ChannelConfig>(&content) {
                    Ok(config) => {
                        let warnings = config.validate(&format!("channel_preset '{}'", stem));
                        for w in &warnings {
                            log::warn!("Preset {}: {}", path.display(), w);
                        }
                        self.channel_presets
                            .push(ChannelPreset { name: stem, config });
                    }
                    Err(e) => {
                        log::warn!("Failed to parse channel preset {}: {}", path.display(), e)
                    }
                }
            }
        }
        // Sort by name for stable ordering
        if is_deck {
            self.deck_presets.sort_by(|a, b| a.name.cmp(&b.name));
        } else {
            self.channel_presets.sort_by(|a, b| a.name.cmp(&b.name));
        }
    }
}

/// Sanitize a user-provided name into a safe filename.
pub fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let truncated = if sanitized.len() > 64 {
        sanitized[..64].to_string()
    } else if sanitized.is_empty() {
        "preset".to_string()
    } else {
        sanitized
    };
    format!("{}.json", truncated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::*;

    fn sample_deck_config() -> DeckConfig {
        DeckConfig {
            uuid: crate::deck::generate_short_uuid(),
            name: "test_deck".to_string(),
            source: SourceConfig::SolidColor {
                color: [1.0, 0.0, 0.0, 1.0],
            },
            effects: vec![],
            opacity: 0.8,
            blend_mode: BlendModeConfig::Normal,
            mute: false,
            solo: false,
            z_index: 0,
            auto_transition: None,
            modulation: vec![],
            render_fps: crate::channel::DeckRenderFps::default(),
        }
    }

    fn sample_channel_config() -> ChannelConfig {
        ChannelConfig {
            uuid: crate::deck::generate_short_uuid(),
            name: "test_channel".to_string(),
            opacity: 1.0,
            blend_mode: BlendModeConfig::Normal,
            decks: vec![sample_deck_config()],
            effects: vec![],
        }
    }

    #[test]
    fn test_save_load_deck_preset_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().to_path_buf());
        let config = sample_deck_config();

        PresetLibrary::save_deck_preset(&ws, "my deck", &config).unwrap();

        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.deck_presets.len(), 1);
        assert_eq!(lib.deck_presets[0].name, "my_deck");
        assert_eq!(lib.deck_presets[0].config.name, "test_deck");
        assert_eq!(lib.deck_presets[0].config.opacity, 0.8);
    }

    #[test]
    fn test_save_load_channel_preset_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().to_path_buf());
        let config = sample_channel_config();

        PresetLibrary::save_channel_preset(&ws, "my channel", &config).unwrap();

        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.channel_presets.len(), 1);
        assert_eq!(lib.channel_presets[0].name, "my_channel");
        assert_eq!(lib.channel_presets[0].config.decks.len(), 1);
    }

    #[test]
    fn test_preset_library_scan_ignores_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().to_path_buf());
        ws.ensure_preset_dirs().unwrap();

        // Write a valid preset
        let config = sample_deck_config();
        PresetLibrary::save_deck_preset(&ws, "good", &config).unwrap();

        // Write an invalid JSON file
        std::fs::write(ws.deck_presets_dir().join("bad.json"), "not json").unwrap();

        // Write a non-json file (should be skipped entirely)
        std::fs::write(ws.deck_presets_dir().join("readme.txt"), "ignore me").unwrap();

        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.deck_presets.len(), 1);
        assert_eq!(lib.deck_presets[0].name, "good");
    }

    #[test]
    fn test_filename_sanitization() {
        assert_eq!(sanitize_filename("My Cool Preset!"), "My_Cool_Preset_.json");
        assert_eq!(sanitize_filename("hello-world_v2"), "hello-world_v2.json");
        assert_eq!(sanitize_filename(""), "preset.json");
        assert_eq!(sanitize_filename("a/b\\c:d"), "a_b_c_d.json");
        // Long name gets truncated
        let long = "a".repeat(100);
        let result = sanitize_filename(&long);
        assert!(result.len() <= 69); // 64 + ".json"
    }

    #[test]
    fn test_deck_preset_roundtrip_with_modulation() {
        use crate::modulation::ModulationSource;
        use crate::scene::{ModulationRecipe, ModulationRecipeAssignment};

        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().to_path_buf());
        let mut config = sample_deck_config();
        config.modulation = vec![ModulationRecipe {
            source_uuid: "test0001".to_string(),
            source: ModulationSource::sine_lfo(2.0),
            assignments: vec![
                ModulationRecipeAssignment {
                    param: "brightness".into(),
                    amount: 0.5,
                    component: None,
                },
                ModulationRecipeAssignment {
                    param: "fx0:amount".into(),
                    amount: 0.3,
                    component: None,
                },
            ],
        }];
        PresetLibrary::save_deck_preset(&ws, "mod_test", &config).unwrap();
        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.deck_presets.len(), 1);
        let loaded = &lib.deck_presets[0].config;
        assert_eq!(loaded.modulation.len(), 1);
        assert_eq!(loaded.modulation[0].assignments.len(), 2);
        assert_eq!(loaded.modulation[0].assignments[0].param, "brightness");
        assert_eq!(loaded.modulation[0].assignments[0].amount, 0.5);
    }

    #[test]
    fn test_modulation_recipe_serde() {
        use crate::modulation::ModulationSource;
        use crate::scene::{ModulationRecipe, ModulationRecipeAssignment};

        let mut config = sample_deck_config();
        config.modulation = vec![ModulationRecipe {
            source_uuid: "test0002".to_string(),
            source: ModulationSource::adsr(0.1, 0.2, 0.7, 0.3),
            assignments: vec![ModulationRecipeAssignment {
                param: "scale".into(),
                amount: -0.8,
                component: Some(1),
            }],
        }];
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"modulation\""));
        assert!(json.contains("\"scale\""));
        let deser: DeckConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.modulation.len(), 1);
        assert_eq!(deser.modulation[0].assignments[0].component, Some(1));
    }

    #[test]
    fn test_deck_config_without_modulation_deserializes() {
        // Old presets without modulation field should deserialize fine
        let json = r#"{"name":"old","source":{"type":"SolidColor","color":[1,0,0,1]},"opacity":1.0,"blend_mode":"normal"}"#;
        let config: DeckConfig = serde_json::from_str(json).unwrap();
        assert!(config.modulation.is_empty());
    }

    #[test]
    fn test_preset_validation_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().to_path_buf());
        // Deck with invalid opacity — should still save (logs error but doesn't block)
        let mut config = sample_deck_config();
        config.opacity = 5.0;
        assert!(PresetLibrary::save_deck_preset(&ws, "bad_opacity", &config).is_ok());
        // Verify it's loadable despite validation warnings
        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.deck_presets.len(), 1);
    }
}
