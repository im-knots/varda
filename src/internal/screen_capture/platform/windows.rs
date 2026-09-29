//! Windows screen and window capture via Windows Graphics Capture (WGC).
//!
//! Push-based like `ScreenCaptureKit`: a `Direct3D11CaptureFramePool` raises
//! `FrameArrived` on a thread-pool thread, the handler copies the frame into
//! tightly packed BGRA in a shared slot, and [`WindowsBackend::next_frame`]
//! takes it. The capture thread never blocks on the OS, and a stalled stream
//! yields `None`.
//!
//! Differences from macOS:
//!
//! - The pool has no rate control and delivers at the display refresh rate.
//!   `CaptureConfig.rate` is enforced in the arrival handler before the
//!   GPU-to-CPU copy, so 30 fps on a 144 Hz display costs 30 readbacks a second.
//! - No capture-time scaling. Crop is a smaller `CopySubresourceRegion`, but
//!   `scale_to` needs a CPU downsample; see [`downscale`].
//!
//! Frames are BGRA8 uploaded to a `Bgra8UnormSrgb` texture with no CPU
//! swizzle, through CPU readback.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11_BOX, D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GWL_EXSTYLE, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
};
use windows::core::{BOOL, Interface};

use crate::screen_capture::resample::{Geometry, downscale};

use crate::screen_capture::backend::{
    CaptureConfig, CaptureError, CaptureFrame, CapturePixelFormat, CaptureTargetInfo,
    CaptureTargetKind, PermissionState, ScreenCaptureBackend,
};

/// Frame pool buffers. Same as the macOS `queueDepth`: absorbs compositor
/// jitter without adding latency.
const FRAME_POOL_BUFFERS: i32 = 3;

/// Size threshold below which windows are helpers (like menu-bar extras) and
/// are skipped.
const MIN_WINDOW_EDGE: i32 = 32;

pub fn backend_name() -> &'static str {
    "WindowsGraphicsCapture"
}

pub fn permission_state() -> PermissionState {
    // Windows has no capture permission. WGC draws a yellow border around the
    // target on most builds; it cannot be turned off without a packaged-app
    // identity.
    PermissionState::NotRequired
}

pub fn request_permission() {}

/// Enumerates monitors and top-level windows.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if this Windows build lacks Graphics
/// Capture (pre-1903), so an empty list is not mistaken for "no displays".
pub fn enumerate() -> Result<Vec<CaptureTargetInfo>, CaptureError> {
    ensure_supported()?;
    let mut targets = enumerate_monitors();
    targets.extend(enumerate_windows());
    Ok(targets)
}

/// Opens a capture session for `target`.
///
/// # Errors
///
/// Returns [`CaptureError::Unavailable`] on a build without WGC,
/// [`CaptureError::TargetNotFound`] if the monitor or window is gone, or
/// [`CaptureError::Backend`] for any D3D11 or `WinRT` failure.
pub fn open(
    target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    ensure_supported()?;
    Ok(Box::new(WindowsBackend::new(target, config)?))
}

fn ensure_supported() -> Result<(), CaptureError> {
    // Keeps the process in the MTA for its lifetime. WGC's free-threaded frame
    // pool needs an initialized apartment, and this avoids setting a threading
    // model on threads Varda does not own.
    static MTA: OnceLock<()> = OnceLock::new();
    MTA.get_or_init(|| unsafe {
        let _ = CoIncrementMTAUsage();
    });

    match GraphicsCaptureSession::IsSupported() {
        Ok(true) => Ok(()),
        Ok(false) => Err(CaptureError::Unavailable(
            "Windows Graphics Capture is not available on this build of Windows (needs 1903+)"
                .into(),
        )),
        Err(e) => Err(CaptureError::Backend(format!(
            "GraphicsCaptureSession::IsSupported failed: {e}"
        ))),
    }
}

// ── Enumeration ─────────────────────────────────────────────────────

/// Filled by the `EnumDisplayMonitors` / `EnumWindows` callbacks, which take
/// only a raw pointer across FFI.
struct Collector {
    targets: Vec<CaptureTargetInfo>,
    our_pid: u32,
}

