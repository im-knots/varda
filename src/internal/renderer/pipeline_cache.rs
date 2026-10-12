//! Compiled pipelines kept on disk between runs. Vulkan only: it is the one
//! wgpu backend with `Features::PIPELINE_CACHE`.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// A pipeline that took at least this long to build is written to the cache
/// file at once, so a crash does not throw the compile away.
pub const SAVE_AFTER_BUILD: Duration = Duration::from_secs(1);

/// A pipeline cache and the file it is saved to. Clones share one cache.
#[derive(Clone, Debug)]
pub struct PipelineCacheFile {
    cache: wgpu::PipelineCache,
    path: PathBuf,
}

impl PipelineCacheFile {
    /// The cache for this device in `dir`, loaded with what an earlier run
    /// saved. `None` without `Features::PIPELINE_CACHE`.
    pub fn open(device: &wgpu::Device, info: &wgpu::AdapterInfo, dir: &Path) -> Option<Self> {
        if !device.features().contains(wgpu::Features::PIPELINE_CACHE) {
            return None;
        }
        let path = cache_path(dir, info)?;
        let data = std::fs::read(&path).ok();
        // SAFETY: `data` is a file `save` wrote from `PipelineCache::get_data`,
        // replaced by rename so it is never partly written. wgpu checks its
        // header against this adapter, driver and length, and `fallback` gives
        // an empty cache when the check fails.
        let cache = unsafe {
            device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                label: Some("Varda Pipeline Cache"),
                data: data.as_deref(),
                fallback: true,
            })
        };
        log::info!(
            "Pipeline cache {}: {} bytes loaded",
            path.display(),
            data.as_ref().map_or(0, Vec::len)
        );
        Some(Self { cache, path })
    }

    pub fn cache(&self) -> &wgpu::PipelineCache {
        &self.cache
    }

    /// Write the cache to its file. A failure is logged; the file is never required.
    pub fn save(&self) {
        let Some(data) = self.cache.get_data() else {
            return;
        };
        match write_atomic(&self.path, &data) {
            Ok(()) => log::info!(
                "Pipeline cache {}: {} bytes saved",
                self.path.display(),
                data.len()
            ),
            Err(e) => log::warn!("Pipeline cache {}: not saved: {e}", self.path.display()),
        }
    }

    /// [`Self::save`] when a build took at least [`SAVE_AFTER_BUILD`].
    pub fn save_after(&self, build: Duration) {
        if build >= SAVE_AFTER_BUILD {
            self.save();
        }
    }
}

/// `<user cache dir>/varda/pipelines`.
pub fn default_dir() -> Option<PathBuf> {
    dirs::cache_dir().map(|dir| dir.join("varda").join("pipelines"))
}

/// The file for `info`'s adapter in `dir`, or `None` on a backend wgpu keeps no
/// cache for.
fn cache_path(dir: &Path, info: &wgpu::AdapterInfo) -> Option<PathBuf> {
    wgpu::util::pipeline_cache_key(info).map(|key| dir.join(format!("{key}.bin")))
}

/// Replace `path` with `data` through a temporary file and a rename.
fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    // Unique per save, so build threads saving at once never share a file.
    static SAVES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let n = SAVES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temp = path.with_extension(format!("{}.{n}.tmp", std::process::id()));
    std::fs::write(&temp, data)?;
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(backend: wgpu::Backend) -> Option<wgpu::AdapterInfo> {
        let gpu = crate::testing::headless_gpu()?;
        let mut info = gpu.adapter.get_info();
        info.backend = backend;
        info.vendor = 0x1002;
        info.device = 0x73bf;
        Some(info)
    }

    #[test]
    fn a_vulkan_adapter_has_a_file_named_for_its_device() {
        let Some(info) = info(wgpu::Backend::Vulkan) else {
            return;
        };
        let path = cache_path(Path::new("/cache"), &info).expect("Vulkan has a key");
        assert_eq!(path.parent(), Some(Path::new("/cache")));
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        assert!(name.contains("4098") && name.contains("29631"), "{name}");
    }

    #[test]
    fn other_backends_have_no_file() {
        for backend in [wgpu::Backend::Metal, wgpu::Backend::Dx12] {
            let Some(info) = info(backend) else {
                return;
            };
            assert_eq!(cache_path(Path::new("/cache"), &info), None);
        }
    }

    #[test]
    fn a_write_replaces_the_file_and_leaves_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("cache.bin");
        write_atomic(&path, b"first").unwrap();
        write_atomic(&path, b"second").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"second");
        let entries = std::fs::read_dir(path.parent().unwrap()).unwrap().count();
        assert_eq!(entries, 1, "no temporary file is left behind");
    }

    #[test]
    fn without_the_feature_there_is_no_cache() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        if gpu
            .device
            .features()
            .contains(wgpu::Features::PIPELINE_CACHE)
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let info = gpu.adapter.get_info();
        assert!(PipelineCacheFile::open(&gpu.device, &info, dir.path()).is_none());
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    fn build_a_pipeline(device: &wgpu::Device, cache: &wgpu::PipelineCache) {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(
                "@vertex fn vs() -> @builtin(position) vec4f { return vec4f(0.0); }
                 @fragment fn fs() -> @location(0) vec4f { return vec4f(1.0); }"
                    .into(),
            ),
        });
        let _ = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: None,
            layout: None,
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs"),
                targets: &[Some(wgpu::TextureFormat::Rgba8Unorm.into())],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: Some(cache),
        });
    }

    /// Runs where the adapter has the feature: CI's Linux job.
    #[test]
    fn a_saved_cache_is_loaded_by_the_next_run() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        if !gpu
            .device
            .features()
            .contains(wgpu::Features::PIPELINE_CACHE)
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let info = gpu.adapter.get_info();
        let first = PipelineCacheFile::open(&gpu.device, &info, dir.path()).expect("cache");
        build_a_pipeline(&gpu.device, first.cache());
        first.save();
        let path = cache_path(dir.path(), &info).expect("path");
        let saved = std::fs::read(&path).expect("saved");
        assert_ne!(saved.len(), 0, "the build left data to save");

        let second = PipelineCacheFile::open(&gpu.device, &info, dir.path()).expect("cache");
        let reloaded = second.cache().get_data().expect("data");
        assert_ne!(reloaded.len(), 0, "the next run starts from the saved data");
    }

    /// Runs where the adapter has the feature: CI's Linux job.
    #[test]
    fn a_shader_deck_builds_into_the_cache() {
        let Some(mut gpu) = crate::testing::headless_gpu() else {
            return;
        };
        if !gpu
            .device
            .features()
            .contains(wgpu::Features::PIPELINE_CACHE)
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        gpu.open_pipeline_cache(dir.path());
        let cache = gpu.pipeline_cache.clone().expect("cache");
        let before = cache.cache().get_data().map_or(0, |d| d.len());

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("shaders/plasma.fs");
        let shader = crate::isf::ISFShader::from_file(path.to_str().unwrap()).expect("parse");
        crate::deck::Deck::from_shader(&gpu, shader, 64, 64).expect("build");

        let after = cache.cache().get_data().map_or(0, |d| d.len());
        assert!(after > before, "the deck's pipeline went into the cache");
    }
}
