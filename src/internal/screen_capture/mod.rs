//! Screen and window capture manager.
//!
//! Like `CameraManager`: each captured display or window has one capture
//! thread and one shared GPU texture that any number of decks read. The thread
//! publishes into an `Arc<Mutex<Option<CaptureFrame>>>`; the render thread
//! does a non-blocking `try_lock` and `write_texture`.
//!
//! Sessions are keyed by a [`CaptureId`] assigned at `open`, not by an index
//! into the target list, because a rescan reorders that list.

pub mod backend;
pub mod platform;
pub mod provider;
pub mod resample;

use backend::{
    CaptureConfig, CaptureError, CaptureFrame, CapturePixelFormat, CaptureTargetInfo,
    CaptureTargetKind, MockBackend, PermissionState, ScreenCaptureBackend, TargetIdentity,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Opaque handle to an open capture session.
pub type CaptureId = u32;

/// Session id of a capture deck whose saved target is not on screen. The deck
/// keeps its effect chain, opacity and MIDI mappings and renders black until
/// repointed. The manager never assigns this id.
pub const UNBOUND_CAPTURE_ID: CaptureId = CaptureId::MAX;

/// An open capture session and its shared GPU texture.
struct ActiveCapture {
    /// Handle-free identity, used to reuse an existing session on `open`.
    identity: TargetIdentity,
    label: String,
    texture: wgpu::Texture,
    texture_view: wgpu::TextureView,
    width: u32,
    height: u32,
    format: CapturePixelFormat,
    /// Number of decks using this capture.
    ref_count: u32,
    /// Latest frame. The capture thread swaps it in, the render thread takes it.
    frame_data: Arc<Mutex<Option<CaptureFrame>>>,
    /// Live config, re-read by the capture thread each tick.
    config: Arc<Mutex<CaptureConfig>>,
    stop_flag: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

/// Target enumeration, capture sessions and shared textures.
pub struct ScreenCaptureManager {
    targets: Vec<CaptureTargetInfo>,
    active: HashMap<CaptureId, ActiveCapture>,
    next_id: CaptureId,
    permission: PermissionState,
    disabled: bool,
}

impl Default for ScreenCaptureManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenCaptureManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            targets: Vec::new(),
            active: HashMap::new(),
            next_id: 0,
            permission: platform::permission_state(),
            disabled: false,
        };
        mgr.scan_targets();
        mgr
    }

    /// Disabled manager for the `--no-screen-capture` flag.
    pub fn new_disabled() -> Self {
        Self {
            targets: Vec::new(),
            active: HashMap::new(),
            next_id: 0,
            permission: PermissionState::NotRequired,
            disabled: true,
        }
    }

    /// Whether a capture can be opened.
    ///
    /// Checks for a real backend, not the cargo feature: the feature is on
    /// everywhere, but a platform without a backend would list capture and then
    /// fail every open.
    pub fn is_available(&self) -> bool {
        !self.disabled && platform::backend_name() != platform::unsupported::backend_name()
    }

    pub fn backend_name(&self) -> &'static str {
        if self.disabled {
            "disabled"
        } else {
            platform::backend_name()
        }
    }

    pub fn permission_state(&self) -> PermissionState {
        self.permission
    }

    /// Asks the OS for capture permission and refreshes the cached state.
    pub fn request_permission(&mut self) {
        if self.disabled {
            return;
        }
        platform::request_permission();
        self.permission = platform::permission_state();
    }

    /// Re-enumerates displays and windows. Manual only; window lists change
    /// constantly and polling would churn the library panel.
    pub fn scan_targets(&mut self) {
        if self.disabled {
            self.targets.clear();
            return;
        }
        self.permission = platform::permission_state();
        match platform::enumerate() {
            Ok(targets) => {
                log::info!(
                    "Screen capture scan: {} target(s) via {}",
                    targets.len(),
                    platform::backend_name()
                );
                self.targets = targets;
            }
            Err(e) => {
                log::warn!("Screen capture enumeration failed: {e}");
                self.targets.clear();
            }
        }
    }

    pub fn targets(&self) -> &[CaptureTargetInfo] {
        &self.targets
    }

    /// Finds an enumerated target by its handle-free identity, so a restored
    /// scene can rebind to a live target.
    ///
    /// Windows match on `(app, title)` first, then on `app` alone if exactly one
    /// window of that app exists. Retitled windows are common; with two windows of
    /// one app there is no way to choose, so the fallback fails.
    pub fn find_target(&self, identity: &TargetIdentity) -> Option<&CaptureTargetInfo> {
        if let Some(exact) = self.targets.iter().find(|t| &t.identity() == identity) {
            return Some(exact);
        }
        match identity {
            TargetIdentity::Display { .. } => None,
            TargetIdentity::Window { app, .. } => {
                let mut matches = self
                    .targets
                    .iter()
                    .filter(|t| t.kind == CaptureTargetKind::Window)
                    .filter(|t| t.app.as_deref() == Some(app.as_str()));
                let first = matches.next()?;
                if matches.next().is_some() {
                    None
                } else {
                    Some(first)
                }
            }
        }
    }

    /// Opens a capture and starts its thread. An identical target already open
    /// gets its ref count incremented and its session reused.
    ///
    /// Returns `(id, width, height)` of the shared texture.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Unavailable`] when disabled or unsupported,
    /// [`CaptureError::PermissionDenied`] when the OS refuses, or
    /// [`CaptureError::Backend`] if the session or its thread cannot start.
    pub fn open(
        &mut self,
        target: &CaptureTargetInfo,
        config: CaptureConfig,
        device: &wgpu::Device,
    ) -> Result<(CaptureId, u32, u32), CaptureError> {
        if self.disabled {
            return Err(CaptureError::Unavailable(
                "screen capture disabled via --no-screen-capture".into(),
            ));
        }
        if !self.permission.allows_capture() {
            // Re-check instead of using the startup state: on macOS the user may
            // have granted access in System Settings since.
            self.permission = platform::permission_state();
            if !self.permission.allows_capture() {
                return Err(CaptureError::PermissionDenied);
            }
        }

        let identity = target.identity();
        if let Some((id, active)) = self
            .active
            .iter_mut()
            .find(|(_, a)| a.identity == identity)
            .map(|(id, a)| (*id, a))
        {
            active.ref_count += 1;
            return Ok((id, active.width, active.height));
        }

        let config = config.sanitized();
        let session = platform::open(target, &config)?;
        self.start_session(target, session, config, device)
    }

    /// Opens a synthetic capture. Always available; used by tests, benches and
    /// headless CI, which have no display server or permission.
    ///
    /// # Errors
    ///
    /// Returns [`CaptureError::Backend`] if the capture thread cannot spawn.
    pub fn open_mock(
        &mut self,
        target: &CaptureTargetInfo,
        config: CaptureConfig,
        device: &wgpu::Device,
    ) -> Result<(CaptureId, u32, u32), CaptureError> {
        let identity = target.identity();
        if let Some((id, active)) = self
            .active
            .iter_mut()
            .find(|(_, a)| a.identity == identity)
            .map(|(id, a)| (*id, a))
        {
            active.ref_count += 1;
            return Ok((id, active.width, active.height));
        }
        let config = config.sanitized();
        let session: Box<dyn ScreenCaptureBackend> = Box::new(MockBackend::new(
            target.label.clone(),
            target.width,
            target.height,
            config.clone(),
        ));
        self.start_session(target, session, config, device)
    }

    fn start_session(
        &mut self,
        target: &CaptureTargetInfo,
        mut session: Box<dyn ScreenCaptureBackend>,
        config: CaptureConfig,
        device: &wgpu::Device,
    ) -> Result<(CaptureId, u32, u32), CaptureError> {
        let (width, height) = {
            let (w, h) = session.resolution();
            (w.max(1), h.max(1))
        };
        // Allocate the texture in the backend's layout so no CPU swizzle is
        // needed. Use an arrived frame's format if there is one, otherwise the
        // backend's declared format.
        let first = session.next_frame();
        let format = first
            .as_ref()
            .map_or_else(|| session.pixel_format(), |f| f.format);

        let id = self.next_id;
        // Skip the unbound sentinel so an unbound deck never matches a live session.
        self.next_id = self.next_id.wrapping_add(1);
        if self.next_id == UNBOUND_CAPTURE_ID {
            self.next_id = 0;
        }

        let (texture, texture_view) =
            Self::create_texture(device, id, &target.label, width, height, format);

        let frame_data: Arc<Mutex<Option<CaptureFrame>>> = Arc::new(Mutex::new(first));
        let config = Arc::new(Mutex::new(config));
        let stop_flag = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));

        let frame_tx = Arc::clone(&frame_data);
        let config_rx = Arc::clone(&config);
        let stop_rx = Arc::clone(&stop_flag);
        let connected_tx = Arc::clone(&connected);

        let thread = std::thread::Builder::new()
            .name(format!("screen-capture-{id}"))
            .spawn(move || {
                capture_loop(
                    session.as_mut(),
                    id,
                    &frame_tx,
                    &config_rx,
                    &stop_rx,
                    &connected_tx,
                );
            })
            .map_err(|e| CaptureError::Backend(format!("failed to spawn capture thread: {e}")))?;

        log::info!(
            "Opened screen capture {id}: '{}' {width}x{height} ({:?})",
            target.label,
            format
        );

        self.active.insert(
            id,
            ActiveCapture {
                identity: target.identity(),
                label: target.label.clone(),
                texture,
                texture_view,
                width,
                height,
                format,
                ref_count: 1,
                frame_data,
                config,
                stop_flag,
                connected,
                thread: Some(thread),
            },
        );

        Ok((id, width, height))
    }

    fn create_texture(
        device: &wgpu::Device,
        id: CaptureId,
        label: &str,
        width: u32,
        height: u32,
        format: CapturePixelFormat,
    ) -> (wgpu::Texture, wgpu::TextureView) {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(&format!("ScreenCapture {id} ({label})")),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: format.wgpu_format(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view)
    }

    /// Releases a capture reference. Stops the thread when the count hits zero.
    pub fn release(&mut self, id: CaptureId) {
        let Some(active) = self.active.get_mut(&id) else {
            return;
        };
        active.ref_count = active.ref_count.saturating_sub(1);
        if active.ref_count > 0 {
            return;
        }
        log::info!("Closing screen capture {id} (no more references)");
        let Some(mut removed) = self.active.remove(&id) else {
            return;
        };
        removed.stop_flag.store(true, Ordering::Relaxed);
        if let Some(t) = removed.thread.take() {
            let _ = t.join();
        }
    }

    /// Reconciles live sessions with the decks that hold them.
    ///
    /// `holders` maps each capture id to the number of decks using it now.
    /// Sessions no deck holds are stopped, and the rest get their ref count reset
    /// to the holder count.
    ///
    /// Needed because `open`/`release` pair only on the explicit remove-deck path.
    /// Decks also disappear through scene diffs, undo, removed or truncated
    /// channels and scene loads, which would leave capture threads running.
    ///
    /// Safe to call every frame: a session is opened and attached to its deck
    /// within one command.
    pub fn reconcile_holders(&mut self, holders: &HashMap<CaptureId, u32>) {
        let orphans: Vec<CaptureId> = self
            .active
            .keys()
            .copied()
            .filter(|id| !holders.contains_key(id))
            .collect();
        for id in orphans {
            let Some(mut removed) = self.active.remove(&id) else {
                continue;
            };
            log::warn!(
                "Screen capture {id} ('{}') outlived its last deck — stopping it",
                removed.label
            );
            removed.stop_flag.store(true, Ordering::Relaxed);
            if let Some(t) = removed.thread.take() {
                let _ = t.join();
            }
        }
        for (id, active) in &mut self.active {
            if let Some(&count) = holders.get(id) {
                active.ref_count = count;
            }
        }
    }

    /// Shared texture view that decks read.
    pub fn texture_view(&self, id: CaptureId) -> Option<&wgpu::TextureView> {
        self.active.get(&id).map(|a| &a.texture_view)
    }

    pub fn resolution(&self, id: CaptureId) -> Option<(u32, u32)> {
        self.active.get(&id).map(|a| (a.width, a.height))
    }

    pub fn label(&self, id: CaptureId) -> Option<&str> {
        self.active.get(&id).map(|a| a.label.as_str())
    }

    pub fn identity(&self, id: CaptureId) -> Option<&TargetIdentity> {
        self.active.get(&id).map(|a| &a.identity)
    }

    pub fn is_active(&self, id: CaptureId) -> bool {
        self.active.contains_key(&id)
    }

    /// Whether the capture has delivered a frame and is still healthy.
    pub fn is_connected(&self, id: CaptureId) -> bool {
        self.active
            .get(&id)
            .is_some_and(|a| a.connected.load(Ordering::SeqCst))
    }

    pub fn active_ids(&self) -> Vec<CaptureId> {
        let mut ids: Vec<CaptureId> = self.active.keys().copied().collect();
        ids.sort_unstable();
        ids
    }

    pub fn config(&self, id: CaptureId) -> Option<CaptureConfig> {
        let active = self.active.get(&id)?;
        active.config.lock().ok().map(|c| c.clone())
    }

    /// Updates a live capture's config. The capture thread applies it on its
    /// next tick without restarting.
    pub fn set_config(&mut self, id: CaptureId, config: CaptureConfig) {
        let Some(active) = self.active.get_mut(&id) else {
            return;
        };
        if let Ok(mut guard) = active.config.lock() {
            *guard = config.sanitized();
        }
    }

    /// Changes one field of a live capture's config.
    pub fn update_config(&mut self, id: CaptureId, f: impl FnOnce(&mut CaptureConfig)) {
        let Some(active) = self.active.get_mut(&id) else {
            return;
        };
        if let Ok(mut guard) = active.config.lock() {
            f(&mut guard);
            *guard = guard.clone().sanitized();
        }
    }

    /// Uploads frames only for captures in `needed`, so an invisible capture deck
    /// costs nothing.
    pub fn update_selective(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        needed: &HashSet<CaptureId>,
    ) {
        for (id, active) in &mut self.active {
            if needed.contains(id) {
                Self::upload_frame(*id, active, device, queue);
            }
        }
    }

    /// Uploads frames for every active capture.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        for (id, active) in &mut self.active {
            Self::upload_frame(*id, active, device, queue);
        }
    }

    fn upload_frame(
        id: CaptureId,
        active: &mut ActiveCapture,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) {
        let Some(frame) = active
            .frame_data
            .try_lock()
            .ok()
            .and_then(|mut slot| slot.take())
        else {
            return;
        };

        // A crop or target resize changes the frame size mid-session. Reallocate
        // instead of dropping the frame, so crop works live.
        if frame.width != active.width
            || frame.height != active.height
            || frame.format != active.format
        {
            let (w, h) = (frame.width.max(1), frame.height.max(1));
            let (texture, view) =
                Self::create_texture(device, id, &active.label, w, h, frame.format);
            active.texture = texture;
            active.texture_view = view;
            active.width = w;
            active.height = h;
            active.format = frame.format;
        }

        let expected = (active.width as usize) * (active.height as usize) * 4;
        if frame.data.len() < expected {
            return;
        }

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &active.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.data[..expected],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(active.width * 4),
                rows_per_image: Some(active.height),
            },
            wgpu::Extent3d {
                width: active.width,
                height: active.height,
                depth_or_array_layers: 1,
            },
        );
    }
}

