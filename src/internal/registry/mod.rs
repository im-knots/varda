use crate::isf::ISFShader;
use anyhow::{Context, Result};
use notify::{Event, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use walkdir::WalkDir;

/// Event types for shader changes
#[derive(Debug, Clone)]
pub enum ShaderEvent {
    /// A shader was added or modified
    Changed(PathBuf),
    /// A shader was removed
    Removed(PathBuf),
    /// A shader failed to load/reload
    Error(PathBuf, String),
}

/// Registry of discovered ISF shaders with hot-reload support
pub struct ShaderRegistry {
    /// Map of shader name to shader data
    shaders: HashMap<String, ISFShader>,

    /// Map of file path to shader name (for hot-reload lookup)
    path_to_name: HashMap<PathBuf, String>,

    /// Map of shader name to every file that provides it. Lets hot-reload and
    /// removal re-resolve the correct winner instead of clobbering the active
    /// shader, keeping library precedence stable across a running session, not
    /// just at the initial scan.
    name_to_paths: HashMap<String, Vec<PathBuf>>,

    /// Library paths being watched. Order is priority: later paths win name
    /// collisions (built-in, then workdir, then any extra override dirs).
    library_paths: Vec<PathBuf>,

    /// File watcher (kept alive to maintain watch)
    #[allow(dead_code)]
    watcher: Option<notify::RecommendedWatcher>,

    /// Channel for receiving file change events
    change_receiver: Option<Receiver<notify::Result<Event>>>,
}

impl ShaderRegistry {
    /// Create a new shader registry
    pub fn new() -> Self {
        Self {
            shaders: HashMap::new(),
            path_to_name: HashMap::new(),
            name_to_paths: HashMap::new(),
            library_paths: Vec::new(),
            watcher: None,
            change_receiver: None,
        }
    }

    /// Register a directory to scan for shaders.
    ///
    /// The path must already exist. A missing directory is an error, not
    /// something to create: silently `mkdir -p`ing a mistyped or unmounted
    /// directory just yields an empty library and hides the mistake, so
    /// callers get an `Err` and decide whether to warn-and-skip. Duplicate
    /// paths are ignored so overlapping libraries aren't scanned or watched
    /// twice.
    pub fn add_library_path<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref().to_path_buf();

        if !path.is_dir() {
            anyhow::bail!("Shader library path does not exist: {}", path.display());
        }

        // Canonicalize so `./shaders`, `shaders`, and an absolute path to the
        // same directory dedup against each other (and so watcher event paths,
        // which arrive absolute, line up with library paths for precedence).
        let path = path.canonicalize().unwrap_or(path);

        if self.library_paths.contains(&path) {
            log::debug!("Shader library already registered: {}", path.display());
            return Ok(());
        }

        self.library_paths.push(path);
        Ok(())
    }

    /// Scan all library paths for ISF shaders
    pub fn scan(&mut self) -> Result<usize> {
        self.shaders.clear();
        self.path_to_name.clear();
        self.name_to_paths.clear();

        for lib_path in &self.library_paths {
            log::info!("Scanning library: {}", lib_path.display());

            for entry in WalkDir::new(lib_path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
            {
                let path = entry.path();

                // Only process .fs and .comp files
                let ext = path.extension().and_then(|s| s.to_str());
                if ext != Some("fs") && ext != Some("comp") {
                    continue;
                }

                match ISFShader::from_file(path) {
                    Ok(shader) => {
                        let name = shader.name();
                        log::info!("  Loaded: {} ({})", name, path.display());
                        self.path_to_name.insert(path.to_path_buf(), name.clone());
                        // Libraries are scanned in priority order, so the last
                        // write for a name is the highest-priority provider.
                        self.name_to_paths
                            .entry(name.clone())
                            .or_default()
                            .push(path.to_path_buf());
                        self.shaders.insert(name, shader);
                    }
                    Err(e) => {
                        log::warn!("  ✗ Failed to load {}: {}", path.display(), e);
                    }
                }
            }
        }

        // Count unique shaders, not files loaded: a name overridden across
        // libraries collapses to one entry, so counting files would inflate.
        let count = self.shaders.len();
        log::info!(
            "Loaded {} shaders from {} libraries",
            count,
            self.library_paths.len()
        );
        Ok(count)
    }

    /// Start watching library paths for changes
    pub fn start_watching(&mut self) -> Result<()> {
        let (tx, rx) = mpsc::channel();

        let mut watcher =
            notify::recommended_watcher(tx).context("Failed to create file watcher")?;

        for lib_path in &self.library_paths {
            if lib_path.exists() {
                watcher
                    .watch(lib_path, RecursiveMode::Recursive)
                    .with_context(|| format!("Failed to watch path: {}", lib_path.display()))?;
                log::info!("Watching library path: {}", lib_path.display());
            }
        }

        self.watcher = Some(watcher);
        self.change_receiver = Some(rx);

        Ok(())
    }

    /// Check for and process file changes (non-blocking)
    /// Returns a list of shader events that occurred
    pub fn poll_changes(&mut self) -> Vec<ShaderEvent> {
        // First, collect all pending notify events without holding a borrow
        let pending_events: Vec<(notify::EventKind, Vec<PathBuf>)> = {
            let receiver = match &self.change_receiver {
                Some(r) => r,
                None => return Vec::new(),
            };

            let mut collected = Vec::new();
            loop {
                match receiver.try_recv() {
                    Ok(Ok(event)) => {
                        collected.push((event.kind, event.paths));
                    }
                    Ok(Err(e)) => {
                        log::error!("File watcher error: {}", e);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        log::warn!("File watcher disconnected");
                        break;
                    }
                }
            }
            collected
        };

        // Check if we got disconnected
        if let Some(receiver) = &self.change_receiver {
            if receiver.try_recv().is_err() {
                // Don't clear on Empty, only on Disconnected
            }
        }

        // Now process the collected events with mutable self access
        let mut shader_events = Vec::new();

        for (kind, paths) in pending_events {
            for path in paths {
                // Only process .fs and .comp files
                let ext = path.extension().and_then(|s| s.to_str());
                if ext != Some("fs") && ext != Some("comp") {
                    continue;
                }

                match kind {
                    notify::EventKind::Create(_) | notify::EventKind::Modify(_) => {
                        // Reload the shader
                        match self.reload_shader(&path) {
                            Ok(()) => {
                                shader_events.push(ShaderEvent::Changed(path));
                            }
                            Err(e) => {
                                let err_msg = format!("{}", e);
                                log::warn!(
                                    "Failed to reload shader {}: {}",
                                    path.display(),
                                    err_msg
                                );
                                shader_events.push(ShaderEvent::Error(path, err_msg));
                            }
                        }
                    }
                    notify::EventKind::Remove(_) => {
                        // Drop this file as a provider, then re-resolve the name.
                        // If a lower-priority file still provides it (e.g. a
                        // built-in that was shadowed by this override), it's
                        // promoted back instead of the shader vanishing.
                        if let Some(name) = self.path_to_name.remove(&path) {
                            self.forget_provider(&name, &path);
                            if self.resolve_winner(&name) {
                                log::info!(
                                    "Removed {}; restored shadowed provider",
                                    path.display()
                                );
                                shader_events.push(ShaderEvent::Changed(path));
                            } else {
                                log::info!("Removed shader: {}", name);
                                shader_events.push(ShaderEvent::Removed(path));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }

        shader_events
    }

    /// Reload a single shader from disk.
    ///
    /// Respects library precedence: a file only becomes the active shader if it
    /// is the highest-priority provider of its name. Editing a built-in shader
    /// that a higher-priority library currently overrides reloads it into the
    /// registry's knowledge but does not clobber the override.
    fn reload_shader(&mut self, path: &Path) -> Result<()> {
        let shader = ISFShader::from_file(path)?;
        let name = shader.name();

        // If this file previously provided a different name (its NAME field was
        // edited), stop attributing it to the old name and re-resolve that one.
        if let Some(old_name) = self.path_to_name.get(path).cloned() {
            if old_name != name {
                self.forget_provider(&old_name, path);
                self.resolve_winner(&old_name);
            }
        }

        self.path_to_name.insert(path.to_path_buf(), name.clone());
        self.record_provider(&name, path);

        if self.is_highest_priority_provider(&name, path) {
            log::info!("Hot-reloaded shader: {} ({})", name, path.display());
            self.shaders.insert(name, shader);
        } else {
            log::info!(
                "Reloaded shadowed shader, override kept: {} ({})",
                name,
                path.display()
            );
            // The override should already be active; make sure of it.
            if !self.shaders.contains_key(&name) {
                self.resolve_winner(&name);
            }
        }

        Ok(())
    }

    /// Priority of a path = index of the highest-index (last-added) library path
    /// that contains it, +1 so an unknown path is strictly lowest. Later
    /// libraries win, matching the built-in, workdir, override-dir hierarchy.
    fn path_priority(&self, path: &Path) -> usize {
        self.library_paths
            .iter()
            .enumerate()
            .filter(|(_, lib)| path.starts_with(lib))
            .map(|(i, _)| i + 1)
            .max()
            .unwrap_or(0)
    }

    /// Record `path` as a provider of `name` (deduped).
    fn record_provider(&mut self, name: &str, path: &Path) {
        let providers = self.name_to_paths.entry(name.to_string()).or_default();
        if !providers.iter().any(|p| p == path) {
            providers.push(path.to_path_buf());
        }
    }

    /// Drop `path` as a provider of `name`.
    fn forget_provider(&mut self, name: &str, path: &Path) {
        if let Some(providers) = self.name_to_paths.get_mut(name) {
            providers.retain(|p| p != path);
        }
    }

    /// Is `path` the highest-priority provider currently registered for `name`?
    fn is_highest_priority_provider(&self, name: &str, path: &Path) -> bool {
        match self.name_to_paths.get(name) {
            Some(providers) => providers
                .iter()
                .max_by_key(|p| self.path_priority(p))
                .map(|winner| winner.as_path() == path)
                .unwrap_or(true),
            None => true,
        }
    }

    /// Re-pick the active shader for `name` from its remaining providers,
    /// loading the highest-priority one. Returns whether the shader still
    /// exists afterward. Ties resolve to the last-recorded provider.
    fn resolve_winner(&mut self, name: &str) -> bool {
        let providers = match self.name_to_paths.get(name) {
            Some(p) if !p.is_empty() => p.clone(),
            _ => {
                self.name_to_paths.remove(name);
                self.shaders.remove(name);
                return false;
            }
        };

        let winner = providers
            .iter()
            .max_by_key(|p| self.path_priority(p))
            .cloned()
            .expect("providers is non-empty");

        match ISFShader::from_file(&winner) {
            Ok(shader) => {
                self.shaders.insert(name.to_string(), shader);
                true
            }
            Err(e) => {
                log::warn!(
                    "Failed to load fallback provider {} for shader {}: {}",
                    winner.display(),
                    name,
                    e
                );
                self.shaders.remove(name);
                false
            }
        }
    }

    /// Get a shader by name
    pub fn get(&self, name: &str) -> Option<&ISFShader> {
        self.shaders.get(name)
    }

    /// Get all shader names
    pub fn shader_names(&self) -> Vec<String> {
        self.shaders.keys().cloned().collect()
    }

    /// Get all generators
    pub fn generators(&self) -> Vec<&ISFShader> {
        self.shaders
            .values()
            .filter(|s| s.metadata.is_generator())
            .collect()
    }

    /// Get all filters (excludes transitions)
    pub fn filters(&self) -> Vec<&ISFShader> {
        self.shaders
            .values()
            .filter(|s| s.metadata.is_filter() && !s.metadata.is_transition())
            .collect()
    }

    /// Get all transition shaders
    pub fn transitions(&self) -> Vec<&ISFShader> {
        self.shaders
            .values()
            .filter(|s| s.metadata.is_transition())
            .collect()
    }

    /// Get shader count
    pub fn count(&self) -> usize {
        self.shaders.len()
    }
}

impl Default for ShaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Get the bundled shader path relative to the current executable.
/// Used when Varda is packaged as a .app (macOS) or AppImage (Linux).
pub fn get_bundled_shader_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;

    // macOS .app: exe is at Contents/MacOS/varda, shaders at Contents/Resources/shaders
    #[cfg(target_os = "macos")]
    {
        let app_resources = exe_dir.join("../Resources/shaders");
        if app_resources.is_dir() {
            return Some(app_resources);
        }
    }

    // Linux portable tarball: exe is at bin/varda, shaders at shaders/
    #[cfg(target_os = "linux")]
    {
        let tarball_shaders = exe_dir.join("../shaders");
        if tarball_shaders.is_dir() {
            return Some(tarball_shaders);
        }
    }

    // Windows portable: shaders/ next to varda.exe
    #[cfg(target_os = "windows")]
    {
        let exe_shaders = exe_dir.join("shaders");
        if exe_shaders.is_dir() {
            return Some(exe_shaders);
        }
    }

    None
}

/// Get the default library paths for the current platform
pub fn get_default_library_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let mut path = PathBuf::from(home);
            path.push("Library");
            path.push("Application Support");
            path.push("Varda");
            path.push("Shaders");
            paths.push(path);
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            let mut path = PathBuf::from(home);
            path.push(".local");
            path.push("share");
            path.push("varda");
            path.push("shaders");
            paths.push(path);
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let mut path = PathBuf::from(appdata);
            path.push("Varda");
            path.push("Shaders");
            paths.push(path);
        }
    }

    paths
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write_shader(dir: &Path, name: &str, category: &str, is_filter: bool) {
        let input = if is_filter {
            r#"{"NAME": "inputImage", "TYPE": "image"}"#
        } else {
            r#"{"NAME": "brightness", "TYPE": "float"}"#
        };
        let content = format!(
            "/*{{\n\"CATEGORIES\": [\"{category}\"],\n\"INPUTS\": [{input}]\n}}*/\nvoid main() {{}}"
        );
        fs::write(dir.join(format!("{name}.fs")), content).unwrap();
    }

    #[test]
    fn new_registry_is_empty() {
        let reg = ShaderRegistry::new();
        assert_eq!(reg.count(), 0);
        assert!(reg.shader_names().is_empty());
        assert!(reg.generators().is_empty());
        assert!(reg.filters().is_empty());
        assert!(reg.transitions().is_empty());
    }

    #[test]
    fn scan_empty_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        let count = reg.scan().unwrap();
        assert_eq!(count, 0);
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn scan_finds_shaders() {
        let tmp = tempfile::tempdir().unwrap();
        write_shader(tmp.path(), "TestGen", "Generator", false);
        write_shader(tmp.path(), "TestFilter", "Filter", true);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        let count = reg.scan().unwrap();

        assert_eq!(count, 2);
        assert_eq!(reg.count(), 2);
        assert!(reg.get("TestGen").is_some());
        assert!(reg.get("TestFilter").is_some());
    }

    #[test]
    fn generators_and_filters_classified() {
        let tmp = tempfile::tempdir().unwrap();
        write_shader(tmp.path(), "Gen1", "Generator", false);
        write_shader(tmp.path(), "Gen2", "Generator", false);
        write_shader(tmp.path(), "Filt1", "Filter", true);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        reg.scan().unwrap();

        assert_eq!(reg.generators().len(), 2);
        assert_eq!(reg.filters().len(), 1);
    }

    #[test]
    fn transitions_classified() {
        let tmp = tempfile::tempdir().unwrap();
        // Transition = has "Transition" category + image input
        let content = r#"/*{
"CATEGORIES": ["Transition"],
"INPUTS": [{"NAME": "inputImage", "TYPE": "image"}, {"NAME": "startImage", "TYPE": "image"}]
}*/
void main() {}"#;
        fs::write(tmp.path().join("Dissolve.fs"), content).unwrap();
        write_shader(tmp.path(), "Gen1", "Generator", false);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        reg.scan().unwrap();

        assert_eq!(reg.transitions().len(), 1);
        assert_eq!(reg.generators().len(), 1);
        // Transitions are filters too, but filters() excludes transitions
        assert_eq!(reg.filters().len(), 0);
    }

    #[test]
    fn get_nonexistent_returns_none() {
        let reg = ShaderRegistry::new();
        assert!(reg.get("DoesNotExist").is_none());
    }

    #[test]
    fn ignores_non_fs_files() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("readme.txt"), "not a shader").unwrap();
        fs::write(tmp.path().join("data.json"), "{}").unwrap();
        write_shader(tmp.path(), "RealShader", "Generator", false);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        let count = reg.scan().unwrap();

        assert_eq!(count, 1);
    }

    #[test]
    fn skips_malformed_shaders() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("bad.fs"), "not valid ISF content").unwrap();
        write_shader(tmp.path(), "Good", "Generator", false);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        let count = reg.scan().unwrap();

        assert_eq!(count, 1);
        assert!(reg.get("Good").is_some());
    }

    #[test]
    fn scan_subdirectories() {
        let tmp = tempfile::tempdir().unwrap();
        let sub = tmp.path().join("sub");
        fs::create_dir(&sub).unwrap();
        write_shader(&sub, "Nested", "Generator", false);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        let count = reg.scan().unwrap();

        assert_eq!(count, 1);
        assert!(reg.get("Nested").is_some());
    }

    #[test]
    fn rescan_clears_and_reloads() {
        let tmp = tempfile::tempdir().unwrap();
        write_shader(tmp.path(), "S1", "Generator", false);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(tmp.path()).unwrap();
        reg.scan().unwrap();
        assert_eq!(reg.count(), 1);

        // Add another shader and rescan
        write_shader(tmp.path(), "S2", "Filter", true);
        let count = reg.scan().unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn add_nonexistent_path_errors() {
        let mut reg = ShaderRegistry::new();
        let result = reg.add_library_path("/nonexistent/path/that/should/not/exist");
        assert!(result.is_err());
    }

    #[test]
    fn multiple_library_paths_merge() {
        let lib_a = tempfile::tempdir().unwrap();
        let lib_b = tempfile::tempdir().unwrap();
        write_shader(lib_a.path(), "ShaderA", "Generator", false);
        write_shader(lib_b.path(), "ShaderB", "Filter", true);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(lib_a.path()).unwrap();
        reg.add_library_path(lib_b.path()).unwrap();
        let count = reg.scan().unwrap();

        assert_eq!(count, 2);
        assert!(reg.get("ShaderA").is_some());
        assert!(reg.get("ShaderB").is_some());
    }

    #[test]
    fn later_library_path_overrides_by_name() {
        let builtin = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        // Both dirs have a shader named "Glow" but with different categories
        write_shader(builtin.path(), "Glow", "Generator", false);
        write_shader(user.path(), "Glow", "Filter", true);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(builtin.path()).unwrap();
        reg.add_library_path(user.path()).unwrap();
        reg.scan().unwrap();

        // Should have exactly 1 shader named "Glow" (user version wins)
        assert_eq!(reg.count(), 1);
        let glow = reg.get("Glow").unwrap();
        assert!(
            glow.metadata.is_filter(),
            "User shader should override builtin"
        );
    }

    #[test]
    fn skips_nonexistent_optional_paths_gracefully() {
        let real = tempfile::tempdir().unwrap();
        write_shader(real.path(), "Real", "Generator", false);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(real.path()).unwrap();
        // Don't add a nonexistent path — just verify that only real paths load
        let count = reg.scan().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn get_default_library_paths_returns_platform_path() {
        let paths = get_default_library_paths();
        // Should return exactly one path on macOS, Linux (when HOME is set), or Windows (when APPDATA is set)
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        if std::env::var_os("HOME").is_some() {
            assert_eq!(paths.len(), 1);
            let path_str = paths[0].to_string_lossy();
            #[cfg(target_os = "macos")]
            assert!(path_str.contains("Library/Application Support/Varda/Shaders"));
            #[cfg(target_os = "linux")]
            assert!(path_str.contains(".local/share/varda/shaders"));
        }
        #[cfg(target_os = "windows")]
        if std::env::var_os("APPDATA").is_some() {
            assert_eq!(paths.len(), 1);
            let path_str = paths[0].to_string_lossy();
            assert!(path_str.contains("Varda\\Shaders"));
        }
    }

    /// Path a scanned shader file resolves to (library paths are canonicalized
    /// on registration, so reconstructed paths must be too).
    fn shader_file(dir: &Path, name: &str) -> PathBuf {
        dir.canonicalize().unwrap().join(format!("{name}.fs"))
    }

    /// Build a builtin (generator) + user-override (filter) pair named "Glow"
    /// across two library paths, scan, and confirm the override wins initially.
    fn overridden_glow() -> (tempfile::TempDir, tempfile::TempDir, ShaderRegistry) {
        let builtin = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        write_shader(builtin.path(), "Glow", "Generator", false);
        write_shader(user.path(), "Glow", "Filter", true);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(builtin.path()).unwrap();
        reg.add_library_path(user.path()).unwrap();
        reg.scan().unwrap();
        assert!(
            reg.get("Glow").unwrap().metadata.is_filter(),
            "user override should win at scan"
        );
        (builtin, user, reg)
    }

    #[test]
    fn hot_reload_of_shadowed_builtin_keeps_override() {
        let (builtin, _user, mut reg) = overridden_glow();
        // Simulate the shadowed built-in file being touched/edited.
        reg.reload_shader(&shader_file(builtin.path(), "Glow"))
            .unwrap();
        assert!(
            reg.get("Glow").unwrap().metadata.is_filter(),
            "editing the shadowed built-in must not clobber the override"
        );
    }

    #[test]
    fn hot_reload_of_override_stays_active() {
        let (_builtin, user, mut reg) = overridden_glow();
        // The winning override reloads as itself and stays active.
        reg.reload_shader(&shader_file(user.path(), "Glow")).unwrap();
        assert!(reg.get("Glow").unwrap().metadata.is_filter());
    }

    #[test]
    fn removing_override_restores_shadowed_builtin() {
        let (_builtin, user, mut reg) = overridden_glow();
        let override_path = shader_file(user.path(), "Glow");
        // Mirror what poll_changes does on a Remove event.
        reg.path_to_name.remove(&override_path);
        reg.forget_provider("Glow", &override_path);
        assert!(reg.resolve_winner("Glow"), "built-in should be promoted");
        assert!(
            !reg.get("Glow").unwrap().metadata.is_filter(),
            "the shadowed built-in generator should be restored"
        );
    }

    #[test]
    fn removing_shadowed_builtin_keeps_override() {
        let (builtin, _user, mut reg) = overridden_glow();
        let builtin_path = shader_file(builtin.path(), "Glow");
        // Deleting the losing (shadowed) file must not disturb the winner.
        reg.path_to_name.remove(&builtin_path);
        reg.forget_provider("Glow", &builtin_path);
        assert!(reg.resolve_winner("Glow"));
        assert!(
            reg.get("Glow").unwrap().metadata.is_filter(),
            "override must survive removal of the shadowed built-in"
        );
    }

    #[test]
    fn removing_sole_provider_drops_shader() {
        let dir = tempfile::tempdir().unwrap();
        write_shader(dir.path(), "Solo", "Generator", false);
        let mut reg = ShaderRegistry::new();
        reg.add_library_path(dir.path()).unwrap();
        reg.scan().unwrap();

        let path = shader_file(dir.path(), "Solo");
        reg.path_to_name.remove(&path);
        reg.forget_provider("Solo", &path);
        assert!(!reg.resolve_winner("Solo"), "no providers left");
        assert!(reg.get("Solo").is_none());
    }

    #[test]
    fn scan_count_is_unique_not_files() {
        let builtin = tempfile::tempdir().unwrap();
        let user = tempfile::tempdir().unwrap();
        // Two files, same shader name across libraries, one unique shader.
        write_shader(builtin.path(), "Glow", "Generator", false);
        write_shader(user.path(), "Glow", "Filter", true);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(builtin.path()).unwrap();
        reg.add_library_path(user.path()).unwrap();
        let count = reg.scan().unwrap();

        assert_eq!(count, 1, "override collapses to one shader, not two files");
        assert_eq!(count, reg.count());
    }

    #[test]
    fn add_library_path_does_not_create_missing_dir() {
        let parent = tempfile::tempdir().unwrap();
        let missing = parent.path().join("not-there");

        let mut reg = ShaderRegistry::new();
        assert!(reg.add_library_path(&missing).is_err());
        assert!(!missing.exists(), "must not mkdir a missing library path");
    }

    #[test]
    fn add_library_path_dedups_same_dir() {
        let dir = tempfile::tempdir().unwrap();
        write_shader(dir.path(), "Once", "Generator", false);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(dir.path()).unwrap();
        // Same directory via a different spelling should not double-register.
        reg.add_library_path(dir.path().join(".")).unwrap();
        assert_eq!(reg.library_paths.len(), 1);
    }
}
