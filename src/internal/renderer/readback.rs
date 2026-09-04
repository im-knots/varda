//! GPU readback — async texture-to-CPU transfer with double buffering.
//!
//! Used by headless outputs (NDI send, Syphon server, recording) and the
//! analyzer pipeline to read rendered frames back to the CPU without stalling
//! the render thread. Each staging buffer advances through a non-blocking state
//! machine: a copy is enqueued (`Copied`), then mapped asynchronously
//! (`Mapping`), then read once the GPU signals completion. The render thread
//! never blocks on `poll(Wait)` — it only ever does a non-blocking `poll(Poll)`
//! and `try_recv`, accepting a couple frames of latency instead.

use std::sync::mpsc::{Receiver, TryRecvError};

/// Storage copied from the GPU texture into each staging row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadbackFormat {
    /// Four eight-bit RGBA channels.
    Rgba8,
    /// Four eight-bit BGRA channels.
    Bgra8,
    /// Packed 10:10:10:2 normalized channels.
    Rgb10A2,
    /// Four IEEE-754 half-float channels.
    Rgba16Float,
    /// Four normalized unsigned 16-bit integer channels.
    Rgba16Unorm,
    /// Packed 4:2:2 eight-bit video (`U0 Y0 V0 Y1`).
    Uyvy,
    /// Planar 4:2:2 ten-bit video in sixteen-bit words.
    P216,
}

impl ReadbackFormat {
    const fn bytes_per_pixel(self) -> u32 {
        match self {
            Self::Rgba8 | Self::Bgra8 | Self::Rgb10A2 | Self::P216 => 4,
            Self::Rgba16Float | Self::Rgba16Unorm => 8,
            Self::Uyvy => 2,
        }
    }
}

fn row_layout(width: u32, format: ReadbackFormat) -> (u32, u32) {
    let unpadded = width.saturating_mul(format.bytes_per_pixel());
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = unpadded.div_ceil(align).saturating_mul(align);
    (unpadded, padded)
}

/// Tightly packed CPU frame with explicit storage metadata.
pub struct ReadbackFrame {
    format: ReadbackFormat,
    width: u32,
    height: u32,
    stride: u32,
    color_profile: crate::engine::value::render::PresentationColorProfile,
    alpha_mode: crate::engine::value::render::AlphaMode,
    bytes: Vec<u8>,
}

impl ReadbackFrame {
    /// Pixel storage format.
    pub fn format(&self) -> ReadbackFormat {
        self.format
    }

    /// Frame width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Frame height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Bytes in one tightly packed row.
    pub fn stride(&self) -> u32 {
        self.stride
    }

    /// Transfer/range contract associated with these bytes.
    pub fn color_profile(&self) -> crate::engine::value::render::PresentationColorProfile {
        self.color_profile
    }

    /// Alpha interpretation associated with these bytes.
    pub fn alpha_mode(&self) -> crate::engine::value::render::AlphaMode {
        self.alpha_mode
    }

    /// Tightly packed frame bytes.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Consume the frame and return its bytes.
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

/// Per-buffer state in the non-blocking readback cycle.
enum SlotState {
    /// Available to be used as the target of a new copy.
    Free,
    /// A texture→buffer copy has been enqueued (and is/will be submitted by the
    /// caller). The buffer can be mapped once that submission has executed.
    Copied,
    /// `map_async` has been issued; awaiting the completion callback.
    Mapping(Receiver<Result<(), wgpu::BufferAsyncError>>),
}

/// Double-buffered GPU→CPU readback. Alternates two staging buffers
/// so the GPU copy and CPU map never contend on the same buffer.
pub struct ReadbackBuffer {
    buffers: [wgpu::Buffer; 2],
    /// Width of the source texture
    width: u32,
    /// Height of the source texture
    height: u32,
    /// Storage copied from the source texture.
    format: ReadbackFormat,
    color_profile: crate::engine::value::render::PresentationColorProfile,
    alpha_mode: crate::engine::value::render::AlphaMode,
    /// Bytes in one tightly packed source row.
    unpadded_bytes_per_row: u32,
    /// Bytes per row (aligned to wgpu requirements)
    padded_bytes_per_row: u32,
    /// Non-blocking state machine state for each staging buffer.
    slots: [SlotState; 2],
}

impl ReadbackBuffer {
    /// Create a new `ReadbackBuffer` for a resolution and explicit format.
    pub fn new(device: &wgpu::Device, width: u32, height: u32, format: ReadbackFormat) -> Self {
        Self::new_with_contract(
            device,
            width,
            height,
            format,
            crate::engine::value::render::PresentationColorProfile::SrgbFull,
            crate::engine::value::render::AlphaMode::Opaque,
        )
    }