impl Drop for ScreenCaptureManager {
    fn drop(&mut self) {
        for (_, mut active) in self.active.drain() {
            active.stop_flag.store(true, Ordering::Relaxed);
            if let Some(t) = active.thread.take() {
                let _ = t.join();
            }
        }
    }
}

/// Poll gap bounds for a self-paced backend. The OS sets the delivery rate, so
/// this only limits how long a delivered frame waits to be taken.
const SELF_PACED_MIN_POLL: std::time::Duration = std::time::Duration::from_millis(1);
const SELF_PACED_MAX_POLL: std::time::Duration = std::time::Duration::from_millis(4);

/// Sleep between capture-thread polls.
///
/// A polled backend makes a frame per call, so the loop sleeps a full frame
/// interval. A self-paced backend is oversampled, because polling at the same
/// nominal rate on a second clock drops and doubles frames. See
/// [`ScreenCaptureBackend::is_self_paced`].
fn poll_interval(frame_interval: std::time::Duration, self_paced: bool) -> std::time::Duration {
    if !self_paced {
        return frame_interval;
    }
    (frame_interval / 8).clamp(SELF_PACED_MIN_POLL, SELF_PACED_MAX_POLL)
}

/// Capture loop, one per open target.
///
/// Paced by `config.rate`, independent of the render rate, so a self-capture
/// does not add render cost every frame.
fn capture_loop(
    session: &mut dyn ScreenCaptureBackend,
    id: CaptureId,
    frame_data: &Mutex<Option<CaptureFrame>>,
    config: &Mutex<CaptureConfig>,
    stop: &AtomicBool,
    connected: &AtomicBool,
) {
    let self_paced = session.is_self_paced();
    let mut applied = config.lock().ok().map(|c| c.clone()).unwrap_or_default();
    let mut interval = poll_interval(applied.frame_interval(), self_paced);
    log::debug!(
        "Screen capture {id} thread started ('{}', self_paced={self_paced}, poll={interval:?})",
        session.label()
    );

    while !stop.load(Ordering::Relaxed) {
        let tick = std::time::Instant::now();

        if let Ok(current) = config.lock()
            && *current != applied
        {
            applied = current.clone();
            interval = poll_interval(applied.frame_interval(), self_paced);
            drop(current);
            if let Err(e) = session.set_config(&applied) {
                log::warn!("Screen capture {id}: config change rejected: {e}");
            }
        }

        if let Some(frame) = session.next_frame() {
            connected.store(true, Ordering::SeqCst);
            // Latest wins: overwrite any frame the render thread has not taken.
            if let Ok(mut slot) = frame_data.lock() {
                *slot = Some(frame);
            }
        }

        // No-op when the backend blocked until a frame was ready and used up the
        // interval.
        if let Some(rest) = interval.checked_sub(tick.elapsed()) {
            std::thread::sleep(rest);
        }
    }

    connected.store(false, Ordering::SeqCst);
    log::debug!("Screen capture {id} thread stopped");
}

