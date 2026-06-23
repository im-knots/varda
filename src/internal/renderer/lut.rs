//! LUT file parsing and GPU pipeline — supports .cube (Resolve/Adobe) and .3dl (broadcast) formats.
//! Produces a common `ParsedLut` structure and uploads it to the GPU for post-process application.

use anyhow::{bail, Context, Result};
use std::num::NonZeroU64;
use std::path::Path;
use wgpu::util::DeviceExt;

/// A parsed 3D LUT with optional 1D shaper, ready for GPU upload.
#[derive(Debug)]
pub struct ParsedLut {
    /// Human-readable title from the file (if present).
    pub title: Option<String>,
    /// 3D LUT grid size per axis (e.g. 17, 33, 65).
    pub size_3d: u32,
    /// 3D LUT data as flat RGB triplets, R-major ordering. Length = size_3d^3 * 3.
    pub data_3d: Vec<f32>,
    /// Domain minimum for the 3D LUT input range.
    pub domain_min: [f32; 3],
    /// Domain maximum for the 3D LUT input range.
    pub domain_max: [f32; 3],
    /// Optional 1D shaper LUT. If present, applied before the 3D lookup.
    pub shaper: Option<ShaperLut>,
}

/// A 1D shaper LUT that redistributes precision in the input range.
#[derive(Debug)]
pub struct ShaperLut {
    /// Number of entries in the shaper.
    pub size: u32,
    /// Shaper data as flat RGB triplets. Length = size * 3.
    pub data: Vec<f32>,
    /// Domain minimum for the shaper input range.
    pub domain_min: [f32; 3],
    /// Domain maximum for the shaper input range.
    pub domain_max: [f32; 3],
}

/// Parse a LUT file, auto-detecting format from extension.
pub fn parse_lut_file(path: &Path) -> Result<ParsedLut> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read LUT file: {}", path.display()))?;
    match ext.as_str() {
        "cube" => parse_cube(&content),
        "3dl" => parse_3dl(&content),
        _ => bail!("Unsupported LUT format: .{ext}. Supported: .cube, .3dl"),
    }
}

