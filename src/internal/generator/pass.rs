//! Shared ISF plumbing for shader decks and effects: pass buffers and the
//! multi-pass loop, imported textures, size expressions, uniforms.

use crate::isf::{ISFMetadata, ISFPass, PassFormat};
use crate::renderer::{GpuContext, ISFUniforms, UnifiedPipeline};
use std::collections::HashMap;
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

/// The texture format of an ISF pass format.
pub fn wgpu_pass_format(format: PassFormat) -> wgpu::TextureFormat {
    match format {
        PassFormat::Rgba16Float => wgpu::TextureFormat::Rgba16Float,
        PassFormat::Rgba32Float => wgpu::TextureFormat::Rgba32Float,
        PassFormat::R32Float => wgpu::TextureFormat::R32Float,
    }
}

/// Target formats of one pass, in attachment order. Undeclared targets use
/// `default`.
fn pass_formats(pass: &ISFPass, default: wgpu::TextureFormat) -> Vec<wgpu::TextureFormat> {
    pass.target_formats()
        .into_iter()
        .map(|f| f.map_or(default, wgpu_pass_format))
        .collect()
}

/// One run of a targeted pass, for choosing its uniforms.
#[derive(Debug, Clone, Copy)]
pub struct PassStep {
    /// `PASSINDEX`.
    pub pass_index: usize,
    /// Which run of the pass this frame, from 0.
    pub substep: usize,
    /// Runs of the pass this frame: more than 1 for a `PERSISTENT` pass.
    pub substeps: usize,
    /// The pass buffer's size.
    pub size: [f32; 2],
}

/// What every pass of one shader binds besides the pass buffers.
pub struct PassBindings<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub pipeline: &'a UnifiedPipeline,
    /// A filter's input image.
    pub input: Option<&'a wgpu::TextureView>,
    pub imported: &'a [&'a wgpu::TextureView],
    pub preprocessors: &'a [&'a wgpu::TextureView],
    pub user_params: &'a wgpu::Buffer,
}

/// The pass buffers of one ISF shader and the loop that renders its targeted
/// passes into them.
///
/// Pass buffers bind in declaration order: each targeted pass's targets, in
/// attachment order.
pub struct PassSet {
    passes: Vec<ISFPass>,
    buffers: HashMap<String, PassBuffer>,
    default_format: wgpu::TextureFormat,
    label: &'static str,
    /// Frames finished since the buffers were created. 0 means `HISTORY`
    /// targets hold nothing yet.
    frames: u32,
}

impl PassSet {
    /// Allocate the buffers `passes` declare, sized against `width × height`.
    /// Targets without a `FORMAT` use `default_format`.
    pub fn new(
        gpu: &GpuContext,
        passes: Vec<ISFPass>,
        width: u32,
        height: u32,
        default_format: wgpu::TextureFormat,
        label: &'static str,
    ) -> Self {
        let buffers = create_pass_buffers(gpu, &passes, width, height, default_format, label);
        Self {
            passes,
            buffers,
            default_format,
            label,
            frames: 0,
        }
    }

    /// Reallocate for a new size. History starts over.
    pub fn resize(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        self.buffers = create_pass_buffers(
            gpu,
            &self.passes,
            width,
            height,
            self.default_format,
            self.label,
        );
        self.frames = 0;
    }

    /// Whether any pass renders into a buffer.
    pub fn has_targeted_passes(&self) -> bool {
        self.passes.iter().any(ISFPass::is_targeted)
    }

    /// All passes, the output pass included.
    pub fn passes(&self) -> &[ISFPass] {
        &self.passes
    }

    /// Whether `HISTORY` targets hold last frame's output. `HISTORYVALID`.
    pub fn history_valid(&self) -> bool {
        self.frames > 0
    }

    /// Per binding, whether the pass buffer is filterable, for the pipeline
    /// layout.
    pub fn binding_filterability(
        passes: &[ISFPass],
        default_format: wgpu::TextureFormat,
    ) -> Vec<bool> {
        passes
            .iter()
            .flat_map(|pass| pass_formats(pass, default_format))
            .map(|format| format == wgpu::TextureFormat::Rgba16Float)
            .collect()
    }

    /// Target formats of every targeted pass, for the pipeline.
    pub fn target_formats(
        passes: &[ISFPass],
        default_format: wgpu::TextureFormat,
    ) -> Vec<Vec<wgpu::TextureFormat>> {
        passes
            .iter()
            .filter(|pass| pass.is_targeted())
            .map(|pass| pass_formats(pass, default_format))
            .collect()
    }

    /// Uniform slots one frame needs: one per targeted pass iteration and
    /// one for the output pass.
    pub fn uniform_slots(&self, persistent_substeps: usize) -> usize {
        self.passes
            .iter()
            .filter(|pass| pass.is_targeted())
            .map(|pass| iterations(pass, persistent_substeps))
            .sum::<usize>()
            + 1
    }

    /// Every pass buffer's current read view, in binding order.
    pub fn binding_views(&self) -> Vec<&wgpu::TextureView> {
        self.passes
            .iter()
            .flat_map(ISFPass::target_names)
            .filter_map(|name| self.buffers.get(name))
            .map(PassBuffer::read_view)
            .collect()
    }

