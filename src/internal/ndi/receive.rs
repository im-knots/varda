//! The receive side's frame handoff and GPU conversion. The receive thread
//! copies what the SDK captured into a buffer reused across frames; the render
//! thread uploads it and, for UYVY, expands it to RGB on the GPU.

use std::sync::Mutex;

use super::ffi;

/// The color format a receiver asks the SDK for: UYVY from a source without
/// alpha, RGBA from one with it. Neither needs a conversion in the SDK.
pub(super) const RECEIVE_COLOR_FORMAT: i32 = 3; // NDIlib_recv_color_format_UYVY_RGBA

/// The deck texture a receiver renders into.
const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Pixel layouts a receiver accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ReceivedFormat {
    #[default]
    Uyvy,
    Rgba,
}

impl ReceivedFormat {
    fn from_fourcc(fourcc: ffi::NDIlib_FourCC_video_type_e) -> Option<Self> {
        if fourcc == ffi::NDIlib_FourCC_video_type_e::UYVY {
            Some(Self::Uyvy)
        } else if fourcc == ffi::NDIlib_FourCC_video_type_e::RGBA {
            Some(Self::Rgba)
        } else {
            None
        }
    }

    /// Width in four-byte texels: UYVY packs each pixel pair into one.
    fn texels(self, width: u32) -> u32 {
        match self {
            Self::Uyvy => width.div_ceil(2),
            Self::Rgba => width,
        }
    }
}

/// One captured frame's bytes, rows `stride` bytes apart.
#[derive(Default)]
struct ReceivedFrame {
    format: ReceivedFormat,
    bytes: Vec<u8>,
    width: u32,
    height: u32,
    stride: u32,
}

impl ReceivedFrame {
    /// Copy `vf` in, reusing this frame's buffer. False when the SDK handed
    /// over a layout the receiver did not ask for, or a malformed frame.
    fn fill(&mut self, vf: &ffi::NDIlib_video_frame_v2_t) -> bool {
        let Some(format) = ReceivedFormat::from_fourcc(vf.FourCC) else {
            return false;
        };
        let (Ok(width), Ok(height)) = (u32::try_from(vf.xres), u32::try_from(vf.yres)) else {
            return false;
        };
        let row = format.texels(width) * 4;
        let stride = u32::try_from(vf.line_stride_in_bytes)
            .ok()
            .filter(|&s| s > 0)
            .unwrap_or(row);
        if vf.p_data.is_null() || width == 0 || height == 0 || stride < row || stride % 4 != 0 {
            return false;
        }
        let len = stride as usize * height as usize;
        // SAFETY: the SDK guarantees `p_data` holds `yres` rows of
        // `line_stride_in_bytes` until the frame is freed, which the caller
        // does only after this returns.
        let src = unsafe { std::slice::from_raw_parts(vf.p_data, len) };
        self.bytes.clear();
        self.bytes.extend_from_slice(src);
        self.format = format;
        self.width = width;
        self.height = height;
        self.stride = stride;
        true
    }
}

#[derive(Default)]
struct Exchange {
    ready: ReceivedFrame,
    fresh: bool,
    spare: ReceivedFrame,
}

/// The latest captured frame, passed from the receive thread to the render
/// thread. Two buffers take turns, so once they have grown to the frame's size
/// neither side allocates.
#[derive(Default)]
pub(super) struct FrameSlot(Mutex<Exchange>);

impl FrameSlot {
    /// Receive thread: make `vf` the latest frame. False, storing nothing,
    /// when its layout is not one the receiver asked for.
    pub(super) fn deposit(&self, vf: &ffi::NDIlib_video_frame_v2_t) -> bool {
        let Ok(mut exchange) = self.0.lock() else {
            return false;
        };
        let mut frame = std::mem::take(&mut exchange.spare);
        drop(exchange);
        let stored = frame.fill(vf);
        let Ok(mut exchange) = self.0.lock() else {
            return false;
        };
        if stored {
            std::mem::swap(&mut exchange.ready, &mut frame);
            exchange.fresh = true;
        }
        exchange.spare = frame;
        stored
    }
}

/// The GPU pass that expands packed UYVY into a receiver's deck texture.
pub(super) struct UyvyUnpacker {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

impl UyvyUnpacker {
    pub(super) fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("NDI receive UYVY"),
            source: wgpu::ShaderSource::Wgsl(include_str!("receive.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("NDI receive UYVY"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("NDI receive UYVY"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("NDI receive UYVY"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("fullscreen"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("uyvy"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: TARGET_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        Self { pipeline, layout }
    }
}

/// The half-width texture a UYVY frame is uploaded into before expansion.
struct PackedUpload {
    texture: wgpu::Texture,
    bind_group: wgpu::BindGroup,
}

/// A receiver's deck texture, and what it takes to fill it.
pub(super) struct ReceiveTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    width: u32,
    height: u32,
    packed: Option<PackedUpload>,
}

impl ReceiveTarget {
    pub(super) fn new(device: &wgpu::Device, label: &str, width: u32, height: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: extent(width, height),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            width,
            height,
            packed: None,
        }
    }

