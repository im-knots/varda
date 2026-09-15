//! The DMX driver: latest-wins handoff, transmit thread, refresh discipline, shutdown.
//!
//! Implements contracts C1, C2, C6 and C7 of /spec/dmx-output.md. Each exists because it has
//! been observed to fail on a real rig; the provenance is in /notes/dmx-prior-art.md.

use super::transport::DmxTransport;
use super::universe::{UNIVERSE_SIZE, UniverseId, UniverseSet};
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Upper bound on transmit rate. DMX512 caps near 44 Hz and both protocols inherit it.
pub const MAX_FPS: u32 = 44;
/// Default transmit rate. Matches DMX512 practice and the Art-Net convention.
pub const DEFAULT_FPS: u32 = 40;

/// How often an unchanged universe is retransmitted.
///
/// Art-Net requires a resend if data has not changed for 4 seconds; sACN receivers apply a
/// 2.5 second data-loss timeout. One second satisfies both with margin.
pub const KEEPALIVE: Duration = Duration::from_secs(1);

/// How long shutdown waits for the transmit thread before detaching it.
///
/// A healthy `send_to` on a bound UDP socket returns in microseconds. If the thread has not
/// come back in 200ms it is blocked somewhere that waiting longer will not fix, and holding up
/// the whole application's exit for it is worse than detaching.
const JOIN_TIMEOUT: Duration = Duration::from_millis(200);

// ── latest-wins mailbox ─────────────────────────────────────────────────────

/// Single-slot render-to-transmit handoff with **latest-wins** semantics.
///
/// The contrast with a bounded channel is *which* frame survives contention. `sync_channel(1)`
/// plus `try_send` retains the **oldest** undelivered frame and drops newer ones, so an
/// operator blackout is discarded behind a stale lit frame. Here the producer always
/// overwrites and the consumer always gets the newest. Dark wins. See C1.
#[derive(Debug)]
pub struct Mailbox {
    inner: Mutex<MailboxInner>,
    cv: Condvar,
}

#[derive(Debug)]
struct MailboxInner {
    slot: Option<UniverseSet>,
    closed: bool,
}

impl Default for Mailbox {
    fn default() -> Self {
        Self::new()
    }
}

impl Mailbox {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MailboxInner {
                slot: None,
                closed: false,
            }),
            cv: Condvar::new(),
        }
    }

    /// Overwrite the slot with the newest frame set.
    ///
    /// Returns `true` when an unsent frame was displaced. That is normal under load and is
    /// surfaced as a dropped count, but overwrites that persist without a drain mean the
    /// consumer is wedged, which is a different condition worth reporting separately.
    pub fn store(&self, frames: UniverseSet) -> bool {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let displaced = inner.slot.is_some();
        inner.slot = Some(frames);
        drop(inner);
        self.cv.notify_one();
        displaced
    }

    /// Block until a frame set is available, then take it. `None` once closed and drained.
    #[must_use]
    pub fn take(&self) -> Option<UniverseSet> {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(frames) = inner.slot.take() {
                return Some(frames);
            }
            if inner.closed {
                return None;
            }
            inner = self
                .cv
                .wait(inner)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }

    /// Signal the consumer to exit once drained.
    pub fn close(&self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        inner.closed = true;
        drop(inner);
        self.cv.notify_all();
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .slot
            .is_none()
    }
}

// ── refresh discipline ──────────────────────────────────────────────────────

/// Decides, per universe, whether a frame needs transmitting this pass.
///
/// Rate is per universe, not global: a show with twelve universes must not emit twelve times
/// the packets because one of them is moving. See C7.
#[derive(Debug, Default)]
pub struct RefreshTracker {
    last: HashMap<UniverseId, ([u8; UNIVERSE_SIZE], Instant)>,
}

impl RefreshTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True when this universe should be transmitted now.
    #[must_use]
    pub fn should_send(
        &self,
        universe: UniverseId,
        data: &[u8; UNIVERSE_SIZE],
        now: Instant,
        keepalive: Duration,
    ) -> bool {
        match self.last.get(&universe) {
            None => true,
            Some((prev, at)) => prev != data || now.duration_since(*at) >= keepalive,
        }
    }

    /// Record that this universe was transmitted.
    pub fn record(&mut self, universe: UniverseId, data: &[u8; UNIVERSE_SIZE], now: Instant) {
        self.last.insert(universe, (*data, now));
    }

    /// Forget a universe, so its next frame is treated as a change.
    pub fn forget(&mut self, universe: UniverseId) {
        self.last.remove(&universe);
    }
}

