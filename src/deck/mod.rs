use crate::isf::{ISFShader, ISFPass, compile_glsl_to_spirv};
use crate::modulation::ModulationEngine;
use crate::params::ShaderParams;
use crate::renderer::{RenderContext, UnifiedPipeline, ISFUniforms, BlitPipeline, HapConvertPipeline};
use crate::video::{VideoPlayer, HapTextureFormat, hap::HapPlayer};
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

/// Scaling mode for non-shader sources (images, video)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScalingMode {
    /// Scale to fill the entire target, cropping edges if aspect ratio differs
    Fill,
    /// Scale to fit within the target, letterboxing if aspect ratio differs
    Fit,
    /// Stretch to exactly match target dimensions (may distort)
    Stretch,
    /// No scaling, center at native resolution
    Center,
}

impl Default for ScalingMode {
    fn default() -> Self {
        ScalingMode::Fill
    }
}

impl ScalingMode {
    /// Compute UV scale and offset for blitting source into target
    /// Returns (uv_scale, uv_offset) to transform target UVs to source UVs
    pub fn compute_uv_transform(
        &self,
        source_w: u32, source_h: u32,
        target_w: u32, target_h: u32,
    ) -> ([f32; 2], [f32; 2]) {
        let src_aspect = source_w as f32 / source_h as f32;
        let tgt_aspect = target_w as f32 / target_h as f32;

        match self {
            ScalingMode::Stretch => {
                // 1:1 UV mapping, no transform needed
                ([1.0, 1.0], [0.0, 0.0])
            }
            ScalingMode::Fill => {
                // Scale to fill: crop the dimension that overflows
                if src_aspect > tgt_aspect {
                    // Source is wider: crop horizontally
                    let scale_x = tgt_aspect / src_aspect;
                    let offset_x = (1.0 - scale_x) * 0.5;
                    ([scale_x, 1.0], [offset_x, 0.0])
                } else {
                    // Source is taller: crop vertically
                    let scale_y = src_aspect / tgt_aspect;
                    let offset_y = (1.0 - scale_y) * 0.5;
                    ([1.0, scale_y], [0.0, offset_y])
                }
            }
            ScalingMode::Fit => {
                // Scale to fit: letterbox/pillarbox
                if src_aspect > tgt_aspect {
                    // Source is wider: pillarbox (bars top/bottom in UV terms)
                    let scale_y = src_aspect / tgt_aspect;
                    let offset_y = (1.0 - scale_y) * 0.5;
                    ([1.0, scale_y], [0.0, offset_y])
                } else {
                    // Source is taller: letterbox (bars left/right)
                    let scale_x = tgt_aspect / src_aspect;
                    let offset_x = (1.0 - scale_x) * 0.5;
                    ([scale_x, 1.0], [offset_x, 0.0])
                }
            }
            ScalingMode::Center => {
                // No scaling, center at native resolution
                let scale_x = target_w as f32 / source_w as f32;
                let scale_y = target_h as f32 / source_h as f32;
                let offset_x = (1.0 - scale_x) * 0.5;
                let offset_y = (1.0 - scale_y) * 0.5;
                ([scale_x, scale_y], [offset_x, offset_y])
            }
        }
    }
}

/// Source type for a deck - what generates the base image
pub enum DeckSource {
    /// ISF shader generator
    Shader {
        shader: ISFShader,
        pipeline: UnifiedPipeline,
        pass_buffers: HashMap<String, PassBuffer>,
        passes: Vec<ISFPass>,
    },
    /// Video file playback (ffmpeg CPU decode → RGBA)
    Video {
        player: VideoPlayer,
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        blit_pipeline: BlitPipeline,
    },
    /// HAP video playback (GPU-native BCn compressed textures)
    HapVideo {
        player: HapPlayer,
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        /// Alpha-plane texture (for HAP Q Alpha dual-plane)
        alpha_texture: Option<wgpu::Texture>,
        alpha_texture_view: Option<wgpu::TextureView>,
        /// Dummy 1x1 texture for single-plane (shader always needs a binding)
        dummy_alpha_view: wgpu::TextureView,
        /// YCoCg/dual-plane conversion pipeline (used instead of blit for HAP Q/Q Alpha)
        convert_pipeline: HapConvertPipeline,
        blit_pipeline: BlitPipeline,
        hap_format: HapTextureFormat,
    },
    /// Static image
    Image {
        texture: wgpu::Texture,
        texture_view: wgpu::TextureView,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
    /// Solid color fill
    SolidColor {
        color: [f64; 4], // RGBA as f64 for wgpu::Color compatibility
    },
    /// Live camera feed (reads shared texture from CameraManager)
    Camera {
        camera_id: crate::camera::CameraId,
        blit_pipeline: BlitPipeline,
        source_width: u32,
        source_height: u32,
        scaling_mode: ScalingMode,
    },
}

/// An effect in the deck's effect chain (ISF filter)
pub struct Effect {
    /// The ISF filter shader
    pub shader: ISFShader,
    /// The unified pipeline (handles simple and multi-pass filters)
    pub pipeline: UnifiedPipeline,
    /// Effect enabled state
    pub enabled: bool,
    /// User-controllable parameters
    pub params: ShaderParams,
    /// Pass buffers for multi-pass effects (ping-pong textures)
    pub pass_buffers: HashMap<String, PassBuffer>,
    /// ISF pass definitions (from metadata)
    pub passes: Vec<ISFPass>,
    /// Target texture format (needed for final pass pipeline selection)
    pub target_format: wgpu::TextureFormat,
}

impl Effect {
    /// Create a new effect from an ISF filter shader
    pub fn new(context: &RenderContext, shader: ISFShader) -> Result<Self> {
        Self::new_with_format(context, shader, wgpu::TextureFormat::Rgba8Unorm)
    }

