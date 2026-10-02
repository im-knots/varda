//! Deck and channel presets in `.varda/presets/`, plus read-only deck
//! presets shipped in shader library folders (`presets/decks/`).

use super::Workspace;
use crate::scene::{ChannelConfig, DeckConfig};
use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub struct DeckPreset {
    pub name: String,
    pub config: DeckConfig,
    /// Shipped with Varda rather than saved by the user.
    pub built_in: bool,
}

#[derive(Debug, Clone)]
pub struct ChannelPreset {
    pub name: String,
    pub config: ChannelConfig,
}

/// Presets loaded from disk.
pub struct PresetLibrary {
    pub deck_presets: Vec<DeckPreset>,
    pub channel_presets: Vec<ChannelPreset>,
    /// Folders of shipped deck presets, scanned again on refresh.
    built_in_dirs: Vec<std::path::PathBuf>,
}

/// Where a shader library folder keeps its shipped deck presets.
pub fn built_in_deck_presets_dir(shader_library: &std::path::Path) -> std::path::PathBuf {
    shader_library.join("presets").join("decks")
}

impl PresetLibrary {
    /// Load every valid JSON file in the preset directories.
    pub fn load(workspace: &Workspace) -> Self {
        Self::load_with_built_in(workspace, Vec::new())
    }

    /// [`Self::load`], plus the read-only deck presets in `built_in_dirs`. A
    /// user preset with the same name as a shipped one replaces it.
    pub fn load_with_built_in(
        workspace: &Workspace,
        built_in_dirs: Vec<std::path::PathBuf>,
    ) -> Self {
        let mut lib = Self {
            deck_presets: Vec::new(),
            channel_presets: Vec::new(),
            built_in_dirs,
        };
        lib.scan_all(workspace);
        lib
    }