/// Parse a .cube file (Resolve/Adobe format).
fn parse_cube(content: &str) -> Result<ParsedLut> {
    let mut title: Option<String> = None;
    let mut size_1d: Option<u32> = None;
    let mut size_3d: Option<u32> = None;
    let mut domain_min_1d = [0.0f32; 3];
    let mut domain_max_1d = [1.0f32; 3];
    let mut domain_min_3d = [0.0f32; 3];
    let mut domain_max_3d = [1.0f32; 3];
    let mut data_values: Vec<f32> = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if let Some(rest) = line.strip_prefix("TITLE") {
            let rest = rest.trim().trim_matches('"');
            title = Some(rest.to_string());
            continue;
        }
        if let Some(rest) = line.strip_prefix("LUT_1D_SIZE") {
            size_1d = Some(rest.trim().parse().context("Invalid LUT_1D_SIZE")?);
            continue;
        }
        if let Some(rest) = line.strip_prefix("LUT_3D_SIZE") {
            size_3d = Some(rest.trim().parse().context("Invalid LUT_3D_SIZE")?);
            continue;
        }
        if let Some(rest) = line.strip_prefix("LUT_1D_INPUT_RANGE") {
            let vals = parse_float_line(rest, 2)?;
            domain_min_1d = [vals[0]; 3];
            domain_max_1d = [vals[1]; 3];
            continue;
        }
        if let Some(rest) = line.strip_prefix("LUT_3D_INPUT_RANGE") {
            let vals = parse_float_line(rest, 2)?;
            domain_min_3d = [vals[0]; 3];
            domain_max_3d = [vals[1]; 3];
            continue;
        }
        if let Some(rest) = line.strip_prefix("DOMAIN_MIN") {
            let vals = parse_float_line(rest, 3)?;
            if size_3d.is_some() || size_1d.is_none() {
                domain_min_3d = [vals[0], vals[1], vals[2]];
            }
            if size_1d.is_some() && size_3d.is_none() {
                domain_min_1d = [vals[0], vals[1], vals[2]];
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix("DOMAIN_MAX") {
            let vals = parse_float_line(rest, 3)?;
            if size_3d.is_some() || size_1d.is_none() {
                domain_max_3d = [vals[0], vals[1], vals[2]];
            }
            if size_1d.is_some() && size_3d.is_none() {
                domain_max_1d = [vals[0], vals[1], vals[2]];
            }
            continue;
        }

        // Skip unknown keywords (lines that don't start with a number)
        let first_char = line.chars().next().unwrap_or(' ');
        if !first_char.is_ascii_digit()
            && first_char != '-'
            && first_char != '+'
            && first_char != '.'
        {
            continue;
        }

        // Data line: 3 floats
        let vals = parse_float_line(line, 3)?;
        data_values.extend_from_slice(&vals);
    }

    let size_3d = size_3d.context("Missing LUT_3D_SIZE in .cube file")?;
    let expected_3d = (size_3d as usize).pow(3) * 3;

    // If we have both 1D and 3D, split the data
    let shaper = if let Some(s1d) = size_1d {
        let expected_1d = s1d as usize * 3;
        if data_values.len() < expected_1d + expected_3d {
            bail!(
                "Not enough data in .cube file: expected {} (1D) + {} (3D) values, got {}",
                expected_1d,
                expected_3d,
                data_values.len()
            );
        }
        let shaper_data: Vec<f32> = data_values.drain(..expected_1d).collect();
        Some(ShaperLut {
            size: s1d,
            data: shaper_data,
            domain_min: domain_min_1d,
            domain_max: domain_max_1d,
        })
    } else {
        None
    };

    if data_values.len() < expected_3d {
        bail!(
            "Not enough 3D LUT data in .cube file: expected {} values, got {}",
            expected_3d,
            data_values.len()
        );
    }

    Ok(ParsedLut {
        title,
        size_3d,
        data_3d: data_values[..expected_3d].to_vec(),
        domain_min: domain_min_3d,
        domain_max: domain_max_3d,
        shaper,
    })
}

/// Parse a .3dl file (broadcast/projection format).
/// .3dl files contain integer-encoded LUT values with a header line
/// specifying the input range.
fn parse_3dl(content: &str) -> Result<ParsedLut> {
    let mut data_lines: Vec<Vec<f32>> = Vec::new();
    let mut input_range: Option<(f32, f32)> = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            continue;
        }

        // Try to parse as numbers
        let nums: Result<Vec<f32>, _> = parts.iter().map(|s| s.parse::<f32>()).collect();
        let nums = match nums {
            Ok(n) => n,
            Err(_) => continue,
        };

        // First numeric line is the input range definition (any length).
        // Subsequent lines with exactly 3 values are data.
        if input_range.is_none() {
            let min_val = nums[0];
            let max_val = *nums.last().unwrap();
            input_range = Some((min_val, max_val));
            continue;
        }

        // Data line: 3 values (R G B)
        if nums.len() == 3 {
            data_lines.push(nums);
        }
    }

    if data_lines.is_empty() {
        bail!("No LUT data found in .3dl file");
    }

    // Determine grid size from cube root of data count
    let count = data_lines.len();
    let size = (count as f64).cbrt().round() as u32;
    if (size as usize).pow(3) != count {
        bail!(
            ".3dl file has {} data lines, which is not a perfect cube (closest: {}^3 = {})",
            count,
            size,
            (size as usize).pow(3)
        );
    }

    // Determine the normalization factor from max value in data
    let max_val = data_lines
        .iter()
        .flat_map(|v| v.iter())
        .cloned()
        .fold(0.0f32, f32::max);
    // Common .3dl ranges: 0-1023 (10-bit), 0-4095 (12-bit), 0-65535 (16-bit), or 0-1 (float)
    let scale = if max_val > 4095.0 {
        65535.0
    } else if max_val > 1023.0 {
        4095.0
    } else if max_val > 1.0 {
        1023.0
    } else {
        1.0
    };

    let mut data_3d = Vec::with_capacity(count * 3);
    for rgb in &data_lines {
        data_3d.push(rgb[0] / scale);
        data_3d.push(rgb[1] / scale);
        data_3d.push(rgb[2] / scale);
    }

    Ok(ParsedLut {
        title: None,
        size_3d: size,
        data_3d,
        domain_min: [0.0; 3],
        domain_max: [1.0; 3],
        shaper: None,
    })
}

fn parse_float_line(s: &str, expected: usize) -> Result<Vec<f32>> {
    let vals: Result<Vec<f32>, _> = s.split_whitespace().map(|v| v.parse::<f32>()).collect();
    let vals = vals.context("Failed to parse float values")?;
    if vals.len() < expected {
        bail!("Expected {} values, got {}", expected, vals.len());
    }
    Ok(vals)
}

