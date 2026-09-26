//! NDI (Network Device Interface) — send and receive video over LAN.
//!
//! Uses dynamic loading (`libloading`) so the NDI SDK is only required at runtime.
//! Input: a background receive thread copies each captured frame into a reused
//! buffer; the render thread uploads it and converts UYVY on the GPU.

mod convert;
#[allow(non_camel_case_types, non_snake_case, dead_code)]
pub mod ffi;
mod receive;
pub mod sdk;

use std::collections::HashMap;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use crate::engine::value::render::{
    AlphaMode, PresentationCapabilities, PresentationColorProfile, PresentationDepth,
    PresentationFormat, PresentationPixelFormat, PresentationRequest, PresentationTransfer,
    ResolvedPresentation,
};

/// Frame rate declared to receivers when the caller's rate will not fit the
/// SDK's signed field. Only reachable for absurd rates; NDI has no way to say
/// "unspecified", so it needs some honest-looking number.
const DEFAULT_NDI_FPS: i32 = 60;

/// Discovered NDI source on the network.
#[derive(Debug, Clone)]
pub struct NdiSource {
    /// Source name (e.g. "MY-PC (Source 1)")
    pub name: String,
}

/// Borrowed GPU resources for one frame's conversion to what a sender publishes.
pub(crate) struct FrameConversion<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub source: &'a wgpu::TextureView,
    pub width: u32,
    pub height: u32,
    pub dither: bool,
    /// The output's resolved pixel format: P216 or UYVY.
    pub pixel_format: crate::engine::value::render::PresentationPixelFormat,
}

/// Manages NDI discovery, receive, and send.
pub struct NdiManager {
    sdk: Option<sdk::NdiSdk>,
    send_capability: sdk::NdiSendCapability,
    sources: Vec<NdiSource>,
    receivers: Vec<NdiReceiver>,
    /// Each receiver's deck texture, by receiver index.
    targets: Vec<receive::ReceiveTarget>,
    /// Expands UYVY frames; built on the first one.
    unpacker: Option<receive::UyvyUnpacker>,
    /// Active NDI senders keyed by sender name.
    senders: HashMap<String, NdiSender>,
    /// GPU conversion and readback state, keyed by sender name.
    converters: HashMap<String, convert::SendConverter>,
}

struct NdiReceiver {
    #[allow(dead_code)]
    source_name: String,
    frame_data: Arc<receive::FrameSlot>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    #[allow(dead_code)]
    recv_instance: ffi::NDIlib_recv_instance_t,
}

struct NdiSender {
    instance: ffi::NDIlib_send_instance_t,
}

impl Default for NdiManager {
    fn default() -> Self {
        Self::new()
    }
}

impl NdiManager {
    pub fn new() -> Self {
        let mut sdk = sdk::NdiSdk::load();
        if let Some(loaded) = sdk.as_ref() {
            let ok = unsafe { (loaded.initialize)() };
            if ok {
                log::info!(
                    "NDI SDK {} initialized successfully",
                    loaded.runtime_version
                );
            } else {
                log::error!("NDI SDK initialize() returned false");
                // A loaded library that rejected initialization cannot submit
                // either UYVY or P216, so do not advertise runtime capability.
            }
            if !ok {
                sdk = None;
            }
        } else {
            log::info!("NDI SDK not found — NDI features disabled");
        }
        let send_capability = sdk::NdiSendCapability::from_runtime_evidence(
            sdk.as_ref()
                .map(|loaded| loaded.runtime_version.as_str())
                .filter(|version| !version.is_empty()),
            sdk.is_some(),
        );
        Self {
            sdk,
            send_capability,
            sources: Vec::new(),
            receivers: Vec::new(),
            targets: Vec::new(),
            unpacker: None,
            senders: HashMap::new(),
            converters: HashMap::new(),
        }
    }

    /// Create a disabled NDI manager (CLI `--no-ndi` flag).
    pub fn new_disabled() -> Self {
        Self {
            sdk: None,
            send_capability: sdk::NdiSendCapability::from_runtime_evidence(None, false),
            sources: Vec::new(),
            receivers: Vec::new(),
            targets: Vec::new(),
            unpacker: None,
            senders: HashMap::new(),
            converters: HashMap::new(),
        }
    }

