//! Building a deck around a source.

use super::{Deck, Effect, FramePacing, generator_params_for};
use crate::isf::ISFShader;
use crate::renderer::GpuContext;
use crate::source::DeckSourceInstance;
use anyhow::Result;
use std::time::Instant;

/// One of a deck's two ping-pong render targets. `COPY_DST` so a compute
/// source can copy its output in, and `COPY_SRC` so analyzers can read back.
pub(super) fn deck_target(
    gpu: &GpuContext,
    label: &str,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: gpu.compositing_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

impl Deck {
    /// A `width × height` deck drawing `source`.
    pub fn from_source(
        gpu: &GpuContext,
        source: Box<dyn DeckSourceInstance>,
        width: u32,
        height: u32,
    ) -> Self {
        let (texture, texture_view) = deck_target(gpu, "Deck Texture (Linear)", width, height);
        let (texture_b, texture_b_view) =
            deck_target(gpu, "Deck Texture B (Linear)", width, height);
        let (generator_params, generator_phase_inputs) = generator_params_for(source.as_ref());
        let uuid = crate::ids::generate_short_uuid();
        let param_prefix = crate::engine::value::param::deck_param_prefix(&uuid);
        Self {
            uuid,
            param_prefix,
            source_name: source.label(),
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
            pacing: FramePacing {
                step: 1.0 / 60.0,
                live: false,
            },
            frame_count: 0,
            last_frame_time: Instant::now(),
            depth_prepro: None,
            fps_smoothed: 0.0,
            phase_accumulators: [0.0; 4],
            generator_phase_inputs,
            analyzers: crate::analyzer::DeckAnalyzers::new(),
            host_inline: crate::analyzer::HostInlineSet::new(),
            gpu_error: None,
        }
    }

    /// A deck running the generator `shader`.
    ///
    /// # Errors
    ///
    /// Fails when the shader does not compile or its pipeline cannot be built.
    pub fn from_shader(
        gpu: &GpuContext,
        shader: ISFShader,
        width: u32,
        height: u32,
    ) -> Result<Self> {
        let source = crate::generator::Shader::new(gpu, shader, width, height)?;
        Ok(Self::from_source(gpu, Box::new(source), width, height))
    }

    /// A deck filling with `color`.
    pub fn solid_color(gpu: &GpuContext, color: [f32; 4], width: u32, height: u32) -> Self {
        Self::from_source(
            gpu,
            Box::new(crate::solid_color::SolidColor::new(color)),
            width,
            height,
        )
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
}
