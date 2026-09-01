//! Deck constructors — creating decks from shaders, videos, images, cameras, and solid colors.

use super::{
    svg, Deck, DeckSource, Effect, ExternalSourceKind, PassBuffer, ScalingMode, VideoStagingBuffers,
};
use crate::isf::{compile_glsl_compute_to_spirv, compile_glsl_to_spirv, ISFMetadata, ISFShader};
use crate::params::ShaderParams;
use crate::renderer::{
    BlitPipeline, ComputePipeline, DispatchMode, GpuContext, HapConvertPipeline, UnifiedPipeline,
};
use crate::video::{hap::HapPlayer, HapTextureFormat, VideoDecodeHandle, VideoPlayer};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

/// Create the pair of blit pipelines used by every external/live source deck:
/// `(replace, over_black)`. The REPLACE pipeline writes the source's straight
/// RGBA verbatim (used when the deck is flagged `transparent`, preserving alpha);
/// the `ALPHA_BLENDING` pipeline flattens the source over an opaque black clear (the
/// default, so an HTML source with alpha<1 stays opaque). See /spec/html-source.md §2.
fn external_blit_pipelines(context: &GpuContext) -> Result<(BlitPipeline, BlitPipeline)> {
    let replace = BlitPipeline::new(&context.device, context.compositing_format)?;
    let over_black = BlitPipeline::with_blend(
        &context.device,
        context.compositing_format,
        wgpu::BlendState::ALPHA_BLENDING,
    )?;
    Ok((replace, over_black))
}

/// Upload straight-alpha RGBA pixels as an image deck's source texture.
///
/// Shared with the SVG re-rasterization path, which replaces this texture
/// whenever the master resolution changes.
pub(super) fn upload_image_texture(
    context: &GpuContext,
    rgba: &image::RgbaImage,
) -> (wgpu::Texture, wgpu::TextureView) {
    let (img_w, img_h) = rgba.dimensions();
    let texture = context.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Image Source Texture"),
        size: wgpu::Extent3d {
            width: img_w,
            height: img_h,
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
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * img_w),
            rows_per_image: Some(img_h),
        },
        wgpu::Extent3d {
            width: img_w,
            height: img_h,
            depth_or_array_layers: 1,
        },
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

/// Load ISF IMPORTED images from metadata and create GPU textures.
/// Returns (name, texture, view) sorted alphabetically by name for deterministic binding order.
///
/// PNG decoding runs in parallel across threads; GPU uploads are sequential.
pub(crate) fn load_imported_textures(
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

    // Collect paths for parallel decode
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

    // Parallel PNG decode on threads, sequential GPU upload
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

    // GPU upload (fast, sequential)
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

    // Sort by name for deterministic binding order (threads may complete out of order)
    result.sort_by(|a, b| a.0.cmp(&b.0));

    let elapsed = t0.elapsed();
    log::info!(
        "IMPORTED: loaded {} textures in {:.0?} (parallel decode)",
        result.len(),
        elapsed
    );

    result
}

impl Deck {
    /// Create a new deck from an ISF shader
    ///
    /// # Errors
    ///
    /// Returns an error if the ISF fragment source fails to compile to SPIR-V,
    /// or if the render pipeline cannot be created.
    pub fn new(context: &GpuContext, shader: ISFShader, width: u32, height: u32) -> Result<Self> {
        // Compile to SPIR-V
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile shader to SPIR-V")?;

        // Extract passes from metadata
        let passes = shader.metadata.passes.clone().unwrap_or_default();

        // Create render target textures (two for ping-pong effect chain)
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture (Linear)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: context.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture B (Linear)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: context.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        // Create pass buffers (with ping-pong for persistent buffers)
        let mut pass_buffers = HashMap::new();

        for pass in &passes {
            let target_name = match &pass.target {
                Some(name) => name.clone(),
                None => continue,
            };

            let pass_width = Self::parse_size_expression(pass.width.as_deref(), width);
            let pass_height = Self::parse_size_expression(pass.height.as_deref(), height);
            let is_persistent = pass.persistent.unwrap_or(false);

            // All pass buffers use the unified color-path format. ISF `"FLOAT": true`
            // is still parsed but no longer selects a format — Rgba16Float is both
            // filterable and blendable, which Rgba32Float was not.
            let format = context.compositing_format;

            let tex_a = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Pass Buffer A: {target_name}")),
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
            let view_a = tex_a.create_view(&wgpu::TextureViewDescriptor::default());

            // Every pass buffer is double-buffered, persistent or not.
            //
            // The bind group binds *all* pass buffers as sampled textures on
            // every pass, so a single-textured target would be simultaneously
            // COLOR_TARGET and RESOURCE in its own pass — which wgpu rejects
            // outright. Persistent buffers were already safe because they
            // ping-pong; non-persistent ones aliased and aborted the process
            // the first time anyone declared one.
            let (tex_b_buf, view_b) = {
                let tex = context.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!("Pass Buffer B: {target_name}")),
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
                let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
                (Some(tex), Some(view))
            };

            pass_buffers.insert(
                target_name.clone(),
                PassBuffer {
                    name: target_name,
                    texture_a: tex_a,
                    view_a,
                    texture_b: tex_b_buf,
                    view_b,
                    persistent: is_persistent,
                    read_idx: 0,
                },
            );
        }