    /// Encode every targeted pass. `PERSISTENT` passes run
    /// `persistent_substeps` times. `uniforms` gives each run's uniforms;
    /// `HISTORYVALID` is filled in here. Returns the next free uniform slot.
    pub fn encode_targeted(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        bindings: &PassBindings<'_>,
        persistent_substeps: usize,
        mut uniforms: impl FnMut(PassStep) -> ISFUniforms,
    ) -> usize {
        let history_valid = i32::from(self.history_valid());
        let mut slot = 0;
        for (pass_idx, pass) in self.passes.iter().enumerate() {
            let names = pass.target_names();
            let Some(first) = names.first().and_then(|n| self.buffers.get(*n)) else {
                continue;
            };
            let size = first.texture_a.size();
            let size = [size.width as f32, size.height as f32];
            let formats = pass_formats(pass, self.default_format);
            let substeps = iterations(pass, persistent_substeps);
            for substep in 0..substeps {
                let mut pass_uniforms = uniforms(PassStep {
                    pass_index: pass_idx,
                    substep,
                    substeps,
                    size,
                });
                pass_uniforms.history_valid = history_valid;
                bindings
                    .pipeline
                    .write_pass_uniforms(bindings.queue, slot, &pass_uniforms);
                {
                    let views = self.binding_views();
                    let targets: Vec<&wgpu::TextureView> = names
                        .iter()
                        .filter_map(|n| self.buffers.get(*n))
                        .map(PassBuffer::write_view)
                        .collect();
                    encode_pass(encoder, bindings, slot, &views, &targets, &formats);
                }
                slot += 1;
                for name in &names {
                    if let Some(buffer) = self.buffers.get_mut(*name) {
                        buffer.swap();
                    }
                }
            }
        }
        slot
    }

    /// Encode the output pass into `target`, reading uniforms from `slot`.
    pub fn encode_output(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        bindings: &PassBindings<'_>,
        slot: usize,
        mut uniforms: ISFUniforms,
        target: &wgpu::TextureView,
    ) {
        uniforms.history_valid = i32::from(self.history_valid());
        bindings
            .pipeline
            .write_pass_uniforms(bindings.queue, slot, &uniforms);
        let views = self.binding_views();
        encode_pass(
            encoder,
            bindings,
            slot,
            &views,
            &[target],
            &[bindings.pipeline.surface_format],
        );
    }

    /// Mark the frame rendered: from the next frame, history is valid.
    pub fn finish_frame(&mut self) {
        self.frames = self.frames.saturating_add(1);
    }
}

fn iterations(pass: &ISFPass, persistent_substeps: usize) -> usize {
    if pass.is_persistent() {
        persistent_substeps
    } else {
        1
    }
}

fn encode_pass(
    encoder: &mut wgpu::CommandEncoder,
    bindings: &PassBindings<'_>,
    slot: usize,
    pass_views: &[&wgpu::TextureView],
    targets: &[&wgpu::TextureView],
    formats: &[wgpu::TextureFormat],
) {
    let bind_group = bindings.pipeline.create_pass_bind_group(
        bindings.device,
        slot,
        bindings.input,
        pass_views,
        bindings.imported,
        bindings.preprocessors,
        Some(bindings.user_params),
    );
    let attachments: Vec<Option<wgpu::RenderPassColorAttachment<'_>>> = targets
        .iter()
        .map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })
        })
        .collect();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("ISF Pass"),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(bindings.pipeline.pipeline_for(formats));
    pass.set_bind_group(0, &bind_group, &[]);
    pass.draw(0..3, 0..1);
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
/// `width × height`. A target named by two passes is allocated once, by the
/// first.
///
/// Every pass buffer is double-buffered: each pass samples all pass buffers,
/// so a single texture would be both target and resource in its own pass,
/// which wgpu rejects. The same double-buffering gives a `HISTORY` pass last
/// frame's output.
fn create_pass_buffers(
    gpu: &GpuContext,
    passes: &[ISFPass],
    width: u32,
    height: u32,
    default_format: wgpu::TextureFormat,
    label: &str,
) -> HashMap<String, PassBuffer> {
    let mut buffers = HashMap::new();
    for pass in passes {
        let pass_width = parse_size_expression(pass.width.as_deref(), width);
        let pass_height = parse_size_expression(pass.height.as_deref(), height);
        for (target_name, format) in pass
            .target_names()
            .into_iter()
            .zip(pass_formats(pass, default_format))
        {
            if buffers.contains_key(target_name) {
                continue;
            }
            let target_name = target_name.to_owned();
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
                    persistent: pass.is_persistent(),
                    read_idx: 0,
                },
            );
        }
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
        jitter: jitter(frame_index),
        jitter_index: i32::try_from(frame_index % JITTER_CYCLE).unwrap_or(0),
        history_valid: 1,
    }
}

/// Frames in one jitter cycle.
pub const JITTER_CYCLE: u32 = 16;

/// The `JITTER` offset for a frame: the Halton (2, 3) sequence over
/// [`JITTER_CYCLE`] frames, centered on the pixel.
pub fn jitter(frame_index: u32) -> [f32; 2] {
    let index = frame_index % JITTER_CYCLE + 1;
    [halton(index, 2) - 0.5, halton(index, 3) - 0.5]
}

fn halton(mut index: u32, base: u32) -> f32 {
    let mut fraction = 1.0;
    let mut result = 0.0;
    while index > 0 {
        fraction /= base as f32;
        result += fraction * (index % base) as f32;
        index /= base;
    }
    result
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
    fn jitter_follows_halton_2_3() {
        assert_eq!(jitter(0), [0.0, 1.0 / 3.0 - 0.5]);
        assert_eq!(jitter(1), [-0.25, 2.0 / 3.0 - 0.5]);
        assert_eq!(jitter(2), [0.25, 1.0 / 9.0 - 0.5]);
        assert_eq!(jitter(JITTER_CYCLE), jitter(0));
        for frame in 0..JITTER_CYCLE {
            let [x, y] = jitter(frame);
            assert!((-0.5..0.5).contains(&x) && (-0.5..0.5).contains(&y));
        }
    }

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
