//! GPU conversion and asynchronous readback for NDI P216 frames.

use std::sync::mpsc::{Receiver, TryRecvError};

use wgpu::util::DeviceExt;

/// Rec.709 SDR metadata accepted in `NDIlib_video_frame_v2_t::p_metadata`.
pub(super) const REC709_METADATA: &[u8] =
    b"<ndi_color_info primaries=\"bt_709\" transfer=\"bt_709\" matrix=\"bt_709\" />\0";

/// Exact semi-planar storage contract consumed by the NDI SDK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct P216Layout {
    pub stride: u32,
    pub y_offset: u64,
    pub uv_offset: u64,
    pub byte_len: u64,
}

impl P216Layout {
    pub fn new(width: u32, height: u32) -> Result<Self, P216Error> {
        if width == 0 || height == 0 {
            return Err(P216Error::EmptyFrame);
        }
        if !width.is_multiple_of(2) {
            return Err(P216Error::OddWidth(width));
        }
        let stride = width.checked_mul(2).ok_or(P216Error::FrameTooLarge)?;
        let plane_len = u64::from(stride)
            .checked_mul(u64::from(height))
            .ok_or(P216Error::FrameTooLarge)?;
        let byte_len = plane_len.checked_mul(2).ok_or(P216Error::FrameTooLarge)?;
        Ok(Self {
            stride,
            y_offset: 0,
            uv_offset: plane_len,
            byte_len,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum P216Error {
    EmptyFrame,
    OddWidth(u32),
    FrameTooLarge,
    InvalidBuffer { expected: u64, actual: usize },
}

impl std::fmt::Display for P216Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyFrame => formatter.write_str("P216 requires non-zero dimensions"),
            Self::OddWidth(width) => {
                write!(formatter, "P216 requires an even width, received {width}")
            }
            Self::FrameTooLarge => {
                formatter.write_str("P216 frame size exceeds addressable storage")
            }
            Self::InvalidBuffer { expected, actual } => write!(
                formatter,
                "P216 buffer has {actual} bytes, expected {expected}"
            ),
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ConvertParams {
    width: u32,
    height: u32,
    dither: u32,
    _padding: u32,
}

enum SlotState {
    Free,
    Copied,
    Mapping(Receiver<Result<(), wgpu::BufferAsyncError>>),
}

/// Per-sender converter. One storage buffer feeds two asynchronous map slots.
pub(super) struct P216Converter {
    width: u32,
    height: u32,
    layout: P216Layout,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    output: wgpu::Buffer,
    staging: [wgpu::Buffer; 2],
    slots: [SlotState; 2],
}

impl P216Converter {
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Result<Self, P216Error> {
        let layout = P216Layout::new(width, height)?;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("NDI P216 Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("NDI P216 Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NDI P216 Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("p216.wgsl").into()),
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("NDI P216 Pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("NDI P216 Params"),
            contents: bytemuck::bytes_of(&ConvertParams {
                width,
                height,
                dither: 0,
                _padding: 0,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let output = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("NDI P216 GPU Output"),
            size: layout.byte_len,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let staging = std::array::from_fn(|index| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(if index == 0 {
                    "NDI P216 Readback A"
                } else {
                    "NDI P216 Readback B"
                }),
                size: layout.byte_len,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        });

        Ok(Self {
            width,
            height,
            layout,
            pipeline,
            bind_group_layout,
            params,
            output,
            staging,
            slots: [SlotState::Free, SlotState::Free],
        })
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Encode and enqueue readback. Returns false when both staging slots are busy.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::TextureView,
        dither: bool,
    ) -> bool {
        let Some(slot) = self
            .slots
            .iter()
            .position(|state| matches!(state, SlotState::Free))
        else {
            return false;
        };
        queue.write_buffer(
            &self.params,
            0,
            bytemuck::bytes_of(&ConvertParams {
                width: self.width,
                height: self.height,
                dither: u32::from(dither),
                _padding: 0,
            }),
        );
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("NDI P216 Bind Group"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: self.params.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.output.as_entire_binding(),
                },
            ],
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("NDI P216 Convert"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.dispatch_workgroups(self.width.div_ceil(16), self.height.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(
            &self.output,
            0,
            &self.staging[slot],
            0,
            self.layout.byte_len,
        );
        self.slots[slot] = SlotState::Copied;
        true
    }

    pub fn try_read(&mut self, device: &wgpu::Device) -> Option<Vec<u8>> {
        let _ = device.poll(wgpu::PollType::Poll);
        let mut frame = None;
        for index in 0..self.slots.len() {
            match std::mem::replace(&mut self.slots[index], SlotState::Free) {
                SlotState::Free => {}
                SlotState::Copied => {
                    let (sender, receiver) = std::sync::mpsc::channel();
                    self.staging[index]
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |result| {
                            let _ = sender.send(result);
                        });
                    self.slots[index] = SlotState::Mapping(receiver);
                }
                SlotState::Mapping(receiver) => match receiver.try_recv() {
                    Ok(Ok(())) => {
                        frame = Some(
                            self.staging[index]
                                .slice(..)
                                .get_mapped_range()
                                .expect("successful P216 map has a mapped range")
                                .to_vec(),
                        );
                        self.staging[index].unmap();
                    }
                    Err(TryRecvError::Empty) => {
                        self.slots[index] = SlotState::Mapping(receiver);
                    }
                    Ok(Err(error)) => {
                        log::warn!("NDI P216 readback failed: {error}");
                        self.staging[index].unmap();
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.staging[index].unmap();
                    }
                },
            }
        }
        frame
    }

    pub const fn layout(&self) -> P216Layout {
        self.layout
    }
}

#[cfg(test)]
mod tests {
    use super::{P216Converter, P216Error, P216Layout, REC709_METADATA};

    fn encode_once(
        context: &crate::renderer::context::GpuContext,
        converter: &mut P216Converter,
        source: &wgpu::TextureView,
        dither: bool,
    ) -> Vec<u8> {
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("P216 Test Encoder"),
            });
        assert!(converter.encode(
            &context.device,
            &context.queue,
            &mut encoder,
            source,
            dither
        ));
        context.submit(std::iter::once(encoder.finish()));
        assert!(converter.try_read(&context.device).is_none());
        let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
        converter
            .try_read(&context.device)
            .expect("P216 GPU readback")
    }

    #[test]
    fn layout_places_uv_plane_after_luma_plane() {
        let layout = P216Layout::new(1920, 1080).unwrap();
        assert_eq!(layout.stride, 3840);
        assert_eq!(layout.y_offset, 0);
        assert_eq!(layout.uv_offset, 4_147_200);
        assert_eq!(layout.byte_len, 8_294_400);
    }

    #[test]
    fn odd_width_is_rejected() {
        assert_eq!(P216Layout::new(1919, 1080), Err(P216Error::OddWidth(1919)));
    }

    #[test]
    fn chaos_empty_and_overflowing_frames_are_rejected() {
        assert_eq!(P216Layout::new(0, 1080), Err(P216Error::EmptyFrame));
        assert_eq!(P216Layout::new(1920, 0), Err(P216Error::EmptyFrame));
        assert_eq!(P216Layout::new(2, 0), Err(P216Error::EmptyFrame));
        assert_eq!(
            P216Layout::new(u32::MAX, 2),
            Err(P216Error::OddWidth(u32::MAX))
        );
        assert_eq!(
            P216Layout::new(u32::MAX - 1, u32::MAX),
            Err(P216Error::FrameTooLarge)
        );
    }

    #[test]
    fn metadata_names_rec709_components() {
        let metadata = std::ffi::CStr::from_bytes_with_nul(REC709_METADATA)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(metadata.contains("primaries=\"bt_709\""));
        assert!(metadata.contains("transfer=\"bt_709\""));
        assert!(metadata.contains("matrix=\"bt_709\""));
    }

    #[test]
    fn gpu_packs_known_black_white_pair_into_high_bit_planes() {
        let Ok(context) = crate::renderer::context::GpuContext::new_headless() else {
            return;
        };
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("P216 Known Values"),
            size: wgpu::Extent3d {
                width: 2,
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
        let black = half::f16::ZERO.to_bits().to_le_bytes();
        let white = half::f16::ONE.to_bits().to_le_bytes();
        let mut pixels = Vec::with_capacity(16);
        for channel in [black, black, black, white, white, white, white, white] {
            pixels.extend_from_slice(&channel);
        }
        context.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(16),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 2,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut converter = P216Converter::new(&context.device, 2, 1).unwrap();
        let bytes = encode_once(&context, &mut converter, &view, false);

        // Y plane: limited-range black 64 and white 940. UV plane: neutral 512.
        // Every ten-bit value occupies the high bits of its little-endian u16.
        assert_eq!(bytes, [0x00, 0x10, 0x00, 0xeb, 0x00, 0x80, 0x00, 0x80]);
    }

    #[test]
    fn gpu_dither_is_stable_and_within_one_ten_bit_lsb() {
        const WIDTH: u32 = 32;
        let Ok(context) = crate::renderer::context::GpuContext::new_headless() else {
            return;
        };
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("P216 Dither Values"),
            size: wgpu::Extent3d {
                width: WIDTH,
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
        let gray = half::f16::from_f32(0.42).to_bits().to_le_bytes();
        let alpha = half::f16::ONE.to_bits().to_le_bytes();
        let mut pixels = Vec::with_capacity(WIDTH as usize * 8);
        for _ in 0..WIDTH {
            for channel in [gray, gray, gray, alpha] {
                pixels.extend_from_slice(&channel);
            }
        }
        context.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(WIDTH * 8),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: WIDTH,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut converter = P216Converter::new(&context.device, WIDTH, 1).unwrap();

        let undithered = encode_once(&context, &mut converter, &view, false);
        let first = encode_once(&context, &mut converter, &view, true);
        let second = encode_once(&context, &mut converter, &view, true);

        assert_eq!(first, second);
        assert_ne!(first, undithered);
        for (plain, dithered) in undithered
            .as_chunks::<2>()
            .0
            .iter()
            .zip(first.as_chunks::<2>().0)
        {
            let plain = i32::from(u16::from_le_bytes(*plain));
            let dithered = i32::from(u16::from_le_bytes(*dithered));
            assert!(
                (plain - dithered).abs() <= 64,
                "dither changed a component by more than one ten-bit LSB"
            );
        }
    }
}