#[cfg(test)]
mod tests {
    use super::backend::{CaptureConfig, CaptureError, CropRect, TargetIdentity, mock_targets};
    use super::{SELF_PACED_MAX_POLL, SELF_PACED_MIN_POLL, ScreenCaptureManager, poll_interval};
    use std::time::Duration;

    // ── Capture-thread pacing ───────────────────────────────────────

    #[test]
    fn polled_backend_is_paced_by_the_loop_at_exactly_the_frame_interval() {
        let interval = Duration::from_millis(33);
        assert_eq!(poll_interval(interval, false), interval);
    }

    /// Polling a self-paced backend at its own rate drops and doubles frames as
    /// the two clocks drift, so it must be oversampled.
    #[test]
    fn self_paced_backend_is_oversampled_not_matched() {
        for fps in [1.0_f32, 15.0, 30.0, 60.0, 120.0] {
            let interval = Duration::from_secs_f32(1.0 / fps);
            let poll = poll_interval(interval, true);
            assert!(
                poll < interval,
                "{fps} fps: poll {poll:?} must be shorter than the frame interval {interval:?}"
            );
            assert!(
                poll >= SELF_PACED_MIN_POLL && poll <= SELF_PACED_MAX_POLL,
                "{fps} fps: poll {poll:?} outside the allowed bounds"
            );
        }
    }