    pub fn is_available(&self) -> bool {
        self.sdk.is_some()
    }

    /// Resolve an NDI presentation request from runtime evidence.
    ///
    /// NDI 6 plus the resolved v2 submission symbol enables P216. Older or
    /// unidentified runtimes retain the byte-compatible UYVY fallback.
    pub fn resolve_presentation(&self, request: PresentationRequest) -> ResolvedPresentation {
        Self::resolve_presentation_for_capability(request, self.send_capability)
    }

    /// Every mode with the reason this sender cannot deliver it, from the live
    /// send capability.
    ///
    /// NDI is the one headless target whose capability is not derivable from the
    /// target alone: it depends on the runtime the SDK loaded. So this comes from
    /// here, exactly as `resolved_presentation` already does.
    /// See /spec/presentation-mode-offering.md.
    #[must_use]
    pub fn mode_availability(&self) -> Vec<crate::engine::value::render::ModeAvailability> {
        Self::capabilities_for(self.send_capability).mode_availability()
    }

    fn resolve_presentation_for_capability(
        request: PresentationRequest,
        capability: sdk::NdiSendCapability,
    ) -> ResolvedPresentation {
        Self::capabilities_for(capability)
            .resolve(request)
            .expect("NDI always provides the UYVY fallback")
    }

    fn capabilities_for(capability: sdk::NdiSendCapability) -> PresentationCapabilities {
        let mut formats = Vec::with_capacity(2);
        if capability.p216_confirmed() {
            formats.push(PresentationFormat {
                depth: PresentationDepth::Sdr10,
                transfer: PresentationTransfer::Sdr,
                pixel_format: PresentationPixelFormat::P216,
                color_profile: PresentationColorProfile::Rec709Limited,
                alpha_mode: AlphaMode::Opaque,
            });
        }
        formats.push(PresentationFormat {
            depth: PresentationDepth::Sdr8,
            transfer: PresentationTransfer::Sdr,
            pixel_format: PresentationPixelFormat::Uyvy,
            color_profile: PresentationColorProfile::Rec709Limited,
            alpha_mode: AlphaMode::Opaque,
        });

        PresentationCapabilities::new(formats, Some(capability.p216_unavailable_reason()))
    }

    pub fn sources(&self) -> &[NdiSource] {
        &self.sources
    }

    /// Return discovered source names for UI display.
    pub fn discovered_sources(&self) -> Vec<String> {
        self.sources.iter().map(|s| s.name.clone()).collect()
    }

    /// Scan for NDI sources on the network (2s timeout).
    pub fn discover(&mut self) {
        let Some(sdk) = &self.sdk else {
            return;
        };
        self.sources.clear();

        unsafe {
            let find_settings = ffi::NDIlib_find_create_t::default();
            let finder = (sdk.find_create_v2)(&raw const find_settings);
            if finder.is_null() {
                log::warn!("NDI find_create_v2 returned null");
                return;
            }

            // Wait up to 2 seconds for sources to appear
            (sdk.find_wait_for_sources)(finder, 2000);

            let mut count: std::os::raw::c_uint = 0;
            let sources_ptr = (sdk.find_get_current_sources)(finder, &raw mut count);

            if !sources_ptr.is_null() && count > 0 {
                let sources_slice = std::slice::from_raw_parts(sources_ptr, count as usize);
                for src in sources_slice {
                    if !src.p_ndi_name.is_null() {
                        let name = std::ffi::CStr::from_ptr(src.p_ndi_name)
                            .to_string_lossy()
                            .into_owned();
                        self.sources.push(NdiSource { name });
                    }
                }
            }
            log::info!("NDI discovery found {} sources", self.sources.len());
            (sdk.find_destroy)(finder);
        }
    }

