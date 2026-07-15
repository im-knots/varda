//! Audio input and analysis for audio-reactive shaders

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use crossbeam_channel::{bounded, Receiver, Sender};
use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Opaque audio source identifier.
pub type AudioSourceId = u32;

/// Opaque token identifying a single PCM passthrough subscription.
pub type PcmToken = u64;

/// Bounded capacity (in chunks) of a passthrough PCM channel. One chunk is
/// produced per cpal callback (~10ms), so 32 ≈ 320ms of slack before drops.
const PCM_CHANNEL_CAPACITY: usize = 32;

/// Monotonic source of unique [`PcmToken`]s.
static NEXT_PCM_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Native PCM layout of a capture device, reported to passthrough subscribers
/// so they can build matching ffmpeg input args.
#[derive(Debug, Clone, Copy)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// A chunk of raw interleaved PCM (native channel count, native sample rate),
/// tee'd from the cpal capture callback for output passthrough. Distinct from
/// [`AudioData`], which is mono-downmixed and FFT-processed for analysis.
pub struct PcmChunk {
    /// Interleaved `f32` samples in the device's native channel count.
    pub samples: Vec<f32>,
}

/// A registered passthrough consumer. The cpal callback fans raw PCM out to
/// every subscriber on a source (a "tee").
#[derive(Clone)]
struct PcmSubscriber {
    token: PcmToken,
    sender: Sender<PcmChunk>,
    dropped: Arc<AtomicU64>,
}

/// Handle returned by [`AudioManager::subscribe_pcm`]. The receiver yields raw
/// PCM; `format` describes its layout; `token` identifies the subscription for
/// [`AudioManager::unsubscribe_pcm`]; `dropped` counts backpressure drops.
pub struct PcmSubscription {
    pub receiver: Receiver<PcmChunk>,
    pub format: AudioFormat,
    pub token: PcmToken,
    pub dropped: Arc<AtomicU64>,
}

/// Fan a chunk of raw interleaved PCM out to every passthrough subscriber.
///
/// Never blocks: on a full channel the chunk is dropped (newest-drop, matching
/// the video frame-drop philosophy in `FfmpegSubprocess`) and counted so the
/// owning output can surface a health warning. Disconnected subscribers (the
/// output stopped without unsubscribing) are silently skipped.
fn fan_out_pcm(subs: &[PcmSubscriber], samples: &[f32]) {
    for sub in subs {
        match sub.sender.try_send(PcmChunk {
            samples: samples.to_vec(),
        }) {
            Ok(()) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                sub.dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
        }
    }
}

/// Audio buffer size (samples per channel) — 256 @ 48kHz ≈ 5.3ms latency
pub const AUDIO_BUFFER_SIZE: usize = 256;
/// FFT size — 2048 @ 48kHz = 23Hz/bin resolution for clean bass separation
pub const FFT_SIZE: usize = 2048;
/// Hop size for overlapping analysis frames
const FFT_HOP: usize = AUDIO_BUFFER_SIZE;
/// Number of beat intervals to keep for BPM calculation
const BPM_HISTORY_SIZE: usize = 16;
/// Minimum time between beats (in seconds) to avoid double-triggering
const MIN_BEAT_INTERVAL: f32 = 0.2; // Max ~300 BPM
/// Maximum time between beats before we reset BPM tracking
const MAX_BEAT_INTERVAL: f32 = 2.0; // Min ~30 BPM
/// Window size for adaptive onset threshold (median of recent spectral flux)
const ONSET_MEDIAN_WINDOW: usize = 8;
/// Spectral flux must exceed median * this multiplier + offset to trigger onset
const ONSET_THRESHOLD_MULTIPLIER: f32 = 1.5;
/// Minimum spectral flux to trigger onset (prevents triggers in silence)
const ONSET_THRESHOLD_OFFSET: f32 = 0.01;
/// Reject beat intervals that deviate >15% from median
const TEMPO_TOLERANCE: f32 = 0.15;

/// Information about a detected audio input device.
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub id: AudioSourceId,
    pub name: String,
}

/// Audio analysis data sent to the rendering thread
#[derive(Clone)]
pub struct AudioData {
    /// Raw waveform data (normalized -1.0 to 1.0)
    pub waveform: Vec<f32>,
    /// FFT magnitude spectrum (0.0 to 1.0, normalized)
    pub fft: Vec<f32>,
    /// Current RMS level (0.0 to 1.0)
    pub level: f32,
    /// Detected BPM (if available)
    pub bpm: Option<f32>,
    /// Time since last beat (in seconds)
    pub time_since_beat: f32,
    /// Sample rate of the audio stream (needed to convert FFT bins to Hz)
    pub sample_rate: f32,
}

impl Default for AudioData {
    fn default() -> Self {
        Self {
            waveform: vec![0.0; AUDIO_BUFFER_SIZE],
            fft: vec![0.0; FFT_SIZE / 2],
            level: 0.0,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        }
    }
}

impl AudioData {
    /// Hz width of each FFT bin (derived from actual FFT data length)
    fn bin_width(&self) -> f32 {
        let fft_size = self.fft.len() * 2;
        self.sample_rate / fft_size as f32
    }

