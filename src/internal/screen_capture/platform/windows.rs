//! Windows screen/window capture via Windows Graphics Capture (WGC).
//!
//! WGC is push-based, like `ScreenCaptureKit`: a `Direct3D11CaptureFramePool`
//! raises `FrameArrived` on a thread-pool thread, the handler copies the frame
//! into a tightly-packed BGRA buffer and drops it into a shared slot, and
//! [`WindowsBackend::next_frame`] takes whatever is there. The capture thread
//! never blocks on the OS, and a stalled stream simply yields `None`.
//!
//! Two things differ from the macOS backend and shape the code below:
//!
//! - **The pool has no rate control.** WGC delivers on the compositor's clock,
//!   which is the display refresh rate. `CaptureConfig.rate` is therefore
//!   enforced in the arrival handler, *before* the GPU-to-CPU copy, so a 30 fps
//!   capture on a 144 Hz display genuinely costs 30 readbacks a second rather
//!   than throttling after paying for all 144.
//! - **There is no capture-time scaling.** `SCStreamConfiguration.width/height`
//!   has no WGC equivalent; the frame pool's size selects a region, it does not
//!   resample. Crop is free (it becomes a smaller `CopySubresourceRegion`), but
//!   `scale_to` costs a CPU downsample. See [`downscale`].
//!
//! Frames are BGRA8 uploaded to a `Bgra8UnormSrgb` texture, so no CPU swizzle.
//! This is CPU readback by design; sharing the D3D11 texture with wgpu is a
//! measured follow-up, exactly as on macOS. See spec/screen-capture.md.

#![allow(unsafe_code)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use windows::core::{Interface, BOOL};
use windows::Foundation::TypedEventHandler;
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Win32::Foundation::{HMODULE, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_BOX,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE,
    D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};
use windows::Win32::System::Com::CoIncrementMTAUsage;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowLongPtrW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, WS_EX_TOOLWINDOW,
};

use crate::screen_capture::resample::{downscale, Geometry};

use crate::screen_capture::backend::{
    CaptureConfig, CaptureError, CaptureFrame, CapturePixelFormat, CaptureTargetInfo,
    CaptureTargetKind, PermissionState, ScreenCaptureBackend,
};

/// Buffers in the capture frame pool. Matching the macOS `queueDepth`: deep
/// enough to absorb compositor jitter, shallow enough not to bank latency.
const FRAME_POOL_BUFFERS: i32 = 3;

/// Menu-bar-extra equivalents: tiny helper windows that are noise in the picker.
const MIN_WINDOW_EDGE: i32 = 32;

pub fn backend_name() -> &'static str {
    "WindowsGraphicsCapture"
}

pub fn permission_state() -> PermissionState {
    // Windows has no capture permission gate. WGC does draw a yellow border
    // around the captured target on most builds, which is the OS's chosen
    // consent signal and cannot be suppressed without a packaged-app identity.
    PermissionState::NotRequired
}

pub fn request_permission() {}

/// Enumerate monitors and top-level windows.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if this build of Windows has no Graphics
/// Capture support (pre-1903), which is the one case where an empty list would
/// otherwise look like "no displays attached".
pub fn enumerate() -> Result<Vec<CaptureTargetInfo>, CaptureError> {
    ensure_supported()?;
    let mut targets = enumerate_monitors();
    targets.extend(enumerate_windows());
    Ok(targets)
}

/// Open a capture session for `target`.
///
/// # Errors
///
/// Returns [`CaptureError::Unavailable`] on a build without WGC,
/// [`CaptureError::TargetNotFound`] if the monitor or window has gone away
/// since enumeration, or [`CaptureError::Backend`] for any D3D11 or `WinRT`
/// failure.
pub fn open(
    target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    ensure_supported()?;
    Ok(Box::new(WindowsBackend::new(target, config)?))
}

