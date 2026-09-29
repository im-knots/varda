//! Wayland screen and window capture via the XDG Desktop Portal and `PipeWire`.
//!
//! Wayland clients cannot read pixels they do not own, so capture goes through
//! `org.freedesktop.portal.ScreenCast`: the compositor's picker opens, the user
//! chooses a monitor or window, and the portal returns a `PipeWire` node id and
//! a daemon file descriptor. A `process` callback repacks each buffer into a
//! tightly packed frame in a shared slot, and [`WaylandBackend::next_frame`]
//! takes it. The capture thread never blocks on the compositor, and a stalled
//! cast yields `None`.
//!
//! [`enumerate`] reports one synthetic target. The compositor does not list
//! displays or windows, and the portal picker decides what is cast, so listed
//! entries would not match what the user then picks.
//!
//! Differences from macOS and Windows:
//!
//! - The portal has no source rectangle or output size, so crop and `scale_to`
//!   run on the CPU through [`Geometry`] / [`downscale`], as on Windows.
//! - The negotiated `VideoFramerate` is only an upper bound and most
//!   compositors deliver on damage, so `CaptureConfig.rate` is gated in the
//!   `process` callback before the repack.
//! - No `PipeWire` type is `Send`. `MainLoop`, `Context`, `Core` and `Stream`
//!   live on one thread spawned by [`open`]; only `Arc<CastState>` is shared.
//!
//! Only CPU-mapped buffers are accepted. A DMA-BUF frame is dropped with one
//! warning, because importing it needs Vulkan external-memory interop that
//! `wgpu` does not expose.

use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, ResponseError, Session};
use pipewire as pw;
use pipewire::spa;

use crate::screen_capture::backend::{
    CaptureConfig, CaptureError, CaptureFrame, CapturePixelFormat, CaptureTargetInfo,
    CaptureTargetKind, ScreenCaptureBackend,
};
use crate::screen_capture::resample::{Geometry, downscale};

/// Reported by the Linux dispatcher when the session is Wayland.
pub const BACKEND_NAME: &str = "PipeWire";

/// Timeout for the `PipeWire` side of the cast, counted from after the portal
/// dialog is answered. Exceeding it means a wedged daemon.
const PIPEWIRE_START_TIMEOUT: Duration = Duration::from_secs(5);

/// Size hint in the `EnumFormat` when no scale is requested. It sits inside a
/// 1×1..8192×8192 range; the compositor delivers its actual output size.
const PREFERRED_SIZE: (u32, u32) = (1920, 1080);

/// Enumerates the single portal target.
///
/// `width` and `height` are zero because nothing is known until the user
/// picks. The manager clamps the placeholder to 1×1 and reallocates on the
/// first frame.
///
/// # Errors
///
/// Never fails; the signature matches the other platform providers.
pub fn enumerate() -> Result<Vec<CaptureTargetInfo>, CaptureError> {
    Ok(vec![portal_target()])
}

fn portal_target() -> CaptureTargetInfo {
    CaptureTargetInfo {
        kind: CaptureTargetKind::Display,
        platform_id: 0,
        label: "Pick a window or display…".into(),
        app: None,
        title: None,
        width: 0,
        height: 0,
        // The picker may choose a Varda window, but that is unknown until after
        // the dialog, and the portal cannot exclude windows anyway.
        is_varda: false,
    }
}

/// Opens a screen cast, raising the portal picker.
///
/// `target` is otherwise ignored: the portal makes the selection, and the
/// deck's label comes from the portal's reply.
///
/// # Errors
///
/// Returns [`CaptureError::PermissionDenied`] if the user dismisses the picker,
/// [`CaptureError::TargetNotFound`] if the portal grants a session with no
/// stream, and [`CaptureError::Backend`] for any D-Bus, portal or `PipeWire`
/// failure.
pub fn open(
    _target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    Ok(Box::new(WaylandBackend::new(config)?))
}

// ── Portal handshake ────────────────────────────────────────────────

/// Single-worker runtime for the portal handshake.
///
/// Separate from the HTTP API's tokio runtime because `Start` waits for the
/// user to answer the dialog, which would stall API responses.
///
/// Never dropped: `ashpd` caches its `zbus::Connection` in a `static`, and the
/// connection's socket task runs on the runtime that created it. Dropping the
/// runtime would make the next `open` hang.
fn portal_runtime() -> Result<&'static tokio::runtime::Runtime, CaptureError> {
    static RUNTIME: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("varda-screencast-portal")
                .enable_all()
                .build()
                .map_err(|e| e.to_string())
        })
        .as_ref()
        .map_err(|e| CaptureError::Backend(format!("screen cast portal runtime unavailable: {e}")))
}

/// What the portal granted: the D-Bus objects that keep the cast alive and
/// the `PipeWire` endpoint.
struct PortalCast {
    /// Held for the life of the cast: the compositor ends the cast when the
    /// session is dropped.
    proxy: Screencast,
    session: Session<Screencast>,
    node_id: u32,
    /// Compositor coordinates, which differ from pixels on a scaled output. An
    /// initial guess until `param_changed` sets the real size.
    size: Option<(i32, i32)>,
    label: String,
    fd: OwnedFd,
}