fn enumerate_monitors() -> Vec<CaptureTargetInfo> {
    let mut collector = Collector {
        targets: Vec::new(),
        our_pid: unsafe { GetCurrentProcessId() },
    };
    let lparam = LPARAM(std::ptr::from_mut(&mut collector) as isize);
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(monitor_proc), lparam);
    }
    // Enumeration order changes on hot-plug, so number entries as they arrive
    // and match identity on the label, as on macOS.
    for (i, t) in collector.targets.iter_mut().enumerate() {
        t.label = format!("Display {}", i + 1);
    }
    collector.targets
}

unsafe extern "system" fn monitor_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _clip: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let collector = unsafe { &mut *(lparam.0 as *mut Collector) };
    let mut info = MONITORINFOEXW {
        monitorInfo: MONITORINFO {
            cbSize: u32::try_from(std::mem::size_of::<MONITORINFOEXW>()).unwrap_or(0),
            ..Default::default()
        },
        ..Default::default()
    };
    if unsafe { GetMonitorInfoW(monitor, std::ptr::from_mut(&mut info).cast()) }.as_bool() {
        let r = info.monitorInfo.rcMonitor;
        collector.targets.push(CaptureTargetInfo {
            kind: CaptureTargetKind::Display,
            platform_id: monitor.0 as u64,
            label: String::new(),
            app: None,
            title: None,
            width: (r.right - r.left).max(0) as u32,
            height: (r.bottom - r.top).max(0) as u32,
            is_varda: false,
        });
    }
    BOOL::from(true)
}

fn enumerate_windows() -> Vec<CaptureTargetInfo> {
    let mut collector = Collector {
        targets: Vec::new(),
        our_pid: unsafe { GetCurrentProcessId() },
    };
    let lparam = LPARAM(std::ptr::from_mut(&mut collector) as isize);
    unsafe {
        let _ = EnumWindows(Some(window_proc), lparam);
    }
    collector.targets
}

unsafe extern "system" fn window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let collector = unsafe { &mut *(lparam.0 as *mut Collector) };
    if let Some(target) = unsafe { describe_window(hwnd, collector.our_pid) } {
        collector.targets.push(target);
    }
    BOOL::from(true)
}

/// Builds a target for `hwnd`, or `None` if it is not a user-facing window.
unsafe fn describe_window(hwnd: HWND, our_pid: u32) -> Option<CaptureTargetInfo> {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }
    // Skip tool windows (palettes, tooltips).
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return None;
    }
    // Cloaked windows pass every legacy visibility test but render nothing
    // (suspended UWP apps, other virtual desktops). They would capture black.
    if unsafe { is_cloaked(hwnd) } {
        return None;
    }

    let mut rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut rect) }.is_err() {
        return None;
    }
    let (width, height) = (rect.right - rect.left, rect.bottom - rect.top);
    if width < MIN_WINDOW_EDGE || height < MIN_WINDOW_EDGE {
        return None;
    }

    let title = unsafe { window_title(hwnd) };
    if title.is_empty() {
        return None;
    }

    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&raw mut pid)) };
    let app = unsafe { process_name(pid) };

    let label = match &app {
        Some(app) => format!("{app} — {title}"),
        None => title.clone(),
    };

    Some(CaptureTargetInfo {
        kind: CaptureTargetKind::Window,
        platform_id: hwnd.0 as u64,
        label,
        // The executable name stands in for a bundle id: it survives a retitle,
        // and persistence matches on it.
        app,
        title: Some(title),
        width: width as u32,
        height: height as u32,
        is_varda: pid == our_pid,
    })
}

unsafe fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked = 0u32;
    let ok = unsafe {
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            std::ptr::from_mut(&mut cloaked).cast(),
            u32::try_from(std::mem::size_of::<u32>()).unwrap_or(4),
        )
    };
    ok.is_ok() && cloaked != 0
}

unsafe fn window_title(hwnd: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(hwnd) };
    if len <= 0 {
        return String::new();
    }
    let mut buf = vec![0u16; (len as usize) + 1];
    let written = unsafe { GetWindowTextW(hwnd, &mut buf) };
    if written <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..written as usize])
}

