//! Content light level measurement for HDR10 mastering metadata.
//!
//! CTA-861.3 expects MaxCLL and MaxFALL to describe the content, measured across
//! the programme. Declaring them from a configured peak is true only because the
//! encoder clamps to it, and is not what the standard asks for.
//!
//! Measuring on the CPU from the readback the recording already performs was
//! tried first and rejected on measurement: 2.44 ms per frame at 1080p and
//! 6.58 ms at 4K, the latter 40% of a 60fps budget. Replacing the float
//! accumulation with an integer histogram barely moved it (2.63 to 2.44 ms),
//! which showed the cost was bandwidth reading the frame a second time rather
//! than arithmetic. On the GPU the frame is already resident.
//!
//! See /spec/hdr-recording-output.md § Mastering and Content Metadata.

use anyhow::Result;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;

/// Reduction factor per pass, in each dimension.
const FACTOR: u32 = 4;
/// Reduce until no dimension exceeds this, then finish on the CPU.
const FINAL_EDGE: u32 = 4;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct MeasureParams {
    decode_pq: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

/// One frame's measured light levels, in cd/m².
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameLightLevels {
    /// Largest single-pixel light level in the frame.
    pub max: f32,
    /// Frame-average light level.
    pub average: f32,
}

/// Running content light levels across a recording.
///
/// MaxCLL is the largest pixel seen anywhere; MaxFALL is the largest *frame
/// average* seen, not the average of averages, so a single bright frame raises
/// it and a long dark passage does not lower it.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContentLightLevels {
    max_cll: f32,
    max_fall: f32,
    frames: u64,
}

impl ContentLightLevels {
    /// Fold one frame in.
    pub fn observe(&mut self, frame: FrameLightLevels) {
        if frame.max.is_finite() {
            self.max_cll = self.max_cll.max(frame.max);
        }
        if frame.average.is_finite() {
            self.max_fall = self.max_fall.max(frame.average);
        }
        self.frames += 1;
    }

    /// Measured MaxCLL and MaxFALL, rounded as the metadata carries them.
    ///
    /// `None` until at least one frame has been observed: declaring measured
    /// values for a recording nothing was measured from would be worse than
    /// saying it was declared.
    #[must_use]
    pub fn measured(self) -> Option<(u32, u32)> {
        (self.frames > 0).then(|| {
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            (
                self.max_cll.round().max(0.0) as u32,
                self.max_fall.round().max(0.0) as u32,
            )
        })
    }

    /// Frames folded in so far.
    #[must_use]
    pub const fn frames(self) -> u64 {
        self.frames
    }
}

/// GPU reduction chain plus its readback.
pub struct ContentLightMeter {
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    first_params: wgpu::Buffer,
    later_params: wgpu::Buffer,
    /// Progressively smaller `(max, sum, count)` targets.
    levels: Vec<(wgpu::Texture, wgpu::TextureView)>,
    /// Dimensions the chain was built for.
    source_size: (u32, u32),
    readback: Option<super::ReadbackBuffer>,
}