/// Builds the deck label for the cast from the portal's reply.
///
/// The source type is always used. The stream `id` is opaque and some portals
/// use a small integer, so it is appended only when it looks like a name
/// (a connector such as "DP-1" or a window title).
fn cast_label(source: Option<SourceType>, id: Option<&str>) -> String {
    let kind = match source {
        Some(SourceType::Monitor) => "Display cast",
        Some(SourceType::Window) => "Window cast",
        _ => "Screen cast",
    };
    match id {
        Some(id) if id.chars().any(char::is_alphabetic) => format!("{kind} ({id})"),
        _ => kind.to_string(),
    }
}

async fn portal_handshake(config: &CaptureConfig) -> Result<PortalCast, CaptureError> {
    let proxy = Screencast::new().await.map_err(|e| portal_error(&e))?;
    let session = proxy
        .create_session(CreateSessionOptions::default())
        .await
        .map_err(|e| portal_error(&e))?;

    let cursor_mode = if config.show_cursor {
        CursorMode::Embedded
    } else {
        CursorMode::Hidden
    };

    proxy
        .select_sources(
            &session,
            SelectSourcesOptions::default()
                .set_cursor_mode(cursor_mode)
                // Offer both kinds; the user chooses in the picker.
                .set_sources(SourceType::Monitor | SourceType::Window)
                .set_multiple(false)
                // No restore token: skipping the dialog on reload needs the token saved
                // per target in the scene format.
                .set_persist_mode(PersistMode::DoNot),
        )
        .await
        .map_err(|e| portal_error(&e))?;

    let streams = proxy
        .start(&session, None, StartCastOptions::default())
        .await
        .map_err(|e| portal_error(&e))?
        .response()
        .map_err(|e| portal_error(&e))?;

    let (node_id, size, label) = {
        let stream = streams.streams().first().ok_or_else(|| {
            CaptureError::TargetNotFound("the portal picker returned no stream".to_string())
        })?;
        (
            stream.pipe_wire_node_id(),
            stream.size(),
            cast_label(stream.source_type(), stream.id()),
        )
    };

    let fd = proxy
        .open_pipe_wire_remote(&session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(|e| portal_error(&e))?;

    Ok(PortalCast {
        proxy,
        session,
        node_id,
        size,
        label,
        fd,
    })
}

/// Classifies a portal failure.
///
/// A dismissed picker is reported as a permission refusal, not a backend
/// error, since the user chose it and the manager already shows that state
/// quietly.
fn portal_error(err: &ashpd::Error) -> CaptureError {
    match err {
        ashpd::Error::Response(ResponseError::Cancelled) => CaptureError::PermissionDenied,
        other => CaptureError::Backend(format!("screen cast portal: {other}")),
    }
}

// ── Shared cast state ───────────────────────────────────────────────

const FORMAT_CODE_BGRA: u8 = 0;
const FORMAT_CODE_RGBA: u8 = 1;

fn format_code(format: CapturePixelFormat) -> u8 {
    match format {
        CapturePixelFormat::Bgra8UnormSrgb => FORMAT_CODE_BGRA,
        CapturePixelFormat::Rgba8UnormSrgb => FORMAT_CODE_RGBA,
    }
}

fn format_from_code(code: u8) -> CapturePixelFormat {
    if code == FORMAT_CODE_RGBA {
        CapturePixelFormat::Rgba8UnormSrgb
    } else {
        CapturePixelFormat::Bgra8UnormSrgb
    }
}

/// State shared by the `PipeWire` loop thread and the capture thread.
struct CastState {
    slot: Mutex<Option<CaptureFrame>>,
    /// Live geometry: source rectangle and delivered size. Replaced as a whole so
    /// the `process` callback never copies a region that disagrees with the size
    /// it reports.
    geometry: Mutex<Geometry>,
    /// Negotiated pixel size from `param_changed`, so `set_config` can re-resolve
    /// geometry without a renegotiation.
    native: Mutex<(u32, u32)>,
    /// Latest config, so `param_changed` can re-resolve geometry when the frame
    /// size changes mid-cast.
    config: Mutex<CaptureConfig>,
    /// Rate gate. Kept separate from the config so a discarded frame skips the
    /// config lock.
    min_interval: Mutex<Duration>,
    last_delivered: Mutex<Option<Instant>>,
    /// Negotiated pixel layout. Written by `param_changed`, read by
    /// [`WaylandBackend::pixel_format`] on another thread.
    format: AtomicU8,
}

impl CastState {
    fn new(config: &CaptureConfig, native: (u32, u32)) -> Self {
        Self {
            slot: Mutex::new(None),
            geometry: Mutex::new(Geometry::resolve(native.0, native.1, config)),
            native: Mutex::new(native),
            config: Mutex::new(config.clone()),
            min_interval: Mutex::new(config.frame_interval()),
            last_delivered: Mutex::new(None),
            format: AtomicU8::new(format_code(CapturePixelFormat::Bgra8UnormSrgb)),
        }
    }

    /// Stores the negotiated format and re-resolves geometry against the real
    /// pixel size.
    fn renegotiated(&self, width: u32, height: u32, format: CapturePixelFormat) {
        self.format.store(format_code(format), Ordering::Relaxed);
        if let Ok(mut native) = self.native.lock() {
            *native = (width, height);
        }
        let Some(config) = self.config.lock().ok().map(|c| c.clone()) else {
            return;
        };
        if let Ok(mut geometry) = self.geometry.lock() {
            *geometry = Geometry::resolve(width, height, &config);
        }
    }

    /// Applies a live config change. No renegotiation: crop, scale and rate are
    /// all applied on the consumer side.
    fn reconfigure(&self, config: &CaptureConfig) {
        if let Ok(mut current) = self.config.lock() {
            *current = config.clone();
        }
        let native = self.native.lock().map_or((1, 1), |n| *n);
        if let Ok(mut geometry) = self.geometry.lock() {
            *geometry = Geometry::resolve(native.0, native.1, config);
        }
        if let Ok(mut interval) = self.min_interval.lock() {
            *interval = config.frame_interval();
        }
    }

    fn geometry(&self) -> Option<Geometry> {
        self.geometry.lock().ok().map(|g| *g)
    }

    fn pixel_format(&self) -> CapturePixelFormat {
        format_from_code(self.format.load(Ordering::Relaxed))
    }

    /// Whether enough time has passed to accept another frame; records the
    /// delivery if so. Checked before the repack so a discarded frame costs only
    /// the dequeue.
    fn accept_frame(&self) -> bool {
        let Ok(min_interval) = self.min_interval.lock() else {
            return false;
        };
        let Ok(mut last) = self.last_delivered.lock() else {
            return false;
        };
        let now = Instant::now();
        if let Some(prev) = *last
            && now.duration_since(prev) < *min_interval
        {
            return false;
        }
        *last = Some(now);
        true
    }
}

// ── Pixel handling ──────────────────────────────────────────────────

/// Maps a negotiated SPA video format to the texture layout to allocate.
///
/// `None` for formats outside the offer. Guessing a layout would give wrong
/// colors instead of an obvious failure.
fn spa_format_to_capture_format(
    format: spa::param::video::VideoFormat,
) -> Option<CapturePixelFormat> {
    use spa::param::video::VideoFormat;
    if format == VideoFormat::BGRx || format == VideoFormat::BGRA {
        Some(CapturePixelFormat::Bgra8UnormSrgb)
    } else if format == VideoFormat::RGBx || format == VideoFormat::RGBA {
        Some(CapturePixelFormat::Rgba8UnormSrgb)
    } else {
        None
    }
}

/// Whether the negotiated format has an alpha channel.
///
/// The `x` layouts leave the fourth byte undefined, which would make the deck
/// randomly transparent, so the repack overwrites it.
fn format_has_alpha(format: spa::param::video::VideoFormat) -> bool {
    use spa::param::video::VideoFormat;
    format == VideoFormat::BGRA || format == VideoFormat::RGBA
}

/// Copies `geometry`'s source rectangle out of a strided 4-bytes-per-pixel
/// buffer into a tightly packed `src_w * 4` frame.
///
/// `stride` is `chunk.stride`, padded by the producer; the manager uploads at
/// `width * 4`. Cropping happens in the same pass, so a crop reduces the copy
/// as well as the upload.
///
/// Returns `None` when the buffer is too short for the rectangle, which
/// happens while a renegotiation is in flight.
fn repack_rows(src: &[u8], stride: usize, geometry: Geometry, opaque: bool) -> Option<Vec<u8>> {
    let (width, height) = (geometry.src_w as usize, geometry.src_h as usize);
    if width == 0 || height == 0 {
        return None;
    }
    let row_bytes = width.checked_mul(4)?;
    let row_start = (geometry.src_x as usize).checked_mul(4)?;
    let row_end = row_start.checked_add(row_bytes)?;
    if row_end > stride {
        return None;
    }
    let last_row = (geometry.src_y as usize).checked_add(height - 1)?;
    let needed = last_row.checked_mul(stride)?.checked_add(row_end)?;
    if needed > src.len() {
        return None;
    }

    let mut out = vec![0u8; row_bytes * height];
    for row in 0..height {
        let from = (geometry.src_y as usize + row) * stride + row_start;
        out[row * row_bytes..(row + 1) * row_bytes].copy_from_slice(&src[from..from + row_bytes]);
    }
    if opaque {
        for texel in out.as_chunks_mut::<4>().0 {
            texel[3] = 0xFF;
        }
    }
    Some(out)
}

/// Builds the `EnumFormat` offered at connect time.
///
/// Only the four 32-bit packed layouts are offered, so frames map directly to
/// `Bgra8UnormSrgb` or `Rgba8UnormSrgb` with no CPU swizzle. `BGRx` comes first
/// because every tested compositor produces it. Size and framerate are ranges
/// because a fixed request the compositor cannot meet fails negotiation.
///
/// # Errors
///
/// Returns [`CaptureError::Backend`] if the pod cannot be serialized.
fn format_pod(config: &CaptureConfig) -> Result<Vec<u8>, CaptureError> {
    let (width, height) = config.scale_to.unwrap_or(PREFERRED_SIZE);
    let rate = (config.rate.round() as u32).max(1);

    let object = spa::pod::object!(
        spa::utils::SpaTypes::ObjectParamFormat,
        spa::param::ParamType::EnumFormat,
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaType,
            Id,
            spa::param::format::MediaType::Video
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::MediaSubtype,
            Id,
            spa::param::format::MediaSubtype::Raw
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFormat,
            Choice,
            Enum,
            Id,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRx,
            spa::param::video::VideoFormat::BGRA,
            spa::param::video::VideoFormat::RGBx,
            spa::param::video::VideoFormat::RGBA,
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoSize,
            Choice,
            Range,
            Rectangle,
            spa::utils::Rectangle {
                width: width.max(1),
                height: height.max(1)
            },
            spa::utils::Rectangle {
                width: 1,
                height: 1
            },
            spa::utils::Rectangle {
                width: 8192,
                height: 8192
            }
        ),
        spa::pod::property!(
            spa::param::format::FormatProperties::VideoFramerate,
            Choice,
            Range,
            Fraction,
            spa::utils::Fraction {
                num: rate,
                denom: 1
            },
            spa::utils::Fraction { num: 0, denom: 1 },
            spa::utils::Fraction {
                num: 1000,
                denom: 1
            }
        ),
    );

    spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(object),
    )
    .map(|(cursor, _)| cursor.into_inner())
    .map_err(|e| CaptureError::Backend(format!("failed to build the video format offer: {e}")))
}