    pub(super) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub(super) fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// Render thread: put the slot's frame into the deck texture if a new one
    /// has arrived, resizing the texture to match it. A UYVY frame's expansion
    /// is recorded into `encoder`, created on first need, for the caller to
    /// submit.
    pub(super) fn update(
        &mut self,
        slot: &FrameSlot,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        unpacker: &mut Option<UyvyUnpacker>,
        encoder: &mut Option<wgpu::CommandEncoder>,
    ) {
        let Ok(mut exchange) = slot.0.try_lock() else {
            return;
        };
        if !exchange.fresh {
            return;
        }
        exchange.fresh = false;
        let frame = &exchange.ready;
        if (frame.width, frame.height) != (self.width, self.height) {
            log::info!(
                "NDI receiver: resolution changed {}×{} → {}×{}",
                self.width,
                self.height,
                frame.width,
                frame.height
            );
            *self = Self::new(device, "NDI Receive (resized)", frame.width, frame.height);
        }
        match frame.format {
            ReceivedFormat::Rgba => write(queue, &self.texture, frame, frame.width),
            ReceivedFormat::Uyvy => {
                let unpacker = unpacker.get_or_insert_with(|| UyvyUnpacker::new(device));
                let texels = ReceivedFormat::Uyvy.texels(frame.width);
                let packed = self
                    .packed
                    .get_or_insert_with(|| packed_upload(device, unpacker, texels, frame.height));
                write(queue, &packed.texture, frame, texels);
                let encoder = encoder.get_or_insert_with(|| {
                    device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("NDI receive"),
                    })
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("NDI receive UYVY"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &self.view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                pass.set_pipeline(&unpacker.pipeline);
                pass.set_bind_group(0, &packed.bind_group, &[]);
                pass.draw(0..3, 0..1);
            }
        }
    }
}

fn packed_upload(
    device: &wgpu::Device,
    unpacker: &UyvyUnpacker,
    texels: u32,
    height: u32,
) -> PackedUpload {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("NDI receive UYVY upload"),
        size: extent(texels, height),
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("NDI receive UYVY"),
        layout: &unpacker.layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    PackedUpload {
        texture,
        bind_group,
    }
}

fn write(queue: &wgpu::Queue, texture: &wgpu::Texture, frame: &ReceivedFrame, texels: u32) {
    queue.write_texture(
        texture.as_image_copy(),
        &frame.bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(frame.stride),
            rows_per_image: Some(frame.height),
        },
        extent(texels, frame.height),
    );
}

