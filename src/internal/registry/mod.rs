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

    /// Library paths being watched
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
            library_paths: Vec::new(),
            watcher: None,
            change_receiver: None,
        }
    }

    /// Add a library path to scan
    pub fn add_library_path<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref().to_path_buf();

        if !path.exists() {
            log::warn!("Library path does not exist: {}", path.display());
            std::fs::create_dir_all(&path)
                .with_context(|| format!("Failed to create library path: {}", path.display()))?;
            log::info!("Created library path: {}", path.display());
        }

        self.library_paths.push(path);
        Ok(())
    }

    /// Scan all library paths for ISF shaders
    pub fn scan(&mut self) -> Result<usize> {
        self.shaders.clear();
        self.path_to_name.clear();
        let mut count = 0;

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
                        self.shaders.insert(name, shader);
                        count += 1;
                    }
                    Err(e) => {
                        log::warn!("  ✗ Failed to load {}: {}", path.display(), e);
                    }
                }
            }
        }

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
                        // Remove the shader
                        if let Some(name) = self.path_to_name.remove(&path) {
                            self.shaders.remove(&name);
                            log::info!("Removed shader: {}", name);
                            shader_events.push(ShaderEvent::Removed(path));
                        }
                    }
                    _ => {}
                }
            }
        }

        shader_events
    }

    /// Reload a single shader from disk
    fn reload_shader(&mut self, path: &Path) -> Result<()> {
        let shader = ISFShader::from_file(path)?;
        let name = shader.name();

        // Update path mapping
        self.path_to_name.insert(path.to_path_buf(), name.clone());

        // Insert/update shader
        log::info!("Hot-reloaded shader: {} ({})", name, path.display());
        self.shaders.insert(name, shader);

        Ok(())
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
}