    /// Start receiving from a named NDI source.
    /// Spawns a background thread that captures frames into shared memory.
    ///
    /// # Panics
    ///
    /// Panics only if the fixed receiver-name literal `"Varda Receiver"` cannot be
    /// converted to a `CString`, which is impossible for that literal.
    pub fn start_receive(&mut self, source_name: &str, device: &wgpu::Device) -> Option<usize> {
        let Some(sdk) = &self.sdk else {
            log::warn!("Cannot receive NDI: SDK not available");
            return None;
        };

        let frame_data = Arc::new(receive::FrameSlot::default());
        let stop_flag = Arc::new(AtomicBool::new(false));
        let (width, height) = (1920u32, 1080u32);

        // Create the NDI source struct
        let name_c = std::ffi::CString::new(source_name).ok()?;
        let ndi_source = ffi::NDIlib_source_t {
            p_ndi_name: name_c.as_ptr(),
            p_url_address: std::ptr::null(),
        };

        let recv_name = std::ffi::CString::new("Varda Receiver").unwrap();
        let recv_settings = ffi::NDIlib_recv_create_v3_t {
            source_to_connect_to: ndi_source,
            color_format: receive::RECEIVE_COLOR_FORMAT,
            bandwidth: 100, // highest quality
            allow_video_fields: false,
            p_ndi_recv_name: recv_name.as_ptr(),
        };

        let recv_instance = unsafe { (sdk.recv_create_v3)(&raw const recv_settings) };
        if recv_instance.is_null() {
            log::error!("NDI recv_create_v3 returned null for '{source_name}'");
            return None;
        }

        let target = receive::ReceiveTarget::new(
            device,
            &format!("NDI Receive: {source_name}"),
            width,
            height,
        );

        let connected = Arc::new(AtomicBool::new(false));

        // Spawn background receive thread
        let frame_clone = Arc::clone(&frame_data);
        let stop_clone = Arc::clone(&stop_flag);
        let connected_clone = Arc::clone(&connected);
        // Note: recv_instance is a raw pointer, sent across thread boundary.
        // Safe because NDI SDK guarantees thread safety for recv instances.
        let recv_ptr = recv_instance as usize;
        let recv_destroy_fn = sdk.recv_destroy as usize;
        let recv_capture_fn = sdk.recv_capture_v3 as usize;
        let recv_free_fn = sdk.recv_free_video_v2 as usize;

        let source_name_log = source_name.to_string();
        let thread = std::thread::Builder::new()
            .name(format!("ndi-recv-{source_name}"))
            .spawn(move || {
                let recv = recv_ptr as ffi::NDIlib_recv_instance_t;
                let capture_fn: unsafe extern "C" fn(ffi::NDIlib_recv_instance_t, *mut ffi::NDIlib_video_frame_v2_t, *mut std::ffi::c_void, *mut std::ffi::c_void, std::os::raw::c_uint) -> ffi::NDIlib_frame_type_e
                    = unsafe { std::mem::transmute(recv_capture_fn) };
                let free_fn: unsafe extern "C" fn(ffi::NDIlib_recv_instance_t, *const ffi::NDIlib_video_frame_v2_t)
                    = unsafe { std::mem::transmute(recv_free_fn) };

                let mut frame_count: u64 = 0;
                let mut none_count: u64 = 0;
                let mut warned_fourcc = false;

                while !stop_clone.load(Ordering::SeqCst) {
                    let mut video_frame = std::mem::MaybeUninit::<ffi::NDIlib_video_frame_v2_t>::zeroed();
                    let frame_type = unsafe {
                        capture_fn(recv, video_frame.as_mut_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), 100)
                    };

                    if frame_type == ffi::NDIlib_frame_type_e::VIDEO {
                        let vf = unsafe { video_frame.assume_init() };
                        if !vf.p_data.is_null() && vf.xres > 0 && vf.yres > 0 {
                            if frame_count == 0 {
                                log::info!("NDI '{}': first frame {}×{} FourCC={:?} stride={}", source_name_log, vf.xres, vf.yres, vf.FourCC, vf.line_stride_in_bytes);
                            }
                            frame_count += 1;
                            none_count = 0;
                            connected_clone.store(true, Ordering::SeqCst);
                            if !frame_clone.deposit(&vf) && !warned_fourcc {
                                warned_fourcc = true;
                                log::warn!("NDI '{source_name_log}': unexpected FourCC {:?}, frames dropped", vf.FourCC);
                            }
                        }
                        unsafe { free_fn(recv, &raw const vf) };
                    } else if frame_type == ffi::NDIlib_frame_type_e::NONE {
                        none_count += 1;
                        if none_count == 30 {
                            log::warn!("NDI '{source_name_log}': 30 consecutive empty captures — source may not be sending");
                        }
                        if none_count >= 50 {
                            connected_clone.store(false, Ordering::SeqCst);
                        }
                    } else if frame_type == ffi::NDIlib_frame_type_e::STATUS_CHANGE {
                        log::info!("NDI '{source_name_log}': connection status changed");
                    } else if frame_type == ffi::NDIlib_frame_type_e::ERROR {
                        log::warn!("NDI '{source_name_log}': received ERROR frame");
                        connected_clone.store(false, Ordering::SeqCst);
                    }
                }

                log::info!("NDI '{source_name_log}': receive thread stopping (captured {frame_count} frames)");
                let destroy_fn: unsafe extern "C" fn(ffi::NDIlib_recv_instance_t)
                    = unsafe { std::mem::transmute(recv_destroy_fn) };
                unsafe { destroy_fn(recv) };
            })
            .ok();