    /// Create a readback buffer with an explicit pixel and color contract.
    pub fn new_with_contract(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        format: ReadbackFormat,
        color_profile: crate::engine::value::render::PresentationColorProfile,
        alpha_mode: crate::engine::value::render::AlphaMode,
    ) -> Self {
        let (unpadded_bytes_per_row, padded_bytes_per_row) = row_layout(width, format);
        let buffer_size = u64::from(padded_bytes_per_row * height);

        let buffers = [
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Readback Buffer A"),
                size: buffer_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Readback Buffer B"),
                size: buffer_size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
        ];

        Self {
            buffers,
            width,
            height,
            format,
            color_profile,
            alpha_mode,
            unpadded_bytes_per_row,
            padded_bytes_per_row,
            slots: [SlotState::Free, SlotState::Free],
        }
    }

    /// Width of the readback target.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of the readback target.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Pixel storage copied by this readback buffer.
    pub fn format(&self) -> ReadbackFormat {
        self.format
    }

    /// Color contract attached to future frames.
    pub fn color_profile(&self) -> crate::engine::value::render::PresentationColorProfile {
        self.color_profile
    }

    /// Alpha contract attached to future frames.
    pub fn alpha_mode(&self) -> crate::engine::value::render::AlphaMode {
        self.alpha_mode
    }