    /// Get energy in an arbitrary frequency range (Hz).
    /// This is the core method — bass/mid/treble are just presets on top of it.
    pub fn energy_in_range(&self, freq_low: f32, freq_high: f32) -> f32 {
        if self.fft.is_empty() {
            return 0.0;
        }
        let bw = self.bin_width();
        if bw <= 0.0 {
            return 0.0;
        }
        let bin_low = ((freq_low / bw).floor() as usize).min(self.fft.len() - 1);
        let bin_high = ((freq_high / bw).ceil() as usize).min(self.fft.len());
        if bin_high <= bin_low {
            return 0.0;
        }
        let slice = &self.fft[bin_low..bin_high];
        // RMS energy then dB-based perceptual mapping
        let rms = (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt();
        if rms < 1e-6 {
            return 0.0;
        }
        let db = 20.0 * rms.log10();
        ((db + 60.0) / 60.0).clamp(0.0, 1.0)
    }

    /// Get bass level (low frequencies, ~20-250Hz)
    pub fn bass(&self) -> f32 {
        self.energy_in_range(20.0, 250.0)
    }

    /// Get mid level (mid frequencies, ~250-2000Hz)
    pub fn mid(&self) -> f32 {
        self.energy_in_range(250.0, 2000.0)
    }

    /// Get treble level (high frequencies, ~2000Hz+)
    pub fn treble(&self) -> f32 {
        self.energy_in_range(2000.0, 20000.0)
    }

    /// Get beat phase (0.0 to 1.0, where 0.0 is on the beat)
    pub fn beat_phase(&self) -> f32 {
        if let Some(bpm) = self.bpm {
            let beat_duration = 60.0 / bpm;
            (self.time_since_beat / beat_duration).fract()
        } else {
            0.0
        }
    }
}

/// Compute the adaptive onset threshold from a window of spectral flux values.
/// Returns `median(flux_history) * ONSET_THRESHOLD_MULTIPLIER + ONSET_THRESHOLD_OFFSET`.
///
/// Uses `select_nth_unstable_by` (quickselect, O(n)) instead of a full sort
/// to find the median without allocating a new Vec.
pub fn compute_onset_threshold(flux_history: &[f32]) -> f32 {
    if flux_history.is_empty() {
        return ONSET_THRESHOLD_OFFSET;
    }
    let mut buf = flux_history.to_vec();
    let mid = buf.len() / 2;
    buf.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap());
    buf[mid] * ONSET_THRESHOLD_MULTIPLIER + ONSET_THRESHOLD_OFFSET
}

/// Estimate BPM from a history of beat intervals using median-based outlier rejection.
/// Returns `None` if fewer than 4 intervals, or if the resulting BPM is outside 30-300.
///
/// Uses `select_nth_unstable_by` (quickselect, O(n)) for the median, then
/// filters outliers using the original (unsorted) slice to avoid a second allocation.
pub fn estimate_bpm(beat_intervals: &[f32]) -> Option<f32> {
    if beat_intervals.len() < 4 {
        return None;
    }
    let mut buf = beat_intervals.to_vec();
    let mid = buf.len() / 2;
    buf.select_nth_unstable_by(mid, |a, b| a.partial_cmp(b).unwrap());
    let median = buf[mid];
    // Filter outliers and compute average in a single pass — no intermediate Vec.
    let (sum, count) = beat_intervals.iter().fold((0.0f32, 0u32), |(s, c), &iv| {
        if (iv - median).abs() / median < TEMPO_TOLERANCE {
            (s + iv, c + 1)
        } else {
            (s, c)
        }
    });
    if count >= 2 {
        let bpm = 60.0 / (sum / count as f32);
        if (30.0..=300.0).contains(&bpm) {
            return Some(bpm);
        }
    }
    None
}

/// An active audio source with its own capture stream.
struct ActiveAudioSource {
    _stream: cpal::Stream,
    receiver: Receiver<AudioData>,
    /// Latest polled data (cached between polls)
    pub latest: AudioData,
    /// Native sample rate (Hz) of the capture device.
    sample_rate_hz: u32,
    /// Native channel count of the capture device.
    channels: u16,
    /// Passthrough subscribers. Swapped lock-free so the real-time cpal
    /// callback reads without locking while subscribe/unsubscribe mutate.
    pcm_subs: Arc<ArcSwap<Vec<PcmSubscriber>>>,
}

/// Manages audio device enumeration and multiple simultaneous audio input streams.
///
/// Capture is **lazy and derived from state**: a device is open only while it has
/// at least one *holder*. Holders come in three kinds (see
/// [/spec/audio-capture-lifecycle.md](/spec/audio-capture-lifecycle.md)):
/// modulation references (`mod_refs`, reconciled per-frame via
/// [`set_modulation_refs`](Self::set_modulation_refs)), PCM passthrough
/// subscribers (`pcm_subs` on each source), and manual pins (`manual_pins`, via
/// [`open_source`](Self::open_source)). A device with none of these is orphaned
/// and closed.
pub struct AudioManager {
    /// Detected audio input devices (refreshed on scan).
    devices: Vec<AudioDeviceInfo>,
    /// Active audio sources keyed by AudioSourceId.
    active: HashMap<AudioSourceId, ActiveAudioSource>,
    /// Resolved default input id (OS default matched by name, else first),
    /// cached at scan time. Used to resolve `AudioBand { source_id: None }`.
    default_source_id: Option<AudioSourceId>,
    /// Devices explicitly pinned open by a consumer (`open_source`).
    manual_pins: HashSet<AudioSourceId>,
    /// Devices referenced by AudioBand modulators (last reconciled set).
    mod_refs: HashSet<AudioSourceId>,
}