/// Executable file stem for `pid`, e.g. `firefox`. `None` when the process
/// cannot be queried, which is normal for elevated and system processes.
unsafe fn process_name(pid: u32) -> Option<String> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buf = [0u16; 260];
    let mut len = u32::try_from(buf.len()).ok()?;
    let ok = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            windows::core::PWSTR(buf.as_mut_ptr()),
            &raw mut len,
        )
    };
    // The handle is owned here and held nowhere else.
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
    ok.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
}

// ── Capture session ─────────────────────────────────────────────────

/// Latest-wins frame slot shared by the WGC thread pool and the capture
/// thread.
type FrameSlot = Arc<Mutex<Option<CaptureFrame>>>;

/// State for the arrival handler, in one allocation so the closure holds a
/// single `Arc`.
struct ArrivalState {
    slot: FrameSlot,
    /// Live geometry: `(crop_box, output_width, output_height)`. `set_config`
    /// replaces it as a whole so the handler never copies a region that
    /// disagrees with the size it reports.
    geometry: Mutex<Geometry>,
    /// Rate gate. Kept separate from the config so discarded frames skip the
    /// geometry lock.
    min_interval: Mutex<Duration>,
    last_delivered: Mutex<Option<Instant>>,
    /// Set when the pool is stopping, so an in-flight frame does not revive the
    /// session.
    stopped: Arc<AtomicBool>,
}

/// The COM/WinRT objects behind a live capture.
///
/// In the MTA none of them is apartment-bound. The `windows` crate marks them
/// `!Send` because an STA caller could pin them; here they are created on the
/// calling thread and moved to one capture thread, which owns them alone.
struct SessionHandles {
    _item: GraphicsCaptureItem,
    _device: ID3D11Device,
    _context: ID3D11DeviceContext,
    frame_pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

// SAFETY: MTA objects moved to a single owner; see `SessionHandles`.
unsafe impl Send for SessionHandles {}

pub struct WindowsBackend {
    label: String,
    handles: SessionHandles,
    state: Arc<ArrivalState>,
    stopped: Arc<AtomicBool>,
    native_w: u32,
    native_h: u32,
    width: u32,
    height: u32,
    config: CaptureConfig,
}

impl WindowsBackend {
    fn new(target: &CaptureTargetInfo, config: &CaptureConfig) -> Result<Self, CaptureError> {
        let item = capture_item(target)?;
        let size = item
            .Size()
            .map_err(|e| CaptureError::Backend(format!("capture item size unavailable: {e}")))?;
        let native_w = u32::try_from(size.Width).unwrap_or(1).max(1);
        let native_h = u32::try_from(size.Height).unwrap_or(1).max(1);
        let geometry = Geometry::resolve(native_w, native_h, config);

        let (device, context) = create_d3d_device()?;
        let winrt_device = winrt_device(&device)?;

        let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &winrt_device,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            FRAME_POOL_BUFFERS,
            size,
        )
        .map_err(|e| CaptureError::Backend(format!("frame pool creation failed: {e}")))?;

        let stopped = Arc::new(AtomicBool::new(false));
        let state = Arc::new(ArrivalState {
            slot: Arc::new(Mutex::new(None)),
            geometry: Mutex::new(geometry),
            min_interval: Mutex::new(config.frame_interval()),
            last_delivered: Mutex::new(None),
            stopped: Arc::clone(&stopped),
        });

        let handler_state = Arc::clone(&state);
        let handler_context = context.clone();
        frame_pool
            .FrameArrived(&TypedEventHandler::new(
                move |pool: windows::core::Ref<'_, Direct3D11CaptureFramePool>, _| {
                    if let Some(pool) = pool.as_ref() {
                        on_frame_arrived(pool, &handler_context, &handler_state);
                    }
                    Ok(())
                },
            ))
            .map_err(|e| CaptureError::Backend(format!("FrameArrived subscription failed: {e}")))?;

        let session = frame_pool
            .CreateCaptureSession(&item)
            .map_err(|e| CaptureError::Backend(format!("capture session creation failed: {e}")))?;
        // Both need 1903 or later and throw on older builds, where cursor and
        // border cannot be changed.
        let _ = session.SetIsCursorCaptureEnabled(config.show_cursor);
        let _ = session.SetIsBorderRequired(false);
        session
            .StartCapture()
            .map_err(|e| CaptureError::Backend(format!("StartCapture failed: {e}")))?;