// ── status ──────────────────────────────────────────────────────────────────

/// Live driver status, read by the UI and `/api/state/lighting` without touching the send path.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriverStatus {
    pub running: bool,
    pub transports: Vec<String>,
    pub packets_sent: u64,
    /// Consecutive send failures, reset on any success. Distinct from a cumulative total: this
    /// is what `healthy` is derived from.
    pub consecutive_errors: u64,
    pub healthy: bool,
    /// Frames displaced in the mailbox before the transmit thread drained them.
    pub frames_dropped: u64,
    /// Set when overwrites persisted past the wedge threshold with no drain at all, which is a
    /// stuck consumer rather than ordinary backpressure.
    pub consumer_wedged: bool,
    pub last_error: Option<String>,
}

/// Threshold past which persistent undrained overwrites mean a wedged consumer.
pub const WEDGE_THRESHOLD: Duration = Duration::from_secs(1);

pub type SharedStatus = Arc<Mutex<DriverStatus>>;

/// Consecutive failures before the driver reports itself unhealthy.
const ERRORS_BEFORE_UNHEALTHY: u64 = 10;

// ── transmit loop ───────────────────────────────────────────────────────────

/// Drain the mailbox and transmit, until closed. Then release every universe.
fn transmit_loop(
    mailbox: &Arc<Mailbox>,
    transports: &mut Vec<Box<dyn DmxTransport>>,
    status: &SharedStatus,
    keepalive: Duration,
) {
    let mut refresh = RefreshTracker::new();
    let mut active: Vec<UniverseId> = Vec::new();

    while let Some(frames) = mailbox.take() {
        let now = Instant::now();
        for (universe, data) in frames.iter() {
            if !active.contains(&universe) {
                active.push(universe);
            }
            if !refresh.should_send(universe, data, now, keepalive) {
                continue;
            }
            let mut delivered = false;
            for t in transports.iter_mut() {
                match t.send(universe, data) {
                    Ok(()) => delivered = true,
                    Err(e) => {
                        if let Ok(mut s) = status.try_lock() {
                            s.consecutive_errors += 1;
                            s.last_error = Some(e.to_string());
                            if s.consecutive_errors > ERRORS_BEFORE_UNHEALTHY {
                                s.healthy = false;
                            }
                        }
                    }
                }
            }
            if delivered {
                refresh.record(universe, data, now);
                if let Ok(mut s) = status.try_lock() {
                    s.packets_sent += 1;
                    s.consecutive_errors = 0;
                    s.healthy = true;
                }
            }
        }
    }

    // Shutdown: a dark frame for every universe we touched, then release it. Closing the socket
    // without this leaves fixtures latched at their last value, so quitting during a lit cue
    // leaves the room lit. See C6.
    let dark = [0u8; UNIVERSE_SIZE];
    for universe in active {
        for t in transports.iter_mut() {
            let _ = t.send(universe, &dark);
            let _ = t.terminate(universe);
        }
    }
    if let Ok(mut s) = status.lock() {
        s.running = false;
    }
}

// ── driver ──────────────────────────────────────────────────────────────────

/// Owns the transmit thread and paces the producer.
pub struct DmxDriver {
    mailbox: Arc<Mailbox>,
    thread: Option<thread::JoinHandle<()>>,
    status: SharedStatus,
    frame_interval: Duration,
    last_store: Option<Instant>,
    last_drained: Option<Instant>,
}

impl DmxDriver {
    /// Start a driver transmitting to `transports` at `fps`.
    ///
    /// # Panics
    ///
    /// Panics only if the OS refuses to start a thread, which is not recoverable here.
    #[must_use]
    pub fn start(transports: Vec<Box<dyn DmxTransport>>, fps: u32) -> Self {
        Self::start_with_keepalive(transports, fps, KEEPALIVE)
    }