    #[test]
    fn self_paced_poll_never_busy_spins() {
        // A degenerate interval must still sleep, or the thread spins a core.
        assert!(poll_interval(Duration::ZERO, true) >= SELF_PACED_MIN_POLL);
    }

    #[test]
    fn disabled_manager_reports_unavailable_and_enumerates_nothing() {
        let mut mgr = ScreenCaptureManager::new_disabled();
        assert!(!mgr.is_available());
        assert!(mgr.targets().is_empty());
        mgr.scan_targets();
        assert!(mgr.targets().is_empty());
        assert_eq!(mgr.backend_name(), "disabled");
    }

    #[test]
    fn accessors_on_unknown_id_return_none_not_panic() {
        let mgr = ScreenCaptureManager::new_disabled();
        assert!(mgr.texture_view(0).is_none());
        assert!(mgr.resolution(7).is_none());
        assert!(mgr.label(7).is_none());
        assert!(mgr.config(7).is_none());
        assert!(!mgr.is_active(7));
        assert!(!mgr.is_connected(7));
        assert!(mgr.active_ids().is_empty());
    }

    #[test]
    fn release_of_unknown_id_is_a_noop() {
        let mut mgr = ScreenCaptureManager::new_disabled();
        mgr.release(123);
        assert!(mgr.active_ids().is_empty());
    }

