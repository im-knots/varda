//! Helpers shared by the GPU integration test binaries, included with
//! `mod common;`.

use varda::renderer::context::GpuContext;

/// Open a headless GPU context, or `None` without a usable adapter. See
/// [`varda::testing::headless_gpu`].
///
/// # Panics
///
/// Panics if no context can be created while `VARDA_REQUIRE_GPU` is set.
pub fn headless_gpu() -> Option<GpuContext> {
    varda::testing::headless_gpu()
}

/// Read an `Rgba16Float` texture back as RGBA `f32`, row-major. Blocks on the
/// GPU.
#[allow(dead_code)] // not every test binary that includes `common` reads textures back
pub fn read_rgba16f(
    ctx: &GpuContext,
    tex: &wgpu::Texture,
    width: u32,
    height: u32,
) -> Vec<[f32; 4]> {
    let bytes_per_pixel = 8u32;
    let padded = (width * bytes_per_pixel).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = ctx.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("test readback"),
        size: u64::from(padded * height),
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut encoder = ctx
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    ctx.queue.submit(std::iter::once(encoder.finish()));
    let (tx, rx) = std::sync::mpsc::channel();
    buffer.slice(..).map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    ctx.device
        .poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        })
        .ok();
    rx.recv().expect("map channel").expect("map ok");
    let mut out = Vec::with_capacity((width * height) as usize);
    {
        let data = buffer
            .slice(..)
            .get_mapped_range()
            .expect("mapped readback");
        for row in 0..height {
            let base = (row * padded) as usize;
            for col in 0..width {
                let px = base + (col * bytes_per_pixel) as usize;
                out.push(std::array::from_fn(|i| {
                    half::f16::from_bits(u16::from_le_bytes([
                        data[px + i * 2],
                        data[px + i * 2 + 1],
                    ]))
                    .to_f32()
                }));
            }
        }
    }
    buffer.unmap();
    out
}