        log::info!(
            "Windows Graphics Capture started for '{}' at {}x{}",
            target.label,
            geometry.out_w,
            geometry.out_h
        );

        Ok(Self {
            label: target.label.clone(),
            handles: SessionHandles {
                _item: item,
                _device: device,
                _context: context,
                frame_pool,
                session,
            },
            state,
            stopped,
            native_w,
            native_h,
            width: geometry.out_w,
            height: geometry.out_h,
            config: config.clone(),
        })
    }
}

/// Resolves a target to a live monitor or window and wraps it in a
/// `GraphicsCaptureItem`.
fn capture_item(target: &CaptureTargetInfo) -> Result<GraphicsCaptureItem, CaptureError> {
    let interop: IGraphicsCaptureItemInterop =
        windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().map_err(
            |e| CaptureError::Backend(format!("GraphicsCaptureItem interop unavailable: {e}")),
        )?;
    let handle = usize::try_from(target.platform_id).unwrap_or(0) as *mut core::ffi::c_void;
    let item = match target.kind {
        CaptureTargetKind::Display => unsafe { interop.CreateForMonitor(HMONITOR(handle)) },
        CaptureTargetKind::Window => unsafe { interop.CreateForWindow(HWND(handle)) },
    };
    item.map_err(|_| CaptureError::TargetNotFound(target.label.clone()))
}

fn create_d3d_device() -> Result<(ID3D11Device, ID3D11DeviceContext), CaptureError> {
    // WARP is the fallback when the session has no hardware device (RDP,
    // some VMs). Capture works, slower.
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                // WGC surfaces are BGRA, and the runtime rejects devices created
                // without BGRA support.
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                Some(&raw mut context),
            )
        };
        if hr.is_ok()
            && let (Some(device), Some(context)) = (device, context)
        {
            return Ok((device, context));
        }
    }
    Err(CaptureError::Backend(
        "no Direct3D 11 device available (tried hardware and WARP)".into(),
    ))
}

fn winrt_device(device: &ID3D11Device) -> Result<IDirect3DDevice, CaptureError> {
    let dxgi: IDXGIDevice = device
        .cast()
        .map_err(|e| CaptureError::Backend(format!("device is not a DXGI device: {e}")))?;
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
        .map_err(|e| CaptureError::Backend(format!("WinRT device wrap failed: {e}")))?;
    inspectable
        .cast()
        .map_err(|e| CaptureError::Backend(format!("WinRT device cast failed: {e}")))
}

/// `FrameArrived` handler, on a WGC thread-pool thread.
///
/// Checks the rate gate first so a discarded frame costs only a
/// `TryRecycle`; otherwise 30 fps on a 144 Hz display would still pay for 144
/// GPU-to-CPU copies a second.
fn on_frame_arrived(
    pool: &Direct3D11CaptureFramePool,
    context: &ID3D11DeviceContext,
    state: &ArrivalState,
) {
    let Ok(frame) = pool.TryGetNextFrame() else {
        return;
    };
    if state.stopped.load(Ordering::Relaxed) {
        return;
    }

    {
        let Ok(min_interval) = state.min_interval.lock() else {
            return;
        };
        let Ok(mut last) = state.last_delivered.lock() else {
            return;
        };
        let now = Instant::now();
        if let Some(prev) = *last
            && now.duration_since(prev) < *min_interval
        {
            return;
        }
        *last = Some(now);
    }

    let Ok(geometry) = state.geometry.lock().map(|g| *g) else {
        return;
    };
    let Ok(surface) = frame.Surface() else {
        return;
    };
    let Ok(access) = surface.cast::<IDirect3DDxgiInterfaceAccess>() else {
        return;
    };
    let Ok(texture) = (unsafe { access.GetInterface::<ID3D11Texture2D>() }) else {
        return;
    };

    if let Some(captured) = read_back(context, &texture, geometry)
        && let Ok(mut slot) = state.slot.lock()
    {
        *slot = Some(captured);
    }
}

