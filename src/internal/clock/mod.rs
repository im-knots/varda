//! Unified clock system for Varda.
//!
//! Receives BPM/beat from MIDI clock (24 PPQ), OSC messages, and audio detection.
//! Exposes a single `ClockState` consumed by all beat-synced features.
//!
//! Priority: MIDI Clock > OSC Clock > Audio Detection
//! Fallback: When external clock goes stale (>2s), reverts to audio BPM.

use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use crate::midi::DeviceId;

/// Ticks per quarter note in the MIDI clock protocol.
const MIDI_PPQ: usize = 24;

/// Duration after which a clock source is considered stale.
const STALE_TIMEOUT_SECS: f32 = 2.0;

/// EMA smoothing factor for BPM values.
const EMA_ALPHA: f32 = 0.3;

/// Identifies the active clock source.
#[derive(Debug, Clone)]
pub enum ClockSource {
    /// BPM derived from audio beat detection (lowest priority).
    Audio,
    /// BPM derived from MIDI clock ticks (highest priority).
    MidiClock { device_id: DeviceId, device_name: String },
    /// BPM received via OSC messages.
    OscClock,
    /// BPM set manually by the user.
    Manual,
}

/// User preference for which clock source to use.
#[derive(Debug, Clone, PartialEq)]
pub enum ClockPreference {
    /// Auto-detect: use priority resolution (MIDI > OSC > Audio).
    Auto,
    /// Force a specific MIDI device's clock.
    ForceMidi { device_id: DeviceId },
    /// Force OSC clock.
    ForceOsc,
    /// Force audio-only (ignore external clocks).
    ForceAudio,
    /// Force a manually set BPM (user beatmatches by ear).
    ForceManual { bpm: f32 },
}

impl Default for ClockPreference {
    fn default() -> Self {
        Self::Auto
    }
}

/// A MIDI device that has been detected as sending clock ticks.
#[derive(Debug, Clone)]
pub struct DetectedClockSource {
    pub device_id: DeviceId,
    pub device_name: String,
    pub bpm: Option<f32>,
    pub last_tick: Instant,
}

/// Resolved clock state for the current frame.
#[derive(Debug, Clone)]
pub struct ClockState {
    /// Current BPM (smoothed).
    pub bpm: f32,
    /// Beat phase 0.0–1.0 (0.0 = on the beat).
    pub beat_phase: f32,
    /// Which source is providing the clock.
    pub source: ClockSource,
    /// Whether any valid clock source is active.
    pub active: bool,
}

impl Default for ClockState {
    fn default() -> Self {
        Self { bpm: 120.0, beat_phase: 0.0, source: ClockSource::Audio, active: false }
    }
}

/// Per-device MIDI clock tracking state.
struct MidiDeviceClock {
    ticks: VecDeque<Instant>,
    bpm: Option<f32>,
    running: bool,
    device_name: String,
    beat_phase: f32,
    last_tick: Instant,
    smoothed_bpm: Option<f32>,
}

/// Manages clock sources and resolves priority.
pub struct ClockManager {
    // ── Per-device MIDI clock tracking ──────────────────────────
    midi_devices: HashMap<DeviceId, MidiDeviceClock>,

    // ── OSC clock state ─────────────────────────────────────────
    osc_bpm: Option<f32>,
    osc_beat_phase: Option<f32>,
    osc_last_message: Option<Instant>,

    // ── Audio fallback ──────────────────────────────────────────
    audio_bpm: Option<f32>,
    audio_beat_phase: f32,

    // ── Manual BPM ────────────────────────────────────────────────
    manual_start_time: Option<Instant>,

    // ── User preference ─────────────────────────────────────────
    preference: ClockPreference,

    // ── Resolved state ──────────────────────────────────────────
    state: ClockState,
}

impl ClockManager {
    pub fn new() -> Self {
        Self {
            midi_devices: HashMap::new(),
            osc_bpm: None,
            osc_beat_phase: None,
            osc_last_message: None,
            audio_bpm: None,
            audio_beat_phase: 0.0,
            manual_start_time: None,
            preference: ClockPreference::Auto,
            state: ClockState::default(),
        }
    }

    // ── Preference ──────────────────────────────────────────────