    /// Create a new effect with a specific target format
    pub fn new_with_format(context: &RenderContext, shader: ISFShader, target_format: wgpu::TextureFormat) -> Result<Self> {
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile filter shader to SPIR-V")?;

        let passes: Vec<ISFPass> = shader.metadata.passes.clone().unwrap_or_default();
        let num_passes = passes.iter().filter(|p| p.target.is_some()).count();
        let uses_float = passes.iter().any(|p| p.float.unwrap_or(false));

        let pipeline = UnifiedPipeline::new(
            &context.device,
            &spirv,
            target_format,
            true,  // has_input_image — it's a filter
            num_passes,
            uses_float,
        ).context("Failed to create effect pipeline")?;

        // Create pass buffers for multi-pass effects
        let width = 1920u32;  // Internal resolution
        let height = 1080u32;
        let mut pass_buffers = HashMap::new();

        for pass in &passes {
            let target_name = match &pass.target {
                Some(name) => name.clone(),
                None => continue,
            };

            let pass_width = Deck::parse_size_expression(&pass.width, width);
            let pass_height = Deck::parse_size_expression(&pass.height, height);
            let is_persistent = pass.persistent.unwrap_or(false);

            let format = if pass.float.unwrap_or(false) {
                wgpu::TextureFormat::Rgba32Float
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            };

            let tex_a = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Effect Pass Buffer A: {}", target_name)),
                size: wgpu::Extent3d { width: pass_width, height: pass_height, depth_or_array_layers: 1 },
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

            let (tex_b, view_b) = if is_persistent {
                let tex = context.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!("Effect Pass Buffer B: {}", target_name)),
                    size: wgpu::Extent3d { width: pass_width, height: pass_height, depth_or_array_layers: 1 },
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
            } else {
                (None, None)
            };

            pass_buffers.insert(target_name.clone(), PassBuffer {
                name: target_name,
                texture_a: tex_a,
                view_a,
                texture_b: tex_b,
                view_b,
                persistent: is_persistent,
                read_idx: 0,
            });
        }

        // Initialize parameters from shader inputs
        let inputs = shader.metadata.inputs.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
        let params = ShaderParams::from_inputs(inputs);

        Ok(Self {
            shader,
            pipeline,
            enabled: true,
            params,
            pass_buffers,
            passes,
            target_format,
        })
    }

    /// Apply this effect to an input texture, outputting to target texture
    /// Optionally applies modulation to effect parameters using the given prefix
    pub fn apply(
        &mut self,
        context: &RenderContext,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        self.apply_with_modulation(context, input_view, output_view, uniforms, None, None, cmd_buffers)
    }

    /// Apply this effect with modulation support
    pub fn apply_with_modulation(
        &mut self,
        context: &RenderContext,
        input_view: &wgpu::TextureView,
        output_view: &wgpu::TextureView,
        uniforms: &ISFUniforms,
        modulation: Option<&crate::modulation::ModulationEngine>,
        mod_prefix: Option<&str>,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        // Ensure user params buffer exists and update it (with modulation if available)
        self.params.ensure_buffer(&context.device);
        if let Some(mod_engine) = modulation {
            self.params.update_buffer_with_modulation(&context.queue, mod_engine, mod_prefix);
        } else {
            self.params.update_buffer(&context.queue);
        }
        let user_params_buffer = self.params.buffer().expect("Buffer should exist after ensure_buffer");

        let has_targeted_passes = self.passes.iter().any(|p| p.target.is_some());

        if has_targeted_passes {
            // Multi-pass effect: run targeted passes first, then final pass to output
            for pass_idx in 0..self.passes.len() {
                let pass = &self.passes[pass_idx];

                let target_name = match &pass.target {
                    Some(name) => name.clone(),
                    None => continue, // Final pass handled below
                };

                let format = if pass.float.unwrap_or(false) {
                    wgpu::TextureFormat::Rgba32Float
                } else {
                    wgpu::TextureFormat::Rgba8Unorm
                };

                // Effects use 1 iteration for persistent passes — the persistence means
                // "keep buffer contents between frames", not "run multiple simulation steps".
                // Multi-iteration is only for generator simulation passes (reaction-diffusion etc).
                let iterations = 1;

                for _iter in 0..iterations {
                    let mut pass_uniforms = *uniforms;
                    pass_uniforms.pass_index = pass_idx as i32;
                    self.pipeline.update_uniforms(&context.queue, &pass_uniforms);

                    let pass_buffer_views: Vec<&wgpu::TextureView> = self.passes
                        .iter()
                        .filter_map(|p| p.target.as_ref().and_then(|t| self.pass_buffers.get(t)))
                        .map(|pb| pb.read_view())
                        .collect();

                    let bind_group = self.pipeline.create_bind_group(
                        &context.device,
                        Some(input_view),
                        &pass_buffer_views,
                        Some(user_params_buffer),
                    );

                    let target_view = self.pass_buffers.get(&target_name)
                        .map(|pb| pb.write_view())
                        .unwrap_or(output_view);

                    let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some(&format!("Effect Pass {} Encoder", pass_idx)),
                    });

                    {
                        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some(&format!("Effect Pass {} Render", pass_idx)),
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
                        });

                        render_pass.set_pipeline(self.pipeline.pipeline_for_format(format));
                        render_pass.set_bind_group(0, &bind_group, &[]);
                        render_pass.draw(0..3, 0..1);
                    }

                    context.queue.submit(std::iter::once(encoder.finish()));