impl Default for AudioManager {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            devices: Vec::new(),
            active: HashMap::new(),
            default_source_id: None,
            manual_pins: HashSet::new(),
            mod_refs: HashSet::new(),
        };
        // Enumerate devices and cache the default input, but open nothing —
        // capture starts only when a holder appears (issue #76).
        mgr.scan_devices();
        mgr
    }

    /// Choose which enumerated input is the default: the OS default input matched
    /// by name if it is present in the scanned list, otherwise the first
    /// enumerated device. Returns `None` when no inputs exist. Pure so it can be
    /// unit-tested without audio hardware.
    fn pick_default_input(
        default_name: Option<&str>,
        devices: &[AudioDeviceInfo],
    ) -> Option<AudioSourceId> {
        if let Some(name) = default_name {
            if let Some(dev) = devices.iter().find(|d| d.name == name) {
                return Some(dev.id);
            }
        }
        devices.first().map(|d| d.id)
    }

    /// Scan for available audio input devices and cache the resolved default.
    pub fn scan_devices(&mut self) {
        let host = cpal::default_host();
        self.devices = host
            .input_devices()
            .map(|devs| {
                devs.enumerate()
                    .map(|(i, d)| {
                        let name = d
                            .description()
                            .map(|desc| desc.name().to_string())
                            .unwrap_or_else(|_| format!("Audio Input {}", i));
                        AudioDeviceInfo {
                            id: i as AudioSourceId,
                            name,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Cache the default input (matched by OS-default name so we prefer the
        // user's mic/interface over a silent BlackHole loopback that merely
        // enumerates first). Queried here, off the per-frame hot path.
        let default_name = host
            .default_input_device()
            .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()));
        self.default_source_id = Self::pick_default_input(default_name.as_deref(), &self.devices);
        log::info!("Audio scan: found {} input device(s)", self.devices.len());
        for dev in &self.devices {
            log::info!("  Audio {}: {}", dev.id, dev.name);
        }
    }

    /// Get the list of detected audio input devices.
    pub fn devices(&self) -> &[AudioDeviceInfo] {
        &self.devices
    }

    /// Resolved default input id, used to resolve `AudioBand { source_id: None }`.
    pub fn default_source_id(&self) -> Option<AudioSourceId> {
        self.default_source_id
    }

    /// Manually pin a source open (a co-equal-consumer holder).
    ///
    /// Pins the device so it stays open regardless of modulation/passthrough
    /// holders, and starts capture if it isn't already running. Used by the HTTP
    /// API (`POST /audio/sources/{id}/open`) and `ToggleAudioSource`. Release
    /// with [`close_source`](Self::close_source).
    pub fn open_source(&mut self, id: AudioSourceId) -> Result<()> {
        self.manual_pins.insert(id);
        self.ensure_open(id)
    }

    /// Start capture for a source if it isn't already active. Idempotent.
    /// Does not register any holder — callers manage holder sets themselves.
    fn ensure_open(&mut self, id: AudioSourceId) -> Result<()> {
        if self.active.contains_key(&id) {
            return Ok(()); // Already open
        }

        let host = cpal::default_host();
        let device = host
            .input_devices()
            .context("Failed to enumerate audio devices")?
            .nth(id as usize)
            .context("Audio device not found")?;

        let dev_name = device
            .description()
            .map(|desc| desc.name().to_string())
            .unwrap_or_else(|_| format!("Audio {}", id));
        log::info!("Opening audio source {}: {}", id, dev_name);

        let config = device
            .default_input_config()
            .context("Failed to get default audio input config")?;
        let sample_rate = config.sample_rate() as f32;
        let sample_rate_hz = config.sample_rate() as u32;
        let channels = config.channels();
        log::info!("Audio config for '{}': {:?}", dev_name, config);

        let (sender, receiver) = bounded::<AudioData>(16);
        let pcm_subs = Arc::new(ArcSwap::from_pointee(Vec::new()));

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => Self::build_stream::<f32>(
                &device,
                &config.into(),
                sender,
                sample_rate,
                pcm_subs.clone(),
            )?,
            cpal::SampleFormat::I16 => Self::build_stream::<i16>(
                &device,
                &config.into(),
                sender,
                sample_rate,
                pcm_subs.clone(),
            )?,
            cpal::SampleFormat::U16 => Self::build_stream::<u16>(
                &device,
                &config.into(),
                sender,
                sample_rate,
                pcm_subs.clone(),
            )?,
            _ => anyhow::bail!("Unsupported sample format"),
        };

        stream.play().context("Failed to start audio stream")?;

        self.active.insert(
            id,
            ActiveAudioSource {
                _stream: stream,
                receiver,
                latest: AudioData {
                    sample_rate,
                    ..AudioData::default()
                },
                sample_rate_hz,
                channels,
                pcm_subs,
            },
        );

        Ok(())
    }

    /// Subscribe to raw PCM passthrough for a source, opening it if needed.
    ///
    /// Returns a fresh receiver plus the device's native [`AudioFormat`] so the
    /// caller can build matching ffmpeg input args. The audio callback fans raw
    /// interleaved `f32` to this and every other subscriber off one hardware
    /// clock, keeping analysis and passthrough coherent. Returns `None` if the
    /// source can't be opened.
    pub fn subscribe_pcm(&mut self, id: AudioSourceId) -> Option<PcmSubscription> {
        if !self.active.contains_key(&id) {
            if let Err(e) = self.ensure_open(id) {
                log::warn!("subscribe_pcm: failed to open audio source {}: {}", id, e);
                return None;
            }
        }
        let source = self.active.get(&id)?;
        let format = AudioFormat {
            sample_rate: source.sample_rate_hz,
            channels: source.channels,
        };
        let (sender, receiver) = bounded::<PcmChunk>(PCM_CHANNEL_CAPACITY);
        let token = NEXT_PCM_TOKEN.fetch_add(1, Ordering::Relaxed);
        let dropped = Arc::new(AtomicU64::new(0));
        let sub = PcmSubscriber {
            token,
            sender,
            dropped: dropped.clone(),
        };
        source.pcm_subs.rcu(|cur| {
            let mut next = Vec::with_capacity(cur.len() + 1);
            next.extend(cur.iter().cloned());
            next.push(sub.clone());
            next
        });
        log::info!(
            "PCM passthrough subscriber {} added to source {}",
            token,
            id
        );
        Some(PcmSubscription {
            receiver,
            format,
            token,
            dropped,
        })
    }

    /// Remove a PCM passthrough subscriber when its output stops.
    ///
    /// Dropping the last passthrough subscriber releases that holder; the device
    /// is closed if no modulation ref or manual pin still needs it.
    pub fn unsubscribe_pcm(&mut self, id: AudioSourceId, token: PcmToken) {
        if let Some(source) = self.active.get(&id) {
            source.pcm_subs.rcu(|cur| {
                cur.iter()
                    .filter(|s| s.token != token)
                    .cloned()
                    .collect::<Vec<_>>()
            });
            log::info!(
                "PCM passthrough subscriber {} removed from source {}",
                token,
                id
            );
        }
        self.close_if_orphaned(id);
    }

    /// Release a manual pin (`open_source`) and close the device if orphaned.
    pub fn close_source(&mut self, id: AudioSourceId) {
        self.manual_pins.remove(&id);
        self.close_if_orphaned(id);
    }

    /// Whether a source still has any holder: a manual pin, a modulation ref,
    /// or at least one PCM passthrough subscriber.
    fn has_holder(&self, id: AudioSourceId) -> bool {
        self.manual_pins.contains(&id)
            || self.mod_refs.contains(&id)
            || self
                .active
                .get(&id)
                .map(|s| !s.pcm_subs.load().is_empty())
                .unwrap_or(false)
    }

    /// Close and stop the `cpal` stream for a source if it has no remaining
    /// holder. No-op if the source is not open or is still held.
    fn close_if_orphaned(&mut self, id: AudioSourceId) {
        if self.has_holder(id) {
            return;
        }
        if self.active.remove(&id).is_some() {
            log::info!("Closed audio source {} (no remaining holders)", id);
        }
    }

    /// Reconcile the set of devices referenced by AudioBand modulators.
    ///
    /// Declarative per-frame entry point: `needed` is the resolved set of device
    /// ids that modulators currently require (with `None` already resolved to the
    /// default input). Opens newly-needed devices and closes devices that dropped
    /// out of the set and are no longer held by a pin or passthrough subscriber.
    /// See [/spec/audio-capture-lifecycle.md](/spec/audio-capture-lifecycle.md).
    pub fn set_modulation_refs(&mut self, needed: &BTreeSet<AudioSourceId>) {
        // Devices leaving the modulation set: drop the ref, then close if orphaned.
        let dropped: Vec<AudioSourceId> = self
            .mod_refs
            .iter()
            .filter(|id| !needed.contains(id))
            .copied()
            .collect();
        for id in dropped {
            self.mod_refs.remove(&id);
            self.close_if_orphaned(id);
        }
        // Devices entering the modulation set: register the ref, then ensure open.
        for &id in needed {
            if self.mod_refs.insert(id) || !self.active.contains_key(&id) {
                if let Err(e) = self.ensure_open(id) {
                    log::warn!(
                        "set_modulation_refs: failed to open audio source {}: {}",
                        id,
                        e
                    );
                    self.mod_refs.remove(&id);
                }
            }
        }
    }

    /// Resolve a list of AudioBand device selections into the concrete set of
    /// device ids that must be open. `None` selections resolve to `default` (and
    /// are dropped when no default input exists). Pure so it can be unit-tested
    /// without audio hardware.
    pub fn needed_from_bands(
        bands: &[Option<AudioSourceId>],
        default: Option<AudioSourceId>,
    ) -> BTreeSet<AudioSourceId> {
        bands.iter().filter_map(|sel| sel.or(default)).collect()
    }

    /// Poll all active sources for latest data. Call once per frame.
    pub fn poll(&mut self) {
        for source in self.active.values_mut() {
            while let Ok(data) = source.receiver.try_recv() {
                source.latest = data;
            }
        }
    }

    /// Get the latest AudioData for a specific source.
    pub fn get_data(&self, id: AudioSourceId) -> Option<&AudioData> {
        self.active.get(&id).map(|s| &s.latest)
    }

    /// Get the first active source's data (convenience for default/primary audio).
    pub fn get_primary_data(&self) -> &AudioData {
        // Return first active source's data, or a static default
        self.active
            .values()
            .next()
            .map(|s| &s.latest)
            .unwrap_or_else(|| {
                static DEFAULT: std::sync::LazyLock<AudioData> =
                    std::sync::LazyLock::new(AudioData::default);
                &DEFAULT
            })
    }

    /// Get IDs of all active (open) sources.
    pub fn active_source_ids(&self) -> Vec<AudioSourceId> {
        self.active.keys().copied().collect()
    }

    /// Check if any source is active.
    pub fn has_active_source(&self) -> bool {
        !self.active.is_empty()
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        sender: Sender<AudioData>,
        sample_rate: f32,
        pcm_subs: Arc<ArcSwap<Vec<PcmSubscriber>>>,
    ) -> Result<cpal::Stream>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let channels = config.channels as usize;
        let mut sample_buffer: Vec<f32> = Vec::with_capacity(AUDIO_BUFFER_SIZE);
        let mut fft_planner = FftPlanner::new();
        let fft = Arc::new(fft_planner.plan_fft_forward(FFT_SIZE));

        // Pre-compute Hann window to reduce spectral leakage
        let hann_window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
            })
            .collect();

        // Ring buffer for overlapping FFT frames
        let mut ring_buffer: Vec<f32> = vec![0.0; FFT_SIZE];
        let mut ring_write_pos: usize = 0;

        // Spectral flux onset detection state
        let mut prev_fft_magnitudes: Vec<f32> = vec![0.0; FFT_SIZE / 2];
        let mut flux_history: Vec<f32> = Vec::with_capacity(ONSET_MEDIAN_WINDOW + 1);

        // Pre-allocated FFT input buffer (H4: avoid per-callback allocation)
        let mut fft_input: Vec<Complex<f32>> = vec![Complex::new(0.0, 0.0); FFT_SIZE];

        // BPM detection state
        let mut last_beat_time = Instant::now();
        let mut beat_intervals: Vec<f32> = Vec::with_capacity(BPM_HISTORY_SIZE);
        let mut current_bpm: Option<f32> = None;

        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // Passthrough tee: forward raw interleaved PCM, untouched by the
                // FFT pipeline, to every subscriber. Skipped entirely when there
                // are no subscribers so the video-only path pays nothing.
                let subs = pcm_subs.load();
                if !subs.is_empty() {
                    let pcm: Vec<f32> = data
                        .iter()
                        .map(|s| <f32 as Sample>::from_sample(*s))
                        .collect();
                    fan_out_pcm(&subs, &pcm);
                }

                for chunk in data.chunks(channels) {
                    let sample: f32 = chunk
                        .iter()
                        .map(|s| <f32 as Sample>::from_sample(*s))
                        .sum::<f32>()
                        / channels as f32;
                    sample_buffer.push(sample);
                }

                while sample_buffer.len() >= FFT_HOP {
                    // Extract waveform chunk for GPU (256 samples)
                    let waveform: Vec<f32> = sample_buffer.drain(..FFT_HOP).collect();
                    let level =
                        (waveform.iter().map(|s| s * s).sum::<f32>() / FFT_HOP as f32).sqrt();

                    // Write hop into ring buffer (wrapping)
                    for (i, &s) in waveform.iter().enumerate() {
                        ring_buffer[(ring_write_pos + i) % FFT_SIZE] = s;
                    }
                    ring_write_pos = (ring_write_pos + FFT_HOP) % FFT_SIZE;

                    // Extract linearized 2048-sample frame, apply Hann window
                    // Reuse pre-allocated fft_input buffer (H4 optimisation)
                    for i in 0..FFT_SIZE {
                        let idx = (ring_write_pos + i) % FFT_SIZE;
                        fft_input[i] = Complex::new(ring_buffer[idx] * hann_window[i], 0.0);
                    }
                    fft.process(&mut fft_input);

                    // Magnitude scaling with noise floor
                    const NOISE_FLOOR: f32 = 1e-4;
                    let scale = 2.0 / FFT_SIZE as f32;
                    let fft_magnitudes: Vec<f32> = fft_input[..FFT_SIZE / 2]
                        .iter()
                        .map(|c: &Complex<f32>| {
                            let mag = c.norm() * scale;
                            if mag < NOISE_FLOOR {
                                0.0
                            } else {
                                mag
                            }
                        })
                        .collect();

                    // Spectral flux onset detection
                    let spectral_flux: f32 = fft_magnitudes
                        .iter()
                        .zip(prev_fft_magnitudes.iter())
                        .map(|(curr, prev)| (curr - prev).max(0.0))
                        .sum();

                    flux_history.push(spectral_flux);
                    if flux_history.len() > ONSET_MEDIAN_WINDOW {
                        flux_history.remove(0);
                    }
                    let onset_threshold = compute_onset_threshold(&flux_history);

                    let now = Instant::now();
                    let elapsed = now.duration_since(last_beat_time).as_secs_f32();
                    let is_onset = spectral_flux > onset_threshold && elapsed > MIN_BEAT_INTERVAL;
                    prev_fft_magnitudes.clone_from(&fft_magnitudes);

                    // BPM estimation with outlier rejection
                    if is_onset {
                        if elapsed < MAX_BEAT_INTERVAL {
                            beat_intervals.push(elapsed);
                            if beat_intervals.len() > BPM_HISTORY_SIZE {
                                beat_intervals.remove(0);
                            }
                            if let Some(bpm) = estimate_bpm(&beat_intervals) {
                                current_bpm = Some(bpm);
                            }
                        } else {
                            beat_intervals.clear();
                            current_bpm = None;
                        }
                        last_beat_time = now;
                    }

                    let time_since_beat = now.duration_since(last_beat_time).as_secs_f32();

                    let data = AudioData {
                        waveform,
                        fft: fft_magnitudes,
                        level,
                        bpm: current_bpm,
                        time_since_beat,
                        sample_rate,
                    };

                    let _ = sender.try_send(data);
                }
            },
            |err| log::error!("Audio stream error: {}", err),
            None,
        )?;

        Ok(stream)
    }
}