    /// Set the user's clock source preference.
    pub fn set_preference(&mut self, pref: ClockPreference) {
        // Initialize manual start time when entering manual mode.
        if matches!(pref, ClockPreference::ForceManual { .. }) && self.manual_start_time.is_none() {
            self.manual_start_time = Some(Instant::now());
        } else if !matches!(pref, ClockPreference::ForceManual { .. }) {
            self.manual_start_time = None;
        }
        self.preference = pref;
    }

    /// Get the current clock preference.
    pub fn preference(&self) -> &ClockPreference {
        &self.preference
    }

    // ── Manual BPM methods ──────────────────────────────────────

    /// Update the manual BPM value (clamped 20–300). Only effective in ForceManual mode.
    pub fn set_manual_bpm(&mut self, bpm: f32) {
        let bpm = bpm.clamp(20.0, 300.0);
        if let ClockPreference::ForceManual { bpm: ref mut stored } = self.preference {
            *stored = bpm;
        }
    }

    /// Get the current manual BPM (if in ForceManual mode).
    pub fn manual_bpm(&self) -> Option<f32> {
        match &self.preference {
            ClockPreference::ForceManual { bpm } => Some(*bpm),
            _ => None,
        }
    }

    // ── MIDI clock methods ──────────────────────────────────────

    /// Process a MIDI clock tick (0xF8). Called 24 times per quarter note.
    pub fn process_midi_tick(&mut self, device_id: DeviceId, device_name: &str) {
        let now = Instant::now();
        let dev = self.midi_devices.entry(device_id).or_insert_with(|| MidiDeviceClock {
            ticks: VecDeque::with_capacity(MIDI_PPQ + 1),
            bpm: None,
            running: false,
            device_name: device_name.to_string(),
            beat_phase: 0.0,
            last_tick: now,
            smoothed_bpm: None,
        });

        dev.device_name = device_name.to_string();
        dev.last_tick = now;
        dev.ticks.push_back(now);
        if dev.ticks.len() > MIDI_PPQ + 1 {
            dev.ticks.pop_front();
        }

        // Need at least 24 ticks to calculate one quarter note duration
        if dev.ticks.len() > MIDI_PPQ {
            let oldest = dev.ticks[0];
            let elapsed = now.duration_since(oldest).as_secs_f32();
            if elapsed > 0.0 {
                let raw_bpm = 60.0 / elapsed;
                // Per-device EMA smoothing
                let smoothed = match dev.smoothed_bpm {
                    Some(prev) => EMA_ALPHA * raw_bpm + (1.0 - EMA_ALPHA) * prev,
                    None => raw_bpm,
                };
                dev.smoothed_bpm = Some(smoothed);
                dev.bpm = Some(smoothed);
            }
        }

        // Advance beat phase: each tick = 1/24 of a quarter note.
        // Always advance — free-running clocks (no 0xFA) should still track phase.
        dev.beat_phase = (dev.beat_phase + 1.0 / MIDI_PPQ as f32) % 1.0;
    }

    /// Process MIDI Start (0xFA).
    pub fn process_midi_start(&mut self) {
        for dev in self.midi_devices.values_mut() {
            dev.running = true;
            dev.beat_phase = 0.0;
            dev.ticks.clear();
        }
    }

    /// Process MIDI Continue (0xFB).
    pub fn process_midi_continue(&mut self) {
        for dev in self.midi_devices.values_mut() {
            dev.running = true;
        }
    }

    /// Process MIDI Stop (0xFC).
    pub fn process_midi_stop(&mut self) {
        for dev in self.midi_devices.values_mut() {
            dev.running = false;
        }
    }

    // ── OSC clock methods ───────────────────────────────────────

    /// Process an OSC /clock/bpm message.
    pub fn process_osc_bpm(&mut self, bpm: f32) {
        self.osc_bpm = Some(bpm);
        self.osc_last_message = Some(Instant::now());
    }

    /// Process an OSC /clock/beat message (beat phase 0.0–1.0).
    pub fn process_osc_beat(&mut self, phase: f32) {
        self.osc_beat_phase = Some(phase.clamp(0.0, 1.0));
        self.osc_last_message = Some(Instant::now());
    }

    // ── Audio fallback ──────────────────────────────────────────

    /// Update audio-detected BPM and beat phase (called each frame).
    pub fn update_audio(&mut self, bpm: Option<f32>, beat_phase: f32) {
        self.audio_bpm = bpm;
        self.audio_beat_phase = beat_phase;
    }

    // ── Detected sources ────────────────────────────────────────