/// Copies the cropped region of a GPU texture into a tightly packed BGRA
/// frame.
///
/// Returns `None` on any D3D failure; dropping a frame beats a torn one.
fn read_back(
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    geometry: Geometry,
) -> Option<CaptureFrame> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&raw mut desc) };

    // The frame has the target's current size, which may differ from the size
    // the crop was computed for if the window was resized. Clamp instead of
    // reading out of bounds.
    let src_x = geometry.src_x.min(desc.Width.saturating_sub(1));
    let src_y = geometry.src_y.min(desc.Height.saturating_sub(1));
    let src_w = geometry.src_w.min(desc.Width - src_x).max(1);
    let src_h = geometry.src_h.min(desc.Height - src_y).max(1);

    let staging_desc = D3D11_TEXTURE2D_DESC {
        Width: src_w,
        Height: src_h,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_B8G8R8A8_UNORM,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };
    let mut staging: Option<ID3D11Texture2D> = None;
    let device = unsafe { texture.GetDevice() }.ok()?;
    unsafe { device.CreateTexture2D(&raw const staging_desc, None, Some(&raw mut staging)) }
        .ok()?;
    let staging = staging?;

    let region = D3D11_BOX {
        left: src_x,
        top: src_y,
        front: 0,
        right: src_x + src_w,
        bottom: src_y + src_h,
        back: 1,
    };
    unsafe {
        context.CopySubresourceRegion(&staging, 0, 0, 0, 0, texture, 0, Some(&raw const region));
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe { context.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&raw mut mapped)) }.ok()?;
    // Everything below must reach the Unmap, so no `?` past here.
    let packed = {
        let row_bytes = (src_w as usize) * 4;
        let stride = mapped.RowPitch as usize;
        if mapped.pData.is_null() || stride < row_bytes {
            None
        } else {
            let mut data = vec![0u8; row_bytes * (src_h as usize)];
            for y in 0..src_h as usize {
                let src = unsafe { mapped.pData.cast::<u8>().add(y * stride) };
                let dst = &mut data[y * row_bytes..(y + 1) * row_bytes];
                unsafe { std::ptr::copy_nonoverlapping(src, dst.as_mut_ptr(), row_bytes) };
            }
            Some(data)
        }
    };
    unsafe { context.Unmap(&staging, 0) };
    let data = packed?;

    let (data, width, height) = if (src_w, src_h) == (geometry.out_w, geometry.out_h) {
        (data, src_w, src_h)
    } else {
        (
            downscale(&data, src_w, src_h, geometry.out_w, geometry.out_h),
            geometry.out_w,
            geometry.out_h,
        )
    };

    Some(CaptureFrame {
        data,
        width,
        height,
        format: CapturePixelFormat::Bgra8UnormSrgb,
    })
}

impl ScreenCaptureBackend for WindowsBackend {
    fn label(&self) -> &str {
        &self.label
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self) -> Option<CaptureFrame> {
        let frame = self.state.slot.try_lock().ok()?.take()?;
        // A resize applies asynchronously, so trust the frame size.
        self.width = frame.width;
        self.height = frame.height;
        Some(frame)
    }

    fn pixel_format(&self) -> CapturePixelFormat {
        // The frame pool is created as `B8G8R8A8UIntNormalized`.
        CapturePixelFormat::Bgra8UnormSrgb
    }

    fn is_self_paced(&self) -> bool {
        // The arrival handler enforces `rate`, so the capture thread must
        // oversample. See `ScreenCaptureBackend::is_self_paced`.
        true
    }

    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError> {
        if *config == self.config {
            return Ok(());
        }
        let cursor_changed = config.show_cursor != self.config.show_cursor;
        self.config = config.clone();

        let geometry = Geometry::resolve(self.native_w, self.native_h, config);
        if let Ok(mut g) = self.state.geometry.lock() {
            *g = geometry;
        }
        if let Ok(mut interval) = self.state.min_interval.lock() {
            *interval = config.frame_interval();
        }
        if cursor_changed {
            let _ = self
                .handles
                .session
                .SetIsCursorCaptureEnabled(config.show_cursor);
        }
        Ok(())
    }
}

impl Drop for WindowsBackend {
    fn drop(&mut self) {
        // Set the flag first so a frame already dispatched returns without
        // touching a half-dismantled session.
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.handles.session.Close();
        let _ = self.handles.frame_pool.Close();
        log::debug!("Windows Graphics Capture stopped for '{}'", self.label);
    }
}
