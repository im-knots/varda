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

                // Only process .fs files
                if path.extension().and_then(|s| s.to_str()) != Some("fs") {
                    continue;
                }

                match ISFShader::from_file(path) {
                    Ok(shader) => {
                        let name = shader.name();
                        log::info!("  ✓ Loaded: {} ({})", name, path.display());
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

        log::info!("Loaded {} shaders from {} libraries", count, self.library_paths.len());
        Ok(count)
    }

    /// Start watching library paths for changes
    pub fn start_watching(&mut self) -> Result<()> {
        let (tx, rx) = mpsc::channel();

        let mut watcher = notify::recommended_watcher(tx)
            .context("Failed to create file watcher")?;

        for lib_path in &self.library_paths {
            if lib_path.exists() {
                watcher.watch(lib_path, RecursiveMode::Recursive)
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
                // Only process .fs files
                if path.extension().and_then(|s| s.to_str()) != Some("fs") {
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
                                log::warn!("Failed to reload shader {}: {}", path.display(), err_msg);
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
    
    paths
}