/// Audio texture manager - creates wgpu textures for ISF audio inputs
pub struct AudioTextures {
    /// Waveform texture (ISF "audio" input type)
    pub waveform_texture: wgpu::Texture,
    pub waveform_view: wgpu::TextureView,

    /// FFT texture (ISF "audioFFT" input type)
    pub fft_texture: wgpu::Texture,
    pub fft_view: wgpu::TextureView,
}

impl AudioTextures {
    /// Create audio textures
    pub fn new(device: &wgpu::Device) -> Self {
        // Create 1D-like textures (width=buffer_size, height=1)
        let waveform_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Audio Waveform Texture"),
            size: wgpu::Extent3d {
                width: AUDIO_BUFFER_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let fft_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Audio FFT Texture"),
            size: wgpu::Extent3d {
                width: (FFT_SIZE / 2) as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let waveform_view = waveform_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let fft_view = fft_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            waveform_texture,
            waveform_view,
            fft_texture,
            fft_view,
        }
    }

    /// Update textures with new audio data
    pub fn update(&self, queue: &wgpu::Queue, audio_data: &AudioData) {
        // Update waveform texture
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.waveform_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&audio_data.waveform),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(AUDIO_BUFFER_SIZE as u32 * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: AUDIO_BUFFER_SIZE as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );

        // Update FFT texture
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.fft_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck::cast_slice(&audio_data.fft),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some((FFT_SIZE / 2) as u32 * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: (FFT_SIZE / 2) as u32,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_window_shape() {
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| {
                0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
            })
            .collect();
        // Endpoints should be ~0
        assert!(window[0].abs() < 1e-6, "Hann window start should be ~0");
        assert!(
            window[FFT_SIZE - 1].abs() < 1e-6,
            "Hann window end should be ~0"
        );
        // Middle should be ~1
        let mid = window[FFT_SIZE / 2];
        assert!(
            (mid - 1.0).abs() < 0.01,
            "Hann window midpoint should be ~1.0, got {}",
            mid
        );
        // Should be symmetric
        for i in 0..FFT_SIZE / 2 {
            assert!(
                (window[i] - window[FFT_SIZE - 1 - i]).abs() < 1e-6,
                "Hann window should be symmetric at index {}",
                i
            );
        }
    }

    #[test]
    fn bin_resolution_at_48khz() {
        let data = AudioData {
            fft: vec![0.0; FFT_SIZE / 2],
            sample_rate: 48000.0,
            ..AudioData::default()
        };
        let bw = data.bin_width();
        // 48000 / 2048 = 23.4375 Hz/bin
        assert!(
            (bw - 23.4375).abs() < 0.01,
            "Bin width should be ~23.4Hz, got {}",
            bw
        );
        // Bass range (20-250Hz) should span ~10 bins
        let bass_bins = (250.0 / bw).ceil() as usize - (20.0 / bw).floor() as usize;
        assert!(
            bass_bins >= 9,
            "Bass range should span >=9 bins, got {}",
            bass_bins
        );
    }

    #[test]
    fn spectral_flux_detects_onset_not_steady_state() {
        // Steady-state: identical frames produce zero flux
        let frame_a = vec![0.1_f32; FFT_SIZE / 2];
        let flux_steady: f32 = frame_a
            .iter()
            .zip(frame_a.iter())
            .map(|(c, p)| (c - p).max(0.0))
            .sum();
        assert!(
            flux_steady.abs() < 1e-6,
            "Steady-state spectral flux should be ~0"
        );

        // Onset: sharp increase produces positive flux
        let frame_b: Vec<f32> = vec![0.5; FFT_SIZE / 2];
        let flux_onset: f32 = frame_b
            .iter()
            .zip(frame_a.iter())
            .map(|(c, p)| (c - p).max(0.0))
            .sum();
        assert!(flux_onset > 0.0, "Onset spectral flux should be positive");

        // Decrease: energy drop produces zero flux (half-wave rectified)
        let flux_decrease: f32 = frame_a
            .iter()
            .zip(frame_b.iter())
            .map(|(c, p)| (c - p).max(0.0))
            .sum();
        assert!(
            flux_decrease.abs() < 1e-6,
            "Energy decrease should produce zero flux"
        );
    }

    #[test]
    fn bpm_outlier_rejection() {
        // Simulate beat intervals: mostly ~0.5s (120 BPM) with one outlier
        let intervals: Vec<f32> = vec![0.50, 0.51, 0.49, 0.50, 0.52, 0.48, 1.2, 0.50];
        let mut sorted = intervals.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];

        let stable: Vec<f32> = intervals
            .iter()
            .filter(|&&iv| (iv - median).abs() / median < TEMPO_TOLERANCE)
            .copied()
            .collect();

        // Outlier (1.2s) should be rejected
        assert!(
            !stable.contains(&1.2),
            "Outlier interval should be rejected"
        );
        assert!(stable.len() >= 6, "Most intervals should survive filtering");

        let avg = stable.iter().sum::<f32>() / stable.len() as f32;
        let bpm = 60.0 / avg;
        assert!((bpm - 120.0).abs() < 5.0, "BPM should be ~120, got {}", bpm);
    }