// ── PipeWire loop thread ────────────────────────────────────────────

/// Wakes the `PipeWire` loop so it can quit.
///
/// `MainLoop::quit` is not thread-safe, so the stop goes over
/// `pipewire::channel`, which writes to a pipe the loop polls. A message sent
/// before `run()` stays queued, so a stop during startup is not lost.
struct Terminate;

/// State shared by the `PipeWire` callbacks. They all run on the loop thread;
/// the locks inside [`CastState`] are for the capture thread.
struct Consumer {
    format: spa::param::video::VideoInfoRaw,
    state: Arc<CastState>,
    warned_dmabuf: bool,
}

/// The `PipeWire` objects behind a live cast, in teardown order: the listener
/// is unhooked before its stream is destroyed.
struct Cast {
    _listener: pw::stream::StreamListener<Consumer>,
    stream: pw::stream::StreamRc,
    main_loop: pw::main_loop::MainLoopRc,
}

/// Body of the cast thread.
///
/// Reports setup success or failure over `ready` so [`WaylandBackend::new`]
/// can fail with a real message, then runs the loop until [`Terminate`]
/// arrives.
fn run_cast(
    node_id: u32,
    fd: OwnedFd,
    state: &Arc<CastState>,
    offer: &[u8],
    quit: pw::channel::Receiver<Terminate>,
    ready: &mpsc::Sender<Result<(), CaptureError>>,
) {
    let cast = match connect_cast(node_id, fd, state, offer) {
        Ok(cast) => {
            if ready.send(Ok(())).is_err() {
                return;
            }
            cast
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    let _quit = quit.attach(cast.main_loop.loop_(), {
        let main_loop = cast.main_loop.clone();
        move |_: Terminate| main_loop.quit()
    });

    cast.main_loop.run();
    let _ = cast.stream.disconnect();
}

fn connect_cast(
    node_id: u32,
    fd: OwnedFd,
    state: &Arc<CastState>,
    offer: &[u8],
) -> Result<Cast, CaptureError> {
    pw::init();

    let main_loop = pw::main_loop::MainLoopRc::new(None).map_err(|e| pipewire_error(&e))?;
    let context = pw::context::ContextRc::new(&main_loop, None).map_err(|e| pipewire_error(&e))?;
    let core = context
        .connect_fd_rc(fd, None)
        .map_err(|e| pipewire_error(&e))?;
    let stream = pw::stream::StreamRc::new(
        core,
        "Varda Screen Capture",
        pw::properties::properties! {
            *pw::keys::MEDIA_TYPE => "Video",
            *pw::keys::MEDIA_CATEGORY => "Capture",
            *pw::keys::MEDIA_ROLE => "Screen",
        },
    )
    .map_err(|e| pipewire_error(&e))?;

    let listener = stream
        .add_local_listener_with_user_data(Consumer {
            format: spa::param::video::VideoInfoRaw::default(),
            state: Arc::clone(state),
            warned_dmabuf: false,
        })
        .param_changed(on_param_changed)
        .process(on_process)
        .register()
        .map_err(|e| pipewire_error(&e))?;

    let pod = spa::pod::Pod::from_bytes(offer)
        .ok_or_else(|| CaptureError::Backend("malformed video format offer".to_string()))?;
    let mut params = [pod];
    stream
        .connect(
            spa::utils::Direction::Input,
            Some(node_id),
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(|e| pipewire_error(&e))?;

    Ok(Cast {
        _listener: listener,
        stream,
        main_loop,
    })
}

fn pipewire_error(err: &pw::Error) -> CaptureError {
    CaptureError::Backend(format!("PipeWire: {err}"))
}

/// Stores the negotiated format.
///
/// This size, not the portal's, is the buffer pixel size: the portal reports
/// compositor coordinates, which differ on fractionally scaled outputs.
fn on_param_changed(
    _stream: &pw::stream::Stream,
    consumer: &mut Consumer,
    id: u32,
    param: Option<&spa::pod::Pod>,
) {
    let Some(param) = param else {
        return;
    };
    if id != spa::param::ParamType::Format.as_raw() {
        return;
    }
    let Ok((media_type, media_subtype)) = spa::param::format_utils::parse_format(param) else {
        return;
    };
    if media_type != spa::param::format::MediaType::Video
        || media_subtype != spa::param::format::MediaSubtype::Raw
    {
        return;
    }
    if consumer.format.parse(param).is_err() {
        return;
    }

    let size = consumer.format.size();
    let video_format = consumer.format.format();
    let Some(pixel_format) = spa_format_to_capture_format(video_format) else {
        log::warn!("PipeWire negotiated {video_format:?}, which was not offered; frames dropped");
        return;
    };

    log::debug!(
        "PipeWire screen cast negotiated {video_format:?} at {}x{}",
        size.width,
        size.height
    );
    consumer
        .state
        .renegotiated(size.width.max(1), size.height.max(1), pixel_format);
}

/// Repacks one buffer into the latest-wins slot.
fn on_process(stream: &pw::stream::Stream, consumer: &mut Consumer) {
    // Always dequeue first: dropping `Buffer` returns it to the stream, and a
    // graph that never gets its buffers back stalls.
    let Some(mut buffer) = stream.dequeue_buffer() else {
        return;
    };
    if !consumer.state.accept_frame() {
        return;
    }
    let Some(geometry) = consumer.state.geometry() else {
        return;
    };
    let pixel_format = consumer.state.pixel_format();
    let opaque = !format_has_alpha(consumer.format.format());

    let Some(data) = buffer.datas_mut().first_mut() else {
        return;
    };
    if data.type_() == spa::buffer::DataType::DmaBuf {
        if !consumer.warned_dmabuf {
            consumer.warned_dmabuf = true;
            log::warn!(
                "PipeWire negotiated DMA-BUF buffers, which this backend cannot import; \
                 the capture will stay black. Zero-copy DMA-BUF import is a follow-up."
            );
        }
        return;
    }

    // `chunk()` borrows immutably and `data()` mutably, so copy the layout out
    // before taking the mapping.
    let (offset, length, stride) = {
        let chunk = data.chunk();
        (
            chunk.offset() as usize,
            chunk.size() as usize,
            usize::try_from(chunk.stride()).unwrap_or(0),
        )
    };
    if stride == 0 {
        return;
    }

    let Some(mapped) = data.data() else {
        return;
    };
    let Some(src) = mapped.get(offset..offset.saturating_add(length)) else {
        return;
    };
    let Some(packed) = repack_rows(src, stride, geometry, opaque) else {
        return;
    };

    let (pixels, width, height) = if geometry.is_identity_scale() {
        (packed, geometry.src_w, geometry.src_h)
    } else {
        (
            downscale(
                &packed,
                geometry.src_w,
                geometry.src_h,
                geometry.out_w,
                geometry.out_h,
            ),
            geometry.out_w,
            geometry.out_h,
        )
    };

    // Latest wins, without blocking: dropping this frame is cheaper than
    // stalling the compositor's graph.
    if let Ok(mut slot) = consumer.state.slot.try_lock() {
        *slot = Some(CaptureFrame {
            data: pixels,
            width,
            height,
            format: pixel_format,
        });
    }
}

// ── Backend ─────────────────────────────────────────────────────────

/// A live Wayland screen cast.
pub struct WaylandBackend {
    label: String,
    state: Arc<CastState>,
    quit: pw::channel::Sender<Terminate>,
    thread: Option<JoinHandle<()>>,
    /// Held for the backend's lifetime: the compositor ends the cast when the
    /// portal session is dropped.
    portal: PortalHandles,
    width: u32,
    height: u32,
    config: CaptureConfig,
}

struct PortalHandles {
    _proxy: Screencast,
    session: Session<Screencast>,
}

impl WaylandBackend {
    fn new(config: &CaptureConfig) -> Result<Self, CaptureError> {
        let config = config.clone().sanitized();

        // `Runtime::block_on` panics inside another runtime. This runs on the
        // engine thread; fail with a message if a caller ever moves it onto the
        // API runtime.
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(CaptureError::Backend(
                "screen capture must be opened from the engine thread, not from inside a tokio runtime"
                    .to_string(),
            ));
        }

        // Blocking: the deck has nothing to show until the user picks.
        let PortalCast {
            proxy,
            session,
            node_id,
            size,
            label,
            fd,
        } = portal_runtime()?.block_on(portal_handshake(&config))?;

        // A missing or invalid size is not fatal: `param_changed` sets the real
        // one before the first frame, and until then the geometry only has to be
        // non-degenerate.
        let (native_w, native_h) = size.map_or((1, 1), |(w, h)| {
            (
                u32::try_from(w).unwrap_or(1).max(1),
                u32::try_from(h).unwrap_or(1).max(1),
            )
        });
        let state = Arc::new(CastState::new(&config, (native_w, native_h)));
        let offer = format_pod(&config)?;
        let (quit_tx, quit_rx) = pw::channel::channel::<Terminate>();
        let (ready_tx, ready_rx) = mpsc::channel();

        let thread = std::thread::Builder::new()
            .name("varda-screen-cast".to_string())
            .spawn({
                let state = Arc::clone(&state);
                move || run_cast(node_id, fd, &state, &offer, quit_rx, &ready_tx)
            })
            .map_err(|e| CaptureError::Backend(format!("failed to spawn the cast thread: {e}")))?;

        let started = ready_rx
            .recv_timeout(PIPEWIRE_START_TIMEOUT)
            .unwrap_or_else(|_| {
                Err(CaptureError::Backend(format!(
                    "PipeWire did not start the cast within {}s",
                    PIPEWIRE_START_TIMEOUT.as_secs()
                )))
            });
        if let Err(e) = started {
            let _ = quit_tx.send(Terminate);
            let _ = thread.join();
            return Err(e);
        }

        let geometry = state
            .geometry()
            .unwrap_or_else(|| Geometry::resolve(native_w, native_h, &config));
        log::info!(
            "PipeWire screen cast started as '{label}' on node {node_id} at {}x{}",
            geometry.out_w,
            geometry.out_h
        );

        Ok(Self {
            label,
            state,
            quit: quit_tx,
            thread: Some(thread),
            portal: PortalHandles {
                _proxy: proxy,
                session,
            },
            width: geometry.out_w,
            height: geometry.out_h,
            config,
        })
    }

    /// Ends the portal session if blocking is safe here.
    ///
    /// Best effort. `block_on` panics inside a runtime and a panic in `drop`
    /// aborts, so on the API thread this is skipped; the compositor still ends
    /// the cast when the D-Bus session closes.
    fn close_portal(&self) {
        if tokio::runtime::Handle::try_current().is_ok() {
            return;
        }
        let Ok(runtime) = portal_runtime() else {
            return;
        };
        runtime.block_on(async {
            let _ = self.portal.session.close().await;
        });
    }
}

impl ScreenCaptureBackend for WaylandBackend {
    fn label(&self) -> &str {
        &self.label
    }

    fn resolution(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(&mut self) -> Option<CaptureFrame> {
        let frame = self.state.slot.try_lock().ok()?.take()?;
        // A renegotiation applies asynchronously, so trust the frame size.
        self.width = frame.width;
        self.height = frame.height;
        Some(frame)
    }

    fn pixel_format(&self) -> CapturePixelFormat {
        self.state.pixel_format()
    }

    fn is_self_paced(&self) -> bool {
        // The `process` callback enforces `rate`, so the capture thread must
        // oversample. See `ScreenCaptureBackend::is_self_paced`.
        true
    }

    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError> {
        let config = config.clone().sanitized();
        if config == self.config {
            return Ok(());
        }

        if config.show_cursor != self.config.show_cursor {
            // `CursorMode` is set by `SelectSources`, allowed once per session.
            // Changing it would need a new picker dialog.
            log::debug!(
                "Screen capture '{}': cursor change takes effect on the next open",
                self.label
            );
        }
        if config.exclude_varda != self.config.exclude_varda {
            // The portal cannot exclude windows, so a display cast containing Varda
            // mirrors. The UI shows a note next to the toggle.
            log::debug!(
                "Screen capture '{}': exclude_varda has no portal equivalent and is ignored",
                self.label
            );
        }

        // Rate, crop and scale apply on the consumer side, without touching the
        // portal session.
        self.state.reconfigure(&config);
        self.config = config;
        Ok(())
    }
}

impl Drop for WaylandBackend {
    fn drop(&mut self) {
        let _ = self.quit.send(Terminate);
        if let Some(thread) = self.thread.take() {
            // The loop wakes on the pipe write, so this join takes at most one loop
            // iteration.
            let _ = thread.join();
        }
        self.close_portal();
        log::debug!("PipeWire screen cast stopped for '{}'", self.label);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen_capture::backend::CropRect;

    /// A `width × height` BGRA image padded to `stride`, each texel tagged with
    /// its coordinates so a crop can be located. Padding is `0xEE` so a repack
    /// that keeps it is visible.
    fn padded_frame(width: u32, height: u32, stride: usize) -> Vec<u8> {
        let mut buf = vec![0xEEu8; stride * height as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let texel = y * stride + x * 4;
                buf[texel] = x as u8;
                buf[texel + 1] = y as u8;
                buf[texel + 2] = 0x10;
                buf[texel + 3] = 0x00;
            }
        }
        buf
    }

    fn rect(x: u32, y: u32, w: u32, h: u32) -> Geometry {
        Geometry {
            src_x: x,
            src_y: y,
            src_w: w,
            src_h: h,
            out_w: w,
            out_h: h,
        }
    }

    #[test]
    fn enumerate_offers_exactly_one_portal_entry() {
        let targets = enumerate().expect("advisory enumeration never fails");
        assert_eq!(
            targets.len(),
            1,
            "the portal owns selection; listing invented targets would be a lie"
        );
    }

    #[test]
    fn the_portal_entry_admits_it_knows_nothing_yet() {
        let target = portal_target();
        assert_eq!(target.platform_id, 0);
        assert_eq!((target.width, target.height), (0, 0));
        assert!(target.app.is_none() && target.title.is_none());
        assert!(!target.is_varda);
        // The label must read as an action, not a target name the user never
        // picked.
        assert!(
            target.label.starts_with("Pick "),
            "label must not impersonate a real target: {}",
            target.label
        );
    }

    #[test]
    fn backend_name_is_the_one_the_dispatcher_reports() {
        assert_eq!(BACKEND_NAME, "PipeWire");
    }

    #[test]
    fn the_label_names_what_the_portal_said_was_picked() {
        assert_eq!(cast_label(Some(SourceType::Monitor), None), "Display cast");
        assert_eq!(cast_label(Some(SourceType::Window), None), "Window cast");
        assert_eq!(
            cast_label(Some(SourceType::Monitor), Some("DP-1")),
            "Display cast (DP-1)"
        );
    }

    #[test]
    fn an_opaque_numeric_stream_id_is_not_shown_to_the_user() {
        // `id` is opaque and several portals use a counter. "Display cast (0)" is
        // a worse label than "Display cast".
        assert_eq!(
            cast_label(Some(SourceType::Monitor), Some("0")),
            "Display cast"
        );
        assert_eq!(cast_label(None, Some("42")), "Screen cast");
        assert_eq!(cast_label(None, None), "Screen cast");
    }

    #[test]
    fn offered_formats_all_map_to_a_texture_layout() {
        use spa::param::video::VideoFormat;
        // Every offered format needs a mapping, or the compositor can pick one
        // whose frames are all refused.
        assert_eq!(
            spa_format_to_capture_format(VideoFormat::BGRx),
            Some(CapturePixelFormat::Bgra8UnormSrgb)
        );
        assert_eq!(
            spa_format_to_capture_format(VideoFormat::BGRA),
            Some(CapturePixelFormat::Bgra8UnormSrgb)
        );
        assert_eq!(
            spa_format_to_capture_format(VideoFormat::RGBx),
            Some(CapturePixelFormat::Rgba8UnormSrgb)
        );
        assert_eq!(
            spa_format_to_capture_format(VideoFormat::RGBA),
            Some(CapturePixelFormat::Rgba8UnormSrgb)
        );
    }

    #[test]
    fn planar_and_subsampled_formats_are_refused_not_guessed() {
        use spa::param::video::VideoFormat;
        assert!(spa_format_to_capture_format(VideoFormat::I420).is_none());
        assert!(spa_format_to_capture_format(VideoFormat::YUY2).is_none());
        assert!(spa_format_to_capture_format(VideoFormat::RGB).is_none());
    }

    #[test]
    fn only_the_alpha_carrying_layouts_report_alpha() {
        use spa::param::video::VideoFormat;
        assert!(format_has_alpha(VideoFormat::BGRA));
        assert!(format_has_alpha(VideoFormat::RGBA));
        assert!(!format_has_alpha(VideoFormat::BGRx));
        assert!(!format_has_alpha(VideoFormat::RGBx));
    }

    #[test]
    fn pixel_format_code_round_trips() {
        for format in [
            CapturePixelFormat::Bgra8UnormSrgb,
            CapturePixelFormat::Rgba8UnormSrgb,
        ] {
            assert_eq!(format_from_code(format_code(format)), format);
        }
    }

    #[test]
    fn repack_strips_the_row_padding() {
        let src = padded_frame(3, 4, 16);
        let out = repack_rows(&src, 16, rect(0, 0, 3, 4), false).expect("repack");
        assert_eq!(out.len(), 3 * 4 * 4, "output must be tightly packed");
        assert!(
            !out.contains(&0xEE),
            "no padding byte may survive into the frame"
        );
    }

    #[test]
    fn repack_extracts_only_the_cropped_region() {
        let src = padded_frame(4, 4, 32);
        let out = repack_rows(&src, 32, rect(1, 2, 2, 2), false).expect("repack");
        assert_eq!(out.len(), 2 * 2 * 4);
        // The first output texel is the source texel at (1, 2)…
        assert_eq!(&out[0..3], &[1, 2, 0x10]);
        // …and the last must be (2, 3).
        assert_eq!(&out[12..15], &[2, 3, 0x10]);
    }

    #[test]
    fn repack_forces_alpha_when_the_format_carries_none() {
        let src = padded_frame(2, 2, 8);
        let out = repack_rows(&src, 8, rect(0, 0, 2, 2), true).expect("repack");
        assert!(
            out.as_chunks::<4>().0.iter().all(|texel| texel[3] == 0xFF),
            "BGRx leaves the fourth byte undefined; a deck must not go transparent"
        );
    }

    #[test]
    fn repack_preserves_alpha_when_the_format_carries_it() {
        let src = padded_frame(2, 2, 8);
        let out = repack_rows(&src, 8, rect(0, 0, 2, 2), false).expect("repack");
        assert!(out.as_chunks::<4>().0.iter().all(|texel| texel[3] == 0x00));
    }

    #[test]
    fn repack_rejects_a_buffer_shorter_than_the_region() {
        // A renegotiation in flight delivers this; reading past the mapping would
        // crash instead of dropping a frame.
        let src = padded_frame(4, 2, 16);
        assert!(repack_rows(&src, 16, rect(0, 0, 4, 4), false).is_none());
    }

    #[test]
    fn repack_rejects_a_region_wider_than_the_stride() {
        let src = padded_frame(4, 4, 16);
        assert!(repack_rows(&src, 16, rect(2, 0, 4, 4), false).is_none());
    }

    #[test]
    fn repack_rejects_a_degenerate_region() {
        let src = padded_frame(4, 4, 16);
        assert!(repack_rows(&src, 16, rect(0, 0, 0, 4), false).is_none());
        assert!(repack_rows(&src, 16, rect(0, 0, 4, 0), false).is_none());
    }

    #[test]
    fn format_offer_is_a_readable_pod() {
        let offer = format_pod(&CaptureConfig::default()).expect("offer");
        assert!(
            spa::pod::Pod::from_bytes(&offer).is_some(),
            "a malformed offer fails negotiation and leaves a black deck"
        );
    }

    #[test]
    fn format_offer_carries_the_requested_scale() {
        let scaled = format_pod(&CaptureConfig {
            scale_to: Some((640, 360)),
            ..Default::default()
        })
        .expect("offer");
        let unscaled = format_pod(&CaptureConfig::default()).expect("offer");
        assert_ne!(
            scaled, unscaled,
            "`scale_to` must reach the size preference in the offer"
        );
    }

    #[test]
    fn state_reconfigure_retunes_the_rate_gate_and_the_geometry() {
        let state = CastState::new(&CaptureConfig::default(), (1920, 1080));
        assert_eq!(
            state.geometry().map(|g| (g.out_w, g.out_h)),
            Some((1920, 1080))
        );

        state.reconfigure(&CaptureConfig {
            rate: 10.0,
            crop: CropRect {
                x: 0.0,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            },
            scale_to: Some((480, 480)),
            ..Default::default()
        });

        let geometry = state.geometry().expect("geometry");
        assert_eq!((geometry.src_w, geometry.src_h), (960, 1080));
        assert!(geometry.out_w <= 480 && geometry.out_h <= 480);
        let interval = *state.min_interval.lock().expect("interval");
        assert!(
            interval > Duration::from_millis(99) && interval < Duration::from_millis(101),
            "10 fps must gate at ~100ms, got {interval:?}"
        );
    }

    #[test]
    fn renegotiation_re_resolves_the_geometry_against_real_pixels() {
        // The portal reports compositor coordinates. A 2× scaled output delivers
        // twice as many pixels, and a crop against the wrong size reads the wrong
        // region.
        let config = CaptureConfig {
            crop: CropRect {
                x: 0.5,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            },
            ..Default::default()
        };
        let state = CastState::new(&config, (1920, 1080));
        state.renegotiated(3840, 2160, CapturePixelFormat::Rgba8UnormSrgb);

        let geometry = state.geometry().expect("geometry");
        assert_eq!(geometry.src_x, 1920);
        assert_eq!((geometry.src_w, geometry.src_h), (1920, 2160));
        assert_eq!(state.pixel_format(), CapturePixelFormat::Rgba8UnormSrgb);
    }

    #[test]
    fn the_rate_gate_drops_frames_that_arrive_early() {
        let state = CastState::new(
            &CaptureConfig {
                rate: 1.0,
                ..Default::default()
            },
            (16, 16),
        );
        assert!(state.accept_frame(), "the first frame is always accepted");
        assert!(
            !state.accept_frame(),
            "a 1 fps capture must not deliver two frames in the same instant"
        );
    }

    #[test]
    fn declared_pixel_format_defaults_to_the_preferred_layout() {
        // `BGRx` leads the offer, so the shared texture starts as BGRA before the
        // first `param_changed`.
        let state = CastState::new(&CaptureConfig::default(), (16, 16));
        assert_eq!(state.pixel_format(), CapturePixelFormat::Bgra8UnormSrgb);
        assert_eq!(
            CapturePixelFormat::Bgra8UnormSrgb.wgpu_format(),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
    }
}