fn ensure_supported() -> Result<(), CaptureError> {
    // Keeps the process in the MTA for as long as it runs. WGC's free-threaded
    // frame pool needs an initialized apartment, and this is the only way to
    // get one without imposing a threading model on threads we do not own.
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

/// Collected by the `EnumDisplayMonitors` / `EnumWindows` callbacks, which can
/// only carry a raw pointer across the FFI boundary.
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
    // The enumeration order is not stable across hot-plug, so number the
    // entries as they arrive and let identity matching work off the label —
    // the same contract the macOS backend has.
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

/// Build a target for `hwnd`, or `None` if it is not something a performer
/// would recognise as a window.
unsafe fn describe_window(hwnd: HWND, our_pid: u32) -> Option<CaptureTargetInfo> {
    if !unsafe { IsWindowVisible(hwnd) }.as_bool() {
        return None;
    }
    // Tool windows are palettes and tooltips; nobody drags those onto a deck.
    let ex_style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    if ex_style & WS_EX_TOOLWINDOW.0 != 0 {
        return None;
    }
    // Cloaked windows are visible by every legacy test yet render nothing:
    // suspended UWP apps and virtual-desktop ghosts. Without this the picker
    // fills up with entries that capture pure black.
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
        // The executable name is the closest Windows equivalent to a bundle
        // id: it survives a retitle, which is what persistence matches on.
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

/// Executable file stem for `pid`, e.g. `firefox`. `None` for processes we are
/// not allowed to query, which is normal for elevated and system processes.
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
    // The handle is ours and nothing else can be holding it.
    let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
    ok.ok()?;
    let path = String::from_utf16_lossy(&buf[..len as usize]);
    std::path::Path::new(&path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
}

// ── Capture session ─────────────────────────────────────────────────

/// Latest-wins frame slot shared between the WGC thread pool and the capture
/// thread.
type FrameSlot = Arc<Mutex<Option<CaptureFrame>>>;

/// Everything the arrival handler needs, in one allocation so the closure can
/// hold a single `Arc`.
struct ArrivalState {
    slot: FrameSlot,
    /// Live geometry: `(crop_box, output_width, output_height)`. Swapped
    /// wholesale by `set_config` so the handler never sees a half-applied
    /// change and copies a region that does not match the size it reports.
    geometry: Mutex<Geometry>,
    /// Rate gate. Held here rather than read from the config so the handler
    /// does not take the geometry lock on every discarded frame.
    min_interval: Mutex<Duration>,
    last_delivered: Mutex<Option<Instant>>,
    /// Set once the pool has been asked to stop, so a frame still in flight
    /// does not resurrect the session.
    stopped: Arc<AtomicBool>,
}

/// The COM/WinRT objects backing a live capture.
///
/// None of these is apartment-affine once the process is in the MTA, and the
/// session is constructed on the calling thread then owned by exactly one
/// capture thread. The `windows` crate marks them `!Send` because a single-
/// threaded-apartment caller *could* pin them; we opt out of that
/// conservatism, and the single-owner move is what makes it sound.
struct SessionHandles {
    _item: GraphicsCaptureItem,
    _device: ID3D11Device,
    _context: ID3D11DeviceContext,
    frame_pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
}

// SAFETY: see `SessionHandles` docs — MTA-resident objects, single-owner move.
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
        // Both are post-1903 additions and throw on older builds, where the
        // OS behaviour (cursor shown, border drawn) is simply not adjustable.
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

/// Resolve a target back onto a live monitor or window and wrap it in a
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
    // WARP is the documented fallback for machines with no hardware device
    // available to this session (RDP, some VMs). Capture still works, slower.
    for driver in [D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP] {
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let hr = unsafe {
            D3D11CreateDevice(
                None,
                driver,
                HMODULE::default(),
                // WGC surfaces are BGRA, and the runtime refuses to hand them
                // to a device that did not ask for BGRA support.
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&raw mut device),
                None,
                Some(&raw mut context),
            )
        };
        if hr.is_ok() {
            if let (Some(device), Some(context)) = (device, context) {
                return Ok((device, context));
            }
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

/// `FrameArrived` handler. Runs on a WGC thread-pool thread.
///
/// The rate gate is checked before anything expensive: WGC delivers at the
/// compositor's refresh rate and a discarded frame must cost only a
/// `TryRecycle`, or a 30 fps capture on a 144 Hz display would still pay for
/// 144 GPU-to-CPU copies a second.
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
        if let Some(prev) = *last {
            if now.duration_since(prev) < *min_interval {
                return;
            }
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

    if let Some(captured) = read_back(context, &texture, geometry) {
        if let Ok(mut slot) = state.slot.lock() {
            *slot = Some(captured);
        }
    }
}

/// Copy the cropped region of a GPU texture into a tightly-packed BGRA frame.
///
/// Returns `None` on any D3D failure: a dropped frame is always better than a
/// torn one, and the next arrival is 16 ms away.
fn read_back(
    context: &ID3D11DeviceContext,
    texture: &ID3D11Texture2D,
    geometry: Geometry,
) -> Option<CaptureFrame> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { texture.GetDesc(&raw mut desc) };

    // The pool hands back the frame at the target's *current* size, which can
    // differ from the size the crop was computed against if the window was
    // resized between arrivals. Clamp rather than reading out of bounds.
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
    // Everything below must reach the matching Unmap, so no `?` past here.
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
        // A resize lands asynchronously, so trust the frame over our own
        // bookkeeping.
        self.width = frame.width;
        self.height = frame.height;
        Some(frame)
    }

    fn pixel_format(&self) -> CapturePixelFormat {
        // The frame pool is created as `B8G8R8A8UIntNormalized`.
        CapturePixelFormat::Bgra8UnormSrgb
    }

    fn is_self_paced(&self) -> bool {
        // The arrival handler enforces `rate`, so the manager's capture thread
        // must oversample rather than run a second clock at the same nominal
        // frequency. See `ScreenCaptureBackend::is_self_paced`.
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
        // Order matters: flag first, so a frame already dispatched to the
        // thread pool returns without touching a half-torn-down session.
        self.stopped.store(true, Ordering::Relaxed);
        let _ = self.handles.session.Close();
        let _ = self.handles.frame_pool.Close();
        log::debug!("Windows Graphics Capture stopped for '{}'", self.label);
    }
}