                    if let Some(pb) = self.pass_buffers.get_mut(&target_name) {
                        pb.swap();
                    }
                }
            }

            // Final pass: render to output_view using pass buffer results + input
            let mut final_uniforms = *uniforms;
            final_uniforms.pass_index = self.passes.len() as i32;
            self.pipeline.update_uniforms(&context.queue, &final_uniforms);

            let pass_buffer_views: Vec<&wgpu::TextureView> = self.passes
                .iter()
                .filter_map(|p| p.target.as_ref().and_then(|t| self.pass_buffers.get(t)))
                .map(|pb| pb.read_view())
                .collect();

            let bind_group = self.pipeline.create_bind_group(
                &context.device,
                Some(input_view),
                &pass_buffer_views,
                Some(user_params_buffer),
            );

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Effect Final Pass Encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Effect Final Pass Render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: output_view,
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
                });

                render_pass.set_pipeline(self.pipeline.pipeline_for_format(self.target_format));
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            cmd_buffers.push(encoder.finish());
        } else {
            // Simple single-pass effect
            self.pipeline.update_uniforms(&context.queue, uniforms);

            let bind_group = self.pipeline.create_bind_group(
                &context.device,
                Some(input_view),
                &[],
                Some(user_params_buffer),
            );

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Effect Render Encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Effect Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: output_view,
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
                });

                render_pass.set_pipeline(&self.pipeline.pipeline);
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            cmd_buffers.push(encoder.finish());
        }

        Ok(())
    }
}

/// Multi-pass buffer for ISF PASSES array
/// Uses ping-pong buffers for persistent passes to allow read/write in same frame
pub struct PassBuffer {
    /// Buffer name (from ISF PASSES TARGET field)
    pub name: String,
    /// Primary texture (read source for persistent buffers)
    pub texture_a: wgpu::Texture,
    /// Primary texture view
    pub view_a: wgpu::TextureView,
    /// Secondary texture (write target for persistent buffers) - only for persistent
    pub texture_b: Option<wgpu::Texture>,
    /// Secondary texture view
    pub view_b: Option<wgpu::TextureView>,
    /// Whether this buffer persists across frames
    pub persistent: bool,
    /// Current read index (0 = read from A, 1 = read from B)
    pub read_idx: usize,
}

impl PassBuffer {
    /// Get the current read texture view
    pub fn read_view(&self) -> &wgpu::TextureView {
        if self.persistent && self.read_idx == 1 {
            self.view_b.as_ref().unwrap_or(&self.view_a)
        } else {
            &self.view_a
        }
    }

    /// Get the current write texture view (opposite of read for persistent)
    pub fn write_view(&self) -> &wgpu::TextureView {
        if self.persistent {
            if self.read_idx == 0 {
                self.view_b.as_ref().unwrap_or(&self.view_a)
            } else {
                &self.view_a
            }
        } else {
            &self.view_a
        }
    }

    /// Swap read/write buffers (call after rendering for persistent buffers)
    pub fn swap(&mut self) {
        if self.persistent {
            self.read_idx = 1 - self.read_idx;
        }
    }
}

/// A Deck is an independent render unit that outputs a texture
pub struct Deck {
    /// Name of this deck's source
    source_name: String,

    /// Original file path used to create this deck (for persistence).
    /// Shader path, video path, or image path. None for solid color / camera.
    source_path: Option<String>,

    /// Source type and pipeline (shader, video, or image)
    source: DeckSource,

    /// Generator shader parameters (if source is a shader)
    pub generator_params: ShaderParams,

    /// Render target texture (primary)
    pub texture: wgpu::Texture,

    /// Texture view
    pub texture_view: wgpu::TextureView,

    /// Secondary texture for ping-pong rendering in effect chain
    texture_b: wgpu::Texture,
    texture_b_view: wgpu::TextureView,

    /// Effect chain (ISF filters applied to generator output)
    pub effects: Vec<Effect>,

    /// Deck opacity (0.0 - 1.0)
    pub opacity: f32,

    /// Start time for TIME uniform
    start_time: Instant,

    /// Frame counter
    frame_count: u32,

    /// Last frame time
    last_frame_time: Instant,

    /// Camera source texture view (set each frame for Camera decks, cloned from CameraManager)
    pub camera_source_view: Option<wgpu::TextureView>,
}

impl Deck {
    /// Create a new deck from an ISF shader
    pub fn new(
        context: &RenderContext,
        shader: ISFShader,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        // Compile to SPIR-V
        let spirv = compile_glsl_to_spirv(&shader.fragment_source, &shader.name())
            .context("Failed to compile shader to SPIR-V")?;

        // Extract passes from metadata
        let passes = shader.metadata.passes.clone().unwrap_or_default();
        let has_passes = !passes.is_empty();

        // Create render target textures (two for ping-pong effect chain)
        // For multipass shaders, use linear format to avoid sRGB gamma corruption in simulations
        // Note: Deck output textures stay as Rgba8Unorm for egui preview compatibility
        // Only pass buffers use Rgba32Float for float precision
        let (texture, texture_b) = if has_passes {
            let tex = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Deck Texture (Linear)"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let tex_b = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Deck Texture B (Linear)"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            (tex, tex_b)
        } else {
            // Simple generators also need Rgba8Unorm for effect chain compatibility
            let tex = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Deck Texture (Linear)"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let tex_b = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Deck Texture B (Linear)"),
                size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            (tex, tex_b)
        };
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        // Create pass buffers (with ping-pong for persistent buffers)
        let mut pass_buffers = HashMap::new();

        for pass in &passes {
            // Skip passes without a TARGET (they render directly to screen)
            let target_name = match &pass.target {
                Some(name) => name.clone(),
                None => continue,  // Final pass - no buffer needed
            };

            // Parse width/height expressions (e.g., "$WIDTH", "$WIDTH/2")
            let pass_width = Self::parse_size_expression(&pass.width, width);
            let pass_height = Self::parse_size_expression(&pass.height, height);
            let is_persistent = pass.persistent.unwrap_or(false);

            // Determine texture format (float or standard)
            // Use linear format for simulation buffers to avoid sRGB gamma corruption
            let format = if pass.float.unwrap_or(false) {
                wgpu::TextureFormat::Rgba32Float
            } else {
                wgpu::TextureFormat::Rgba8Unorm  // Linear, not sRGB!
            };

            let tex_a = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(&format!("Pass Buffer A: {}", target_name)),
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

            // Create second buffer for ping-pong if persistent
            let (tex_b, view_b) = if is_persistent {
                let tex = context.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(&format!("Pass Buffer B: {}", target_name)),
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
            } else {
                (None, None)
            };

            pass_buffers.insert(target_name.clone(), PassBuffer {
                name: target_name,
                texture_a: tex_a,
                view_a,
                texture_b: tex_b,
                view_b,
                persistent: is_persistent,
                read_idx: 0,
            });
        }