    fn scan_all(&mut self, workspace: &Workspace) {
        self.scan_dir(&workspace.deck_presets_dir(), true, false);
        self.scan_dir(&workspace.channel_presets_dir(), false, false);
        for dir in self.built_in_dirs.clone() {
            self.scan_dir(&dir, true, true);
        }
        let user: std::collections::HashSet<String> = self
            .deck_presets
            .iter()
            .filter(|p| !p.built_in)
            .map(|p| p.name.clone())
            .collect();
        self.deck_presets
            .retain(|p| !p.built_in || !user.contains(&p.name));
        self.deck_presets.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Save a deck preset to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if `presets/decks/` cannot be created, `config` cannot
    /// be serialized, or the write fails.
    pub fn save_deck_preset(workspace: &Workspace, name: &str, config: &DeckConfig) -> Result<()> {
        let errors = config.validate("deck_preset");
        for e in &errors {
            log::error!("Deck preset '{name}' save: {e}");
        }
        workspace.ensure_preset_dirs()?;
        let filename = sanitize_filename(name);
        let path = workspace.deck_presets_dir().join(&filename);
        let json =
            serde_json::to_string_pretty(config).context("Failed to serialize deck preset")?;
        crate::files::atomic_write(&path, &json)?;
        log::info!("Saved deck preset '{}' to {}", name, path.display());
        Ok(())
    }

    /// Save a channel preset to disk.
    ///
    /// # Errors
    ///
    /// Returns an error if `presets/channels/` cannot be created, `config`
    /// cannot be serialized, or the write fails.
    pub fn save_channel_preset(
        workspace: &Workspace,
        name: &str,
        config: &ChannelConfig,
    ) -> Result<()> {
        let errors = config.validate("channel_preset");
        for e in &errors {
            log::error!("Channel preset '{name}' save: {e}");
        }
        workspace.ensure_preset_dirs()?;
        let filename = sanitize_filename(name);
        let path = workspace.channel_presets_dir().join(&filename);
        let json =
            serde_json::to_string_pretty(config).context("Failed to serialize channel preset")?;
        crate::files::atomic_write(&path, &json)?;
        log::info!("Saved channel preset '{}' to {}", name, path.display());
        Ok(())
    }

    /// Rescan the preset directories.
    pub fn refresh(&mut self, workspace: &Workspace) {
        self.deck_presets.clear();
        self.channel_presets.clear();
        self.scan_all(workspace);
    }

    fn scan_dir(&mut self, dir: &std::path::Path, is_deck: bool, built_in: bool) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
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
                    Ok(mut config) => {
                        config.canonicalize_modulation();
                        let warnings = config.validate(&format!("deck_preset '{stem}'"));
                        for w in &warnings {
                            log::warn!("Preset {}: {}", path.display(), w);
                        }
                        self.deck_presets.push(DeckPreset {
                            name: stem,
                            config,
                            built_in,
                        });
                    }
                    Err(e) => log::warn!("Failed to parse deck preset {}: {}", path.display(), e),
                }
            } else {
                match serde_json::from_str::<ChannelConfig>(&content) {
                    Ok(mut config) => {
                        config.canonicalize_modulation();
                        let warnings = config.validate(&format!("channel_preset '{stem}'"));
                        for w in &warnings {
                            log::warn!("Preset {}: {}", path.display(), w);
                        }
                        self.channel_presets
                            .push(ChannelPreset { name: stem, config });
                    }
                    Err(e) => {
                        log::warn!("Failed to parse channel preset {}: {}", path.display(), e);
                    }
                }
            }
        }
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
    format!("{truncated}.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::*;

    fn sample_deck_config() -> DeckConfig {
        DeckConfig {
            uuid: crate::ids::generate_short_uuid(),
            name: "test_deck".to_string(),
            source: crate::solid_color::SolidColor::config_for([1.0, 0.0, 0.0, 1.0]),
            effects: vec![],
            opacity: 0.8,
            transparent: false,
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
            uuid: crate::ids::generate_short_uuid(),
            name: "test_channel".to_string(),
            opacity: 1.0,
            blend_mode: BlendModeConfig::Normal,
            decks: vec![sample_deck_config()],
            effects: vec![],
            modulation: vec![],
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

        let config = sample_deck_config();
        PresetLibrary::save_deck_preset(&ws, "good", &config).unwrap();

        std::fs::write(ws.deck_presets_dir().join("bad.json"), "not json").unwrap();

        // Non-JSON files are skipped.
        std::fs::write(ws.deck_presets_dir().join("readme.txt"), "ignore me").unwrap();

        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.deck_presets.len(), 1);
        assert_eq!(lib.deck_presets[0].name, "good");
    }

    #[test]
    fn shipped_presets_are_listed_and_a_user_preset_of_the_same_name_wins() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().join("show"));
        let library = dir.path().join("shaders");
        let shipped = built_in_deck_presets_dir(&library);
        std::fs::create_dir_all(&shipped).unwrap();
        let json = serde_json::to_string(&sample_deck_config()).unwrap();
        std::fs::write(shipped.join("World A.json"), &json).unwrap();
        std::fs::write(shipped.join("World B.json"), &json).unwrap();
        PresetLibrary::save_deck_preset(&ws, "World_B", &sample_deck_config()).unwrap();
        std::fs::rename(
            ws.deck_presets_dir().join("World_B.json"),
            ws.deck_presets_dir().join("World B.json"),
        )
        .unwrap();

        let lib = PresetLibrary::load_with_built_in(&ws, vec![shipped]);
        let listed: Vec<(&str, bool)> = lib
            .deck_presets
            .iter()
            .map(|p| (p.name.as_str(), p.built_in))
            .collect();
        assert_eq!(listed, [("World A", true), ("World B", false)]);
    }

    #[test]
    fn every_shipped_preset_loads_and_names_a_shipped_shader() {
        let library = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders");
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().to_path_buf());
        let shipped = built_in_deck_presets_dir(&library);
        let files = std::fs::read_dir(&shipped).map_or(0, |d| {
            d.flatten()
                .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
                .count()
        });
        let lib = PresetLibrary::load_with_built_in(&ws, vec![shipped]);
        assert_eq!(
            lib.deck_presets.len(),
            files,
            "a shipped preset failed to parse"
        );
        for preset in &lib.deck_presets {
            let name =
                preset.config.source.str("name").unwrap_or_else(|| {
                    panic!("{}: shipped presets name their shader", preset.name)
                });
            assert!(
                library.join(format!("{name}.fs")).is_file(),
                "{}: no shader {name}",
                preset.name
            );
        }
    }

    #[test]
    fn test_filename_sanitization() {
        assert_eq!(sanitize_filename("My Cool Preset!"), "My_Cool_Preset_.json");
        assert_eq!(sanitize_filename("hello-world_v2"), "hello-world_v2.json");
        assert_eq!(sanitize_filename(""), "preset.json");
        assert_eq!(sanitize_filename("a/b\\c:d"), "a_b_c_d.json");
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
            timebase: crate::timebase::Timebase::FreeRun,
            assignments: vec![
                ModulationRecipeAssignment {
                    param: "brightness".into(),
                    amount: 0.5,
                    legacy_component: None,
                },
                ModulationRecipeAssignment {
                    param: "fx0:amount".into(),
                    amount: 0.3,
                    legacy_component: None,
                },
            ],
        }];
        PresetLibrary::save_deck_preset(&ws, "mod_test", &config).unwrap();
        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.deck_presets.len(), 1);
        let loaded = &lib.deck_presets[0].config;
        assert_eq!(loaded.modulation.len(), 1);
        assert_eq!(loaded.modulation[0].assignments.len(), 2);
        // A pre-v8 relative name is read in the current spelling.
        assert_eq!(
            loaded.modulation[0].assignments[0].param,
            "param/brightness"
        );
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
            timebase: crate::timebase::Timebase::FreeRun,
            assignments: vec![ModulationRecipeAssignment {
                param: "scale".into(),
                amount: -0.8,
                legacy_component: Some(1),
            }],
        }];
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"modulation\""));
        assert!(json.contains("\"scale\""));
        // A component index is read from older files, never written.
        assert!(!json.contains("\"component\""));
        let deser: DeckConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deser.modulation.len(), 1);
        assert_eq!(deser.modulation[0].assignments[0].legacy_component, None);

        let old = json.replace("\"amount\":-0.8", "\"amount\":-0.8,\"component\":1");
        let deser: DeckConfig = serde_json::from_str(&old).unwrap();
        assert_eq!(deser.modulation[0].assignments[0].legacy_component, Some(1));
    }

    #[test]
    fn test_deck_config_without_modulation_deserializes() {
        // Older presets have no modulation field.
        let json = r#"{"name":"old","source":{"type":"SolidColor","color":[1,0,0,1]},"opacity":1.0,"blend_mode":"normal"}"#;
        let config: DeckConfig = serde_json::from_str(json).unwrap();
        assert!(config.modulation.is_empty());
    }

    #[test]
    fn test_preset_validation_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::new(dir.path().to_path_buf());
        // Invalid opacity still saves; validation only logs.
        let mut config = sample_deck_config();
        config.opacity = 5.0;
        assert!(PresetLibrary::save_deck_preset(&ws, "bad_opacity", &config).is_ok());
        let lib = PresetLibrary::load(&ws);
        assert_eq!(lib.deck_presets.len(), 1);
    }
}
