//! Wayland screen/window capture via the XDG Desktop Portal and `PipeWire`.
//!
//! A Wayland compositor never lets a client read pixels it does not own, so
//! capture goes through `org.freedesktop.portal.ScreenCast`: the portal raises
//! the compositor's own picker, the user chooses a monitor or a window, and we
//! are handed a `PipeWire` node id plus a file descriptor onto the `PipeWire`
//! daemon. Consumption is push-based like `ScreenCaptureKit` and Windows
//! Graphics Capture — a `process` callback repacks each buffer into a
//! tightly-packed frame and drops it into a shared slot, and
//! [`WaylandBackend::next_frame`] takes whatever is there. The capture thread
//! never blocks on the compositor, and a stalled cast simply yields `None`.
//!
//! **[`enumerate`] reports exactly one synthetic target.** This is the answer to
//! spec/screen-capture.md § Open Questions "Wayland target selection", and it
//! takes the "single Pick a window… entry" option rather than the
//! looks-like-the-other-platforms option. The reasoning: the compositor will not
//! tell us what displays and windows exist, and the portal's picker is the only
//! authority on what actually gets cast. A list of invented entries would be a
//! lie the user then has to re-pick anyway — they would drag "Display 2" onto a
//! deck, get a dialog, choose something else, and end up with a deck whose label
//! names a target it is not showing. One honest entry, whose label says the
//! choice happens in the dialog, costs a click and tells the truth.
//!
//! Three things differ from the macOS and Windows backends and shape the code:
//!
//! - **The portal has no source rectangle and no output size.** Crop and
//!   `scale_to` are therefore paid for on the CPU, through the shared
//!   [`Geometry`] / [`downscale`] path, exactly like the Windows backend.
//! - **`CaptureConfig.rate` is not enforceable at the source.** The negotiated
//!   `VideoFramerate` is an upper bound the compositor is free to ignore, and
//!   most compositors deliver on damage instead. The rate is therefore gated in
//!   the `process` callback, before the repack, so a 30 fps capture of a 144 Hz
//!   output genuinely costs 30 copies a second.
//! - **Nothing in `PipeWire` is `Send`.** The `MainLoop`, `Context`, `Core`, and
//!   `Stream` are all constructed on, and confined to, one dedicated thread that
//!   [`open`] spawns; only the `Arc<CastState>` crosses the boundary.
//!
//! Only CPU-mapped buffers are accepted. A DMA-BUF frame is dropped with one
//! warning: importing it needs Vulkan external-memory interop that `wgpu` does
//! not expose, which spec/screen-capture.md § Decision: CPU readback first
//! explicitly does not promise.

use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
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
use crate::screen_capture::resample::{downscale, Geometry};

/// Reported by the Linux dispatcher when the session is Wayland.
pub const BACKEND_NAME: &str = "PipeWire";

/// How long to wait for the `PipeWire` side of the cast to come up, measured
/// from *after* the portal dialog has been answered. This bounds a machine, not
/// a person: the node id is already granted by this point, so anything slower
/// than this is a wedged daemon rather than a user still reading the dialog.
const PIPEWIRE_START_TIMEOUT: Duration = Duration::from_secs(5);

/// Size preference offered in the `EnumFormat` when the caller asks for no
/// particular scale. It sits inside a 1×1..8192×8192 range and is a hint, not a
/// demand — the compositor produces whatever its output actually is.
const PREFERRED_SIZE: (u32, u32) = (1920, 1080);

/// Enumerate the one target the library panel can honestly offer.
///
/// See the module documentation for why this is a single synthetic entry rather
/// than a target list. `width` and `height` are zero because nothing is known
/// until the user has picked: the manager clamps the placeholder to 1×1 and
/// reallocates on the first delivered frame, so admitting ignorance here costs
/// one discarded texture.
///
/// # Errors
///
/// Never fails. The signature matches the other platform providers, and the
/// dispatcher in `platform/linux.rs` documents that this branch cannot error.
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
        // The picker may well land on one of Varda's own windows, but only the
        // user knows that and only after the dialog. Claiming it here would make
        // `exclude_varda` look meaningful when the portal has no equivalent.
        is_varda: false,
    }
}

/// Open a screen cast, raising the portal picker.
///
/// `target` is ignored beyond having come from [`enumerate`]: the portal owns
/// the selection, and the label the deck ends up with comes back from the
/// portal rather than from the library entry that was dragged.
///
/// # Errors
///
/// Returns [`CaptureError::PermissionDenied`] if the user dismisses the picker,
/// [`CaptureError::TargetNotFound`] if the portal grants a session but hands
/// back no stream, and [`CaptureError::Backend`] for any D-Bus, portal, or
/// `PipeWire` failure.
pub fn open(
    _target: &CaptureTargetInfo,
    config: &CaptureConfig,
) -> Result<Box<dyn ScreenCaptureBackend>, CaptureError> {
    Ok(Box::new(WaylandBackend::new(config)?))
}

