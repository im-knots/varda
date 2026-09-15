//! Reading video back into the lighting merge.
//!
//! The GPU half of Varda's one-graph claim: a channel's composite (or the program) is
//! downsampled to a thumbnail, read back without stalling the render thread, and handed to the
//! lighting runtime, where it merges as one more source type beside held values and modulators.
//!
//! Costs nothing when unused. A show with no sampled decks allocates no textures, enqueues no
//! copies and reads nothing back — [`LightSampler::sync`] is handed an empty source list and
//! drops everything it was holding.
//! See /spec/lighting-routing.md § Sampled.

use crate::dmx::{PROGRAM_KEY, SAMPLE_EDGE, SampledFrame};
use crate::renderer::blit::BlitPipeline;
use crate::renderer::readback::{ReadbackBuffer, ReadbackFormat};
use std::collections::HashMap;

/// The per-source GPU resources: somewhere to downsample into, and a staging buffer to read.
struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: ReadbackBuffer,
}

/// Downsamples and reads back every source the lighting show is currently sampling.
pub struct LightSampler {
    /// Built lazily on the first sampled deck, so a show that never uses pixel mapping never
    /// creates a pipeline for it.
    blit: Option<BlitPipeline>,
    targets: HashMap<String, Target>,
}

/// The texture format the downsample target uses.
///
/// Eight-bit RGBA, not the compositor's float format: this feeds DMX, which is eight bits per
/// channel, so anything wider would be thrown away on the next line.
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

impl LightSampler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            blit: None,
            targets: HashMap::new(),
        }
    }

    /// Match the held targets to the sources actually being sampled.
    ///
    /// Dropping a target the moment its deck stops sampling is what makes the feature free when
    /// unused, and it also stops a deleted channel's last frame from driving a rig forever.
    pub fn sync(&mut self, device: &wgpu::Device, sources: &[String]) -> Result<(), String> {
        self.targets.retain(|key, _| sources.contains(key));
        if sources.is_empty() {
            self.blit = None;
            return Ok(());
        }
        if self.blit.is_none() {
            self.blit = Some(
                BlitPipeline::new(device, FORMAT)
                    .map_err(|e| format!("light sampler blit: {e}"))?,
            );
        }
        for key in sources {
            if self.targets.contains_key(key) {
                continue;
            }
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("light sample target"),
                size: wgpu::Extent3d {
                    width: SAMPLE_EDGE,
                    height: SAMPLE_EDGE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.targets.insert(
                key.clone(),
                Target {
                    texture,
                    view,
                    readback: ReadbackBuffer::new(
                        device,
                        SAMPLE_EDGE,
                        SAMPLE_EDGE,
                        ReadbackFormat::Rgba8,
                    ),
                },
            );
        }
        Ok(())
    }

    /// Downsample one source and enqueue its readback.
    ///
    /// The blit does the averaging: rendering a full-resolution composite into a 64×64 target
    /// with a linear-filtering sampler is exactly the area reduction a pixel map wants, and it
    /// costs one small draw rather than a full-resolution readback.
    pub fn capture(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        key: &str,
        source: &wgpu::TextureView,
    ) {
        let (Some(blit), Some(target)) = (self.blit.as_ref(), self.targets.get_mut(key)) else {
            return;
        };
        let bind_group = blit.create_bind_group(device, source);
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("light sample downsample"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            blit.render(&mut pass, &bind_group);
        }
        target.readback.begin_readback(encoder, &target.texture);
    }

    /// Hand every frame that has finished mapping to the lighting runtime.
    ///
    /// Non-blocking: a source whose readback has not landed yet simply contributes nothing this
    /// frame and the merge reuses the frame it already has.
    pub fn drain(&mut self, device: &wgpu::Device, lighting: &mut crate::dmx::LightingRuntime) {
        for (key, target) in &mut self.targets {
            let Some(frame) = target.readback.try_read(device) else {
                continue;
            };
            let width = frame.width();
            let height = frame.height();
            let stride = frame.stride() as usize;
            let bytes = frame.bytes();
            // The staging buffer is row-padded to the copy alignment; the merge wants it tight.
            let row_bytes = (width as usize) * 4;
            let mut rgba = Vec::with_capacity(row_bytes * height as usize);
            for row in 0..height as usize {
                let start = row * stride;
                let Some(slice) = bytes.get(start..start + row_bytes) else {
                    break;
                };
                rgba.extend_from_slice(slice);
            }
            if rgba.len() == row_bytes * height as usize {
                lighting.set_sampled_frame(
                    key.clone(),
                    SampledFrame {
                        width,
                        height,
                        rgba,
                    },
                );
            }
        }
        let held: Vec<String> = self.targets.keys().cloned().collect();
        lighting.retain_sampled_frames(&|k| held.iter().any(|h| h == k));
    }

    /// The key the program's frames are stored under.
    #[must_use]
    pub fn program_key() -> &'static str {
        PROGRAM_KEY
    }
}

impl Default for LightSampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A show with no sampled decks must hold nothing at all: no pipeline, no textures, no
    /// staging buffers. Pixel mapping that costs something when unused is a tax on every show
    /// that does not use it.
    #[test]
    fn an_unused_sampler_holds_nothing() {
        let s = LightSampler::new();
        assert!(s.blit.is_none());
        assert!(s.targets.is_empty());
    }

    /// The program's key must not look like a channel uuid, or a channel could shadow it.
    #[test]
    fn the_program_key_is_not_a_channel_uuid() {
        let key = LightSampler::program_key();
        assert!(key.len() != 8 || !key.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