    #[test]
    fn find_target_matches_window_by_app_when_title_changed() {
        let mut mgr = ScreenCaptureManager::new_disabled();
        mgr.targets = mock_targets();
        // Exact identity hit.
        let exact = TargetIdentity::Window {
            app: "com.example.mock".into(),
            title: "Untitled".into(),
        };
        assert!(mgr.find_target(&exact).is_some());
        // A retitled window resolves through the unique-app fallback.
        let retitled = TargetIdentity::Window {
            app: "com.example.mock".into(),
            title: "Something Else".into(),
        };
        assert!(mgr.find_target(&retitled).is_some());
    }

    #[test]
    fn find_target_declines_ambiguous_app_fallback() {
        let mut mgr = ScreenCaptureManager::new_disabled();
        let mut targets = mock_targets();
        // Two windows of the same app: no way to pick one.
        let mut second = targets[2].clone();
        second.platform_id = 200;
        second.title = Some("Second".into());
        targets.push(second);
        mgr.targets = targets;

        let ambiguous = TargetIdentity::Window {
            app: "com.example.mock".into(),
            title: "Gone".into(),
        };
        assert!(
            mgr.find_target(&ambiguous).is_none(),
            "ambiguous app fallback must decline rather than guess"
        );
    }

    #[test]
    fn find_target_does_not_fall_back_for_displays() {
        let mut mgr = ScreenCaptureManager::new_disabled();
        mgr.targets = mock_targets();
        let missing = TargetIdentity::Display {
            label: "Nonexistent".into(),
        };
        assert!(mgr.find_target(&missing).is_none());
    }

