//! Shared ISF plumbing for shader decks and effects: pass buffers, imported
//! textures, size expressions, and the date uniform.

use crate::isf::ISFMetadata;
use crate::renderer::GpuContext;
use std::time::Instant;

/// A buffer from an ISF `PASSES` entry, double-buffered so a pass can read
/// and write it in one frame.
pub struct PassBuffer {
    /// The ISF `PASSES` `TARGET` name.
    pub name: String,
    pub texture_a: wgpu::Texture,
    pub view_a: wgpu::TextureView,
    pub texture_b: Option<wgpu::Texture>,
    pub view_b: Option<wgpu::TextureView>,
    /// Whether the contents carry across frames.
    pub persistent: bool,
    /// 0 = read from A, 1 = read from B.
    pub read_idx: usize,
}

impl PassBuffer {
    /// The current read view.
    pub fn read_view(&self) -> &wgpu::TextureView {
        if self.read_idx == 0 {
            &self.view_a
        } else {
            self.view_b.as_ref().unwrap_or(&self.view_a)
        }
    }

    /// The current write view, always the one not being read.
    pub fn write_view(&self) -> &wgpu::TextureView {
        if self.read_idx == 0 {
            self.view_b.as_ref().unwrap_or(&self.view_a)
        } else {
            &self.view_a
        }
    }

    /// Swap read and write. Call after the pass that targets this buffer, so
    /// later passes read what it wrote.
    ///
    /// Swaps non-persistent buffers too: the read and write views must differ,
    /// or the pass binds one texture as both attachment and sampled resource.
    pub fn swap(&mut self) {
        self.read_idx = 1 - self.read_idx;
    }
}

/// Allocate the pass buffers an ISF shader's `PASSES` declare, sized against
/// `width × height`.
///
/// Every pass buffer is double-buffered: each pass samples all pass buffers,
/// so a single texture would be both target and resource in its own pass,
/// which wgpu rejects.
pub fn create_pass_buffers(
    gpu: &GpuContext,
    passes: &[crate::isf::ISFPass],
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> std::collections::HashMap<String, PassBuffer> {
    let mut buffers = std::collections::HashMap::new();
    for pass in passes {
        let Some(target_name) = pass.target.clone() else {
            continue;
        };
        let pass_width = parse_size_expression(pass.width.as_deref(), width);
        let pass_height = parse_size_expression(pass.height.as_deref(), height);
        let make = |side: &str| {
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("{label} {side}: {target_name}")),
                size: wgpu::Extent3d {
                    width: pass_width,
                    height: pass_height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            (texture, view)
        };
        let (texture_a, view_a) = make("A");
        let (texture_b, view_b) = make("B");
        buffers.insert(
            target_name.clone(),
            PassBuffer {
                name: target_name,
                texture_a,
                view_a,
                texture_b: Some(texture_b),
                view_b: Some(view_b),
                persistent: pass.persistent.unwrap_or(false),
                read_idx: 0,
            },
        );
    }
    buffers
}

/// Placeholder slots for an ISF shader's `PREPROCESSORS`: 1×1 data textures
/// until an analyzer publishes. They hold data, not color, in the declared
/// `FORMAT`.
pub fn create_preprocessor_slots(
    gpu: &GpuContext,
    metadata: &ISFMetadata,
) -> Vec<crate::source::PreprocessorSlot> {
    metadata
        .preprocessors
        .iter()
        .map(|pp| {
            let format = crate::source::preprocessor_texture_format(&pp.format);
            let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Preprocessor: {}", pp.name)),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            crate::source::PreprocessorSlot {
                name: pp.name.clone(),
                analyzer_type: pp.preprocessor_type.clone(),
                options: pp.options.clone(),
                param_bindings: pp.param_bindings.clone(),
                phase_bindings: pp.phase_bindings.clone(),
                texture,
                view,
                format,
                last_uploaded_generation: None,
            }
        })
        .collect()
}

/// Whether each slot binds filterable, which the pipeline layout needs.
pub fn preprocessor_filterability(slots: &[crate::source::PreprocessorSlot]) -> Vec<bool> {
    slots
        .iter()
        .map(|slot| slot.format != wgpu::TextureFormat::Rgba32Float)
        .collect()
}

/// The ISF uniforms for one pass.
pub fn uniforms(
    audio: &crate::audio::AudioData,
    time: f32,
    time_delta: f32,
    frame_index: u32,
    pass_index: usize,
    render_size: [f32; 2],
    phase_times: [f32; 4],
) -> crate::renderer::ISFUniforms {
    crate::renderer::ISFUniforms {
        time,
        time_delta,
        frame_index,
        pass_index: i32::try_from(pass_index).unwrap_or(i32::MAX),
        render_size,
        audio_level: audio.level,
        audio_bass: audio.bass(),
        audio_mid: audio.mid(),
        audio_treble: audio.treble(),
        audio_bpm: audio.bpm.unwrap_or(0.0),
        audio_beat_phase: audio.beat_phase(),
        date: get_current_date(),
        phase_times,
    }
}