// ---------------------------------------------------------------------------
// GPU pipeline
// ---------------------------------------------------------------------------

/// GPU uniform for LUT shader — matches LutParams in lut.wgsl.
/// Must be 16-byte aligned (64 bytes total).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct LutParams {
    domain_min: [f32; 3],
    has_shaper: u32,
    domain_max: [f32; 3],
    _pad: u32,
    shaper_domain_min: [f32; 3],
    _pad2: u32,
    shaper_domain_max: [f32; 3],
    _pad3: u32,
}

/// A loaded LUT on the GPU, ready to be applied as a post-process pass.
pub struct LoadedLut {
    /// The parsed source data (kept for serialization/display).
    pub title: Option<String>,
    /// Source filename for persistence.
    pub filename: String,
    /// Kept alive so the GPU resource backing `lut_3d_view` is not dropped.
    _lut_3d_texture: wgpu::Texture,
    lut_3d_view: wgpu::TextureView,
    /// Kept alive so the GPU resource backing `shaper_view` is not dropped.
    _shaper_texture: Option<wgpu::Texture>,
    shaper_view: Option<wgpu::TextureView>,
    params_buffer: wgpu::Buffer,
}

/// Pipeline for applying a 3D LUT (with optional 1D shaper) as a fullscreen pass.
pub struct LutPipeline {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    source_sampler: wgpu::Sampler,
    lut_sampler: wgpu::Sampler,
    /// 1x1 dummy texture kept alive so `dummy_shaper_view` is not dropped.
    _dummy_shaper_texture: wgpu::Texture,
    dummy_shaper_view: wgpu::TextureView,
}

impl LoadedLut {
    /// Upload a parsed LUT to the GPU.
    pub fn from_parsed(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        parsed: &ParsedLut,
        filename: String,
    ) -> Self {
        // Create 3D texture
        let size = parsed.size_3d;
        let lut_3d_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("LUT 3D Texture"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Convert RGB f32 triplets to RGBA f16 for GPU upload
        let rgba_data = rgb_f32_to_rgba_f16(&parsed.data_3d);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &lut_3d_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size * 8), // 8 bytes per Rgba16Float texel
                rows_per_image: Some(size),
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: size,
            },
        );
        let lut_3d_view = lut_3d_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create shaper texture if present (stored as Nx1 2D texture)
        let (shaper_texture, shaper_view) = if let Some(shaper) = &parsed.shaper {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Shaper 1D Texture"),
                size: wgpu::Extent3d {
                    width: shaper.size,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let rgba_data = rgb_f32_to_rgba_f16(&shaper.data);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(shaper.size * 8),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: shaper.size,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            (Some(tex), Some(view))
        } else {
            (None, None)
        };

        // Create params buffer
        let has_shaper = if parsed.shaper.is_some() { 1u32 } else { 0u32 };
        let shaper_ref = parsed.shaper.as_ref();
        let params = LutParams {
            domain_min: parsed.domain_min,
            has_shaper,
            domain_max: parsed.domain_max,
            _pad: 0,
            shaper_domain_min: shaper_ref.map_or([0.0; 3], |s| s.domain_min),
            _pad2: 0,
            shaper_domain_max: shaper_ref.map_or([1.0; 3], |s| s.domain_max),
            _pad3: 0,
        };
        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("LUT Params Buffer"),
            contents: bytemuck::cast_slice(&[params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            title: parsed.title.clone(),
            filename,
            _lut_3d_texture: lut_3d_texture,
            lut_3d_view,
            _shaper_texture: shaper_texture,
            shaper_view,
            params_buffer,
        }
    }
}

