use crate::isf::ISFShader;
use anyhow::{Context, Result};
use notify::{Event, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use walkdir::WalkDir;

/// A shader library change.
#[derive(Debug, Clone)]
pub enum ShaderEvent {
    Changed(PathBuf),
    Removed(PathBuf),
    /// A shader failed to load or reload.
    Error(PathBuf, String),
}

/// Discovered ISF shaders, with hot reload.
pub struct ShaderRegistry {
    shaders: HashMap<String, ISFShader>,

    /// File path to shader name, for hot reload.
    path_to_name: HashMap<PathBuf, String>,

    /// Shader name to every file that provides it, so hot reload and removal
    /// keep library precedence.
    name_to_paths: HashMap<String, Vec<PathBuf>>,

    /// Watched library paths. Later paths win name collisions (built-in,
    /// then workdir, then override dirs).
    library_paths: Vec<PathBuf>,

    /// Held only to keep the watch alive.
    #[allow(dead_code)]
    watcher: Option<notify::RecommendedWatcher>,

    change_receiver: Option<Receiver<notify::Result<Event>>>,
}

impl ShaderRegistry {
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

    /// Register an existing directory to scan for shaders. A missing directory
    /// is not created, so a typo surfaces as an error. Duplicates are ignored.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` does not exist or is not a directory.
    pub fn add_library_path<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let path = path.as_ref().to_path_buf();

        if !path.is_dir() {
            anyhow::bail!("Shader library path does not exist: {}", path.display());
        }

        // Canonicalized so spellings of one directory dedup, and so absolute
        // watcher event paths match library paths.
        let path = path.canonicalize().unwrap_or(path);

        if self.library_paths.contains(&path) {
            log::debug!("Shader library already registered: {}", path.display());
            return Ok(());
        }

        self.library_paths.push(path);
        Ok(())
    }

    /// Scan all library paths for ISF shaders. Returns the unique shader count.
    ///
    /// # Errors
    ///
    /// Never fails: shaders that fail to parse are logged and skipped.
    pub fn scan(&mut self) -> Result<usize> {
        self.shaders.clear();
        self.path_to_name.clear();
        self.name_to_paths.clear();

        for lib_path in &self.library_paths {
            log::info!("Scanning library: {}", lib_path.display());

            for entry in WalkDir::new(lib_path)
                .follow_links(true)
                .into_iter()
                .filter_map(std::result::Result::ok)
            {
                let path = entry.path();

                let ext = path.extension().and_then(|s| s.to_str());
                if ext != Some("fs") && ext != Some("comp") {
                    continue;
                }

                match ISFShader::from_file(path) {
                    Ok(shader) => {
                        let name = shader.name();
                        log::info!("  Loaded: {} ({})", name, path.display());
                        self.path_to_name.insert(path.to_path_buf(), name.clone());
                        // Scanned in priority order, so the last write wins.
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

        // Unique names; overridden files don't count twice.
        let count = self.shaders.len();
        log::info!(
            "Loaded {} shaders from {} libraries",
            count,
            self.library_paths.len()
        );
        Ok(count)
    }

    /// Start watching library paths for changes.
    ///
    /// # Errors
    ///
    /// Returns an error if the file watcher cannot be created or a library path
    /// cannot be watched.
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

    /// Process pending file changes without blocking.
    pub fn poll_changes(&mut self) -> Vec<ShaderEvent> {
        // Collect events first so the receiver borrow ends.
        let pending_events: Vec<(notify::EventKind, Vec<PathBuf>)> = {
            let Some(receiver) = &self.change_receiver else {
                return Vec::new();
            };

            let mut collected = Vec::new();
            loop {
                match receiver.try_recv() {
                    Ok(Ok(event)) => {
                        collected.push((event.kind, event.paths));
                    }
                    Ok(Err(e)) => {
                        log::error!("File watcher error: {e}");
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

        if let Some(receiver) = &self.change_receiver
            && receiver.try_recv().is_err()
        {
            // Cleared only on Disconnected, not Empty.
        }

        let mut shader_events = Vec::new();

        for (kind, paths) in pending_events {
            for path in paths {
                let ext = path.extension().and_then(|s| s.to_str());
                if ext != Some("fs") && ext != Some("comp") {
                    continue;
                }

                match kind {
                    notify::EventKind::Create(_) | notify::EventKind::Modify(_) => {
                        match self.reload_shader(&path) {
                            Ok(()) => {
                                shader_events.push(ShaderEvent::Changed(path));
                            }
                            Err(e) => {
                                let err_msg = format!("{e}");
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
                        // Drop this provider and re-resolve, so a shadowed
                        // lower-priority file takes over.
                        if let Some(name) = self.path_to_name.remove(&path) {
                            self.forget_provider(&name, &path);
                            if self.resolve_winner(&name) {
                                log::info!(
                                    "Removed {}; restored shadowed provider",
                                    path.display()
                                );
                                shader_events.push(ShaderEvent::Changed(path));
                            } else {
                                log::info!("Removed shader: {name}");
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

    /// Reload one shader from disk. It becomes active only if it is the
    /// highest-priority provider of its name.
    fn reload_shader(&mut self, path: &Path) -> Result<()> {
        let shader = ISFShader::from_file(path)?;
        let name = shader.name();

        // If the file's NAME changed, re-resolve the old name.
        if let Some(old_name) = self.path_to_name.get(path).cloned()
            && old_name != name
        {
            self.forget_provider(&old_name, path);
            self.resolve_winner(&old_name);
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
            // The override should already be active.
            if !self.shaders.contains_key(&name) {
                self.resolve_winner(&name);
            }
        }

        Ok(())
    }

    /// Index + 1 of the last library path containing `path`; 0 if none.
    fn path_priority(&self, path: &Path) -> usize {
        self.library_paths
            .iter()
            .enumerate()
            .filter(|(_, lib)| path.starts_with(lib))
            .map(|(i, _)| i + 1)
            .max()
            .unwrap_or(0)
    }

    /// Record `path` as a provider of `name`, once.
    fn record_provider(&mut self, name: &str, path: &Path) {
        let providers = self.name_to_paths.entry(name.to_string()).or_default();
        if !providers.iter().any(|p| p == path) {
            providers.push(path.to_path_buf());
        }
    }

    fn forget_provider(&mut self, name: &str, path: &Path) {
        if let Some(providers) = self.name_to_paths.get_mut(name) {
            providers.retain(|p| p != path);
        }
    }

    fn is_highest_priority_provider(&self, name: &str, path: &Path) -> bool {
        match self.name_to_paths.get(name) {
            Some(providers) => providers
                .iter()
                .max_by_key(|p| self.path_priority(p))
                .is_none_or(|winner| winner.as_path() == path),
            None => true,
        }
    }

    /// Load the highest-priority remaining provider of `name`. Returns whether
    /// the shader still exists. Ties go to the last recorded provider.
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

    pub fn get(&self, name: &str) -> Option<&ISFShader> {
        self.shaders.get(name)
    }

    pub fn shader_names(&self) -> Vec<String> {
        self.shaders.keys().cloned().collect()
    }

    /// Generators, alphabetically.
    pub fn generators(&self) -> Vec<&ISFShader> {
        self.sorted(|s| s.metadata.is_generator())
    }

    /// Filters, excluding transitions, alphabetically.
    pub fn filters(&self) -> Vec<&ISFShader> {
        self.sorted(|s| s.metadata.is_filter() && !s.metadata.is_transition())
    }

    /// Transitions, alphabetically.
    pub fn transitions(&self) -> Vec<&ISFShader> {
        self.sorted(|s| s.metadata.is_transition())
    }

    /// The shaders `keep` accepts, by name ignoring case.
    fn sorted(&self, keep: impl Fn(&ISFShader) -> bool) -> Vec<&ISFShader> {
        let mut shaders: Vec<&ISFShader> = self.shaders.values().filter(|s| keep(s)).collect();
        shaders.sort_by_cached_key(|s| s.name().to_lowercase());
        shaders
    }

    pub fn count(&self) -> usize {
        self.shaders.len()
    }
}

impl Default for ShaderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Bundled shader directory relative to the executable's directory, so prefix
/// installs resolve like system ones.
// Linux: FHS layout from a native package. /usr/bin/varda -> /usr/share/varda/shaders.
#[cfg(target_os = "linux")]
const BUNDLED_SHADERS_RELATIVE: &str = "../share/varda/shaders";
// macOS .app: Contents/MacOS/varda -> Contents/Resources/shaders.
#[cfg(target_os = "macos")]
const BUNDLED_SHADERS_RELATIVE: &str = "../Resources/shaders";
// Windows portable ZIP: shaders/ next to varda.exe.
#[cfg(target_os = "windows")]
const BUNDLED_SHADERS_RELATIVE: &str = "shaders";
// Anything else: FHS layout.
#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
const BUNDLED_SHADERS_RELATIVE: &str = "../share/varda/shaders";

/// Bundled shader directory for `exe_dir`. Separate from
/// `get_bundled_shader_path` for testing.
fn bundled_shaders_for(exe_dir: &Path) -> Option<PathBuf> {
    let dir = exe_dir.join(BUNDLED_SHADERS_RELATIVE);
    dir.is_dir().then_some(dir)
}

/// The bundled shader path for installed builds (Linux package, macOS .app,
/// Windows ZIP).
pub fn get_bundled_shader_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    bundled_shaders_for(exe.parent()?)
}

/// Default library paths for this platform.
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

    /// Generators, filters and transitions come back alphabetically, ignoring
    /// case, whatever order the files were read in.
    #[test]
    fn shader_lists_are_alphabetical() {
        let mut registry = ShaderRegistry::new();
        registry.add_library_path("shaders").expect("shaders");
        registry.scan().expect("scan");
        let names = |list: Vec<&ISFShader>| {
            list.iter()
                .map(|s| s.name().to_lowercase())
                .collect::<Vec<_>>()
        };
        for (kind, list) in [
            ("generators", names(registry.generators())),
            ("filters", names(registry.filters())),
            ("transitions", names(registry.transitions())),
        ] {
            let mut sorted = list.clone();
            sorted.sort();
            assert_ne!(list.len(), 0, "{kind}");
            assert_eq!(list, sorted, "{kind}");
        }
    }

    /// The packaged layout resolves to the bundled shaders.
    #[test]
    fn bundled_shaders_resolve_from_the_packaged_layout() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("bin");
        // Use the platform's rule; Windows puts shaders beside the executable.
        let shaders = exe_dir.join(BUNDLED_SHADERS_RELATIVE);
        fs::create_dir_all(&exe_dir).unwrap();
        fs::create_dir_all(&shaders).unwrap();

        let found = bundled_shaders_for(&exe_dir).expect("packaged layout should resolve");
        assert_eq!(
            found.canonicalize().unwrap(),
            shaders.canonicalize().unwrap()
        );
    }

    /// Each platform's packaging layout matches this constant; a mismatch ships
    /// an empty shader library silently.
    #[test]
    fn bundled_shader_path_matches_what_packaging_installs() {
        use std::path::Component;

        #[cfg(target_os = "linux")]
        let (exe_dir, expected) = ("/usr/bin", "/usr/share/varda/shaders");
        #[cfg(target_os = "macos")]
        let (exe_dir, expected) = (
            "/Applications/Varda.app/Contents/MacOS",
            "/Applications/Varda.app/Contents/Resources/shaders",
        );
        #[cfg(target_os = "windows")]
        let (exe_dir, expected) = (r"C:\Varda", r"C:\Varda\shaders");
        #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
        let (exe_dir, expected) = ("/usr/bin", "/usr/share/varda/shaders");

        let joined = Path::new(exe_dir).join(BUNDLED_SHADERS_RELATIVE);
        // Resolved lexically; these paths don't exist here.
        let resolved = joined
            .components()
            .fold(PathBuf::new(), |mut acc, component| {
                if component == Component::ParentDir {
                    acc.pop();
                } else {
                    acc.push(component);
                }
                acc
            });

        assert_eq!(
            resolved,
            Path::new(expected),
            "packaging installs shaders somewhere this constant does not point"
        );
    }

    /// A missing bundled directory is not an error; dev builds have none.
    #[test]
    fn bundled_shaders_are_absent_rather_than_wrong_when_not_installed() {
        let root = tempfile::tempdir().unwrap();
        let exe_dir = root.path().join("target/release");
        fs::create_dir_all(&exe_dir).unwrap();

        assert!(bundled_shaders_for(&exe_dir).is_none());
    }

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
        assert_eq!(reg.shader_names().len(), 0);
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
        // Transition: "Transition" category plus an image input.
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
        // filters() excludes transitions.
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
        // Both dirs have a "Glow" with different categories.
        write_shader(builtin.path(), "Glow", "Generator", false);
        write_shader(user.path(), "Glow", "Filter", true);

        let mut reg = ShaderRegistry::new();
        reg.add_library_path(builtin.path()).unwrap();
        reg.add_library_path(user.path()).unwrap();
        reg.scan().unwrap();

        // The user version wins.
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
        // Only real paths load.
        let count = reg.scan().unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn get_default_library_paths_returns_platform_path() {
        let paths = get_default_library_paths();
        // One path on macOS, Linux (with HOME), or Windows (with APPDATA).
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

    /// Canonical path of a scanned shader file, matching registered library paths.
    fn shader_file(dir: &Path, name: &str) -> PathBuf {
        dir.canonicalize().unwrap().join(format!("{name}.fs"))
    }

    /// A built-in generator and a user override filter, both named "Glow",
    /// scanned with the override winning.
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
        // The shadowed built-in is edited.
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
        reg.reload_shader(&shader_file(user.path(), "Glow"))
            .unwrap();
        assert!(reg.get("Glow").unwrap().metadata.is_filter());
    }

    #[test]
    fn removing_override_restores_shadowed_builtin() {
        let (_builtin, user, mut reg) = overridden_glow();
        let override_path = shader_file(user.path(), "Glow");
        // As poll_changes does on a Remove event.
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
        // Deleting the shadowed file leaves the winner.
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
        // Another spelling of the same directory is not registered twice.
        reg.add_library_path(dir.path().join(".")).unwrap();
        assert_eq!(reg.library_paths.len(), 1);
    }
}