        let idx = self.receivers.len();
        self.receivers.push(NdiReceiver {
            source_name: source_name.to_string(),
            frame_data,
            stop_flag,
            connected,
            thread,
            recv_instance: std::ptr::null_mut(), // Owned by the thread now
        });
        self.targets.push(target);
        log::info!("NDI receiver started for '{source_name}'");
        Some(idx)
    }

    /// Put the latest frame from each receiver into its deck texture,
    /// resizing the texture when the received resolution changes. Returns the
    /// GPU work that converts UYVY frames, for the caller to submit.
    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Option<wgpu::CommandBuffer> {
        let mut encoder = None;
        for (receiver, target) in self.receivers.iter().zip(&mut self.targets) {
            target.update(
                &receiver.frame_data,
                device,
                queue,
                &mut self.unpacker,
                &mut encoder,
            );
        }
        encoder.map(wgpu::CommandEncoder::finish)
    }

    pub fn texture_view(&self, idx: usize) -> Option<&wgpu::TextureView> {
        self.targets.get(idx).map(receive::ReceiveTarget::view)
    }

    pub fn receiver_dimensions(&self, idx: usize) -> Option<(u32, u32)> {
        self.targets
            .get(idx)
            .map(receive::ReceiveTarget::dimensions)
    }

    /// Check if a receiver is currently connected (receiving video frames).
    /// Returns `false` for out-of-bounds indices.
    pub fn is_connected(&self, idx: usize) -> bool {
        self.receivers
            .get(idx)
            .is_some_and(|r| r.connected.load(Ordering::SeqCst))
    }

    /// Convert a rendered output to the sender's pixel format on the GPU and
    /// enqueue asynchronous readback. When both readback slots are busy the
    /// frame is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error when the format is P216 and P216 is unavailable, when
    /// the format is neither P216 nor UYVY, or when the dimensions cannot form a
    /// valid even-width frame.
    pub(crate) fn begin_frame(
        &mut self,
        sender_name: &str,
        conversion: FrameConversion<'_>,
    ) -> Result<(), String> {
        use crate::engine::value::render::PresentationPixelFormat;
        let FrameConversion {
            device,
            queue,
            encoder,
            source,
            width,
            height,
            dither,
            pixel_format,
        } = conversion;
        let format = match pixel_format {
            PresentationPixelFormat::P216 => {
                if !self.send_capability.p216_confirmed() {
                    return Err(self.send_capability.p216_unavailable_reason());
                }
                convert::SendFormat::P216
            }
            PresentationPixelFormat::Uyvy => convert::SendFormat::Uyvy,
            other => return Err(format!("NDI cannot send {other:?}")),
        };
        let recreate = self.converters.get(sender_name).is_none_or(|converter| {
            converter.dimensions() != (width, height) || converter.format() != format
        });
        if recreate {
            let converter = convert::SendConverter::new(device, format, width, height)
                .map_err(|error| error.to_string())?;
            self.converters.insert(sender_name.to_string(), converter);
        }
        if let Some(converter) = self.converters.get_mut(sender_name) {
            let _ = converter.encode(device, queue, encoder, source, dither);
        }
        Ok(())
    }

    /// The oldest completed conversion for a sender, with its format and row
    /// stride, if one has finished.
    fn try_read_frame(
        &mut self,
        sender_name: &str,
        device: &wgpu::Device,
    ) -> Option<(Vec<u8>, convert::SendFormat, u32)> {
        let converter = self.converters.get_mut(sender_name)?;
        let (format, stride) = (converter.format(), converter.stride());
        converter
            .try_read(device)
            .map(|bytes| (bytes, format, stride))
    }

    /// Collect a finished conversion without publishing it, returning its size.
    /// For benchmarking the render thread's side of a send.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub(crate) fn read_converted(
        &mut self,
        sender_name: &str,
        device: &wgpu::Device,
    ) -> Option<usize> {
        self.try_read_frame(sender_name, device)
            .map(|(bytes, _, _)| bytes.len())
    }

    /// Publish the oldest completed frame through NDI.
    ///
    /// `fps` is the master render rate, declared to receivers as this sender's
    /// frame rate. Pass it already resolved (see `app::state::encoder_fps`), so
    /// an uncapped stage still declares a real rate rather than zero.
    ///
    /// # Errors
    ///
    /// Returns an error when sender creation or frame-contract validation fails.
    pub(crate) fn try_send(
        &mut self,
        sender_name: &str,
        device: &wgpu::Device,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<(), String> {
        let Some((mut bytes, format, stride)) = self.try_read_frame(sender_name, device) else {
            return Ok(());
        };
        if !self.ensure_sender(sender_name) {
            return Err(format!("NDI sender '{sender_name}' could not be created"));
        }
        let Some(sdk) = &self.sdk else {
            return Err("NDI runtime is unavailable".to_string());
        };
        let Some(sender) = self.senders.get(sender_name) else {
            return Err(format!("NDI sender '{sender_name}' disappeared"));
        };
        submit_with(
            format,
            &mut bytes,
            width,
            height,
            stride,
            fps,
            |frame| unsafe {
                (sdk.send_send_video_v2)(sender.instance, frame);
            },
        )
        .map_err(|error| error.to_string())
    }

    fn ensure_sender(&mut self, sender_name: &str) -> bool {
        if self.senders.contains_key(sender_name) {
            return true;
        }
        let Some(sdk) = &self.sdk else {
            return false;
        };
        let Ok(name_c) = std::ffi::CString::new(sender_name) else {
            return false;
        };
        // Varda's render loop is the clock. Enabling NDI clocking would block
        // the render thread inside the submission call.
        let settings = ffi::NDIlib_send_create_t {
            p_ndi_name: name_c.as_ptr(),
            p_groups: std::ptr::null(),
            clock_video: false,
            clock_audio: false,
        };
        let instance = unsafe { (sdk.send_create)(&raw const settings) };
        if instance.is_null() {
            log::error!("NDI send_create returned null for '{sender_name}'");
            return false;
        }
        self.senders
            .insert(sender_name.to_string(), NdiSender { instance });
        log::info!("NDI sender created: '{sender_name}'");
        true
    }

    /// Destroy a specific sender by name.
    pub fn destroy_sender(&mut self, sender_name: &str) {
        self.converters.remove(sender_name);
        if let Some(sender) = self.senders.remove(sender_name) {
            if let Some(ref sdk) = self.sdk {
                unsafe { (sdk.send_destroy)(sender.instance) };
            }
            log::info!("NDI sender destroyed: '{sender_name}'");
        }
    }

    /// A receiver with no network connection, fed by `receive_for_test`.
    /// For benchmarking and testing the receive path without the SDK.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub(crate) fn add_test_receiver(&mut self, device: &wgpu::Device) -> usize {
        self.receivers.push(NdiReceiver {
            source_name: "test".to_string(),
            frame_data: Arc::new(receive::FrameSlot::default()),
            stop_flag: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(true)),
            thread: None,
            recv_instance: std::ptr::null_mut(),
        });
        self.targets.push(receive::ReceiveTarget::new(
            device,
            "NDI Receive (test)",
            1920,
            1080,
        ));
        self.receivers.len() - 1
    }

    /// Run the receive thread's handling of `vf` for receiver `idx`.
    #[cfg(any(test, feature = "test-fixtures"))]
    pub(crate) fn receive_for_test(&self, idx: usize, vf: &ffi::NDIlib_video_frame_v2_t) {
        self.receivers[idx].frame_data.deposit(vf);
    }

    pub fn stop_receive(&mut self, idx: usize) {
        if let Some(r) = self.receivers.get_mut(idx) {
            r.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = r.thread.take() {
                let _ = t.join();
            }
        }
    }
}

