//! The `D3D11On12` bridge Spout's textures have to cross.
//!
//! Spout shares a DirectX 11 texture created with `D3D11_RESOURCE_MISC_SHARED`,
//! whose handle is a **legacy** shared handle. `ID3D12Device::OpenSharedHandle`
//! takes NT handles only, so wgpu's D3D12 device cannot open one, and the Syphon
//! approach of wrapping the shared primitive directly is unavailable.
//!
//! What works is the route Spout's own DX12 support uses: a `D3D11On12` device
//! built on wgpu's existing device and queue. One device, one bridge, one GPU
//! copy in each direction, and no CPU readback. That is not zero-copy and is not
//! described as such.
//!
//! Verified to work end to end on a GitHub Actions runner whose only adapter is
//! `Microsoft Basic Render Driver`, which is WARP. See
//! [`super::capability`] and /spec/spout-output.md.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_RESOURCE_MISC_SHARED, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11DeviceContext, ID3D11Resource, ID3D11Texture2D,
};
use windows::Win32::Graphics::Direct3D11on12::{
    D3D11_RESOURCE_FLAGS, D3D11On12CreateDevice, ID3D11On12Device,
};
use windows::Win32::Graphics::Direct3D12::{
    D3D12_RESOURCE_STATE_COMMON, ID3D12CommandQueue, ID3D12Device,
};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource;
use windows::core::Interface;

use super::protocol::DxgiFormat;

/// wgpu's D3D12 device with a `D3D11On12` layer over it.
///
/// Render-thread only, like every other graphics handle Varda owns.
pub struct Bridge {
    d3d11: ID3D11Device,
    context: ID3D11DeviceContext,
    on12: ID3D11On12Device,
}

impl Bridge {
    /// Build the bridge on wgpu's own device and queue.
    ///
    /// `None` when wgpu is not on the Dx12 backend. Varda builds its instance
    /// with `Backends::all()`, so a machine that prefers Vulkan is a legitimate
    /// configuration in which Spout simply cannot run, and the manager reports
    /// unavailable rather than failing.
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Option<Self> {
        let (d3d12, queue) = raw_handles(device)?;
        Self::from_raw(&d3d12, &queue)
    }

    fn from_raw(d3d12: &ID3D12Device, queue: &ID3D12CommandQueue) -> Option<Self> {
        let queues: [Option<windows::core::IUnknown>; 1] = [Some(queue.cast().ok()?)];
        let mut d3d11 = None;
        let mut context = None;
        unsafe {
            D3D11On12CreateDevice(
                d3d12,
                D3D11_CREATE_DEVICE_BGRA_SUPPORT.0,
                None,
                Some(&queues),
                0,
                Some(std::ptr::from_mut(&mut d3d11)),
                Some(std::ptr::from_mut(&mut context)),
                None,
            )
        }
        .ok()?;
        let d3d11 = d3d11?;
        let on12 = d3d11.cast().ok()?;
        Some(Self {
            d3d11,
            context: context?,
            on12,
        })
    }

    /// Copy a sender's shared texture into a texture Varda owns.
    ///
    /// `handle` is the 32-bit legacy handle read from the sender's shared-memory
    /// record. The destination is a wgpu texture, wrapped for D3D11 so the copy
    /// happens on one device.
    ///
    /// Returns whether the copy was issued. A sender that vanished between the
    /// registry read and this call is an ordinary race, not an error.
    pub fn copy_from_sender(&self, handle: u32, destination: &wgpu::Texture) -> bool {
        let mut source: Option<ID3D11Texture2D> = None;
        if unsafe {
            self.d3d11.OpenSharedResource::<ID3D11Texture2D>(
                HANDLE(std::ptr::without_provenance_mut(handle as usize)),
                std::ptr::from_mut(&mut source),
            )
        }
        .is_err()
        {
            return false;
        }
        let Some(source) = source else { return false };
        let Some(wrapped) = self.wrap(destination) else {
            return false;
        };
        self.copy(&wrapped, &source);
        true
    }

