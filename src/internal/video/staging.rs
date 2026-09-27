//! Double-buffered CPU-to-GPU staging for decoded video frames.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// Double-buffered staging buffers for non-blocking GPU texture uploads.
///
/// Uses a ping-pong pattern: CPU writes to buffer\[current\], GPU copies from
/// buffer\[1-current\]. By the time we circle back two frames later, the GPU
/// is done with the buffer and it can be re-mapped without stalling.
///
/// This eliminates the per-frame staging buffer allocation that
/// `queue.write_texture()` performs internally, which can block for 2-9ms
/// under GPU saturation.
pub struct VideoStagingBuffers {
    buffers: [wgpu::Buffer; 2],
    current: usize,
    mapped: [Arc<AtomicBool>; 2],
    /// Bytes per row padded to `wgpu::COPY_BYTES_PER_ROW_ALIGNMENT` (256).
    padded_bpr: u32,
    /// Unpadded bytes per row (actual source data stride).
    unpadded_bpr: u32,
    /// Number of rows (height for RGBA, `blocks_y` for compressed).
    rows: u32,
    /// Tracks which buffers need `map_async` after the next `queue.submit()`.
    needs_remap: [bool; 2],
}

impl VideoStagingBuffers {
    /// Create a new double-buffered staging pair.
    /// Buffers start unmapped — call `request_remap()` after the first
    /// `queue.submit()` to begin the mapping lifecycle.
    pub fn new(device: &wgpu::Device, unpadded_bpr: u32, rows: u32, label: &str) -> Self {
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bpr = (unpadded_bpr + align - 1) & !(align - 1);
        let buffer_size = u64::from(padded_bpr) * u64::from(rows);

        let make_buf = |idx: usize| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{label} Staging {idx}")),
                size: buffer_size,
                usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            })
        };

        let mapped_0 = Arc::new(AtomicBool::new(false));
        let mapped_1 = Arc::new(AtomicBool::new(false));

        Self {
            buffers: [make_buf(0), make_buf(1)],
            current: 0,
            mapped: [mapped_0, mapped_1],
            padded_bpr,
            unpadded_bpr,
            rows,
            needs_remap: [true, true],
        }
    }

    /// Write frame data into the current staging buffer and encode a copy
    /// to the destination texture. Returns true if the upload was performed.
    ///
    /// # Panics
    ///
    /// Panics if a staging slot marked as mapped no longer exposes its mapped range.
    pub fn upload(
        &mut self,
        data: &[u8],
        texture: &wgpu::Texture,
        width: u32,
        height: u32,
        encoder: &mut wgpu::CommandEncoder,
    ) -> bool {
        let idx = self.current;
        if !self.mapped[idx].load(Ordering::Acquire) {
            // Buffer not yet mapped — skip this upload.
            // The stale texture from last frame will remain on screen.
            return false;
        }

        {
            let buf = &self.buffers[idx];
            let mut view = buf
                .slice(..)
                .get_mapped_range_mut()
                .expect("upload staging buffer must remain mapped");
            if self.padded_bpr == self.unpadded_bpr {
                // Row stride matches — single memcpy
                let copy_len = (self.unpadded_bpr as usize) * (self.rows as usize);
                view.slice(..copy_len).copy_from_slice(&data[..copy_len]);
            } else {
                // Need to copy row-by-row with padding
                for row in 0..self.rows as usize {
                    let src_start = row * self.unpadded_bpr as usize;
                    let dst_start = row * self.padded_bpr as usize;
                    view.slice(dst_start..dst_start + self.unpadded_bpr as usize)
                        .copy_from_slice(&data[src_start..src_start + self.unpadded_bpr as usize]);
                }
            }
        }

        self.buffers[idx].unmap();
        self.mapped[idx].store(false, Ordering::Release);

        encoder.copy_buffer_to_texture(
            wgpu::TexelCopyBufferInfo {
                buffer: &self.buffers[idx],
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.padded_bpr),
                    rows_per_image: Some(self.rows),
                },
            },
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );

        // Mark for re-mapping after submit
        self.needs_remap[idx] = true;

        // Advance to next buffer
        self.current = 1 - self.current;
        true
    }

    /// Request re-mapping of any buffers that were used since the last call.
    /// **Must be called AFTER `queue.submit()`** — calling `map_async` before
    /// submit can complete synchronously on UMA/Metal, leaving the buffer
    /// mapped during submit (which is a validation error).
    pub fn request_remap(&mut self) {
        for i in 0..2 {
            if self.needs_remap[i] {
                self.needs_remap[i] = false;
                let flag = self.mapped[i].clone();
                self.buffers[i]
                    .slice(..)
                    .map_async(wgpu::MapMode::Write, move |result| {
                        if result.is_ok() {
                            flag.store(true, Ordering::Release);
                        }
                    });
            }
        }
    }
}