fn submit_with(
    format: convert::SendFormat,
    data: &mut [u8],
    width: u32,
    height: u32,
    stride: u32,
    fps: u32,
    mut submit: impl FnMut(*const ffi::NDIlib_video_frame_v2_t),
) -> Result<(), convert::ConvertError> {
    let (expected_stride, byte_len) = format.layout(width, height)?;
    if stride != expected_stride || data.len() as u64 != byte_len {
        return Err(convert::ConvertError::InvalidBuffer {
            expected: byte_len,
            actual: data.len(),
        });
    }
    // The NDI C ABI takes signed dimensions; frame sizes come from GPU
    // textures and are orders of magnitude below i32::MAX, so clamp rather
    // than let an implausible value wrap into a negative extent.
    let frame = ffi::NDIlib_video_frame_v2_t {
        xres: i32::try_from(width).unwrap_or(i32::MAX),
        yres: i32::try_from(height).unwrap_or(i32::MAX),
        FourCC: match format {
            convert::SendFormat::P216 => ffi::NDIlib_FourCC_video_type_e::P216,
            convert::SendFormat::Uyvy => ffi::NDIlib_FourCC_video_type_e::UYVY,
        },
        frame_rate_N: i32::try_from(fps).unwrap_or(DEFAULT_NDI_FPS).max(1),
        frame_rate_D: 1,
        picture_aspect_ratio: 0.0,
        frame_format_type: 1, // progressive
        timecode: i64::MAX,   // synthesize
        p_data: data.as_mut_ptr(),
        line_stride_in_bytes: i32::try_from(stride).unwrap_or(i32::MAX),
        p_metadata: convert::REC709_METADATA.as_ptr().cast(),
        timestamp: 0,
    };
    submit(&raw const frame);
    Ok(())
}