        // Load ISF IMPORTED images
        let imported_textures =
            load_imported_textures(&shader.metadata, shader.file_path.as_deref(), context);

        // Create preprocessor texture slots from ISF PREPROCESSORS declarations
        let preprocessor_textures: Vec<super::PreprocessorSlot> = shader
            .metadata
            .preprocessors
            .iter()
            .map(|pp| {
                // Data texture (packed analyzer output) — NOT part of the
                // color path. The declared FORMAT is the encoding contract.
                let format = super::preprocessor_texture_format(&pp.format);
                let texture = context.device.create_texture(&wgpu::TextureDescriptor {
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
                super::PreprocessorSlot {
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
            .collect();

        let preprocessor_filterable: Vec<bool> = preprocessor_textures
            .iter()
            .map(|slot| slot.format != wgpu::TextureFormat::Rgba32Float)
            .collect();
        let pipeline = UnifiedPipeline::new(
            &context.device,
            &spirv,
            context.compositing_format,
            false,
            pass_buffers.len(),
            imported_textures.len(),
            &preprocessor_filterable,
        )
        .context("Failed to create shader pipeline")?;

        let now = Instant::now();
        let source_name = shader.name();

        let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);
        let generator_params = ShaderParams::from_inputs(inputs);
        let generator_phase_inputs = shader.metadata.phase_inputs.clone();

        let source = DeckSource::Shader {
            shader,
            pipeline,
            pass_buffers,
            passes,
            imported_textures,
            preprocessor_textures,
        };

        let source_path = match &source {
            DeckSource::Shader { shader, .. } => shader.file_path.clone(),
            _ => None,
        };

        let uuid = super::generate_short_uuid();
        let param_prefix = format!("deck_{uuid}");

        Ok(Self {
            uuid,
            param_prefix,
            source_name,
            source_path,
            source,
            generator_params,
            texture,
            texture_view,
            texture_b,
            texture_b_view,
            effects: Vec::new(),
            opacity: 1.0,
            transparent: false,
            render_time: 0.0,
            render_dt: 1.0 / 60.0,
            frame_count: 0,
            last_frame_time: now,
            external_source_view: None,
            depth_rgb_view: None,
            depth_intrinsics: None,
            depth_source_size: None,
            point_cloud_params: crate::depth::point_cloud::PointCloudParams::default(),
            point_cloud_pipeline: None,
            depth_prepro: None,
            screen_capture: None,
            tap: None,
            fps_smoothed: 0.0,
            phase_accumulators: [0.0; 4],
            generator_phase_inputs,
            analyzers: crate::analyzer::DeckAnalyzers::new(),
            gpu_error: None,
        })
    }

    /// Parse ISF size expressions like "$WIDTH", "$WIDTH/2", "1024", etc.
    pub(crate) fn parse_size_expression(expr: Option<&str>, base_size: u32) -> u32 {
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

    /// Add an effect (ISF filter) to this deck's effect chain
    pub fn add_effect(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    /// Remove an effect from this deck's effect chain
    pub fn remove_effect(&mut self, index: usize) -> Option<Effect> {
        if index < self.effects.len() {
            Some(self.effects.remove(index))
        } else {
            None
        }
    }

    /// Create a new deck from a video file.
    /// Auto-detects HAP codec and uses GPU-native `BCn` path when available.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be opened as a HAP or generic video
    /// stream, or if the blit/HAP-convert pipelines cannot be created.
    pub fn new_from_video<P: AsRef<Path>>(
        context: &GpuContext,
        path: P,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_path_str = path.as_ref().to_string_lossy().to_string();
        let source_name = path
            .as_ref()
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("video")
            .to_string();

        let gpu_has_bc = context
            .device
            .features()
            .contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let hap_format = if gpu_has_bc {
            crate::video::detect_hap_codec(&path).ok().flatten()
        } else {
            None
        };

        let source = if let Some(hap_fmt) = hap_format {
            let player = HapPlayer::new(&path, hap_fmt)?;
            let vid_w = player.width();
            let vid_h = player.height();
            let tex_format = hap_fmt.wgpu_format();

            let video_texture = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("HAP Video Texture"),
                size: wgpu::Extent3d {
                    width: vid_w,
                    height: vid_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: tex_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let video_texture_view =
                video_texture.create_view(&wgpu::TextureViewDescriptor::default());

            let (alpha_texture, alpha_texture_view) =
                if matches!(hap_fmt, HapTextureFormat::Bc3YCoCg) {
                    let alpha_tex = context.device.create_texture(&wgpu::TextureDescriptor {
                        label: Some("HAP Alpha Texture"),
                        size: wgpu::Extent3d {
                            width: vid_w,
                            height: vid_h,
                            depth_or_array_layers: 1,
                        },
                        mip_level_count: 1,
                        sample_count: 1,
                        dimension: wgpu::TextureDimension::D2,
                        format: wgpu::TextureFormat::Bc4RUnorm,
                        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                        view_formats: &[],
                    });
                    let alpha_view = alpha_tex.create_view(&wgpu::TextureViewDescriptor::default());
                    (Some(alpha_tex), Some(alpha_view))
                } else {
                    (None, None)
                };

            let dummy_alpha = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("HAP Dummy Alpha"),
                size: wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            context.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &dummy_alpha,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[255u8],
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(1),
                    rows_per_image: Some(1),
                },
                wgpu::Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
            );
            let dummy_alpha_view = dummy_alpha.create_view(&wgpu::TextureViewDescriptor::default());

            let convert_pipeline =
                HapConvertPipeline::new(&context.device, context.compositing_format)?;
            let blit_pipeline = BlitPipeline::new(&context.device, context.compositing_format)?;

            log::info!("Using HAP GPU path for '{source_name}' ({hap_fmt:?})");

            let blocks_x = vid_w.div_ceil(4);
            let blocks_y = vid_h.div_ceil(4);
            let color_bpr = blocks_x * hap_fmt.block_bytes();
            let staging =
                VideoStagingBuffers::new(&context.device, color_bpr, blocks_y, "HAP Color");

            let alpha_staging = if matches!(hap_fmt, HapTextureFormat::Bc3YCoCg) {
                let alpha_bpr = blocks_x * HapTextureFormat::Bc4.block_bytes();
                Some(VideoStagingBuffers::new(
                    &context.device,
                    alpha_bpr,
                    blocks_y,
                    "HAP Alpha",
                ))
            } else {
                None
            };

            let handle = VideoDecodeHandle::spawn_hap(player);

            DeckSource::HapVideo {
                handle,
                texture: video_texture,
                texture_view: video_texture_view,
                alpha_texture,
                alpha_texture_view,
                dummy_alpha_view,
                convert_pipeline,
                blit_pipeline,
                hap_format: hap_fmt,
                source_width: vid_w,
                source_height: vid_h,
                scaling_mode: ScalingMode::default(),
                staging,
                alpha_staging,
            }
        } else {
            let player = VideoPlayer::new(&path)?;
            let vid_w = player.width();
            let vid_h = player.height();
            let video_texture = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Video Frame Texture"),
                size: wgpu::Extent3d {
                    width: vid_w,
                    height: vid_h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let video_texture_view =
                video_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let blit_pipeline = BlitPipeline::new(&context.device, context.compositing_format)?;

            let staging = VideoStagingBuffers::new(
                &context.device,
                vid_w * 4, // RGBA = 4 bytes per pixel
                vid_h,
                "Video",
            );

            let handle = VideoDecodeHandle::spawn_video(player);

            DeckSource::Video {
                handle,
                texture: video_texture,
                texture_view: video_texture_view,
                blit_pipeline,
                source_width: vid_w,
                source_height: vid_h,
                scaling_mode: ScalingMode::default(),
                staging,
            }
        };

        Ok(Self::build_media_deck(
            context,
            source_name,
            Some(source_path_str),
            source,
            width,
            height,
        ))
    }

    /// Create a new deck from an image file (PNG, JPG, BMP, SVG, etc.)
    ///
    /// SVG is rasterized rather than decoded: it has no native pixel size, so
    /// it is rendered to fill a `width × height` deck and re-rendered if that
    /// changes. See [`svg`](crate::deck::svg).
    ///
    /// # Errors
    ///
    /// Returns an error if the image cannot be opened, decoded or rasterized,
    /// or if the blit pipeline cannot be created.
    pub fn new_from_image<P: AsRef<Path>>(
        context: &GpuContext,
        path: P,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let path = path.as_ref();
        let source_path_str = path.to_string_lossy().to_string();
        let source_name = path
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("image")
            .to_string();

        if svg::is_svg_path(path) {
            let tree = svg::parse_file(path)?;
            let rgba = svg::rasterize(&tree, width, height)?;
            return Self::new_image_deck(
                context,
                &rgba,
                Some(tree),
                source_name,
                source_path_str,
                width,
                height,
            );
        }

        let img = image::open(path)
            .with_context(|| format!("Failed to load image: {}", path.display()))?;
        let rgba = img.to_rgba8();
        Self::new_image_deck(
            context,
            &rgba,
            None,
            source_name,
            source_path_str,
            width,
            height,
        )
    }

    /// Create a new deck from decoded pixels. `tree` is carried by SVG decks so
    /// a later resolution change can re-render the artwork instead of scaling
    /// up the pixels it happened to be rasterized at first.
    fn new_image_deck(
        context: &GpuContext,
        rgba: &image::RgbaImage,
        tree: Option<usvg::Tree>,
        source_name: String,
        source_path_str: String,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let (img_w, img_h) = rgba.dimensions();
        let (img_texture, img_texture_view) = upload_image_texture(context, rgba);
        let blit_pipeline = BlitPipeline::new(&context.device, context.compositing_format)?;

        let source = DeckSource::Image {
            texture: img_texture,
            texture_view: img_texture_view,
            blit_pipeline,
            source_width: img_w,
            source_height: img_h,
            scaling_mode: ScalingMode::default(),
            svg: tree.map(Box::new),
        };

        Ok(Self::build_media_deck(
            context,
            source_name,
            Some(source_path_str),
            source,
            width,
            height,
        ))
    }

    /// Create a new deck with a solid color fill
    ///
    /// # Errors
    ///
    /// Never fails today — a solid-color source needs no pipeline. The `Result`
    /// keeps this constructor uniform with the other `new_from_*` constructors.
    pub fn new_solid_color(
        context: &GpuContext,
        color: [f32; 4],
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_name = format!(
            "#{:02X}{:02X}{:02X}",
            (color[0] * 255.0) as u8,
            (color[1] * 255.0) as u8,
            (color[2] * 255.0) as u8,
        );

        let source = DeckSource::SolidColor {
            color: [
                f64::from(color[0]),
                f64::from(color[1]),
                f64::from(color[2]),
                f64::from(color[3]),
            ],
        };

        Ok(Self::build_media_deck(
            context,
            source_name,
            None,
            source,
            width,
            height,
        ))
    }

    /// Create a new deck from a camera source.
    /// The camera is managed by `CameraManager` — this deck reads from the shared texture.
    ///
    /// # Errors
    ///
    /// Returns an error if the external-source blit pipelines cannot be created.
    pub fn new_from_camera(
        context: &GpuContext,
        camera_id: crate::camera::CameraId,
        camera_name: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_name = format!("📹 {camera_name}");
        let (blit_pipeline, blit_pipeline_over_black) = external_blit_pipelines(context)?;

        let source = DeckSource::ExternalSource {
            kind: ExternalSourceKind::Camera(camera_id),
            blit_pipeline,
            blit_pipeline_over_black,
            source_width,
            source_height,
            scaling_mode: ScalingMode::default(),
        };

        Ok(Self::build_media_deck(
            context,
            source_name,
            None,
            source,
            width,
            height,
        ))
    }

    /// Create a new deck from a screen or window capture.
    /// The capture is managed by `ScreenCaptureManager` — this deck reads from
    /// the shared texture. See spec/screen-capture.md.
    ///
    /// # Errors
    ///
    /// Returns an error if the external-source blit pipelines cannot be created.
    pub fn new_from_screen_capture(
        context: &GpuContext,
        state: crate::deck::ScreenCaptureState,
        target_label: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_name = format!("🖥 {target_label}");
        let (blit_pipeline, blit_pipeline_over_black) = external_blit_pipelines(context)?;

        let source = DeckSource::ExternalSource {
            kind: ExternalSourceKind::ScreenCapture(state.capture_id),
            blit_pipeline,
            blit_pipeline_over_black,
            source_width,
            source_height,
            scaling_mode: ScalingMode::default(),
        };

        let mut deck = Self::build_media_deck(context, source_name, None, source, width, height);
        deck.screen_capture = Some(state);
        Ok(deck)
    }

    /// Create a new deck that re-enters Varda's own output as a source.
    ///
    /// The tap owns no device and holds no handle: the mixer supplies a texture
    /// view each frame, and a source that cannot be resolved renders black.
    /// See spec/program-tap.md.
    ///
    /// # Errors
    ///
    /// Returns an error if the external-source blit pipelines cannot be created.
    pub fn new_from_tap(
        context: &GpuContext,
        source: crate::deck::TapSource,
        label: &str,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_name = format!("🔁 {label}");
        let (blit_pipeline, blit_pipeline_over_black) = external_blit_pipelines(context)?;

        let deck_source = DeckSource::ExternalSource {
            kind: ExternalSourceKind::Tap,
            blit_pipeline,
            blit_pipeline_over_black,
            // A tap always matches the render resolution, so the default
            // scaling mode is an exact 1:1 blit until the user changes it.
            source_width: width,
            source_height: height,
            scaling_mode: ScalingMode::default(),
        };

        let mut deck =
            Self::build_media_deck(context, source_name, None, deck_source, width, height);
        deck.tap = Some(crate::deck::TapState { source });
        Ok(deck)
    }

    /// Create a new deck from a depth sensor (Kinect/LIDAR point cloud).
    /// The sensor is managed by `DepthSensorManager`; this deck reprojects the
    /// shared depth texture as a point cloud. See spec/depth-sensors.md.
    ///
    /// # Errors
    ///
    /// Returns an error if the external-source blit pipelines cannot be created.
    pub fn new_from_depth_sensor(
        context: &GpuContext,
        sensor_id: crate::depth::DepthSensorId,
        sensor_name: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_name = format!("🛰 {sensor_name}");
        let (blit_pipeline, blit_pipeline_over_black) = external_blit_pipelines(context)?;

        let source = DeckSource::ExternalSource {
            kind: ExternalSourceKind::DepthSensor(sensor_id),
            blit_pipeline,
            blit_pipeline_over_black,
            source_width,
            source_height,
            scaling_mode: ScalingMode::default(),
        };

        Ok(Self::build_media_deck(
            context,
            source_name,
            None,
            source,
            width,
            height,
        ))
    }

    /// Shared helper to build a Deck from a pre-built `DeckSource` with standard render targets.
    fn build_media_deck(
        context: &GpuContext,
        source_name: String,
        source_path: Option<String>,
        source: DeckSource,
        width: u32,
        height: u32,
    ) -> Self {
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: context.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture B"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: context.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        let now = Instant::now();
        let generator_params = ShaderParams::from_inputs(&[]);

        let uuid = super::generate_short_uuid();
        let param_prefix = format!("deck_{uuid}");

        Self {
            uuid,
            param_prefix,
            source_name,
            source_path,
            source,
            generator_params,
            texture,
            texture_view,
            texture_b,
            texture_b_view,
            effects: Vec::new(),
            opacity: 1.0,
            transparent: false,
            render_time: 0.0,
            render_dt: 1.0 / 60.0,
            frame_count: 0,
            last_frame_time: now,
            external_source_view: None,
            depth_rgb_view: None,
            depth_intrinsics: None,
            depth_source_size: None,
            point_cloud_params: crate::depth::point_cloud::PointCloudParams::default(),
            point_cloud_pipeline: None,
            depth_prepro: None,
            screen_capture: None,
            tap: None,
            fps_smoothed: 0.0,
            phase_accumulators: [0.0; 4],
            generator_phase_inputs: None,
            analyzers: crate::analyzer::DeckAnalyzers::new(),
            gpu_error: None,
        }
    }

    /// Create a new deck from an external source (NDI, Syphon, SRT, HLS, DASH, RTMP).
    ///
    /// # Errors
    ///
    /// Returns an error if the external-source blit pipelines cannot be created.
    pub fn new_from_external(
        context: &GpuContext,
        kind: ExternalSourceKind,
        display_name: String,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let (blit_pipeline, blit_pipeline_over_black) = external_blit_pipelines(context)?;

        let source = DeckSource::ExternalSource {
            kind,
            blit_pipeline,
            blit_pipeline_over_black,
            source_width,
            source_height,
            scaling_mode: ScalingMode::default(),
        };

        Ok(Self::build_media_deck(
            context,
            display_name,
            None,
            source,
            width,
            height,
        ))
    }

    /// Create a new deck from an NDI network source.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Deck::new_from_external`].
    pub fn new_from_ndi(
        context: &GpuContext,
        receiver_idx: usize,
        source_name: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_from_external(
            context,
            ExternalSourceKind::Ndi(receiver_idx),
            format!("📡 {source_name}"),
            source_width,
            source_height,
            width,
            height,
        )
    }

    /// Create a new deck from a Syphon server (macOS inter-app sharing).
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Deck::new_from_external`].
    pub fn new_from_syphon(
        context: &GpuContext,
        client_idx: usize,
        server_name: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_from_external(
            context,
            ExternalSourceKind::Syphon(client_idx),
            format!("🔗 {server_name}"),
            source_width,
            source_height,
            width,
            height,
        )
    }

    /// Create a new deck from an SRT network source.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Deck::new_from_external`].
    pub fn new_from_srt(
        context: &GpuContext,
        receiver_idx: usize,
        url: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_from_external(
            context,
            ExternalSourceKind::Srt(receiver_idx),
            format!("📺 {url}"),
            source_width,
            source_height,
            width,
            height,
        )
    }

    /// Create a new deck from an HLS stream source.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Deck::new_from_external`].
    pub fn new_from_hls(
        context: &GpuContext,
        receiver_idx: usize,
        url: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_from_external(
            context,
            ExternalSourceKind::Hls(receiver_idx),
            format!("📡 {url}"),
            source_width,
            source_height,
            width,
            height,
        )
    }

    /// Create a new deck from a DASH stream source.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Deck::new_from_external`].
    pub fn new_from_dash(
        context: &GpuContext,
        receiver_idx: usize,
        url: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_from_external(
            context,
            ExternalSourceKind::Dash(receiver_idx),
            format!("📡 {url}"),
            source_width,
            source_height,
            width,
            height,
        )
    }

    /// Create a new deck from an RTMP stream source.
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Deck::new_from_external`].
    pub fn new_from_rtmp(
        context: &GpuContext,
        receiver_idx: usize,
        url: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_from_external(
            context,
            ExternalSourceKind::Rtmp(receiver_idx),
            format!("📺 {url}"),
            source_width,
            source_height,
            width,
            height,
        )
    }

    /// Create a new deck from an HTML source (Servo offscreen render).
    ///
    /// # Errors
    ///
    /// Propagates errors from [`Deck::new_from_external`].
    pub fn new_from_html(
        context: &GpuContext,
        instance_idx: usize,
        url: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        Self::new_from_external(
            context,
            ExternalSourceKind::Html(instance_idx),
            format!("🌐 {url}"),
            source_width,
            source_height,
            width,
            height,
        )
    }

    /// Create a new deck from a GLSL compute shader (.comp file)
    ///
    /// # Errors
    ///
    /// Returns an error if the shader has no `COMPUTE` configuration block, if
    /// the GLSL fails to compile to SPIR-V, or if the compute pipeline cannot
    /// be created.
    pub fn new_from_compute_shader(
        context: &GpuContext,
        shader: ISFShader,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let compute_config = shader
            .metadata
            .compute
            .as_ref()
            .context("Compute shader missing COMPUTE configuration")?;

        // Compile GLSL compute to SPIR-V
        let spirv = compile_glsl_compute_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile compute shader to SPIR-V")?;

        let workgroup_size = compute_config.workgroup_size;
        let dispatch_mode = match compute_config.dispatch.as_str() {
            "custom" => {
                log::warn!("Custom dispatch mode not yet implemented, using resolution");
                DispatchMode::Resolution
            }
            _ => DispatchMode::Resolution,
        };

        let num_passes = compute_config.num_passes;

        let pipeline = ComputePipeline::new(
            &context.device,
            &spirv,
            width,
            height,
            &shader.metadata.buffers,
            workgroup_size,
            dispatch_mode,
            num_passes,
        )
        .context("Failed to create compute pipeline")?;

        let source_name = shader.name();
        let source_path = shader.file_path.clone();

        let inputs = shader.metadata.inputs.as_deref().unwrap_or(&[]);
        let generator_params = ShaderParams::from_inputs(inputs);
        let generator_phase_inputs = shader.metadata.phase_inputs.clone();

        let source = DeckSource::ComputeShader { shader, pipeline };

        // Create render target textures
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture (Compute)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: context.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture B (Compute)"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: context.compositing_format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        let now = Instant::now();
        let uuid = super::generate_short_uuid();
        let param_prefix = format!("deck_{uuid}");

        Ok(Self {
            uuid,
            param_prefix,
            source_name,
            source_path,
            source,
            generator_params,
            texture,
            texture_view,
            texture_b,
            texture_b_view,
            effects: Vec::new(),
            opacity: 1.0,
            transparent: false,
            render_time: 0.0,
            render_dt: 1.0 / 60.0,
            frame_count: 0,
            last_frame_time: now,
            external_source_view: None,
            depth_rgb_view: None,
            depth_intrinsics: None,
            depth_source_size: None,
            point_cloud_params: crate::depth::point_cloud::PointCloudParams::default(),
            point_cloud_pipeline: None,
            depth_prepro: None,
            screen_capture: None,
            tap: None,
            fps_smoothed: 0.0,
            phase_accumulators: [0.0; 4],
            generator_phase_inputs,
            analyzers: crate::analyzer::DeckAnalyzers::new(),
            gpu_error: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_size_none_returns_base() {
        assert_eq!(Deck::parse_size_expression(None, 1920), 1920);
    }

    #[test]
    fn parse_size_width_variable() {
        assert_eq!(Deck::parse_size_expression(Some("$WIDTH"), 1920), 1920);
        assert_eq!(Deck::parse_size_expression(Some("$HEIGHT"), 1080), 1080);
    }

    #[test]
    fn parse_size_divide() {
        assert_eq!(Deck::parse_size_expression(Some("$WIDTH/2"), 1920), 960);
        assert_eq!(Deck::parse_size_expression(Some("$HEIGHT/4"), 1080), 270);
    }

    #[test]
    fn parse_size_multiply() {
        assert_eq!(Deck::parse_size_expression(Some("$WIDTH*2"), 960), 1920);
        assert_eq!(Deck::parse_size_expression(Some("$HEIGHT*3"), 360), 1080);
    }

    #[test]
    fn parse_size_literal() {
        assert_eq!(Deck::parse_size_expression(Some("512"), 1920), 512);
        assert_eq!(Deck::parse_size_expression(Some("1024"), 1080), 1024);
    }

    #[test]
    fn parse_size_invalid_literal_falls_back() {
        assert_eq!(Deck::parse_size_expression(Some("abc"), 1920), 1920);
    }

    #[test]
    fn parse_size_divide_by_zero_safe() {
        assert_eq!(Deck::parse_size_expression(Some("$WIDTH/0"), 1920), 1920);
    }

    #[test]
    fn parse_size_whitespace_trim() {
        assert_eq!(Deck::parse_size_expression(Some(" $WIDTH "), 1920), 1920);
    }
}