    #[test]
    fn disabled_manager_refuses_to_open() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return; // No adapter: skip.
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(0);
        let err = mgr
            .open(&target, CaptureConfig::default(), &gpu.device)
            .expect_err("disabled manager must refuse");
        assert!(matches!(err, CaptureError::Unavailable(_)));
    }

    #[test]
    fn open_mock_is_refcounted_and_survives_release_of_one_holder() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(0);

        let (id, w, h) = mgr
            .open_mock(&target, CaptureConfig::default(), &gpu.device)
            .expect("mock open");
        assert_eq!((w, h), (1920, 1080));

        // A second deck on the same target reuses the session.
        let (id2, _, _) = mgr
            .open_mock(&target, CaptureConfig::default(), &gpu.device)
            .expect("mock reopen");
        assert_eq!(id, id2, "same target must share one capture session");
        assert_eq!(mgr.active_ids().len(), 1);

        // One holder leaving must not stop the shared session.
        mgr.release(id);
        assert!(
            mgr.is_active(id),
            "session must survive while a holder remains"
        );
        mgr.release(id);
        assert!(!mgr.is_active(id), "session must stop at zero references");
        assert!(mgr.active_ids().is_empty());
    }

    #[test]
    fn open_mock_delivers_frames_and_uploads_to_the_shared_texture() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(2); // 800x600 window
        let cfg = CaptureConfig {
            scale_to: Some((160, 120)),
            ..Default::default()
        };
        let (id, w, h) = mgr.open_mock(&target, cfg, &gpu.device).expect("mock open");
        assert_eq!(
            (w, h),
            (160, 120),
            "scale_to must determine the shared texture size"
        );
        assert!(mgr.texture_view(id).is_some());

        let mut needed = std::collections::HashSet::new();
        needed.insert(id);
        mgr.update_selective(&gpu.device, &gpu.queue, &needed);
        assert_eq!(mgr.resolution(id), Some((160, 120)));
        mgr.release(id);
    }

    #[test]
    fn a_window_shaped_unlike_the_deck_leaves_the_scaling_mode_something_to_do() {
        // Passing the deck size as scale_to made a 4:3 window arrive as a 16:9
        // texture (letterboxed by the OS), so the deck's Scale control did nothing.
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(2); // 800x600 window
        let (deck_w, deck_h) = (1920, 1080);
        let cfg = CaptureConfig {
            scale_to: Some(crate::screen_capture::resample::fit_within(
                target.width,
                target.height,
                deck_w,
                deck_h,
            )),
            ..Default::default()
        };
        let (id, w, h) = mgr.open_mock(&target, cfg, &gpu.device).expect("mock open");
        assert_eq!((w, h), (800, 600), "a small window must not be blown up");

        let (fill, _) = crate::deck::ScalingMode::Fill.compute_uv_transform(w, h, deck_w, deck_h);
        let (fit, _) = crate::deck::ScalingMode::Fit.compute_uv_transform(w, h, deck_w, deck_h);
        assert!(
            (fill[1] - 1.0).abs() > 0.01,
            "Fill must crop vertically, got {fill:?}"
        );
        assert!(
            (fit[0] - 1.0).abs() > 0.01,
            "Fit must pillarbox, got {fit:?}"
        );
        mgr.release(id);
    }

    #[test]
    fn update_selective_skips_captures_not_in_the_needed_set() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(0);
        let (id, _, _) = mgr
            .open_mock(&target, CaptureConfig::default(), &gpu.device)
            .expect("mock open");

        // Empty set: no upload and no panic on the missing entry.
        mgr.update_selective(&gpu.device, &gpu.queue, &std::collections::HashSet::new());
        assert!(mgr.is_active(id));
        mgr.release(id);
    }

    /// A deck dropped without `remove_deck` must not leave its capture thread
    /// running.
    #[test]
    fn reconcile_stops_a_session_no_deck_holds_any_more() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(0);
        let (id, _, _) = mgr
            .open_mock(&target, CaptureConfig::default(), &gpu.device)
            .expect("mock open");

        // Still held, so reconciling leaves it alone.
        mgr.reconcile_holders(&std::collections::HashMap::from([(id, 1)]));
        assert!(mgr.is_active(id));

        // The holding deck is gone without a release.
        mgr.reconcile_holders(&std::collections::HashMap::new());
        assert!(
            !mgr.is_active(id),
            "an unheld session must be stopped, not left running"
        );
    }

    #[test]
    fn reconcile_makes_the_deck_count_authoritative_over_the_ref_count() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(0);
        let (id, _, _) = mgr
            .open_mock(&target, CaptureConfig::default(), &gpu.device)
            .expect("mock open");
        let (id2, _, _) = mgr
            .open_mock(&target, CaptureConfig::default(), &gpu.device)
            .expect("mock reopen");
        assert_eq!(id, id2);

        // One of two decks vanished without releasing, so the count is stale at 2.
        // After reconciling, one release closes the session.
        mgr.reconcile_holders(&std::collections::HashMap::from([(id, 1)]));
        mgr.release(id);
        assert!(
            !mgr.is_active(id),
            "a stale ref count must not keep the session alive"
        );
    }

    #[test]
    fn set_config_clamps_through_the_live_session() {
        let Some(gpu) = crate::testing::headless_gpu() else {
            return;
        };
        let mut mgr = ScreenCaptureManager::new_disabled();
        let target = mock_targets().remove(0);
        let (id, _, _) = mgr
            .open_mock(&target, CaptureConfig::default(), &gpu.device)
            .expect("mock open");

        mgr.update_config(id, |c| c.rate = 10_000.0);
        let rate = mgr.config(id).expect("config").rate;
        assert!(
            (rate - super::backend::MAX_CAPTURE_RATE).abs() < f32::EPSILON,
            "router-supplied rate must be clamped, got {rate}"
        );

        mgr.update_config(id, |c| {
            c.crop = CropRect {
                x: 0.9,
                y: 0.0,
                w: 0.5,
                h: 1.0,
            };
        });
        let crop = mgr.config(id).expect("config").crop;
        assert!(crop.x + crop.w <= 1.0 + f32::EPSILON);
        mgr.release(id);
    }
}