impl Drop for NdiManager {
    fn drop(&mut self) {
        // Stop all receivers and join their threads before SDK cleanup
        for r in &mut self.receivers {
            r.stop_flag.store(true, Ordering::SeqCst);
            if let Some(t) = r.thread.take() {
                let _ = t.join();
            }
        }
        // Destroy all senders
        let sender_names: Vec<String> = self.senders.keys().cloned().collect();
        for name in sender_names {
            self.destroy_sender(&name);
        }
        // Destroy NDI SDK
        if let Some(ref sdk) = self.sdk {
            unsafe { (sdk.destroy)() };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::value::render::{PresentationDepth, PresentationPixelFormat};

    #[test]
    fn ndi_manager_new_no_crash() {
        // NdiManager::new() attempts to load SDK dynamically.
        // On machines without NDI SDK, it gracefully returns with sdk=None.
        let mgr = NdiManager::new();
        // is_available depends on whether SDK is installed — just verify no panic
        let _ = mgr.is_available();
    }

    #[test]
    fn ndi_manager_sources_empty() {
        let mgr = NdiManager::new();
        assert!(mgr.sources().is_empty());
        assert!(mgr.discovered_sources().is_empty());
    }

    #[test]
    fn ndi_manager_texture_view_out_of_bounds() {
        let mgr = NdiManager::new();
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.texture_view(999).is_none());
    }

    #[test]
    fn ndi_manager_receiver_dimensions_out_of_bounds() {
        let mgr = NdiManager::new();
        assert!(mgr.receiver_dimensions(0).is_none());
    }

    #[test]
    fn ndi_manager_is_connected_out_of_bounds() {
        let mgr = NdiManager::new();
        assert!(!mgr.is_connected(0));
        assert!(!mgr.is_connected(999));
    }

    #[test]
    fn ndi_source_debug() {
        let src = NdiSource {
            name: "Test Source".to_string(),
        };
        let debug = format!("{src:?}");
        assert!(debug.contains("Test Source"));
    }

    #[test]
    fn ndi_source_clone() {
        let src = NdiSource {
            name: "Test".to_string(),
        };
        let cloned = src.clone();
        assert_eq!(src.name, cloned.name);
    }

    #[test]
    fn ndi_six_request_resolves_to_p216() {
        let request = PresentationRequest {
            depth: PresentationDepth::Sdr10,
            dither: true,
            ..PresentationRequest::default()
        };
        let capability = sdk::NdiSendCapability::from_runtime_evidence(Some("NDI SDK 6.3.1"), true);

        let resolved = NdiManager::resolve_presentation_for_capability(request, capability);

        assert_eq!(resolved.requested, PresentationDepth::Sdr10);
        assert_eq!(resolved.resolved, PresentationDepth::Sdr10);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::P216);
        assert_eq!(
            resolved.color_profile,
            PresentationColorProfile::Rec709Limited
        );
        assert_eq!(resolved.alpha_mode, AlphaMode::Opaque);
        assert!(resolved.fallback_reason.is_none());
    }