// ── Portal handshake ────────────────────────────────────────────────

/// The runtime the portal handshake runs on.
///
/// Varda already has a tokio runtime for the HTTP API, and the portal must not
/// be scheduled on it: `Start` does not return until a human has answered the
/// compositor's dialog, so a capture being chosen slowly would stall every API
/// response behind it. This runtime is dedicated to capture and has one worker.
///
/// It is process-wide and deliberately never dropped. `ashpd` caches its
/// session `zbus::Connection` in a `static`, and that connection's socket task
/// belongs to whichever runtime first created it; tearing the runtime down after
/// the handshake would leave the cached connection undriven for every later
/// capture, and the second `open` of a session would hang instead of failing.
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

/// What the portal granted: the D-Bus objects that keep the cast alive, plus
/// the `PipeWire` endpoint to consume.
struct PortalCast {
    /// The proxy and the session are held for the whole life of the cast on
    /// purpose — the compositor tears the cast down when the session goes away,
    /// so dropping either early kills the stream.
    proxy: Screencast,
    session: Session<Screencast>,
    node_id: u32,
    /// Compositor coordinates, which are *not* pixels on a scaled output. Used
    /// only as a first guess; `param_changed` replaces it with the real size.
    size: Option<(i32, i32)>,
    label: String,
    fd: OwnedFd,
}

/// Name the cast for the deck it lands on.
///
/// The user picked the target in a dialog Varda never saw, so the caption has to
/// be reconstructed from what the portal reports back. The source type is always
/// meaningful; the stream `id` is documented as opaque and some portals fill it
/// with a small integer, so it is only appended when it reads as a name — which
/// is where the portals that do put a connector name ("DP-1") or a window title
/// in there get honoured.
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
                // Both kinds are offered because the picker, not Varda, decides
                // which one the user wants; the library entry does not narrow it.
                .set_sources(SourceType::Monitor | SourceType::Window)
                .set_multiple(false)
                // A restore token would let a reloaded scene skip the dialog, but
                // it only pays off once it is persisted per target, which belongs
                // with the scene format. See spec/screen-capture.md § Open
                // Questions "Portal session persistence".
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

/// Classify a portal failure.
///
/// A dismissed picker is reported as a permission refusal rather than a backend
/// error: it is the one failure the user caused deliberately, and the manager
/// already knows how to render that state without shouting about it.
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

/// Everything the `PipeWire` loop thread and the capture thread both touch.
struct CastState {
    /// Latest-wins frame slot.
    slot: Mutex<Option<CaptureFrame>>,
    /// Live geometry: source rectangle plus delivered size. Swapped wholesale so
    /// the `process` callback never sees a half-applied change and copies a
    /// region that does not match the size it reports.
    geometry: Mutex<Geometry>,
    /// Negotiated pixel size, learned in `param_changed`. Kept so a later
    /// `set_config` can re-resolve the geometry without waiting for the
    /// compositor to renegotiate.
    native: Mutex<(u32, u32)>,
    /// Latest config, so `param_changed` can re-resolve the geometry when the
    /// compositor changes the frame size mid-cast.
    config: Mutex<CaptureConfig>,
    /// Rate gate. Held separately from the config so a discarded frame does not
    /// take the config lock.
    min_interval: Mutex<Duration>,
    last_delivered: Mutex<Option<Instant>>,
    /// Negotiated pixel layout, written by `param_changed` and read by
    /// [`WaylandBackend::pixel_format`] from another thread.
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

    /// Record the format the compositor settled on and re-resolve the geometry
    /// against the real pixel size.
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

    /// Apply a live config change. Nothing here renegotiates the stream — crop,
    /// scale, and rate are all enforced on our side of the buffer.
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

    /// Whether enough time has passed to accept another frame, recording the
    /// delivery if so. Checked before the repack, which is the whole point:
    /// `PipeWire` pushes on the compositor's clock and a discarded frame must
    /// cost only the dequeue.
    fn accept_frame(&self) -> bool {
        let Ok(min_interval) = self.min_interval.lock() else {
            return false;
        };
        let Ok(mut last) = self.last_delivered.lock() else {
            return false;
        };
        let now = Instant::now();
        if let Some(prev) = *last {
            if now.duration_since(prev) < *min_interval {
                return false;
            }
        }
        *last = Some(now);
        true
    }
}

// ── Pixel handling ──────────────────────────────────────────────────