impl ContentLightMeter {
    /// Build the measurement pipeline.
    ///
    /// # Errors
    ///
    /// Never returns `Err` today; the `Result` keeps the constructor uniform with
    /// the other pipelines so callers can `?` it.
    pub fn new(device: &wgpu::Device) -> Result<Self> {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Measure Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/measure.wgsl").into()),
        });
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Measure Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        // `textureLoad` only, so no filtering is required and the
                        // float target needs no filterable-float feature.
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: NonZeroU64::new(
                            std::mem::size_of::<MeasureParams>() as u64
                        ),
                    },
                    count: None,
                },
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Measure Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Measure Pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba32Float,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: wgpu::PipelineCompilationOptions::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let make_params = |decode_pq: u32, label: &str| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: bytemuck::cast_slice(&[MeasureParams {
                    decode_pq,
                    _pad0: 0,
                    _pad1: 0,
                    _pad2: 0,
                }]),
                usage: wgpu::BufferUsages::UNIFORM,
            })
        };

        Ok(Self {
            pipeline,
            bind_group_layout,
            first_params: make_params(1, "Measure Params (decode PQ)"),
            later_params: make_params(0, "Measure Params (combine)"),
            levels: Vec::new(),
            source_size: (0, 0),
            readback: None,
        })
    }

    /// Sizes of each reduction level for a source, largest first.
    fn level_sizes(width: u32, height: u32) -> Vec<(u32, u32)> {
        let mut sizes = Vec::new();
        let (mut w, mut h) = (width.max(1), height.max(1));
        while w > FINAL_EDGE || h > FINAL_EDGE {
            w = w.div_ceil(FACTOR);
            h = h.div_ceil(FACTOR);
            sizes.push((w, h));
        }
        if sizes.is_empty() {
            sizes.push((w, h));
        }
        sizes
    }

    fn ensure_chain(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        if self.source_size == (width, height) && !self.levels.is_empty() {
            return;
        }
        self.levels.clear();
        for (w, h) in Self::level_sizes(width, height) {
            let tex = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("Measure Level"),
                size: wgpu::Extent3d {
                    width: w,
                    height: h,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            });
            let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
            self.levels.push((tex, view));
        }
        let (fw, fh) = Self::level_sizes(width, height)
            .last()
            .copied()
            .unwrap_or((1, 1));
        self.readback = Some(super::ReadbackBuffer::new(
            device,
            fw,
            fh,
            super::ReadbackFormat::Rgba32Float,
        ));
        self.source_size = (width, height);
    }

    /// Enqueue the reduction for one frame and start its readback.
    ///
    /// `source` is the PQ-encoded frame about to be written, so what is measured
    /// is what the file will contain.
    pub fn measure(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        width: u32,
        height: u32,
    ) {
        self.ensure_chain(device, width, height);

        for index in 0..self.levels.len() {
            let (input, params) = if index == 0 {
                (source, &self.first_params)
            } else {
                (&self.levels[index - 1].1, &self.later_params)
            };
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Measure Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(input),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: params.as_entire_binding(),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Measure Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.levels[index].1,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if let (Some(readback), Some((tex, _))) = (self.readback.as_mut(), self.levels.last()) {
            readback.begin_readback(encoder, tex);
        }
    }

    /// Collect a completed measurement, if one is ready.
    ///
    /// Non-blocking, on the same asynchronous contract as every other readback:
    /// a frame's numbers arrive a frame or two after it was measured, which is
    /// immaterial for a running maximum.
    pub fn try_read(&mut self, device: &wgpu::Device) -> Option<FrameLightLevels> {
        let frame = self.readback.as_mut()?.try_read(device)?;
        let floats: &[f32] = bytemuck::cast_slice(frame.bytes());
        let mut max = 0.0f32;
        let mut total = 0.0f64;
        let mut samples = 0.0f64;
        for texel in floats.chunks_exact(4) {
            max = max.max(texel[0]);
            total += f64::from(texel[1]);
            samples += f64::from(texel[2]);
        }
        if samples <= 0.0 {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        Some(FrameLightLevels {
            max,
            average: (total / samples) as f32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chain_reduces_all_the_way_down() {
        for (w, h) in [(1920, 1080), (3840, 2160), (640, 360), (7, 3)] {
            let sizes = ContentLightMeter::level_sizes(w, h);
            let (last_w, last_h) = *sizes.last().expect("at least one level");
            assert!(
                last_w <= FINAL_EDGE && last_h <= FINAL_EDGE,
                "{w}x{h} stopped at {last_w}x{last_h}"
            );
            // A 4K frame should not need a long chain; each pass is 16:1.
            assert!(sizes.len() <= 8, "{w}x{h} needed {} passes", sizes.len());
        }
    }

    #[test]
    fn a_tiny_source_still_produces_a_level() {
        // Guards the arithmetic, not any real configuration: an empty chain would
        // mean nothing to read back and a silently missing measurement.
        assert!(!ContentLightMeter::level_sizes(1, 1).is_empty());
        assert!(!ContentLightMeter::level_sizes(0, 0).is_empty());
    }

    #[test]
    fn max_cll_is_the_largest_pixel_and_max_fall_the_largest_frame_average() {
        // MaxFALL is the largest frame average, not the average of averages: one
        // bright frame raises it and a long dark passage must not lower it.
        let mut levels = ContentLightLevels::default();
        levels.observe(FrameLightLevels {
            max: 900.0,
            average: 120.0,
        });
        levels.observe(FrameLightLevels {
            max: 400.0,
            average: 310.0,
        });
        for _ in 0..500 {
            levels.observe(FrameLightLevels {
                max: 1.0,
                average: 0.5,
            });
        }
        let (cll, fall) = levels.measured().expect("frames were observed");
        assert_eq!(cll, 900);
        assert_eq!(fall, 310);
        assert_eq!(levels.frames(), 502);
    }

    #[test]
    fn nothing_measured_reports_nothing_rather_than_zero() {
        // Zeroes would be indistinguishable from a genuinely black programme and
        // would be written as if measured.
        assert_eq!(ContentLightLevels::default().measured(), None);
    }

    #[test]
    fn non_finite_frames_do_not_poison_the_running_maximum() {
        let mut levels = ContentLightLevels::default();
        levels.observe(FrameLightLevels {
            max: 500.0,
            average: 100.0,
        });
        levels.observe(FrameLightLevels {
            max: f32::NAN,
            average: f32::INFINITY,
        });
        let (cll, fall) = levels.measured().expect("frames were observed");
        assert_eq!(cll, 500);
        assert_eq!(fall, 100);
    }
}

#[cfg(test)]
mod gpu_tests {
    use super::*;
    use crate::renderer::{GpuContext, hdr};

    /// Encode a frame of known PQ codes and check the reduction recovers the
    /// light levels the metadata will claim.
    fn measure_frame(
        width: u32,
        height: u32,
        fill: impl Fn(u32, u32) -> u32,
    ) -> Option<FrameLightLevels> {
        let ctx = GpuContext::new_headless().ok()?;
        let mut meter = ContentLightMeter::new(&ctx.device).ok()?;

        let texture = ctx.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Measure Source"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgb10a2Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let mut words = Vec::with_capacity((width * height) as usize);
        for y in 0..height {
            for x in 0..width {
                let c = fill(x, y);
                words.push(c | (c << 10) | (c << 20));
            }
        }
        ctx.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&words),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // The readback is asynchronous, so drive frames until it lands.
        for _ in 0..8 {
            let mut encoder = ctx
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Measure"),
                });
            meter.measure(&ctx.device, &mut encoder, &view, width, height);
            ctx.queue.submit([encoder.finish()]);
            let _ = ctx.device.poll(wgpu::PollType::wait_indefinitely());
            if let Some(levels) = meter.try_read(&ctx.device) {
                return Some(levels);
            }
        }
        None
    }

    #[test]
    fn a_uniform_frame_measures_its_own_luminance() {
        // 594 is the PQ code for 203 cd/m², BT.2408 reference white, so this is
        // the value a correctly anchored pipeline puts display white at.
        let Some(levels) = measure_frame(64, 64, |_, _| 594) else {
            return;
        };
        let expected = hdr::nits_from_pq(594.0 / 1023.0);
        assert!(
            (levels.max - expected).abs() / expected < 0.02,
            "max {} against expected {expected}",
            levels.max
        );
        // A uniform frame's average equals its max.
        assert!(
            (levels.average - levels.max).abs() / levels.max < 0.01,
            "average {} against max {}",
            levels.average,
            levels.max
        );
    }

    #[test]
    fn one_bright_pixel_raises_max_but_barely_moves_the_average() {
        // The distinction MaxCLL and MaxFALL exist to draw: a specular highlight
        // is bright but contributes almost nothing to the frame average.
        let Some(levels) = measure_frame(64, 64, |x, y| if x == 0 && y == 0 { 1023 } else { 0 })
        else {
            return;
        };
        let peak = hdr::nits_from_pq(1.0);
        assert!(
            (levels.max - peak).abs() / peak < 0.02,
            "one bright pixel should set max to {peak}, got {}",
            levels.max
        );
        // One pixel in 4096 of a 10000 nit peak is about 2.4 nits.
        assert!(
            levels.average < peak / 1000.0,
            "average {} was pulled up by a single pixel",
            levels.average
        );
        assert!(levels.average > 0.0, "the bright pixel vanished entirely");
    }

    #[test]
    fn a_partial_tile_does_not_bias_the_average() {
        // 65x65 is not a multiple of the 4x4 reduction, so the edge tiles are
        // partial. Carrying a sample count rather than assuming full tiles is
        // what keeps a uniform frame reading as uniform.
        let Some(levels) = measure_frame(65, 65, |_, _| 594) else {
            return;
        };
        assert!(
            (levels.average - levels.max).abs() / levels.max < 0.01,
            "partial tiles biased the average: {} against max {}",
            levels.average,
            levels.max
        );
    }

    #[test]
    fn a_black_frame_measures_black() {
        let Some(levels) = measure_frame(64, 64, |_, _| 0) else {
            return;
        };
        assert!(levels.max < 0.01, "black measured {}", levels.max);
        assert!(levels.average < 0.01, "black averaged {}", levels.average);
    }
}