fn extent(width: u32, height: u32) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width,
        height,
        depth_or_array_layers: 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{FrameSlot, ReceiveTarget, UyvyUnpacker, ffi};
    use crate::renderer::context::GpuContext;

    fn frame(
        fourcc: ffi::NDIlib_FourCC_video_type_e,
        bytes: &mut [u8],
        width: i32,
        height: i32,
        stride: i32,
    ) -> ffi::NDIlib_video_frame_v2_t {
        ffi::NDIlib_video_frame_v2_t {
            xres: width,
            yres: height,
            FourCC: fourcc,
            frame_rate_N: 60,
            frame_rate_D: 1,
            picture_aspect_ratio: 0.0,
            frame_format_type: 1,
            timecode: 0,
            p_data: bytes.as_mut_ptr(),
            line_stride_in_bytes: stride,
            p_metadata: std::ptr::null(),
            timestamp: 0,
        }
    }

    /// Deposit one frame, run the render thread's update, and read the deck
    /// texture back as tightly packed RGBA.
    fn receive(context: &GpuContext, vf: &ffi::NDIlib_video_frame_v2_t) -> (Vec<u8>, (u32, u32)) {
        let slot = FrameSlot::default();
        assert!(slot.deposit(vf));
        let mut target = ReceiveTarget::new(&context.device, "test", 2, 1);
        let mut unpacker: Option<UyvyUnpacker> = None;
        let mut encoder = None;
        target.update(
            &slot,
            &context.device,
            &context.queue,
            &mut unpacker,
            &mut encoder,
        );
        if let Some(encoder) = encoder {
            context.submit(std::iter::once(encoder.finish()));
        }
        let (width, height) = target.dimensions();
        (
            read_rgba(context, &target.texture, width, height),
            (width, height),
        )
    }

    fn read_rgba(
        context: &GpuContext,
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
    ) -> Vec<u8> {
        let padded = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("NDI receive readback"),
            size: u64::from(padded * height),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            super::extent(width, height),
        );
        context.submit(std::iter::once(encoder.finish()));
        buffer.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
        let mapped = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback");
        mapped
            .chunks(padded as usize)
            .flat_map(|row| row[..(width * 4) as usize].to_vec())
            .collect()
    }

    #[test]
    fn frames_in_layouts_the_receiver_did_not_ask_for_are_dropped() {
        let slot = FrameSlot::default();
        let mut bytes = [0u8; 8];
        let bgra = frame(ffi::NDIlib_FourCC_video_type_e::BGRA, &mut bytes, 2, 1, 8);
        assert!(!slot.deposit(&bgra));
        assert!(!slot.0.lock().unwrap().fresh);
    }

    #[test]
    fn the_two_buffers_are_reused_once_grown() {
        let slot = FrameSlot::default();
        let mut bytes = vec![128u8; 1920 * 2 * 4];
        let vf = frame(
            ffi::NDIlib_FourCC_video_type_e::UYVY,
            &mut bytes,
            1920,
            4,
            3840,
        );
        assert!(slot.deposit(&vf));
        assert!(slot.deposit(&vf));
        let pointers = {
            let exchange = slot.0.lock().unwrap();
            (exchange.ready.bytes.as_ptr(), exchange.spare.bytes.as_ptr())
        };
        assert!(slot.deposit(&vf));
        assert!(slot.deposit(&vf));
        let exchange = slot.0.lock().unwrap();
        assert_eq!(
            (exchange.ready.bytes.as_ptr(), exchange.spare.bytes.as_ptr()),
            pointers
        );
    }

    /// Limited range expands to full: luma 16 is black and 235 is white.
    #[test]
    fn limited_range_black_and_white_expand_to_full_range() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let mut bytes = [128, 16, 128, 235];
        let vf = frame(ffi::NDIlib_FourCC_video_type_e::UYVY, &mut bytes, 2, 1, 4);
        let (rgba, _) = receive(&context, &vf);
        assert_eq!(rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
    }

    /// RGBA arrives ready to use: uploaded as-is, row padding skipped.
    #[test]
    fn rgba_frames_upload_unchanged() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let mut bytes = [
            10, 20, 30, 40, 50, 60, 70, 80, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 7, 8, 0, 0, 0, 0,
        ];
        let vf = frame(ffi::NDIlib_FourCC_video_type_e::RGBA, &mut bytes, 2, 2, 12);
        let (rgba, size) = receive(&context, &vf);
        assert_eq!(size, (2, 2));
        assert_eq!(
            rgba,
            [10, 20, 30, 40, 50, 60, 70, 80, 1, 2, 3, 4, 5, 6, 7, 8]
        );
    }

    /// A frame the send path packed comes back to the original. Each pixel
    /// pair shares a color, so chroma subsampling loses nothing and only
    /// eight-bit quantization remains: within one code value, or three near
    /// black, where Rec.709's shallower toe spreads one chroma code over
    /// several sRGB codes.
    #[test]
    fn uyvy_from_the_send_path_round_trips() {
        let Some(context) = crate::testing::headless_gpu() else {
            return;
        };
        let colors: [[u8; 3]; 7] = [
            [0, 0, 0],
            [255, 255, 255],
            [128, 128, 128],
            [255, 0, 0],
            [0, 255, 0],
            [200, 120, 60],
            [60, 180, 220],
        ];
        let width = colors.len() as u32 * 2;
        let original: Vec<u8> = colors
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255, r, g, b, 255])
            .collect();
        let texture = context.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("NDI round trip source"),
            size: super::extent(width, 1),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        context.queue.write_texture(
            texture.as_image_copy(),
            &original,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(1),
            },
            super::extent(width, 1),
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut converter = super::super::convert::SendConverter::new(
            &context.device,
            super::super::convert::SendFormat::Uyvy,
            width,
            1,
        )
        .unwrap();
        let mut encoder = context
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        assert!(converter.encode(&context.device, &context.queue, &mut encoder, &view, false));
        context.submit(std::iter::once(encoder.finish()));
        let mut uyvy = None;
        while uyvy.is_none() {
            let _ = context.device.poll(wgpu::PollType::wait_indefinitely());
            uyvy = converter.try_read(&context.device);
        }
        let mut uyvy = uyvy.unwrap();
        let vf = frame(
            ffi::NDIlib_FourCC_video_type_e::UYVY,
            &mut uyvy,
            width.cast_signed(),
            1,
            (width * 2).cast_signed(),
        );
        let (rgba, _) = receive(&context, &vf);
        for (i, (got, want)) in rgba.iter().zip(&original).enumerate() {
            let tolerance = if *want < 16 { 3 } else { 1 };
            assert!(
                got.abs_diff(*want) <= tolerance,
                "pixel {} channel {}: got {got}, want {want}",
                i / 4,
                i % 4
            );
        }
    }
}
