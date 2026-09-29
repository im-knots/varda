//! A shader's per-pass uniforms: one slot per pass in a single buffer, so all
//! passes encode into one command buffer. Each bind group bakes in its slot's offset.

use std::cell::{Cell, RefCell};

use super::pipeline::ISFUniforms;

/// Per-pass `ISFUniforms` slots. A single-pass shader uses slot 0.
pub struct PassUniforms {
    buffer: RefCell<wgpu::Buffer>,
    slots: Cell<usize>,
    stride: u64,
}

impl PassUniforms {
    /// One slot, holding default uniforms.
    pub fn new(device: &wgpu::Device) -> Self {
        use wgpu::util::DeviceExt as _;
        let align = u64::from(device.limits().min_uniform_buffer_offset_alignment);
        let stride = (std::mem::size_of::<ISFUniforms>() as u64).div_ceil(align) * align;
        let mut contents = bytemuck::bytes_of(&ISFUniforms::default()).to_vec();
        contents.resize(stride as usize, 0);
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ISF Pass Uniforms"),
            contents: &contents,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        Self {
            buffer: RefCell::new(buffer),
            slots: Cell::new(1),
            stride,
        }
    }

    fn allocate(device: &wgpu::Device, stride: u64, slots: usize) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ISF Pass Uniforms"),
            size: stride * slots as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    /// Make room for `slots` passes. Call before writing or binding any slot
    /// this frame: growing replaces the buffer, invalidating earlier bind groups.
    pub fn ensure_slots(&self, device: &wgpu::Device, slots: usize) {
        if slots > self.slots.get() {
            *self.buffer.borrow_mut() = Self::allocate(device, self.stride, slots);
            self.slots.set(slots);
        }
    }

    /// Write `uniforms` into `slot`.
    pub fn write(&self, queue: &wgpu::Queue, slot: usize, uniforms: &ISFUniforms) {
        debug_assert!(
            slot < self.slots.get(),
            "pass uniform slot {slot} not ensured"
        );
        queue.write_buffer(
            &self.buffer.borrow(),
            slot as u64 * self.stride,
            bytemuck::cast_slice(&[*uniforms]),
        );
    }

    /// Call `bind` with the binding resource for `slot`.
    pub fn with_binding<R>(
        &self,
        slot: usize,
        bind: impl FnOnce(wgpu::BindingResource<'_>) -> R,
    ) -> R {
        debug_assert!(
            slot < self.slots.get(),
            "pass uniform slot {slot} not ensured"
        );
        let buffer = self.buffer.borrow();
        bind(wgpu::BindingResource::Buffer(wgpu::BufferBinding {
            buffer: &buffer,
            offset: slot as u64 * self.stride,
            size: std::num::NonZeroU64::new(std::mem::size_of::<ISFUniforms>() as u64),
        }))
    }
}
