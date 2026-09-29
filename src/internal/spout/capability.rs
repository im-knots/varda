//! Probes which of Spout's sharing primitives work on this machine.
//!
//! Microsoft's docs contradict themselves on whether WARP supports shared
//! resources, and CI runners only have WARP, so each step is measured and
//! reported.

use windows::Win32::Foundation::HANDLE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BIND_RENDER_TARGET, D3D11_BIND_SHADER_RESOURCE, D3D11_CREATE_DEVICE_BGRA_SUPPORT,
    D3D11_RESOURCE_MISC_SHARED, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, ID3D11Device,
    ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Direct3D11on12::{
    D3D11_RESOURCE_FLAGS, D3D11On12CreateDevice, ID3D11On12Device,
};
use windows::Win32::Graphics::Direct3D12::D3D12_RESOURCE_STATE_RENDER_TARGET;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIResource;
use windows::core::Interface;

/// One measured step. `NotAttempted` means an earlier step stopped the probe.
#[derive(Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    NotAttempted,
    Worked,
    Failed(String),
}

impl Step {
    fn of(r: windows::core::Result<()>) -> Self {
        match r {
            Ok(()) => Self::Worked,
            Err(e) => Self::Failed(format!("{e:?}")),
        }
    }
}

#[derive(Debug, Default)]
pub struct Report {
    pub backend: String,
    pub adapter: String,
    pub is_dx12: bool,
    pub d3d11on12: Step,
    pub shared_texture: Step,
    pub share_handle: Step,
    pub reopen: Step,
    pub wrap_d3d12: Step,
}

fn failed(e: &windows::core::Error) -> Step {
    Step::Failed(format!("{e:?}"))
}

