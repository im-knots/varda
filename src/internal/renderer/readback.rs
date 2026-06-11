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
    /// Bytes per row (aligned to wgpu requirements)
    padded_bytes_per_row: u32,
    /// Non-blocking state machine state for each staging buffer.
    slots: [SlotState; 2],
}

impl ReadbackBuffer {
    /// Create a new ReadbackBuffer for a given resolution.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let unpadded_bytes_per_row = width * 4; // RGBA8
                                                // wgpu requires COPY_BYTES_PER_ROW_ALIGNMENT (256) alignment
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = unpadded_bytes_per_row.div_ceil(align) * align;
        let buffer_size = (padded_bytes_per_row * height) as u64;

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
        let idx = match self.slots.iter().position(|s| matches!(s, SlotState::Free)) {
            Some(i) => i,
            None => return,
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
    /// Returns RGBA bytes (tightly packed, no padding) for the most recent buffer
    /// whose map has completed, or `None` if nothing is ready yet. This never
    /// blocks the render thread: it advances pending maps with a non-blocking
    /// `poll(Poll)` and checks completion with `try_recv`, accepting a couple
    /// frames of latency. Works regardless of whether the copy was submitted in a
    /// prior frame (analyzer path) or immediately before this call (headless path).
    pub fn try_read(&mut self, device: &wgpu::Device) -> Option<Vec<u8>> {
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
                        result = Some(self.copy_out(idx));
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
        let data = self.buffers[idx].slice(..).get_mapped_range();
        let unpadded_bytes_per_row = (self.width * 4) as usize;
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