/// Map a negotiated SPA video format onto the texture layout the manager should
/// allocate.
///
/// `None` for anything the offer did not ask for. The compositor is not supposed
/// to pick outside the `EnumFormat`, and guessing a layout for a format we do
/// not understand produces wrong colours rather than an obvious failure.
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

/// Whether the negotiated format actually carries an alpha channel.
///
/// The `x` layouts leave the fourth byte undefined. Uploading it into an
/// `…8UnormSrgb` texture makes the deck randomly transparent, so those bytes are
/// overwritten during the repack.
fn format_has_alpha(format: spa::param::video::VideoFormat) -> bool {
    use spa::param::video::VideoFormat;
    format == VideoFormat::BGRA || format == VideoFormat::RGBA
}

/// Copy the source rectangle of `geometry` out of a strided 4-bytes-per-pixel
/// buffer into a tightly-packed `src_w * 4` frame.
///
/// `stride` is `chunk.stride`, which the producer pads to suit its own
/// allocator; the manager uploads at `width * 4` and nothing else. Cropping
/// happens in the same pass rather than after a full repack, so a crop really
/// does shrink the work instead of only shrinking the upload.
///
/// Returns `None` when the buffer is too short for the rectangle, which is the
/// shape a frame arrives in when a renegotiation is in flight and the geometry
/// still describes the previous size.
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
        for texel in out.chunks_exact_mut(4) {
            texel[3] = 0xFF;
        }
    }
    Some(out)
}

/// Build the `EnumFormat` offered at connect time.
///
/// Only the four 32-bit packed layouts are offered, so a frame maps straight
/// onto a `Bgra8UnormSrgb` or `Rgba8UnormSrgb` texture with no CPU swizzle;
/// `BGRx` leads because that is what every compositor tested produces. Size and
/// framerate are ranges rather than fixed values because the compositor decides
/// what it can actually produce, and a fixed request it cannot meet fails
/// negotiation outright — a black deck instead of a slightly wrong one.
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
/// `MainLoop::quit` is not safe to call from another thread, so the stop travels
/// over `pipewire::channel`, which writes a byte to a pipe the loop already
/// polls. A message sent before `run()` starts is still queued, so a stop that
/// races startup is not lost.
struct Terminate;

/// State the `PipeWire` callbacks share. They all run on the loop thread, so
/// nothing here needs locking between themselves; the locks inside
/// [`CastState`] are for the capture thread.
struct Consumer {
    format: spa::param::video::VideoInfoRaw,
    state: Arc<CastState>,
    warned_dmabuf: bool,
}

/// The `PipeWire` objects backing a live cast. Declared in teardown order: the
/// listener must be unhooked before the stream it points at is destroyed.
struct Cast {
    _listener: pw::stream::StreamListener<Consumer>,
    stream: pw::stream::StreamRc,
    main_loop: pw::main_loop::MainLoopRc,
}

/// Body of the dedicated cast thread.
///
/// Reports the outcome of setup over `ready` so [`WaylandBackend::new`] can fail
/// with a real message instead of returning a backend that will never deliver,
/// then blocks in the loop until [`Terminate`] arrives.
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

/// Record the format the compositor settled on.
///
/// The size in here, not the size the portal advertised, is the pixel size of
/// the buffers: the portal reports compositor coordinates, which differ from
/// pixels on any fractionally scaled output.
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