    /// Get all MIDI devices currently detected as sending clock ticks (not stale).
    pub fn detected_midi_sources(&self) -> Vec<DetectedClockSource> {
        let now = Instant::now();
        self.midi_devices.iter()
            .filter(|(_, dev)| now.duration_since(dev.last_tick).as_secs_f32() < STALE_TIMEOUT_SECS)
            .map(|(&id, dev)| DetectedClockSource {
                device_id: id,
                device_name: dev.device_name.clone(),
                bpm: dev.bpm,
                last_tick: dev.last_tick,
            })
            .collect()
    }

    /// Whether OSC clock is currently active (not stale).
    pub fn osc_active(&self) -> bool {
        self.osc_last_message.map_or(false, |t| {
            Instant::now().duration_since(t).as_secs_f32() < STALE_TIMEOUT_SECS
        })
    }

    /// Get the current OSC BPM (if active).
    pub fn osc_bpm(&self) -> Option<f32> {
        if self.osc_active() { self.osc_bpm } else { None }
    }

    // ── Resolution ──────────────────────────────────────────────

    /// Resolve clock priority and update state. Call once per frame.
    pub fn update(&mut self) {
        let now = Instant::now();

        match &self.preference {
            ClockPreference::ForceManual { bpm } => {
                let bpm = *bpm;
                let start = *self.manual_start_time.get_or_insert(now);
                let elapsed = now.duration_since(start).as_secs_f64();
                let phase = ((elapsed * bpm as f64 / 60.0) % 1.0) as f32;
                self.state = ClockState {
                    bpm,
                    beat_phase: phase,
                    source: ClockSource::Manual,
                    active: true,
                };
                return;
            }
            ClockPreference::ForceAudio => {
                self.resolve_audio();
                return;
            }
            ClockPreference::ForceOsc => {
                if self.resolve_osc(now) { return; }
                // OSC not available, fall back to audio
                self.resolve_audio();
                return;
            }
            ClockPreference::ForceMidi { device_id } => {
                let dev_id = *device_id;
                if self.resolve_midi_device(now, dev_id) { return; }
                // Forced device not available, fall back to audio
                self.resolve_audio();
                return;
            }
            ClockPreference::Auto => {
                // Priority resolution: MIDI > OSC > Audio
                // Find the best MIDI device (first fresh one with running + bpm)
                // Select any device with fresh ticks and a valid BPM.
                // Don't require `running` — many devices (e.g. Tascam Model 12)
                // free-run clock ticks without sending MIDI Start (0xFA).
                let best_midi = self.midi_devices.iter()
                    .filter(|(_, dev)| {
                        now.duration_since(dev.last_tick).as_secs_f32() < STALE_TIMEOUT_SECS
                            && dev.bpm.is_some()
                    })
                    .map(|(&id, _)| id)
                    .next();

                if let Some(dev_id) = best_midi {
                    if self.resolve_midi_device(now, dev_id) { return; }
                }

                if self.resolve_osc(now) { return; }
                self.resolve_audio();
            }
        }
    }

    /// Get the current resolved clock state.
    pub fn state(&self) -> &ClockState {
        &self.state
    }

    // ── Private resolution helpers ──────────────────────────────

    fn resolve_midi_device(&mut self, now: Instant, device_id: DeviceId) -> bool {
        if let Some(dev) = self.midi_devices.get(&device_id) {
            // Don't require `running` — free-running clocks send ticks without 0xFA.
            let fresh = now.duration_since(dev.last_tick).as_secs_f32() < STALE_TIMEOUT_SECS;
            if fresh {
                if let Some(bpm) = dev.bpm {
                    self.state = ClockState {
                        bpm,
                        beat_phase: dev.beat_phase,
                        source: ClockSource::MidiClock {
                            device_id,
                            device_name: dev.device_name.clone(),
                        },
                        active: true,
                    };
                    return true;
                }
            }
        }
        false
    }

    fn resolve_osc(&mut self, now: Instant) -> bool {
        let osc_fresh = self.osc_last_message.map_or(false, |t| {
            now.duration_since(t).as_secs_f32() < STALE_TIMEOUT_SECS
        });
        if osc_fresh {
            if let Some(osc_bpm) = self.osc_bpm {
                self.state = ClockState {
                    bpm: osc_bpm,
                    beat_phase: self.osc_beat_phase.unwrap_or(0.0),
                    source: ClockSource::OscClock,
                    active: true,
                };
                return true;
            }
        }
        false
    }