/// Runs every step, stopping at the first failure. Never panics; failures are
/// recorded in the result.
#[must_use]
pub fn probe(context: &crate::renderer::context::GpuContext) -> Report {
    let (device, adapter) = (&context.device, &context.adapter);
    let info = adapter.get_info();
    let mut r = Report {
        backend: format!("{:?}", info.backend),
        adapter: info.name.clone(),
        is_dx12: info.backend == wgpu::Backend::Dx12,
        ..Report::default()
    };
    if !r.is_dx12 {
        return r;
    }
    let Some((d3d12, queue)) = (unsafe {
        device
            .as_hal::<wgpu::hal::api::Dx12>()
            .map(|d| (d.raw_device().clone(), d.raw_queue().clone()))
    }) else {
        r.d3d11on12 = Step::Failed("as_hal::<Dx12> returned None".into());
        return r;
    };

    let mut d3d11: Option<ID3D11Device> = None;
    let mut ctx: Option<ID3D11DeviceContext> = None;
    let queues: [Option<windows::core::IUnknown>; 1] = match queue.cast() {
        Ok(q) => [Some(q)],
        Err(e) => {
            r.d3d11on12 = failed(&e);
            return r;
        }
    };
    if let Err(e) = unsafe {
        D3D11On12CreateDevice(
            &d3d12,
            D3D11_CREATE_DEVICE_BGRA_SUPPORT.0,
            None,
            Some(&queues),
            0,
            Some(std::ptr::from_mut(&mut d3d11)),
            Some(std::ptr::from_mut(&mut ctx)),
            None,
        )
    } {
        r.d3d11on12 = failed(&e);
        return r;
    }
    let Some(d3d11) = d3d11 else {
        r.d3d11on12 = Step::Failed("D3D11On12CreateDevice returned no device".into());
        return r;
    };
    r.d3d11on12 = Step::Worked;

    // A legacy MISC_SHARED texture; WARP's docs are ambiguous about support.
    let desc = D3D11_TEXTURE2D_DESC {
        Width: 64,
        Height: 64,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
        CPUAccessFlags: 0,
        MiscFlags: D3D11_RESOURCE_MISC_SHARED.0 as u32,
    };
    let mut shared: Option<ID3D11Texture2D> = None;
    if let Err(e) = unsafe {
        d3d11.CreateTexture2D(
            std::ptr::from_ref(&desc),
            None,
            Some(std::ptr::from_mut(&mut shared)),
        )
    } {
        r.shared_texture = failed(&e);
        return r;
    }
    // Recorded rather than unwrapped: a device can return success with no texture.
    let Some(shared) = shared else {
        r.shared_texture = Step::Failed("CreateTexture2D succeeded with no texture".into());
        return r;
    };
    r.shared_texture = Step::Worked;

    let handle = match shared
        .cast::<IDXGIResource>()
        .and_then(|res| unsafe { res.GetSharedHandle() })
    {
        Ok(h) => {
            r.share_handle = Step::Worked;
            h
        }
        Err(e) => {
            r.share_handle = failed(&e);
            return r;
        }
    };

    let mut reopened: Option<ID3D11Texture2D> = None;
    r.reopen = Step::of(unsafe {
        d3d11.OpenSharedResource::<ID3D11Texture2D>(
            HANDLE(handle.0),
            std::ptr::from_mut(&mut reopened),
        )
    });

    // Send direction: wrap one of our own D3D12 textures for D3D11.
    let on12: ID3D11On12Device = match d3d11.cast() {
        Ok(v) => v,
        Err(e) => {
            r.wrap_d3d12 = failed(&e);
            return r;
        }
    };
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("probe"),
        size: wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Bgra8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let raw = unsafe {
        tex.as_hal::<wgpu::hal::api::Dx12>()
            .map(|t| t.raw_resource().clone())
    };
    let Some(raw) = raw else {
        r.wrap_d3d12 = Step::Failed("Texture::as_hal returned None".into());
        return r;
    };
    let flags = D3D11_RESOURCE_FLAGS {
        BindFlags: (D3D11_BIND_SHADER_RESOURCE.0 | D3D11_BIND_RENDER_TARGET.0) as u32,
        MiscFlags: 0,
        CPUAccessFlags: 0,
        StructureByteStride: 0,
    };
    let mut wrapped: Option<ID3D11Texture2D> = None;
    r.wrap_d3d12 = Step::of(unsafe {
        on12.CreateWrappedResource(
            &raw,
            std::ptr::from_ref(&flags),
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            D3D12_RESOURCE_STATE_RENDER_TARGET,
            std::ptr::from_mut(&mut wrapped),
        )
    });
    let _ = (ctx, reopened, wrapped);
    r
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "adapter        : {} ({})", self.adapter, self.backend)?;
        if !self.is_dx12 {
            return writeln!(
                f,
                "Spout needs the Dx12 backend; this adapter cannot host it."
            );
        }
        writeln!(f, "D3D11On12      : {:?}", self.d3d11on12)?;
        writeln!(f, "shared texture : {:?}", self.shared_texture)?;
        writeln!(f, "share handle   : {:?}", self.share_handle)?;
        writeln!(f, "reopen shared  : {:?}", self.reopen)?;
        writeln!(f, "wrap D3D12     : {:?}", self.wrap_d3d12)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every step of the bridge Spout needs, asserted.
    ///
    /// Result on a GitHub Actions `windows-latest` runner (WARP):
    ///
    /// ```text
    /// adapter        : Microsoft Basic Render Driver (Dx12)
    /// D3D11On12      : Worked
    /// shared texture : Worked
    /// share handle   : Worked
    /// reopen shared  : Worked
    /// wrap D3D12     : Worked
    /// ```
    ///
    /// The report is printed for diagnosing failures; run with `--nocapture`.
    #[test]
    fn the_d3d11on12_bridge_works_on_this_machine() {
        let Some(context) = crate::testing::headless_gpu() else {
            eprintln!("no GPU adapter, skipping Spout capability probe");
            return;
        };
        let report = probe(&context);
        eprintln!("\n=== Spout capability ===\n{report}");

        // Only asserted on Dx12. wgpu may pick Vulkan, where Spout reports
        // unavailable.
        if !report.is_dx12 {
            eprintln!(
                "backend is {}, not Dx12; Spout cannot run here",
                report.backend
            );
            return;
        }

        assert_eq!(report.d3d11on12, Step::Worked, "D3D11On12 bridge");
        assert_eq!(
            report.shared_texture,
            Step::Worked,
            "shared texture creation"
        );
        assert_eq!(report.share_handle, Step::Worked, "legacy share handle");
        assert_eq!(
            report.reopen,
            Step::Worked,
            "reopening a shared handle (receive)"
        );
        assert_eq!(
            report.wrap_d3d12,
            Step::Worked,
            "wrapping a D3D12 texture (send)"
        );
    }
}