/// Repack one buffer into the latest-wins slot.
fn on_process(stream: &pw::stream::Stream, consumer: &mut Consumer) {
    // Dequeue first and unconditionally: `Buffer`'s Drop is what returns the
    // buffer to the stream, and a graph that never gets its buffers back stalls.
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

    // `chunk()` borrows the data immutably and `data()` borrows it mutably, so
    // the layout has to be copied out before the mapping is taken.
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

    // Latest-wins, and never blocking: the capture thread may be mid-take, and
    // dropping this frame is cheaper than stalling the compositor's graph.
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
    /// Held for the life of the backend: the compositor ends the cast when the
    /// portal session goes away.
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

        // `Runtime::block_on` panics inside another runtime, and this is called
        // from the engine thread by design. Failing with a message beats taking
        // the process down if a future caller ever moves it onto the API runtime.
        if tokio::runtime::Handle::try_current().is_ok() {
            return Err(CaptureError::Backend(
                "screen capture must be opened from the engine thread, not from inside a tokio runtime"
                    .to_string(),
            ));
        }

        // Synchronous on purpose: the portal picker is a modal choice, and there
        // is nothing sensible for the deck to show until it has been made.
        let PortalCast {
            proxy,
            session,
            node_id,
            size,
            label,
            fd,
        } = portal_runtime()?.block_on(portal_handshake(&config))?;

        // A missing or nonsensical size is not fatal: `param_changed` replaces it
        // with the real one before the first frame, and the geometry resolved here
        // only has to be non-degenerate until then.
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

    /// End the portal session, if it is safe to block here.
    ///
    /// Best effort by design. `block_on` panics inside a runtime and a panic in
    /// `drop` aborts the process, so a teardown that somehow happens on the API
    /// thread skips this instead: the compositor ends the cast when the D-Bus
    /// session goes away regardless, and only the promptness is lost.
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
        // A renegotiation lands asynchronously, so trust the frame over our own
        // bookkeeping.
        self.width = frame.width;
        self.height = frame.height;
        Some(frame)
    }

    fn pixel_format(&self) -> CapturePixelFormat {
        self.state.pixel_format()
    }

    fn is_self_paced(&self) -> bool {
        // The `process` callback enforces `rate`, so the manager's capture thread
        // must oversample rather than run a second clock at the same nominal
        // frequency. See `ScreenCaptureBackend::is_self_paced`.
        true
    }

    fn set_config(&mut self, config: &CaptureConfig) -> Result<(), CaptureError> {
        let config = config.clone().sanitized();
        if config == self.config {
            return Ok(());
        }

        if config.show_cursor != self.config.show_cursor {
            // `CursorMode` is fixed by `SelectSources`, which the portal allows
            // exactly once per session. Changing it means a new picker dialog,
            // which is not something a fader move should raise.
            log::debug!(
                "Screen capture '{}': cursor change takes effect on the next open",
                self.label
            );
        }
        if config.exclude_varda != self.config.exclude_varda {
            // The portal exposes no window-exclusion set, so a display cast that
            // contains Varda will mirror. The UI note next to the toggle is the
            // only mitigation available on this platform.
            log::debug!(
                "Screen capture '{}': exclude_varda has no portal equivalent and is ignored",
                self.label
            );
        }

        // Rate, crop, and scale are all enforced on our side of the buffer, so
        // they apply live without touching the portal session.
        self.state.reconfigure(&config);
        self.config = config;
        Ok(())
    }
}

impl Drop for WaylandBackend {
    fn drop(&mut self) {
        let _ = self.quit.send(Terminate);
        if let Some(thread) = self.thread.take() {
            // The loop wakes on the pipe write, so this join is bounded by one
            // loop iteration rather than by the compositor.
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

    /// A `width × height` BGRA image padded to `stride`, with each texel tagged
    /// by its coordinates so a crop can be located in the output. The padding is
    /// `0xEE` so a repack that leaves it in is visible.
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
        // The label has to read as an action, not as a target name, or the deck
        // ends up captioned with something the user never picked.
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
        // The portal documents `id` as opaque and several implementations use a
        // counter. "Display cast (0)" is a worse caption than "Display cast".
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
        // Every format in the `EnumFormat` must have a mapping, or the
        // compositor can legally pick one we then refuse every frame from.
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
        // The first output texel must be the source texel at (1, 2)…
        assert_eq!(&out[0..3], &[1, 2, 0x10]);
        // …and the last must be (2, 3).
        assert_eq!(&out[12..15], &[2, 3, 0x10]);
    }

    #[test]
    fn repack_forces_alpha_when_the_format_carries_none() {
        let src = padded_frame(2, 2, 8);
        let out = repack_rows(&src, 8, rect(0, 0, 2, 2), true).expect("repack");
        assert!(
            out.chunks_exact(4).all(|texel| texel[3] == 0xFF),
            "BGRx leaves the fourth byte undefined; a deck must not go transparent"
        );
    }

    #[test]
    fn repack_preserves_alpha_when_the_format_carries_it() {
        let src = padded_frame(2, 2, 8);
        let out = repack_rows(&src, 8, rect(0, 0, 2, 2), false).expect("repack");
        assert!(out.chunks_exact(4).all(|texel| texel[3] == 0x00));
    }

    #[test]
    fn repack_rejects_a_buffer_shorter_than_the_region() {
        // A renegotiation in flight delivers exactly this, and reading past the
        // mapping would be a crash rather than a dropped frame.
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
        // The portal reports compositor coordinates; a 2× scaled output delivers
        // twice as many pixels, and a crop resolved against the wrong size reads
        // the wrong half of the screen.
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
        // `BGRx` leads the offer, so the shared texture is allocated BGRA before
        // the first `param_changed` lands.
        let state = CastState::new(&CaptureConfig::default(), (16, 16));
        assert_eq!(state.pixel_format(), CapturePixelFormat::Bgra8UnormSrgb);
        assert_eq!(
            CapturePixelFormat::Bgra8UnormSrgb.wgpu_format(),
            wgpu::TextureFormat::Bgra8UnormSrgb
        );
    }
}