    #[test]
    fn eight_bit_request_remains_uyvy_without_fallback_warning() {
        let capability = sdk::NdiSendCapability::from_runtime_evidence(Some("NDI SDK 6.3.1"), true);

        let resolved = NdiManager::resolve_presentation_for_capability(
            PresentationRequest::default(),
            capability,
        );

        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Uyvy);
        assert!(resolved.fallback_reason.is_none());
    }

    #[test]
    fn pre_ndi_six_request_resolves_to_uyvy_fallback() {
        let request = PresentationRequest {
            depth: PresentationDepth::Sdr10,
            dither: true,
            ..PresentationRequest::default()
        };
        let capability = sdk::NdiSendCapability::from_runtime_evidence(Some("NDI SDK 5.6.0"), true);

        let resolved = NdiManager::resolve_presentation_for_capability(request, capability);

        assert_eq!(resolved.resolved, PresentationDepth::Sdr8);
        assert_eq!(resolved.pixel_format, PresentationPixelFormat::Uyvy);
        assert!(
            resolved
                .fallback_reason
                .as_deref()
                .unwrap()
                .contains("predates")
        );
    }

    #[test]
    fn mock_sender_receives_p216_contract_and_planes() {
        let mut bytes = vec![
            0x00, 0x10, 0x00, 0xeb, // Y: limited black, limited white
            0x00, 0x80, 0x00, 0x80, // U,V: neutral
        ];
        let mut captured = None;

        submit_with(
            convert::SendFormat::P216,
            &mut bytes,
            2,
            1,
            4,
            60,
            |frame_ptr| {
                let frame = unsafe { &*frame_ptr };
                let metadata = unsafe { std::ffi::CStr::from_ptr(frame.p_metadata) }
                    .to_str()
                    .unwrap()
                    .to_string();
                let submitted = unsafe { std::slice::from_raw_parts(frame.p_data, 8) }.to_vec();
                captured = Some((
                    frame.FourCC,
                    frame.line_stride_in_bytes,
                    metadata,
                    submitted,
                ));
            },
        )
        .unwrap();

        let (fourcc, stride, metadata, submitted) = captured.unwrap();
        assert_eq!(fourcc, ffi::NDIlib_FourCC_video_type_e::P216);
        assert_eq!(stride, 4);
        assert!(metadata.contains("primaries=\"bt_709\""));
        assert_eq!(submitted, bytes);
    }

    /// Eight-bit UYVY goes out with the same BT.709 declaration as P216.
    #[test]
    fn mock_sender_receives_uyvy_with_the_rec709_declaration() {
        let mut bytes = vec![128, 16, 128, 235];
        let mut captured = None;
        submit_with(
            convert::SendFormat::Uyvy,
            &mut bytes,
            2,
            1,
            4,
            60,
            |frame_ptr| {
                let frame = unsafe { &*frame_ptr };
                let metadata = unsafe { std::ffi::CStr::from_ptr(frame.p_metadata) }
                    .to_str()
                    .unwrap()
                    .to_string();
                captured = Some((frame.FourCC, frame.line_stride_in_bytes, metadata));
            },
        )
        .unwrap();
        let (fourcc, stride, metadata) = captured.unwrap();
        assert_eq!(fourcc, ffi::NDIlib_FourCC_video_type_e::UYVY);
        assert_eq!(stride, 4);
        assert!(metadata.contains("matrix=\"bt_709\""));
        assert!(submit_with(convert::SendFormat::Uyvy, &mut bytes, 2, 2, 4, 60, |_| {}).is_err());
    }
}