        // Determine if any pass uses float format
        let uses_float = passes.iter().any(|p| p.float.unwrap_or(false));

        // Create unified pipeline — handles both simple generators and multi-pass shaders
        let pipeline = UnifiedPipeline::new(
            &context.device,
            &spirv,
            wgpu::TextureFormat::Rgba8Unorm,
            false,  // generators don't have input image
            pass_buffers.len(),
            uses_float,
        ).context("Failed to create shader pipeline")?;

        let now = Instant::now();
        let source_name = shader.name();

        // Initialize generator parameters from shader inputs
        let inputs = shader.metadata.inputs.as_ref().map(|v| v.as_slice()).unwrap_or(&[]);
        let generator_params = ShaderParams::from_inputs(inputs);

        // Build the DeckSource with shader data
        let source = DeckSource::Shader {
            shader,
            pipeline,
            pass_buffers,
            passes,
        };

        // Extract source path for persistence before shader is moved
        let source_path = match &source {
            DeckSource::Shader { shader, .. } => shader.file_path.clone(),
            _ => None,
        };

        Ok(Self {
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
            start_time: now,
            frame_count: 0,
            last_frame_time: now,
            camera_source_view: None,
        })
    }

    /// Parse ISF size expressions like "$WIDTH", "$WIDTH/2", "1024", etc.
    pub(crate) fn parse_size_expression(expr: &Option<String>, base_size: u32) -> u32 {
        match expr {
            None => base_size,
            Some(s) => {
                let s = s.trim();
                if s == "$WIDTH" || s == "$HEIGHT" {
                    base_size
                } else if s.starts_with("$WIDTH/") || s.starts_with("$HEIGHT/") {
                    let divisor: u32 = s.split('/').nth(1)
                        .and_then(|d| d.trim().parse().ok())
                        .unwrap_or(1);
                    base_size / divisor.max(1)
                } else if s.starts_with("$WIDTH*") || s.starts_with("$HEIGHT*") {
                    let multiplier: u32 = s.split('*').nth(1)
                        .and_then(|m| m.trim().parse().ok())
                        .unwrap_or(1);
                    base_size * multiplier
                } else {
                    // Try parsing as integer
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
    /// Auto-detects HAP codec and uses GPU-native BCn path when available.
    pub fn new_from_video<P: AsRef<Path>>(
        context: &RenderContext,
        path: P,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_path_str = path.as_ref().to_string_lossy().to_string();
        let source_name = path.as_ref()
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("video")
            .to_string();

        // Check if the GPU supports BC textures and file uses HAP codec
        let gpu_has_bc = context.device.features().contains(wgpu::Features::TEXTURE_COMPRESSION_BC);
        let hap_format = if gpu_has_bc {
            crate::video::detect_hap_codec(&path).ok().flatten()
        } else {
            None
        };

        let source = if let Some(hap_fmt) = hap_format {
            // ── HAP path: GPU-native compressed textures ──
            let player = HapPlayer::new(&path, hap_fmt)?;
            let vid_w = player.width();
            let vid_h = player.height();
            let tex_format = hap_fmt.wgpu_format();

            let video_texture = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("HAP Video Texture"),
                size: wgpu::Extent3d { width: vid_w, height: vid_h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: tex_format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let video_texture_view = video_texture.create_view(&wgpu::TextureViewDescriptor::default());

            // Create alpha texture for HAP Q Alpha (BC4 single-channel)
            let (alpha_texture, alpha_texture_view) = if matches!(hap_fmt, HapTextureFormat::Bc3YCoCg) {
                // HAP Q Alpha might send dual-plane frames — pre-allocate alpha texture
                let alpha_tex = context.device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("HAP Alpha Texture"),
                    size: wgpu::Extent3d { width: vid_w, height: vid_h, depth_or_array_layers: 1 },
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

            // Dummy 1x1 R8 texture for shader binding when no alpha plane
            let dummy_alpha = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("HAP Dummy Alpha"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            context.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &dummy_alpha, mip_level: 0,
                    origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All,
                },
                &[255u8],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(1), rows_per_image: Some(1) },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            let dummy_alpha_view = dummy_alpha.create_view(&wgpu::TextureViewDescriptor::default());

            let convert_pipeline = HapConvertPipeline::new(&context.device, wgpu::TextureFormat::Rgba8Unorm)?;
            let blit_pipeline = BlitPipeline::new(&context.device, wgpu::TextureFormat::Rgba8Unorm)?;

            log::info!("Using HAP GPU path for '{}' ({:?})", source_name, hap_fmt);
            DeckSource::HapVideo {
                player,
                texture: video_texture,
                texture_view: video_texture_view,
                alpha_texture,
                alpha_texture_view,
                dummy_alpha_view,
                convert_pipeline,
                blit_pipeline,
                hap_format: hap_fmt,
            }
        } else {
            // ── ffmpeg path: CPU decode to RGBA ──
            let player = VideoPlayer::new(&path)?;
            let video_texture = context.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Video Frame Texture"),
                size: wgpu::Extent3d {
                    width: player.width(),
                    height: player.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let video_texture_view = video_texture.create_view(&wgpu::TextureViewDescriptor::default());
            let blit_pipeline = BlitPipeline::new(&context.device, wgpu::TextureFormat::Rgba8Unorm)?;

            DeckSource::Video {
                player,
                texture: video_texture,
                texture_view: video_texture_view,
                blit_pipeline,
            }
        };

        // Create render target textures (Rgba8Unorm for effect chain compatibility)
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Video Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Video Texture B"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        let now = Instant::now();
        let generator_params = ShaderParams::from_inputs(&[]);

        Ok(Self {
            source_name,
            source_path: Some(source_path_str),
            source,
            generator_params,
            texture,
            texture_view,
            texture_b,
            texture_b_view,
            effects: Vec::new(),
            opacity: 1.0,
            start_time: now,
            frame_count: 0,
            last_frame_time: now,
            camera_source_view: None,
        })
    }

    /// Create a new deck from an image file (PNG, JPG, BMP, etc.)
    pub fn new_from_image<P: AsRef<Path>>(
        context: &RenderContext,
        path: P,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_path_str = path.as_ref().to_string_lossy().to_string();
        let img = image::open(&path)
            .with_context(|| format!("Failed to load image: {}", path.as_ref().display()))?;
        let rgba = img.to_rgba8();
        let (img_w, img_h) = rgba.dimensions();

        let source_name = path.as_ref()
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or("image")
            .to_string();

        // Create texture for the image at its native resolution
        let img_texture = context.device.create_texture(&wgpu::TextureDescriptor {
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

        // Upload pixel data
        context.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &img_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
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

        let img_texture_view = img_texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create blit pipeline — must match render target format (Rgba8Unorm), not source format
        let blit_pipeline = BlitPipeline::new(&context.device, wgpu::TextureFormat::Rgba8Unorm)?;

        let source = DeckSource::Image {
            texture: img_texture,
            texture_view: img_texture_view,
            blit_pipeline,
            source_width: img_w,
            source_height: img_h,
            scaling_mode: ScalingMode::default(),
        };

        // Create render target textures (Rgba8Unorm for effect chain compatibility)
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Image Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Image Texture B"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        let now = Instant::now();
        let generator_params = ShaderParams::from_inputs(&[]);

        Ok(Self {
            source_name,
            source_path: Some(source_path_str),
            source,
            generator_params,
            texture,
            texture_view,
            texture_b,
            texture_b_view,
            effects: Vec::new(),
            opacity: 1.0,
            start_time: now,
            frame_count: 0,
            last_frame_time: now,
            camera_source_view: None,
        })
    }

    /// Create a new deck with a solid color fill
    pub fn new_solid_color(
        context: &RenderContext,
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
            color: [color[0] as f64, color[1] as f64, color[2] as f64, color[3] as f64],
        };

        // Create render target textures
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck SolidColor Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck SolidColor Texture B"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        let now = Instant::now();
        let generator_params = ShaderParams::from_inputs(&[]);

        Ok(Self {
            source_name,
            source_path: None, // Solid color — no file path
            source,
            generator_params,
            texture,
            texture_view,
            texture_b,
            texture_b_view,
            effects: Vec::new(),
            opacity: 1.0,
            start_time: now,
            frame_count: 0,
            last_frame_time: now,
            camera_source_view: None,
        })
    }

    /// Create a new deck from a camera source.
    /// The camera is managed by CameraManager — this deck reads from the shared texture.
    pub fn new_from_camera(
        context: &RenderContext,
        camera_id: crate::camera::CameraId,
        camera_name: &str,
        source_width: u32,
        source_height: u32,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source_name = format!("📹 {}", camera_name);
        let blit_pipeline = BlitPipeline::new(&context.device, wgpu::TextureFormat::Rgba8Unorm)?;

        let source = DeckSource::Camera {
            camera_id,
            blit_pipeline,
            source_width,
            source_height,
            scaling_mode: ScalingMode::default(),
        };

        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Camera Texture"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Camera Texture B"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let texture_b_view = texture_b.create_view(&wgpu::TextureViewDescriptor::default());

        let now = Instant::now();
        let generator_params = ShaderParams::from_inputs(&[]);

        Ok(Self {
            source_name,
            source_path: None, // Camera — no file path
            source,
            generator_params,
            texture,
            texture_view,
            texture_b,
            texture_b_view,
            effects: Vec::new(),
            opacity: 1.0,
            start_time: now,
            frame_count: 0,
            last_frame_time: now,
            camera_source_view: None,
        })
    }

    /// Get the source name (shader name, video filename, etc.)
    pub fn source_name(&self) -> &str {
        &self.source_name
    }

    /// Get the source file path (for persistence). None for solid color / camera.
    pub fn source_path(&self) -> Option<&str> {
        self.source_path.as_deref()
    }

    /// Get the source type as a string for serialization
    pub fn source_type(&self) -> &str {
        match &self.source {
            DeckSource::Shader { .. } => "shader",
            DeckSource::Video { .. } | DeckSource::HapVideo { .. } => "video",
            DeckSource::Image { .. } => "image",
            DeckSource::SolidColor { .. } => "solid_color",
            DeckSource::Camera { .. } => "camera",
        }
    }

    /// Get the solid color value (if source is a solid color)
    pub fn solid_color(&self) -> Option<[f32; 4]> {
        match &self.source {
            DeckSource::SolidColor { color } => Some([color[0] as f32, color[1] as f32, color[2] as f32, color[3] as f32]),
            _ => None,
        }
    }

    /// Get the scaling mode (if applicable for this source type)
    pub fn scaling_mode(&self) -> Option<ScalingMode> {
        match &self.source {
            DeckSource::Image { scaling_mode, .. } => Some(*scaling_mode),
            DeckSource::Camera { scaling_mode, .. } => Some(*scaling_mode),
            _ => None,
        }
    }

    /// Set the scaling mode (applies to Image and Camera sources)
    pub fn set_scaling_mode(&mut self, mode: ScalingMode) {
        match &mut self.source {
            DeckSource::Image { scaling_mode, .. } => *scaling_mode = mode,
            DeckSource::Camera { scaling_mode, .. } => *scaling_mode = mode,
            _ => {}
        }
    }

    /// Get the camera ID (if source is a camera)
    pub fn camera_id(&self) -> Option<crate::camera::CameraId> {
        match &self.source {
            DeckSource::Camera { camera_id, .. } => Some(*camera_id),
            _ => None,
        }
    }

    /// Get the shader (if source is a shader)
    pub fn shader(&self) -> Option<&ISFShader> {
        match &self.source {
            DeckSource::Shader { shader, .. } => Some(shader),
            _ => None,
        }
    }

    /// Update video frame (call before render if using video source).
    /// Handles both ffmpeg RGBA uploads and HAP BCn compressed uploads.
    pub fn update_video_frame(&mut self, context: &RenderContext) -> Result<()> {
        match &mut self.source {
            DeckSource::Video { ref mut player, ref texture, .. } => {
                if player.is_playing() {
                    let width = player.width();
                    let height = player.height();
                    if let Some(frame_data) = player.next_frame()? {
                        context.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture, mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            frame_data,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(width * 4),
                                rows_per_image: Some(height),
                            },
                            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                        );
                    }
                }
            }
            DeckSource::HapVideo { ref mut player, ref texture, ref alpha_texture, .. } => {
                if player.is_playing() {
                    let width = player.width();
                    let height = player.height();
                    if let Some(frame) = player.next_frame()? {
                        // Upload color plane
                        let blocks_x = (width + 3) / 4;
                        let blocks_y = (height + 3) / 4;
                        let color_bpr = blocks_x * frame.color_format.block_bytes();
                        context.queue.write_texture(
                            wgpu::TexelCopyTextureInfo {
                                texture, mip_level: 0,
                                origin: wgpu::Origin3d::ZERO,
                                aspect: wgpu::TextureAspect::All,
                            },
                            frame.color_data,
                            wgpu::TexelCopyBufferLayout {
                                offset: 0,
                                bytes_per_row: Some(color_bpr),
                                rows_per_image: Some(blocks_y),
                            },
                            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                        );

                        // Upload alpha plane if dual-plane (HAP Q Alpha)
                        if let (Some(alpha_data), Some(alpha_fmt), Some(alpha_tex)) =
                            (frame.alpha_data, frame.alpha_format, alpha_texture.as_ref())
                        {
                            let alpha_bpr = blocks_x * alpha_fmt.block_bytes();
                            context.queue.write_texture(
                                wgpu::TexelCopyTextureInfo {
                                    texture: alpha_tex, mip_level: 0,
                                    origin: wgpu::Origin3d::ZERO,
                                    aspect: wgpu::TextureAspect::All,
                                },
                                alpha_data,
                                wgpu::TexelCopyBufferLayout {
                                    offset: 0,
                                    bytes_per_row: Some(alpha_bpr),
                                    rows_per_image: Some(blocks_y),
                                },
                                wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
                            );
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// Render the deck to its texture (source + effect chain)
    /// The modulation engine is owned by Stage and passed in for parameter automation
    /// `deck_idx` is the deck index used for modulation key lookup (e.g., "deck0:paramname")
    pub fn render(&mut self, context: &RenderContext, audio_data: &crate::AudioData, modulation: &ModulationEngine, deck_idx: usize, cmd_buffers: &mut Vec<wgpu::CommandBuffer>) -> Result<()> {
        let prefix = format!("deck{}", deck_idx);
        self.render_with_prefix(context, audio_data, modulation, &prefix, cmd_buffers)
    }

    /// Render the deck with a custom param prefix for modulation key lookup
    /// Used by Channel to provide channel-scoped addressing (e.g., "ch0_deck0")
    pub fn render_with_prefix(&mut self, context: &RenderContext, audio_data: &crate::AudioData, modulation: &ModulationEngine, param_prefix: &str, cmd_buffers: &mut Vec<wgpu::CommandBuffer>) -> Result<()> {
        let now = Instant::now();
        let time = (now - self.start_time).as_secs_f32();
        let time_delta = (now - self.last_frame_time).as_secs_f32();
        self.last_frame_time = now;
        self.frame_count += 1;

        // Count enabled effects
        let enabled_effects: Vec<usize> = self.effects.iter()
            .enumerate()
            .filter(|(_, e)| e.enabled)
            .map(|(i, _)| i)
            .collect();

        // Determine where to render the source based on effect chain length
        let source_to_b = enabled_effects.len() % 2 == 1;
        let generator_target = if source_to_b { &self.texture_b_view } else { &self.texture_view };

        // Render based on source type
        match &mut self.source {
            DeckSource::Shader { pipeline, pass_buffers, passes, .. } => {
                if pipeline.num_pass_buffers > 0 {
                    Self::render_multi_pass_static(
                        context, pipeline, passes, pass_buffers,
                        time, time_delta, self.frame_count,
                        self.texture.width(), self.texture.height(),
                        generator_target, audio_data,
                        &mut self.generator_params, modulation, &param_prefix,
                        cmd_buffers,
                    )?;
                } else {
                    Self::render_simple_static(
                        context, pipeline, &self.texture,
                        time, time_delta, self.frame_count, generator_target, audio_data,
                        &mut self.generator_params, modulation, &param_prefix,
                        cmd_buffers,
                    )?;
                }
            }
            DeckSource::Video { ref texture_view, ref blit_pipeline, .. } => {
                let bind_group = blit_pipeline.create_bind_group(&context.device, texture_view);
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Video Blit Encoder"),
                });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Video Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
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
                    });
                    blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::HapVideo {
                ref texture_view, ref alpha_texture_view, ref dummy_alpha_view,
                ref convert_pipeline, ref hap_format, ref player, ..
            } => {
                // Use HAP convert pipeline for YCoCg and/or dual-plane alpha
                let needs_ycocg = hap_format.needs_ycocg_convert();
                let has_alpha = player.is_dual_plane && alpha_texture_view.is_some();
                convert_pipeline.set_params(&context.queue, 1.0, needs_ycocg, has_alpha);

                let alpha_view = if let Some(ref av) = alpha_texture_view {
                    av
                } else {
                    dummy_alpha_view
                };
                let bind_group = convert_pipeline.create_bind_group(
                    &context.device, texture_view, alpha_view,
                );
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("HAP Convert Encoder"),
                });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("HAP Convert Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
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
                    });
                    convert_pipeline.draw(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::Image { texture_view, blit_pipeline, source_width, source_height, scaling_mode, .. } => {
                // Compute UV transform for scaling mode
                let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
                    *source_width, *source_height,
                    self.texture.width(), self.texture.height(),
                );
                blit_pipeline.set_uv_transform(&context.queue, 1.0, uv_scale, uv_offset);

                // Blit image to generator target
                let bind_group = blit_pipeline.create_bind_group(&context.device, texture_view);
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Image Blit Encoder"),
                });
                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("Image Blit Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
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
                    });
                    blit_pipeline.render(&mut render_pass, &bind_group);
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::SolidColor { color } => {
                // Clear the generator target to the solid color
                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("SolidColor Clear Encoder"),
                });
                {
                    let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("SolidColor Clear Pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: generator_target,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color {
                                    r: color[0],
                                    g: color[1],
                                    b: color[2],
                                    a: color[3],
                                }),
                                store: wgpu::StoreOp::Store,
                            },
                            depth_slice: None,
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                    });
                }
                cmd_buffers.push(encoder.finish());
            }
            DeckSource::Camera { blit_pipeline, source_width, source_height, scaling_mode, .. } => {
                // Blit shared camera texture to generator target
                if let Some(cam_view) = &self.camera_source_view {
                    let (uv_scale, uv_offset) = scaling_mode.compute_uv_transform(
                        *source_width, *source_height,
                        self.texture.width(), self.texture.height(),
                    );
                    blit_pipeline.set_uv_transform(&context.queue, 1.0, uv_scale, uv_offset);

                    let bind_group = blit_pipeline.create_bind_group(&context.device, cam_view);
                    let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("Camera Blit Encoder"),
                    });
                    {
                        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("Camera Blit Pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: generator_target,
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
                        });
                        blit_pipeline.render(&mut render_pass, &bind_group);
                    }
                    cmd_buffers.push(encoder.finish());
                }
                // If no camera_source_view, the deck renders black (camera not yet started)
            }
        }

        // Apply effect chain (ping-pong between textures)
        let mut read_from_b = source_to_b;
        for &effect_idx in &enabled_effects {
            let uniforms = ISFUniforms {
                time,
                time_delta,
                frame_index: self.frame_count,
                pass_index: 0,
                render_size: [self.texture.width() as f32, self.texture.height() as f32],
                audio_level: audio_data.level,
                audio_bass: audio_data.bass(),
                audio_mid: audio_data.mid(),
                audio_treble: audio_data.treble(),
                audio_bpm: audio_data.bpm.unwrap_or(0.0),
                audio_beat_phase: audio_data.beat_phase(),
                date: get_current_date(),
            };
            let (input_view, output_view) = if read_from_b {
                (&self.texture_b_view, &self.texture_view)
            } else {
                (&self.texture_view, &self.texture_b_view)
            };
            let fx_prefix = format!("{}_fx{}", param_prefix, effect_idx);
            self.effects[effect_idx].apply_with_modulation(
                context, input_view, output_view, &uniforms,
                Some(modulation), Some(&fx_prefix),
                cmd_buffers,
            )?;
            read_from_b = !read_from_b;
        }

        Ok(())
    }

    /// Render simple (non-multi-pass) shader (static version)
    fn render_simple_static(
        context: &RenderContext,
        pipeline: &UnifiedPipeline,
        texture: &wgpu::Texture,
        time: f32,
        time_delta: f32,
        frame_count: u32,
        target_view: &wgpu::TextureView,
        audio_data: &crate::AudioData,
        generator_params: &mut ShaderParams,
        modulation: &ModulationEngine,
        param_prefix: &str,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        let uniforms = ISFUniforms {
            time,
            time_delta,
            frame_index: frame_count,
            pass_index: 0,
            render_size: [texture.width() as f32, texture.height() as f32],
            audio_level: audio_data.level,
            audio_bass: audio_data.bass(),
            audio_mid: audio_data.mid(),
            audio_treble: audio_data.treble(),
            audio_bpm: audio_data.bpm.unwrap_or(0.0),
            audio_beat_phase: audio_data.beat_phase(),
            date: get_current_date(),
        };

        pipeline.update_uniforms(&context.queue, &uniforms);

        // Ensure user params buffer exists and update it with modulation applied
        generator_params.ensure_buffer(&context.device);
        generator_params.update_buffer_with_modulation(&context.queue, modulation, Some(param_prefix));

        // Create bind group with user params
        let user_params_buffer = generator_params.buffer().expect("Buffer should exist after ensure_buffer");
        let bind_group = pipeline.create_bind_group_with_params(&context.device, user_params_buffer);

        let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Deck Source Render Encoder"),
        });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Deck Source Render Pass"),
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
            });

            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        cmd_buffers.push(encoder.finish());
        Ok(())
    }

    /// Render multi-pass shader with proper ping-pong buffers
    fn render_multi_pass_static(
        context: &RenderContext,
        multi_pass: &UnifiedPipeline,
        passes: &[ISFPass],
        pass_buffers: &mut HashMap<String, PassBuffer>,
        time: f32,
        time_delta: f32,
        frame_count: u32,
        render_width: u32,
        render_height: u32,
        final_target: &wgpu::TextureView,
        audio_data: &crate::AudioData,
        generator_params: &mut ShaderParams,
        modulation: &ModulationEngine,
        param_prefix: &str,
        cmd_buffers: &mut Vec<wgpu::CommandBuffer>,
    ) -> Result<()> {
        // Ensure user params buffer exists and update it with modulation applied
        generator_params.ensure_buffer(&context.device);
        generator_params.update_buffer_with_modulation(&context.queue, modulation, Some(param_prefix));
        let user_params_buffer = generator_params.buffer().expect("Buffer should exist after ensure_buffer");

        // For reaction-diffusion simulations, we need multiple iterations per frame
        // Run persistent passes multiple times before the final render pass
        const SIMULATION_ITERATIONS: usize = 16;

        // First, run all persistent simulation passes multiple times
        for pass_idx in 0..passes.len() {
            let pass = &passes[pass_idx];

            // Only run multiple iterations for persistent passes (simulation buffers)
            let iterations = if pass.persistent.unwrap_or(false) {
                SIMULATION_ITERATIONS
            } else {
                1
            };

            let target_name = match &pass.target {
                Some(name) => name,
                None => continue, // Skip passes without target
            };

            let format = if pass.float.unwrap_or(false) {
                wgpu::TextureFormat::Rgba32Float
            } else {
                wgpu::TextureFormat::Rgba8Unorm
            };

            for iter in 0..iterations {
                // Update uniforms with current iteration info
                // Use frame_count * iterations + iter for unique frame indices
                let effective_frame = frame_count * SIMULATION_ITERATIONS as u32 + iter as u32;

                let uniforms = ISFUniforms {
                    time,
                    time_delta: time_delta / SIMULATION_ITERATIONS as f32,
                    frame_index: effective_frame,
                    pass_index: pass_idx as i32,
                    render_size: [render_width as f32, render_height as f32],
                    audio_level: audio_data.level,
                    audio_bass: audio_data.bass(),
                    audio_mid: audio_data.mid(),
                    audio_treble: audio_data.treble(),
                    audio_bpm: audio_data.bpm.unwrap_or(0.0),
                    audio_beat_phase: audio_data.beat_phase(),
                    date: get_current_date(),
                };

                multi_pass.update_uniforms(&context.queue, &uniforms);

                // Get current read view for sampling
                let pass_buffer_views: Vec<&wgpu::TextureView> = passes
                    .iter()
                    .filter_map(|p| p.target.as_ref().and_then(|t| pass_buffers.get(t)))
                    .map(|pb| pb.read_view())
                    .collect();

                let bind_group = multi_pass.create_bind_group(&context.device, None, &pass_buffer_views, Some(user_params_buffer));

                // Get write view for rendering
                let target_view = pass_buffers.get(target_name)
                    .map(|pb| pb.write_view())
                    .unwrap_or(final_target);

                let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some(&format!("Pass {} Iter {} Encoder", pass_idx, iter)),
                });

                {
                    let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some(&format!("Pass {} Iter {} Render", pass_idx, iter)),
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
                    });

                    render_pass.set_pipeline(multi_pass.pipeline_for_format(format));
                    render_pass.set_bind_group(0, &bind_group, &[]);
                    render_pass.draw(0..3, 0..1);
                }

                context.queue.submit(std::iter::once(encoder.finish()));

                // Swap ping-pong buffers after each iteration
                if let Some(pb) = pass_buffers.get_mut(target_name) {
                    pb.swap();
                }
            }
        }

        // Final render pass to screen
        {
            let uniforms = ISFUniforms {
                time,
                time_delta,
                frame_index: frame_count,
                pass_index: passes.len() as i32, // Final pass index
                render_size: [render_width as f32, render_height as f32],
                audio_level: audio_data.level,
                audio_bass: audio_data.bass(),
                audio_mid: audio_data.mid(),
                audio_treble: audio_data.treble(),
                audio_bpm: audio_data.bpm.unwrap_or(0.0),
                audio_beat_phase: audio_data.beat_phase(),
                date: get_current_date(),
            };

            multi_pass.update_uniforms(&context.queue, &uniforms);

            let pass_buffer_views: Vec<&wgpu::TextureView> = passes
                .iter()
                .filter_map(|p| p.target.as_ref().and_then(|t| pass_buffers.get(t)))
                .map(|pb| pb.read_view())
                .collect();

            let bind_group = multi_pass.create_bind_group(&context.device, None, &pass_buffer_views, Some(user_params_buffer));

            let mut encoder = context.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Final Pass Encoder"),
            });

            {
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Final Pass Render"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: final_target,
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
                });

                render_pass.set_pipeline(multi_pass.pipeline_for_format(wgpu::TextureFormat::Rgba8Unorm));
                render_pass.set_bind_group(0, &bind_group, &[]);
                render_pass.draw(0..3, 0..1);
            }

            cmd_buffers.push(encoder.finish());
        }

        Ok(())
    }

    /// Resize the deck's render targets
    pub fn resize(&mut self, context: &RenderContext, width: u32, height: u32) {
        // All deck textures use Rgba8Unorm for effect chain compatibility
        self.texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture (Linear)"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.texture_view = self.texture.create_view(&wgpu::TextureViewDescriptor::default());
        self.texture_b = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Deck Texture B (Linear)"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        self.texture_b_view = self.texture_b.create_view(&wgpu::TextureViewDescriptor::default());
    }

    /// Get the final output texture view (after effect chain)
    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.texture_view
    }
}

/// Get current date as [year, month, day, seconds_in_day]
pub fn get_current_date() -> [f32; 4] {
    use std::time::SystemTime;
    
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap();
    
    let total_seconds = now.as_secs();
    let seconds_in_day = (total_seconds % 86400) as f32;
    
    // Simplified date calculation (not accurate, but good enough for shaders)
    let days_since_epoch = total_seconds / 86400;
    let year = 1970.0 + (days_since_epoch as f32 / 365.25);
    let day_of_year = (days_since_epoch % 365) as f32;
    let month = (day_of_year / 30.0).floor() + 1.0;
    let day = (day_of_year % 30.0) + 1.0;
    
    [year, month, day, seconds_in_day]
}

