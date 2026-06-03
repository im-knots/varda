//! GPU readback — async texture-to-CPU transfer with double buffering.
//!
//! Used by headless outputs (NDI send, Syphon server, recording) to read
//! rendered frames back to the CPU without stalling the GPU pipeline.
//! One frame of latency: we copy to buffer N, then map buffer N-1.

/// Double-buffered GPU→CPU readback. Alternates two staging buffers
/// so the GPU copy and CPU map never contend on the same buffer.
pub struct ReadbackBuffer {
    buffers: [wgpu::Buffer; 2],
    /// Which buffer to write to this frame (0 or 1)
    write_idx: usize,
    /// Width of the source texture
    width: u32,
    /// Height of the source texture
    height: u32,
    /// Bytes per row (aligned to wgpu requirements)
    padded_bytes_per_row: u32,
    /// Whether we've done at least one copy (so the read buffer has valid data)
    has_previous: bool,
    /// Track which buffers may be mapped so we can unmap before reuse.
    /// [buffer_a_mapped, buffer_b_mapped]
    mapped: [bool; 2],
}

impl ReadbackBuffer {
    /// Create a new ReadbackBuffer for a given resolution.
    pub fn new(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let unpadded_bytes_per_row = width * 4; // RGBA8
                                                // wgpu requires COPY_BYTES_PER_ROW_ALIGNMENT (256) alignment
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded_bytes_per_row = (unpadded_bytes_per_row + align - 1) / align * align;
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
            write_idx: 0,
            width,
            height,
            padded_bytes_per_row,
            has_previous: false,
            mapped: [false; 2],
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
    pub fn begin_readback(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        source_texture: &wgpu::Texture,
    ) {
        // Defensively unmap if a previous map_async completed or timed out
        if self.mapped[self.write_idx] {
            self.buffers[self.write_idx].unmap();
            self.mapped[self.write_idx] = false;
        }
        let buffer = &self.buffers[self.write_idx];

        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: source_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer,
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

        // Swap: next frame we write to the other buffer
        self.write_idx = 1 - self.write_idx;
        self.has_previous = true;
    }

    /// Try to read the previous frame's data. Returns RGBA bytes (tightly packed, no padding)
    /// or None if no previous frame is available yet.
    ///
    /// This maps the buffer synchronously with `poll(Wait)`. For the output thread
    /// this is acceptable since we're 1 frame behind.
    pub fn try_read(&mut self, device: &wgpu::Device) -> Option<Vec<u8>> {
        if !self.has_previous {
            return None;
        }

        // Read from the buffer we're NOT currently writing to
        let read_idx = self.write_idx; // after swap, write_idx points to what was previously read
        let buffer = &self.buffers[read_idx];

        // If already mapped from a previous timed-out attempt, unmap first
        if self.mapped[read_idx] {
            buffer.unmap();
            self.mapped[read_idx] = false;
        }

        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |result| {
            let _ = tx.send(result);
        });
        // Mark mapped so we can clean up on timeout or next call
        self.mapped[read_idx] = true;

        let poll_start = std::time::Instant::now();
        loop {
            match device.poll(wgpu::PollType::Poll) {
                Ok(status) if status.is_queue_empty() => break,
                Err(e) => {
                    log::warn!("GPU poll error during readback: {}", e);
                    // Buffer may or may not have completed mapping; leave mapped flag
                    // set so begin_readback or next try_read will unmap defensively.
                    return None;
                }
                _ => {}
            }
            if poll_start.elapsed() > std::time::Duration::from_millis(16) {
                log::warn!("GPU readback timeout, skipping frame");
                // map_async callback may fire later; mapped flag stays true
                // so begin_readback will unmap before reusing this buffer.
                return None;
            }
            std::thread::yield_now();
        }

        match rx.recv() {
            Ok(Ok(())) => {
                let data = slice.get_mapped_range();
                let unpadded_bytes_per_row = (self.width * 4) as usize;
                let padded = self.padded_bytes_per_row as usize;

                // Strip row padding if needed
                let result = if padded == unpadded_bytes_per_row {
                    data.to_vec()
                } else {
                    let mut out = Vec::with_capacity(unpadded_bytes_per_row * self.height as usize);
                    for row in 0..self.height as usize {
                        let start = row * padded;
                        let end = start + unpadded_bytes_per_row;
                        out.extend_from_slice(&data[start..end]);
                    }
                    out
                };

                drop(data);
                buffer.unmap();
                self.mapped[read_idx] = false;
                Some(result)
            }
            _ => {
                // Map failed — unmap defensively
                buffer.unmap();
                self.mapped[read_idx] = false;
                None
            }
        }
    }
}