/// Parse ISF size expressions like "$WIDTH", "$WIDTH/2", "1024", etc.
pub fn parse_size_expression(expr: Option<&str>, base_size: u32) -> u32 {
    match expr {
        None => base_size,
        Some(s) => {
            let s = s.trim();
            if s == "$WIDTH" || s == "$HEIGHT" {
                base_size
            } else if s.starts_with("$WIDTH/") || s.starts_with("$HEIGHT/") {
                let divisor: u32 = s
                    .split('/')
                    .nth(1)
                    .and_then(|d| d.trim().parse().ok())
                    .unwrap_or(1);
                base_size / divisor.max(1)
            } else if s.starts_with("$WIDTH*") || s.starts_with("$HEIGHT*") {
                let multiplier: u32 = s
                    .split('*')
                    .nth(1)
                    .and_then(|m| m.trim().parse().ok())
                    .unwrap_or(1);
                base_size * multiplier
            } else {
                s.parse().unwrap_or(base_size)
            }
        }
    }
}

/// Load ISF `IMPORTED` images as GPU textures, as (name, texture, view) sorted
/// by name for a stable binding order. Decoding is parallel; uploads are
/// sequential.
pub fn load_imported_textures(
    metadata: &ISFMetadata,
    shader_file_path: Option<&str>,
    context: &GpuContext,
) -> Vec<(String, wgpu::Texture, wgpu::TextureView)> {
    let imported = match &metadata.imported {
        Some(map) if !map.is_empty() => map,
        _ => return Vec::new(),
    };

    let shader_dir = shader_file_path.map_or(std::path::Path::new("."), |p| {
        std::path::Path::new(p)
            .parent()
            .unwrap_or(std::path::Path::new("."))
    });

    let mut entries: Vec<_> = imported.iter().collect();
    entries.sort_by_key(|(name, _)| (*name).clone());

    let load_list: Vec<_> = entries
        .iter()
        .filter_map(|(name, import_def)| {
            let rel_path = import_def.path.as_ref()?;
            Some(((*name).clone(), shader_dir.join(rel_path)))
        })
        .collect();

    if load_list.is_empty() {
        return Vec::new();
    }

    let t0 = Instant::now();

    let decoded: Vec<_> = std::thread::scope(|s| {
        let handles: Vec<_> = load_list
            .iter()
            .map(|(name, path)| {
                let name = name.clone();
                let path = path.clone();
                s.spawn(move || match image::open(&path) {
                    Ok(img) => Some((name, img.to_rgba8())),
                    Err(e) => {
                        log::warn!(
                            "IMPORTED '{}': failed to load '{}': {}",
                            name,
                            path.display(),
                            e
                        );
                        None
                    }
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok().flatten())
            .collect()
    });

    let mut result = Vec::with_capacity(decoded.len());
    for (name, img) in &decoded {
        let (w, h) = img.dimensions();
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("Imported: {name}")),
            size: wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        context.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            img,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * w),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        result.push((name.clone(), texture, view));
    }

    // Sort for a stable binding order.
    result.sort_by(|a, b| a.0.cmp(&b.0));

    let elapsed = t0.elapsed();
    log::info!(
        "IMPORTED: loaded {} textures in {:.0?} (parallel decode)",
        result.len(),
        elapsed
    );

    result
}

/// Current date as [year, month, day, `seconds_in_day`].
pub fn get_current_date() -> [f32; 4] {
    use std::time::SystemTime;

    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();

    let total_seconds = now.as_secs();
    let seconds_in_day = (total_seconds % 86400) as f32;

    let days_since_epoch = total_seconds / 86400;
    let year = 1970.0 + (days_since_epoch as f32 / 365.25);
    let day_of_year = (days_since_epoch % 365) as f32;
    let month = (day_of_year / 30.0).floor() + 1.0;
    let day = (day_of_year % 30.0) + 1.0;

    [year, month, day, seconds_in_day]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_none_returns_base() {
        assert_eq!(parse_size_expression(None, 1920), 1920);
    }

    #[test]
    fn parse_size_width_variable() {
        assert_eq!(parse_size_expression(Some("$WIDTH"), 1920), 1920);
        assert_eq!(parse_size_expression(Some("$HEIGHT"), 1080), 1080);
    }

    #[test]
    fn parse_size_divide() {
        assert_eq!(parse_size_expression(Some("$WIDTH/2"), 1920), 960);
        assert_eq!(parse_size_expression(Some("$HEIGHT/4"), 1080), 270);
    }

    #[test]
    fn parse_size_multiply() {
        assert_eq!(parse_size_expression(Some("$WIDTH*2"), 960), 1920);
        assert_eq!(parse_size_expression(Some("$HEIGHT*3"), 360), 1080);
    }

    #[test]
    fn parse_size_literal() {
        assert_eq!(parse_size_expression(Some("512"), 1920), 512);
        assert_eq!(parse_size_expression(Some("1024"), 1080), 1024);
    }

    #[test]
    fn parse_size_invalid_literal_falls_back() {
        assert_eq!(parse_size_expression(Some("abc"), 1920), 1920);
    }

    #[test]
    fn parse_size_divide_by_zero_safe() {
        assert_eq!(parse_size_expression(Some("$WIDTH/0"), 1920), 1920);
    }

    #[test]
    fn parse_size_whitespace_trim() {
        assert_eq!(parse_size_expression(Some(" $WIDTH "), 1920), 1920);
    }
}