    /// As [`DmxDriver::start`], with an explicit keepalive interval. Exists for tests, which
    /// cannot wait a real second.
    ///
    /// # Panics
    ///
    /// Panics only if the OS refuses to start a thread.
    #[must_use]
    pub fn start_with_keepalive(
        mut transports: Vec<Box<dyn DmxTransport>>,
        fps: u32,
        keepalive: Duration,
    ) -> Self {
        let status: SharedStatus = Arc::new(Mutex::new(DriverStatus {
            running: true,
            transports: transports.iter().map(|t| t.label()).collect(),
            healthy: true,
            ..DriverStatus::default()
        }));
        let mailbox = Arc::new(Mailbox::new());

        let mb = Arc::clone(&mailbox);
        let st = Arc::clone(&status);
        let thread = thread::Builder::new()
            .name("dmx-tx".into())
            .spawn(move || transmit_loop(&mb, &mut transports, &st, keepalive))
            .expect("spawn dmx transmit thread");

        let fps = fps.clamp(1, MAX_FPS);
        Self {
            mailbox,
            thread: Some(thread),
            status,
            frame_interval: Duration::from_secs_f64(1.0 / f64::from(fps)),
            last_store: None,
            last_drained: None,
        }
    }

    #[must_use]
    pub fn status(&self) -> SharedStatus {
        Arc::clone(&self.status)
    }

    #[must_use]
    pub fn status_snapshot(&self) -> DriverStatus {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Hand a frame set to the transmit thread.
    ///
    /// `force` bypasses the producer rate gate and must be set for blackout and any other
    /// safety frame. The gate runs **before** the store, so without it a dark frame arriving
    /// inside the frame interval would never reach the mailbox at all and the transmit thread
    /// would resend the previous lit universe. C1 does not imply C2: the mailbox alone cannot
    /// save a frame the producer never handed it. See C2.
    pub fn submit(&mut self, frames: UniverseSet, force: bool) {
        let now = Instant::now();
        if !force
            && let Some(last) = self.last_store
            && now.duration_since(last) < self.frame_interval
        {
            return;
        }
        if self.mailbox.store(frames) {
            if let Ok(mut s) = self.status.try_lock() {
                s.frames_dropped += 1;
                if let Some(drained) = self.last_drained
                    && now.duration_since(drained) > WEDGE_THRESHOLD
                {
                    s.consumer_wedged = true;
                    s.healthy = false;
                }
            }
        } else {
            self.last_drained = Some(now);
            if let Ok(mut s) = self.status.try_lock() {
                s.consumer_wedged = false;
            }
        }
        self.last_store = Some(now);
    }

    /// Stop the transmit thread, releasing every universe first.
    ///
    /// Idempotent. Called by [`Drop`], and available for an explicit shutdown that wants to
    /// wait for the dark frames to land.
    pub fn stop(&mut self) {
        self.mailbox.close();
        if let Some(handle) = self.thread.take() {
            // Bounded: a healthy send returns in microseconds, so a thread that has not
            // returned in JOIN_TIMEOUT is blocked somewhere waiting will not fix.
            let done = Arc::new((Mutex::new(false), Condvar::new()));
            let flag = Arc::clone(&done);
            let watcher = thread::Builder::new()
                .name("dmx-join".into())
                .spawn(move || {
                    let _ = handle.join();
                    let (lock, cv) = &*flag;
                    if let Ok(mut g) = lock.lock() {
                        *g = true;
                    }
                    cv.notify_all();
                })
                .ok();
            let (lock, cv) = &*done;
            if let Ok(guard) = lock.lock() {
                let _ = cv.wait_timeout_while(guard, JOIN_TIMEOUT, |finished| !*finished);
            }
            drop(watcher);
        }
    }
}

impl Drop for DmxDriver {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dmx::universe::{Resolution, SlotWrite};
    use std::sync::mpsc;

    /// Records what reached the wire, so the discipline can be asserted without sockets.
    struct Recorder {
        tx: mpsc::Sender<Event>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum Event {
        Sent(UniverseId, u8),
        Terminated(UniverseId),
    }

    impl DmxTransport for Recorder {
        fn label(&self) -> String {
            "recorder".into()
        }
        fn send(
            &mut self,
            universe: UniverseId,
            data: &[u8; UNIVERSE_SIZE],
        ) -> Result<(), super::super::transport::DmxError> {
            let _ = self.tx.send(Event::Sent(universe, data[0]));
            Ok(())
        }
        fn terminate(
            &mut self,
            universe: UniverseId,
        ) -> Result<(), super::super::transport::DmxError> {
            let _ = self.tx.send(Event::Terminated(universe));
            Ok(())
        }
    }

    fn recorder() -> (Box<dyn DmxTransport>, mpsc::Receiver<Event>) {
        let (tx, rx) = mpsc::channel();
        (Box::new(Recorder { tx }), rx)
    }