    /// Copy a texture Varda owns into the shared texture a receiver reads.
    ///
    /// The reverse of [`Self::copy_from_sender`], and the only other thing the
    /// bridge does.
    pub fn copy_to_shared(&self, source: &wgpu::Texture, shared: &ID3D11Texture2D) -> bool {
        let Some(wrapped) = self.wrap(source) else {
            return false;
        };
        self.copy(shared, &wrapped);
        true
    }

    /// Present a wgpu texture to D3D11 as a wrapped resource.
    ///
    /// `COMMON` for both states rather than a specific one: wgpu owns this
    /// texture's state tracking, and D3D12 promotes out of `COMMON` implicitly
    /// for the copies here, so handing it back in the state it was lent avoids
    /// telling wgpu's tracker something untrue.
    fn wrap(&self, texture: &wgpu::Texture) -> Option<ID3D11Texture2D> {
        let raw = unsafe {
            texture
                .as_hal::<wgpu::hal::api::Dx12>()
                .map(|t| t.raw_resource().clone())
        }?;
        let flags = D3D11_RESOURCE_FLAGS {
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            MiscFlags: 0,
            CPUAccessFlags: 0,
            StructureByteStride: 0,
        };
        let mut wrapped: Option<ID3D11Texture2D> = None;
        unsafe {
            self.on12.CreateWrappedResource(
                &raw,
                std::ptr::from_ref(&flags),
                D3D12_RESOURCE_STATE_COMMON,
                D3D12_RESOURCE_STATE_COMMON,
                std::ptr::from_mut(&mut wrapped),
            )
        }
        .ok()?;
        wrapped
    }

    /// Copy on the D3D11 side, bracketed by the acquire and release the wrapped
    /// resource requires, and flushed.
    ///
    /// The flush is not optional: Spout's own notes record that a shared texture
    /// updated on one device needs `Flush` before another device sees it, and
    /// omitting it produces a receiver that shows a stale frame rather than an
    /// error.
    fn copy(&self, destination: &ID3D11Texture2D, source: &ID3D11Texture2D) {
        let wrapped: [Option<ID3D11Resource>; 1] = [destination.cast().ok()];
        unsafe {
            self.on12.AcquireWrappedResources(&wrapped);
            self.context.CopyResource(destination, source);
            self.on12.ReleaseWrappedResources(&wrapped);
            self.context.Flush();
        }
    }

    /// Create the shared texture a Spout receiver will open.
    ///
    /// Legacy `MISC_SHARED` rather than an NT handle, because that is what every
    /// existing Spout receiver expects and what the 32-bit `shareHandle` field in
    /// the wire protocol can carry.
    #[must_use]
    pub fn create_shared(
        &self,
        width: u32,
        height: u32,
        format: DxgiFormat,
    ) -> Option<(ID3D11Texture2D, u32)> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: width,
            Height: height,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT(format.as_u32().cast_signed()),
            SampleDesc: DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
            CPUAccessFlags: 0,
            MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
        };
        let mut texture: Option<ID3D11Texture2D> = None;
        unsafe {
            self.d3d11.CreateTexture2D(
                std::ptr::from_ref(&desc),
                None,
                Some(std::ptr::from_mut(&mut texture)),
            )
        }
        .ok()?;
        let texture = texture?;
        let resource: IDXGIResource = texture.cast().ok()?;
        let handle = unsafe { resource.GetSharedHandle() }.ok()?;
        // Truncation is the protocol, not a bug: Spout's record carries this as a
        // u32 and legacy handles fit by construction.
        Some((texture, handle.0 as usize as u32))
    }
}

/// wgpu's raw D3D12 device and queue, or `None` off the Dx12 backend.
fn raw_handles(device: &wgpu::Device) -> Option<(ID3D12Device, ID3D12CommandQueue)> {
    unsafe {
        device
            .as_hal::<wgpu::hal::api::Dx12>()
            .map(|d| (d.raw_device().clone(), d.raw_queue().clone()))
    }
}