    /// Enqueue a texture→buffer copy for this frame. Call during command encoding.
    /// The source texture must have `COPY_SRC` usage.
    ///
    /// Picks any buffer currently in the `Free` state as the copy target. If both
    /// buffers are still in flight (GPU behind), the copy is skipped this frame
    /// rather than blocking — the readback simply refreshes on a later frame.
    pub fn begin_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::Texture,
    ) {
        let Some(idx) = self.slots.iter().position(|s| matches!(s, SlotState::Free)) else {
            return;
        };

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: source_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffers[idx],
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bytes_per_row),
                    rows_per_image: Some(self.height),
                },
            },
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        self.slots[idx] = SlotState::Copied;
    }

    /// Non-blocking attempt to read back a previously copied frame.
    ///
    /// Returns a typed, tightly packed frame for the most recent buffer whose
    /// map has completed, or `None` if nothing is ready yet. This never
    /// blocks the render thread: it advances pending maps with a non-blocking
    /// `poll(Poll)` and checks completion with `try_recv`, accepting a couple
    /// frames of latency. Works regardless of whether the copy was submitted in a
    /// prior frame (analyzer path) or immediately before this call (headless path).
    pub fn try_read(&mut self, device: &wgpu::Device) -> Option<ReadbackFrame> {
        // Give wgpu a chance to fire any completed map callbacks (non-blocking).
        let _ = device.poll(wgpu::PollType::Poll);

        let mut result = None;
        for idx in 0..self.slots.len() {
            match std::mem::replace(&mut self.slots[idx], SlotState::Free) {
                SlotState::Free => {}
                SlotState::Copied => {
                    // The copy has been submitted by now; issue the async map and
                    // check for completion on a subsequent call.
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.buffers[idx]
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |r| {
                            let _ = tx.send(r);
                        });
                    self.slots[idx] = SlotState::Mapping(rx);
                }
                SlotState::Mapping(rx) => match rx.try_recv() {
                    Ok(Ok(())) => {
                        result = Some(ReadbackFrame {
                            format: self.format,
                            width: self.width,
                            height: self.height,
                            stride: self.unpadded_bytes_per_row,
                            color_profile: self.color_profile,
                            alpha_mode: self.alpha_mode,
                            bytes: self.copy_out(idx),
                        });
                        self.buffers[idx].unmap();
                        // slot left Free
                    }
                    Err(TryRecvError::Empty) => {
                        // Still in flight — restore state and check next frame.
                        self.slots[idx] = SlotState::Mapping(rx);
                    }
                    Ok(Err(e)) => {
                        log::warn!("GPU readback map failed: {e}");
                        self.buffers[idx].unmap();
                        // slot left Free
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.buffers[idx].unmap();
                        // slot left Free
                    }
                },
            }
        }
        result
    }

    /// Copy the mapped contents of buffer `idx` into a tightly-packed RGBA vec,
    /// stripping any per-row padding. The buffer must be mapped.
    fn copy_out(&self, idx: usize) -> Vec<u8> {
        let data = self.buffers[idx]
            .slice(..)
            .get_mapped_range()
            .expect("readback callback only exposes successfully mapped buffers");
        let unpadded_bytes_per_row = self.unpadded_bytes_per_row as usize;
        let padded = self.padded_bytes_per_row as usize;

        if padded == unpadded_bytes_per_row {
            data.to_vec()
        } else {
            let mut out = Vec::with_capacity(unpadded_bytes_per_row * self.height as usize);
            for row in 0..self.height as usize {
                let start = row * padded;
                let end = start + unpadded_bytes_per_row;
                out.extend_from_slice(&data[start..end]);
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ReadbackFormat, ReadbackFrame, row_layout};
    use crate::engine::value::render::{AlphaMode, PresentationColorProfile};

    #[test]
    fn row_layout_uses_format_bytes_per_pixel_and_gpu_alignment() {
        assert_eq!(row_layout(64, ReadbackFormat::Rgba8), (256, 256));
        assert_eq!(row_layout(65, ReadbackFormat::Rgb10A2), (260, 512));
        assert_eq!(row_layout(65, ReadbackFormat::Rgba16Float), (520, 768));
        assert_eq!(row_layout(65, ReadbackFormat::Rgba16Unorm), (520, 768));
        assert_eq!(row_layout(65, ReadbackFormat::Uyvy), (130, 256));
        assert_eq!(row_layout(65, ReadbackFormat::P216), (260, 512));
    }

    #[test]
    fn chaos_hostile_widths_stay_aligned_and_do_not_overflow() {
        for format in [
            ReadbackFormat::Rgba8,
            ReadbackFormat::Bgra8,
            ReadbackFormat::Rgb10A2,
            ReadbackFormat::Rgba16Float,
            ReadbackFormat::Rgba16Unorm,
            ReadbackFormat::Uyvy,
            ReadbackFormat::P216,
        ] {
            let (unpadded, padded) = row_layout(0, format);
            assert_eq!(unpadded, 0, "{format:?}");
            assert_eq!(padded, 0, "{format:?}");

            let (unpadded, padded) = row_layout(1, format);
            assert_eq!(unpadded, format.bytes_per_pixel());
            assert_eq!(padded % wgpu::COPY_BYTES_PER_ROW_ALIGNMENT, 0);
            assert!(padded >= unpadded);

            let (_, padded) = row_layout(u32::MAX, format);
            // An aligned stride may not fit in u32 once width saturates.
            assert!(
                padded == u32::MAX || padded.is_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
                "{format:?} hostile padded stride {padded}"
            );
        }
    }

    #[test]
    fn bgra_and_rgba_have_identical_storage_width() {
        assert_eq!(
            row_layout(193, ReadbackFormat::Rgba8),
            row_layout(193, ReadbackFormat::Bgra8)
        );
    }

    #[test]
    fn frame_keeps_storage_color_and_alpha_metadata_together() {
        let frame = ReadbackFrame {
            format: ReadbackFormat::P216,
            width: 1920,
            height: 1080,
            stride: 7680,
            color_profile: PresentationColorProfile::Rec709Limited,
            alpha_mode: AlphaMode::Opaque,
            bytes: Vec::new(),
        };

        assert_eq!(frame.format(), ReadbackFormat::P216);
        assert_eq!(frame.stride(), 7680);
        assert_eq!(
            frame.color_profile(),
            PresentationColorProfile::Rec709Limited
        );
        assert_eq!(frame.alpha_mode(), AlphaMode::Opaque);
    }
}