    fn frame(universe: UniverseId, value: f32) -> UniverseSet {
        let mut set = UniverseSet::new();
        set.apply(SlotWrite {
            universe,
            slot: 0,
            resolution: Resolution::Eight,
            value,
        });
        set
    }

    fn drain(rx: &mpsc::Receiver<Event>, timeout: Duration) -> Vec<Event> {
        let mut out = Vec::new();
        let deadline = Instant::now() + timeout;
        while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            out.push(e);
        }
        out
    }

    // ── C1: dark wins ────────────────────────────────────────────────────

    #[test]
    fn mailbox_keeps_the_newest_frame_not_the_oldest() {
        let mb = Mailbox::new();
        assert!(!mb.store(frame(1, 1.0)), "first store displaces nothing");
        assert!(mb.store(frame(1, 0.0)), "second store displaces the first");
        let got = mb.take().expect("a frame");
        assert_eq!(
            got.get(1).unwrap()[0],
            0,
            "dark must win over a stale lit frame"
        );
    }

    #[test]
    fn mailbox_take_blocks_until_closed_then_returns_none() {
        let mb = Arc::new(Mailbox::new());
        let c = Arc::clone(&mb);
        let h = thread::spawn(move || c.take());
        thread::sleep(Duration::from_millis(20));
        mb.close();
        assert!(h.join().unwrap().is_none());
    }

    #[test]
    fn mailbox_drains_a_pending_frame_before_reporting_closed() {
        let mb = Mailbox::new();
        mb.store(frame(1, 1.0));
        mb.close();
        assert!(mb.take().is_some(), "a stored frame must not be lost");
        assert!(mb.take().is_none());
    }

    // ── C2: safety frames bypass the rate gate ───────────────────────────

    #[test]
    fn a_normal_frame_inside_the_rate_window_is_dropped() {
        let (t, _rx) = recorder();
        let mut d = DmxDriver::start(vec![t], 1); // 1 fps: a very wide window
        d.submit(frame(1, 1.0), false);
        thread::sleep(Duration::from_millis(30));
        d.submit(frame(1, 0.5), false);
        assert!(
            d.mailbox.is_empty(),
            "the second frame was inside the window and must not have been stored"
        );
    }

    /// C1 does not imply C2. The rate gate runs before the store, so without `force` a dark
    /// frame arriving inside the window never reaches the mailbox and the transmit thread
    /// resends the previous lit universe.
    #[test]
    fn a_forced_frame_inside_the_rate_window_reaches_the_mailbox() {
        let (t, rx) = recorder();
        let mut d = DmxDriver::start(vec![t], 1);
        d.submit(frame(1, 1.0), false);
        let _ = drain(&rx, Duration::from_millis(60));
        d.submit(frame(1, 0.0), true);
        let events = drain(&rx, Duration::from_millis(120));
        assert!(
            events.contains(&Event::Sent(1, 0)),
            "blackout must reach the wire inside the rate window, got {events:?}"
        );
    }

    // ── C7: refresh discipline ───────────────────────────────────────────

    #[test]
    fn an_unseen_universe_is_always_sent() {
        let r = RefreshTracker::new();
        assert!(r.should_send(1, &[0u8; UNIVERSE_SIZE], Instant::now(), KEEPALIVE));
    }

    #[test]
    fn an_unchanged_universe_waits_for_the_keepalive() {
        let mut r = RefreshTracker::new();
        let data = [0u8; UNIVERSE_SIZE];
        let t0 = Instant::now();
        r.record(1, &data, t0);
        assert!(
            !r.should_send(1, &data, t0 + Duration::from_millis(100), KEEPALIVE),
            "unchanged data inside the keepalive must not be resent"
        );
        assert!(
            r.should_send(1, &data, t0 + KEEPALIVE, KEEPALIVE),
            "the keepalive must eventually fire"
        );
    }

    #[test]
    fn a_changed_universe_is_sent_immediately() {
        let mut r = RefreshTracker::new();
        let t0 = Instant::now();
        r.record(1, &[0u8; UNIVERSE_SIZE], t0);
        let mut changed = [0u8; UNIVERSE_SIZE];
        changed[7] = 1;
        assert!(r.should_send(1, &changed, t0 + Duration::from_millis(1), KEEPALIVE));
    }