    #[test]
    fn bpm_stability_consistent_beats() {
        // All consistent intervals at 128 BPM (0.46875s)
        let interval = 60.0 / 128.0;
        let intervals: Vec<f32> = vec![interval; BPM_HISTORY_SIZE];
        let mut sorted = intervals.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];

        let stable: Vec<f32> = intervals
            .iter()
            .filter(|&&iv| (iv - median).abs() / median < TEMPO_TOLERANCE)
            .copied()
            .collect();

        assert_eq!(
            stable.len(),
            BPM_HISTORY_SIZE,
            "All consistent intervals should pass"
        );
        let avg = stable.iter().sum::<f32>() / stable.len() as f32;
        let bpm = 60.0 / avg;
        assert!(
            (bpm - 128.0).abs() < 0.1,
            "BPM should be exactly ~128, got {}",
            bpm
        );
    }

    #[test]
    fn energy_in_range_with_new_fft_size() {
        // With 2048-point FFT at 48kHz, bin width is ~23.4Hz
        // Create synthetic FFT with energy only in bins 2-4 (~47-94Hz)
        let mut fft = vec![0.0_f32; FFT_SIZE / 2];
        fft[2] = 0.5;
        fft[3] = 0.5;
        fft[4] = 0.5;

        let data = AudioData {
            fft,
            sample_rate: 48000.0,
            waveform: vec![0.0; AUDIO_BUFFER_SIZE],
            level: 0.0,
            bpm: None,
            time_since_beat: 0.0,
        };

        // Energy in the active range should be non-zero
        let energy = data.energy_in_range(40.0, 100.0);
        assert!(energy > 0.0, "Should detect energy in 40-100Hz range");

        // Energy outside the active range should be zero
        let energy_high = data.energy_in_range(5000.0, 10000.0);
        assert!(
            energy_high < 1e-6,
            "Should detect no energy in 5k-10kHz range"
        );
    }

    // ── Chaos Tests Round 2: Audio frequency edge cases ─────────────────

    #[test]
    fn chaos_energy_in_range_nan_freq_low() {
        let data = AudioData {
            waveform: vec![0.0; 128],
            fft: vec![0.5; 1024],
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(f32::NAN, 1000.0);
        // NaN / bw → NaN, floor as usize → 0 (saturating), .min(len-1) → 0
        // Must not panic
        assert!(
            val.is_finite() || val == 0.0,
            "NaN freq_low should not crash"
        );
    }

    #[test]
    fn chaos_energy_in_range_nan_freq_high() {
        let data = AudioData {
            waveform: vec![0.0; 128],
            fft: vec![0.5; 1024],
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(100.0, f32::NAN);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_energy_in_range_both_nan() {
        let data = AudioData {
            waveform: vec![0.0; 128],
            fft: vec![0.5; 1024],
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(f32::NAN, f32::NAN);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_energy_in_range_negative_frequencies() {
        let data = AudioData {
            waveform: vec![0.0; 128],
            fft: vec![0.5; 1024],
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(-1000.0, -500.0);
        // Negative / bw → negative, floor as usize → 0 (saturating)
        assert!(
            val >= 0.0,
            "negative freq should not produce negative energy"
        );
    }

    #[test]
    fn chaos_energy_in_range_infinity() {
        let data = AudioData {
            waveform: vec![0.0; 128],
            fft: vec![0.5; 1024],
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(0.0, f32::INFINITY);
        let _ = val; // must not panic
    }

    #[test]
    fn chaos_energy_in_range_inverted_range() {
        let data = AudioData {
            waveform: vec![0.0; 128],
            fft: vec![0.5; 1024],
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(5000.0, 100.0);
        assert_eq!(val, 0.0, "inverted range should return 0.0");
    }

    #[test]
    fn chaos_energy_in_range_zero_sample_rate() {
        let data = AudioData {
            waveform: vec![0.0; 128],
            fft: vec![0.5; 1024],
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 0.0,
        };
        let val = data.energy_in_range(100.0, 1000.0);
        // bin_width = 0 / 2048 = 0, guarded by bw <= 0.0 check
        assert_eq!(val, 0.0, "zero sample rate should return 0.0");
    }

    #[test]
    fn chaos_energy_in_range_empty_fft() {
        let data = AudioData {
            waveform: vec![],
            fft: vec![],
            level: 0.0,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(20.0, 20000.0);
        assert_eq!(val, 0.0, "empty FFT should return 0.0");
    }

    // ── Phase 19a: PCM passthrough tap ─────────────────────────────────

    fn make_sub(cap: usize) -> (PcmSubscriber, Receiver<PcmChunk>, Arc<AtomicU64>) {
        let (sender, receiver) = bounded::<PcmChunk>(cap);
        let dropped = Arc::new(AtomicU64::new(0));
        let sub = PcmSubscriber {
            token: NEXT_PCM_TOKEN.fetch_add(1, Ordering::Relaxed),
            sender,
            dropped: dropped.clone(),
        };
        (sub, receiver, dropped)
    }

    #[test]
    fn pcm_fan_out_delivers_to_all_subscribers() {
        let (sub1, rx1, _) = make_sub(4);
        let (sub2, rx2, _) = make_sub(4);
        let subs = vec![sub1, sub2];
        let samples = vec![0.1, -0.2, 0.3, -0.4];

        fan_out_pcm(&subs, &samples);

        assert_eq!(rx1.try_recv().unwrap().samples, samples);
        assert_eq!(rx2.try_recv().unwrap().samples, samples);
    }

    #[test]
    fn pcm_fan_out_counts_drops_when_full() {
        // Capacity 2, receiver never drains: 5 sends → 2 buffered, 3 dropped.
        let (sub, _rx, dropped) = make_sub(2);
        let subs = vec![sub];
        let samples = vec![0.0; 8];

        for _ in 0..5 {
            fan_out_pcm(&subs, &samples);
        }

        assert_eq!(dropped.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn pcm_fan_out_skips_disconnected_without_counting_drops() {
        let (sub, rx, dropped) = make_sub(2);
        drop(rx); // receiver gone → Disconnected, not Full
        let subs = vec![sub];

        fan_out_pcm(&subs, &[0.0; 4]);

        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn pcm_fan_out_empty_is_noop() {
        let subs: Vec<PcmSubscriber> = Vec::new();
        fan_out_pcm(&subs, &[0.0; 4]); // must not panic
    }

    fn dev(id: AudioSourceId, name: &str) -> AudioDeviceInfo {
        AudioDeviceInfo {
            id,
            name: name.to_string(),
        }
    }

    #[test]
    fn pick_default_input_prefers_os_default_by_name() {
        let devices = vec![dev(0, "BlackHole 2ch"), dev(1, "MacBook Pro Microphone")];
        // OS default is the mic (id 1), even though BlackHole enumerates first.
        assert_eq!(
            AudioManager::pick_default_input(Some("MacBook Pro Microphone"), &devices),
            Some(1)
        );
    }

    #[test]
    fn pick_default_input_falls_back_to_first_when_no_default() {
        let devices = vec![dev(0, "BlackHole 2ch"), dev(1, "MacBook Pro Microphone")];
        assert_eq!(AudioManager::pick_default_input(None, &devices), Some(0));
    }

    #[test]
    fn pick_default_input_falls_back_to_first_when_default_absent() {
        let devices = vec![dev(0, "BlackHole 2ch"), dev(1, "MacBook Pro Microphone")];
        // Default reported by the OS isn't in the scanned list → first device.
        assert_eq!(
            AudioManager::pick_default_input(Some("USB Interface"), &devices),
            Some(0)
        );
    }

    #[test]
    fn pick_default_input_none_when_no_devices() {
        assert_eq!(
            AudioManager::pick_default_input(Some("MacBook Pro Microphone"), &[]),
            None
        );
        assert_eq!(AudioManager::pick_default_input(None, &[]), None);
    }

    #[test]
    fn needed_from_bands_resolves_none_to_default() {
        // A `None` band resolves to the default input; an explicit device is kept.
        let bands = vec![None, Some(3)];
        let needed = AudioManager::needed_from_bands(&bands, Some(1));
        assert!(needed.contains(&1), "None must resolve to default (1)");
        assert!(needed.contains(&3), "explicit device 3 must be kept");
        assert_eq!(needed.len(), 2);
    }

    #[test]
    fn needed_from_bands_dedups_shared_device() {
        // Two bands on the same device collapse to one capture.
        let bands = vec![Some(2), Some(2), None];
        let needed = AudioManager::needed_from_bands(&bands, Some(2));
        assert_eq!(needed.len(), 1);
        assert!(needed.contains(&2));
    }

    #[test]
    fn needed_from_bands_drops_none_without_default() {
        // No default input available → `None` bands demand nothing.
        let bands = vec![None, None];
        let needed = AudioManager::needed_from_bands(&bands, None);
        assert!(needed.is_empty());
    }

    #[test]
    fn needed_from_bands_empty_is_empty() {
        let needed = AudioManager::needed_from_bands(&[], Some(0));
        assert!(needed.is_empty(), "no bands → capture nothing (issue #76)");
    }

    #[test]
    fn audio_format_carries_native_layout() {
        let fmt = AudioFormat {
            sample_rate: 44100,
            channels: 2,
        };
        assert_eq!(fmt.sample_rate, 44100);
        assert_eq!(fmt.channels, 2);
    }

    #[test]
    fn chaos_energy_in_range_single_bin_fft() {
        let data = AudioData {
            waveform: vec![0.0; 2],
            fft: vec![1.0],
            level: 1.0,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(0.0, 48000.0);
        // single bin FFT — should not panic
        assert!(val.is_finite());
    }
}