    fn resolve_audio(&mut self) {
        if let Some(audio_bpm) = self.audio_bpm {
            self.state = ClockState {
                bpm: audio_bpm,
                beat_phase: self.audio_beat_phase,
                source: ClockSource::Audio,
                active: true,
            };
        } else {
            self.state.active = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_default_state_inactive() {
        let mgr = ClockManager::new();
        assert!(!mgr.state().active);
    }

    #[test]
    fn test_audio_fallback() {
        let mut mgr = ClockManager::new();
        mgr.update_audio(Some(120.0), 0.5);
        mgr.update();
        assert!(mgr.state().active);
        assert!((mgr.state().bpm - 120.0).abs() < 0.01);
        assert!((mgr.state().beat_phase - 0.5).abs() < 0.01);
        assert!(matches!(mgr.state().source, ClockSource::Audio));
    }

    #[test]
    fn test_no_sources_inactive() {
        let mut mgr = ClockManager::new();
        mgr.update_audio(None, 0.0);
        mgr.update();
        assert!(!mgr.state().active);
    }

    #[test]
    fn test_midi_bpm_from_24_ticks() {
        let mut mgr = ClockManager::new();
        mgr.process_midi_start();

        // Simulate 25 ticks at 120 BPM (0.5s per quarter note, ~20.83ms per tick)
        let tick_interval = Duration::from_secs_f64(0.5 / 24.0);
        let base = Instant::now();
        let dev = mgr.midi_devices.entry(0).or_insert_with(|| MidiDeviceClock {
            ticks: VecDeque::with_capacity(MIDI_PPQ + 1),
            bpm: None,
            running: true,
            device_name: "Test".to_string(),
            beat_phase: 0.0,
            last_tick: base,
            smoothed_bpm: None,
        });
        for i in 0..=MIDI_PPQ {
            let tick_time = base + tick_interval * i as u32;
            dev.ticks.push_back(tick_time);
        }
        dev.last_tick = base + tick_interval * MIDI_PPQ as u32;

        // Calculate BPM from ticks manually
        let oldest = dev.ticks[0];
        let newest = *dev.ticks.back().unwrap();
        let elapsed = newest.duration_since(oldest).as_secs_f32();
        dev.bpm = Some(60.0 / elapsed);

        mgr.update();
        assert!(mgr.state().active);
        assert!((mgr.state().bpm - 120.0).abs() < 2.0);
        assert!(matches!(mgr.state().source, ClockSource::MidiClock { .. }));
    }

    #[test]
    fn test_osc_overrides_audio() {
        let mut mgr = ClockManager::new();
        mgr.update_audio(Some(100.0), 0.3);
        mgr.process_osc_bpm(140.0);
        mgr.update();
        assert!(mgr.state().active);
        assert!((mgr.state().bpm - 140.0).abs() < 1.0);
        assert!(matches!(mgr.state().source, ClockSource::OscClock));
    }

    #[test]
    fn test_osc_beat_phase() {
        let mut mgr = ClockManager::new();
        mgr.process_osc_bpm(128.0);
        mgr.process_osc_beat(0.75);
        mgr.update();
        assert!((mgr.state().beat_phase - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_midi_start_resets_phase() {
        let mut mgr = ClockManager::new();
        // Add a device first
        mgr.process_midi_tick(0, "Test");
        mgr.process_midi_start();
        let dev = mgr.midi_devices.get(&0).unwrap();
        assert!((dev.beat_phase).abs() < 0.001);
        assert!(dev.running);
    }

    #[test]
    fn test_midi_stop_clears_running() {
        let mut mgr = ClockManager::new();
        mgr.process_midi_tick(0, "Test");
        mgr.process_midi_start();
        assert!(mgr.midi_devices.get(&0).unwrap().running);
        mgr.process_midi_stop();
        assert!(!mgr.midi_devices.get(&0).unwrap().running);
    }

    #[test]
    fn test_force_audio_ignores_midi() {
        let mut mgr = ClockManager::new();
        mgr.set_preference(ClockPreference::ForceAudio);
        mgr.update_audio(Some(100.0), 0.3);
        mgr.process_osc_bpm(140.0);
        mgr.update();
        assert!(matches!(mgr.state().source, ClockSource::Audio));
        assert!((mgr.state().bpm - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_detected_midi_sources() {
        let mut mgr = ClockManager::new();
        mgr.process_midi_tick(1, "Device A");
        mgr.process_midi_tick(2, "Device B");
        let sources = mgr.detected_midi_sources();
        assert_eq!(sources.len(), 2);
    }

    #[test]
    fn test_preference_default_is_auto() {
        let mgr = ClockManager::new();
        assert_eq!(*mgr.preference(), ClockPreference::Auto);
    }

    #[test]
    fn test_force_manual_bpm() {
        let mut mgr = ClockManager::new();
        mgr.set_preference(ClockPreference::ForceManual { bpm: 128.0 });
        mgr.update();
        assert!(mgr.state().active);
        assert!((mgr.state().bpm - 128.0).abs() < 0.01);
        assert!(matches!(mgr.state().source, ClockSource::Manual));
    }

    #[test]
    fn test_set_manual_bpm_updates_preference() {
        let mut mgr = ClockManager::new();
        mgr.set_preference(ClockPreference::ForceManual { bpm: 120.0 });
        mgr.set_manual_bpm(140.0);
        assert_eq!(mgr.manual_bpm(), Some(140.0));
        mgr.update();
        assert!((mgr.state().bpm - 140.0).abs() < 0.01);
    }

    #[test]
    fn test_manual_bpm_clamped() {
        let mut mgr = ClockManager::new();
        mgr.set_preference(ClockPreference::ForceManual { bpm: 120.0 });
        mgr.set_manual_bpm(5.0);
        assert_eq!(mgr.manual_bpm(), Some(20.0));
        mgr.set_manual_bpm(999.0);
        assert_eq!(mgr.manual_bpm(), Some(300.0));
    }

    #[test]
    fn test_manual_beat_phase_advances() {
        let mut mgr = ClockManager::new();
        // 120 BPM = 2 beats/sec, so after some time phase should be non-zero
        mgr.set_preference(ClockPreference::ForceManual { bpm: 120.0 });
        // Sleep briefly so elapsed > 0
        std::thread::sleep(Duration::from_millis(50));
        mgr.update();
        assert!(mgr.state().active);
        assert!(mgr.state().beat_phase > 0.0, "beat phase should advance over time");
        assert!(mgr.state().beat_phase < 1.0, "beat phase should be in 0..1");
    }

    #[test]
    fn test_manual_ignores_other_sources() {
        let mut mgr = ClockManager::new();
        mgr.set_preference(ClockPreference::ForceManual { bpm: 100.0 });
        // Feed audio and OSC — they should be ignored
        mgr.update_audio(Some(140.0), 0.5);
        mgr.process_osc_bpm(160.0);
        mgr.update();
        assert!(matches!(mgr.state().source, ClockSource::Manual));
        assert!((mgr.state().bpm - 100.0).abs() < 0.01);
    }

    #[test]
    fn test_set_manual_bpm_noop_when_not_manual() {
        let mut mgr = ClockManager::new();
        // Default is Auto — set_manual_bpm should be a no-op
        mgr.set_manual_bpm(140.0);
        assert_eq!(mgr.manual_bpm(), None);
    }

    #[test]
    fn test_manual_bpm_none_when_not_manual() {
        let mgr = ClockManager::new();
        assert_eq!(mgr.manual_bpm(), None);

        let mut mgr2 = ClockManager::new();
        mgr2.set_preference(ClockPreference::ForceAudio);
        assert_eq!(mgr2.manual_bpm(), None);
    }

    #[test]
    fn test_switching_away_from_manual_clears_start_time() {
        let mut mgr = ClockManager::new();
        mgr.set_preference(ClockPreference::ForceManual { bpm: 120.0 });
        assert!(mgr.manual_start_time.is_some());
        mgr.set_preference(ClockPreference::Auto);
        assert!(mgr.manual_start_time.is_none());
    }

    #[test]
    fn test_re_entering_manual_resets_phase() {
        let mut mgr = ClockManager::new();
        mgr.set_preference(ClockPreference::ForceManual { bpm: 120.0 });
        std::thread::sleep(Duration::from_millis(50));
        mgr.update();
        let phase1 = mgr.state().beat_phase;
        assert!(phase1 > 0.0);

        // Switch away and back — start time should reset
        mgr.set_preference(ClockPreference::Auto);
        mgr.set_preference(ClockPreference::ForceManual { bpm: 120.0 });
        mgr.update();
        let phase2 = mgr.state().beat_phase;
        // phase2 should be very close to 0 since we just re-entered
        assert!(phase2 < phase1, "phase should reset when re-entering manual mode");
    }
}