    /// A show with twelve universes must not emit twelve times the packets because one moved.
    #[test]
    fn universes_are_paced_independently() {
        let mut r = RefreshTracker::new();
        let t0 = Instant::now();
        let quiet = [0u8; UNIVERSE_SIZE];
        r.record(1, &quiet, t0);
        r.record(2, &quiet, t0);
        let mut moved = [0u8; UNIVERSE_SIZE];
        moved[0] = 255;
        let t1 = t0 + Duration::from_millis(10);
        assert!(r.should_send(1, &moved, t1, KEEPALIVE), "universe 1 moved");
        assert!(
            !r.should_send(2, &quiet, t1, KEEPALIVE),
            "universe 2 is unchanged and must stay on its keepalive"
        );
    }

    #[test]
    fn forget_makes_the_next_frame_a_change() {
        let mut r = RefreshTracker::new();
        let data = [0u8; UNIVERSE_SIZE];
        let t0 = Instant::now();
        r.record(1, &data, t0);
        r.forget(1);
        assert!(r.should_send(1, &data, t0, KEEPALIVE));
    }

    #[test]
    fn the_driver_does_not_resend_an_unchanged_universe_every_frame() {
        let (t, rx) = recorder();
        let mut d = DmxDriver::start_with_keepalive(vec![t], 44, Duration::from_secs(60));
        for _ in 0..20 {
            d.submit(frame(1, 1.0), true);
            thread::sleep(Duration::from_millis(5));
        }
        let sends = drain(&rx, Duration::from_millis(120))
            .into_iter()
            .filter(|e| matches!(e, Event::Sent(1, _)))
            .count();
        assert_eq!(
            sends, 1,
            "an unchanged universe must be sent once, got {sends}"
        );
    }

    // ── C6: shutdown ─────────────────────────────────────────────────────

    #[test]
    fn shutdown_darkens_and_releases_every_touched_universe() {
        let (t, rx) = recorder();
        let mut d = DmxDriver::start(vec![t], 44);
        d.submit(frame(1, 1.0), true);
        thread::sleep(Duration::from_millis(30));
        let mut two = frame(2, 1.0);
        two.ensure(1);
        d.submit(two, true);
        thread::sleep(Duration::from_millis(30));
        d.stop();

        let events = drain(&rx, Duration::from_millis(200));
        for u in [1u16, 2] {
            assert!(
                events.contains(&Event::Sent(u, 0)),
                "universe {u} must receive a dark frame on shutdown, got {events:?}"
            );
            assert!(
                events.contains(&Event::Terminated(u)),
                "universe {u} must be released, got {events:?}"
            );
        }
    }

    #[test]
    fn stop_is_idempotent() {
        let (t, _rx) = recorder();
        let mut d = DmxDriver::start(vec![t], 44);
        d.stop();
        d.stop();
    }

    #[test]
    fn dropping_the_driver_stops_the_thread() {
        let (t, rx) = recorder();
        {
            let mut d = DmxDriver::start(vec![t], 44);
            d.submit(frame(1, 1.0), true);
            thread::sleep(Duration::from_millis(30));
        }
        let events = drain(&rx, Duration::from_millis(200));
        assert!(events.contains(&Event::Terminated(1)), "got {events:?}");
    }

    // ── status ───────────────────────────────────────────────────────────

    #[test]
    fn status_reports_transports_and_counts_packets() {
        let (t, rx) = recorder();
        let mut d = DmxDriver::start(vec![t], 44);
        assert_eq!(d.status_snapshot().transports, vec!["recorder".to_string()]);
        d.submit(frame(1, 1.0), true);
        let _ = drain(&rx, Duration::from_millis(80));
        assert!(d.status_snapshot().packets_sent >= 1);
        assert!(d.status_snapshot().healthy);
    }

    #[test]
    fn fps_is_clamped_to_the_protocol_ceiling() {
        let (t, _rx) = recorder();
        let d = DmxDriver::start(vec![t], 10_000);
        assert!(d.frame_interval >= Duration::from_secs_f64(1.0 / f64::from(MAX_FPS)));
    }

    #[test]
    fn zero_fps_does_not_divide_by_zero() {
        let (t, _rx) = recorder();
        let d = DmxDriver::start(vec![t], 0);
        assert!(d.frame_interval.as_secs_f64().is_finite());
        assert!(d.frame_interval <= Duration::from_secs(1));
    }
}