impl LutPipeline {
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Result<Self> {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LUT Bind Group Layout"),
            entries: &[
                // binding 0: source sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // binding 1: source texture (2D)
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 2: LUT params uniform
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(std::mem::size_of::<LutParams>() as u64),
                    },
                    count: None,
                },
                // binding 3: LUT sampler (3D trilinear)
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // binding 4: LUT 3D texture
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                // binding 5: shaper sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // binding 6: shaper 1D texture (as 2D Nx1)
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("LUT Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LUT Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/lut.wgsl").into()),
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("LUT Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview_mask: None,
            cache: None,
        });

        let source_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("LUT Source Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        let lut_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("LUT 3D Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Nearest,
            ..Default::default()
        });

        // Create dummy 1x1 shaper texture (bound when no shaper is active)
        let dummy_shaper_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dummy Shaper Texture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let dummy_shaper_view =
            dummy_shaper_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Ok(Self {
            pipeline,
            bind_group_layout,
            source_sampler,
            lut_sampler,
            _dummy_shaper_texture: dummy_shaper_texture,
            dummy_shaper_view,
        })
    }

    /// Run the LUT pass: reads from source_view, writes to target_view using the loaded LUT.
    pub fn render(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source_view: &wgpu::TextureView,
        target_view: &wgpu::TextureView,
        lut: &LoadedLut,
    ) {
        let shaper_view = lut.shaper_view.as_ref().unwrap_or(&self.dummy_shaper_view);

        let param_size = std::mem::size_of::<LutParams>() as u64;
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("LUT Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Sampler(&self.source_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(source_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &lut.params_buffer,
                        offset: 0,
                        size: Some(NonZeroU64::new(param_size).unwrap()),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.lut_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&lut.lut_3d_view),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&self.source_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(shaper_view),
                },
            ],
        });

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("LUT Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(0, &bind_group, &[]);
        render_pass.draw(0..3, 0..1);
    }
}

/// Convert RGB f32 triplets to RGBA f16 bytes for GPU upload.
fn rgb_f32_to_rgba_f16(rgb_data: &[f32]) -> Vec<u8> {
    let texel_count = rgb_data.len() / 3;
    let mut bytes = Vec::with_capacity(texel_count * 8); // 4 x f16 = 8 bytes per texel
    for i in 0..texel_count {
        let r = half::f16::from_f32(rgb_data[i * 3]);
        let g = half::f16::from_f32(rgb_data[i * 3 + 1]);
        let b = half::f16::from_f32(rgb_data[i * 3 + 2]);
        let a = half::f16::from_f32(1.0);
        bytes.extend_from_slice(&r.to_le_bytes());
        bytes.extend_from_slice(&g.to_le_bytes());
        bytes.extend_from_slice(&b.to_le_bytes());
        bytes.extend_from_slice(&a.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_cube() {
        let content = "\
LUT_3D_SIZE 2
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";
        let lut = parse_cube(content).unwrap();
        assert_eq!(lut.size_3d, 2);
        assert_eq!(lut.data_3d.len(), 8 * 3);
        assert!(lut.shaper.is_none());
        assert_eq!(lut.domain_min, [0.0; 3]);
        assert_eq!(lut.domain_max, [1.0; 3]);
    }

    #[test]
    fn parse_cube_with_title_and_domain() {
        let content = "\
TITLE \"Test LUT\"
LUT_3D_SIZE 2
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 1.0 1.0 1.0
# comment line
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";
        let lut = parse_cube(content).unwrap();
        assert_eq!(lut.title.as_deref(), Some("Test LUT"));
    }

    #[test]
    fn parse_cube_with_1d_shaper() {
        let content = "\
LUT_1D_SIZE 3
LUT_3D_SIZE 2
0.0 0.0 0.0
0.5 0.5 0.5
1.0 1.0 1.0
0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
";
        let lut = parse_cube(content).unwrap();
        assert_eq!(lut.size_3d, 2);
        let shaper = lut.shaper.unwrap();
        assert_eq!(shaper.size, 3);
        assert_eq!(shaper.data.len(), 9);
    }

    #[test]
    fn parse_minimal_3dl() {
        let content = "\
0 512 1023
0 0 0
1023 0 0
0 1023 0
1023 1023 0
0 0 1023
1023 0 1023
0 1023 1023
1023 1023 1023
";
        let lut = parse_3dl(content).unwrap();
        assert_eq!(lut.size_3d, 2);
        assert_eq!(lut.data_3d.len(), 8 * 3);
        assert!((lut.data_3d[0] - 0.0).abs() < 0.001);
        assert!((lut.data_3d[3] - 1.0).abs() < 0.001);
    }

    #[test]
    fn parse_cube_missing_size_fails() {
        let content = "0.0 0.0 0.0\n1.0 1.0 1.0\n";
        assert!(parse_cube(content).is_err());
    }

    #[test]
    fn parse_3dl_non_cube_count_fails() {
        let content = "\
0 1023
0 0 0
1023 0 0
0 1023 0
";
        assert!(parse_3dl(content).is_err());
    }
}
