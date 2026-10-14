//! Audio input and analysis for audio-reactive shaders.

pub(crate) mod analysis;

use anyhow::{Context, Result};
use arc_swap::ArcSwap;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender, bounded};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Opaque audio source identifier.
pub type AudioSourceId = u32;

/// Identifies one PCM passthrough subscription.
pub type PcmToken = u64;

/// Capacity in chunks of a passthrough PCM channel. One chunk per cpal callback
/// (~10 ms), so 32 is ~320 ms of slack before drops.
const PCM_CHANNEL_CAPACITY: usize = 32;

/// Source of unique [`PcmToken`]s.
static NEXT_PCM_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Native PCM layout of a capture device, so passthrough subscribers can build
/// matching ffmpeg input args.
#[derive(Debug, Clone, Copy)]
pub struct AudioFormat {
    pub sample_rate: u32,
    pub channels: u16,
}

/// Raw interleaved PCM at the device's native channel count and rate, teed from
/// the capture callback for output passthrough. [`AudioData`] is the mono,
/// FFT-processed form used for analysis.
pub struct PcmChunk {
    /// Interleaved `f32` samples in the device's native channel count.
    pub samples: Vec<f32>,
}

/// A passthrough consumer. The capture callback sends raw PCM to every
/// subscriber on a source.
#[derive(Clone)]
pub(crate) struct PcmSubscriber {
    token: PcmToken,
    sender: Sender<PcmChunk>,
    dropped: Arc<AtomicU64>,
    lost_samples: Arc<AtomicU64>,
}

/// Returned by [`AudioManager::subscribe_pcm`]. `format` is the PCM layout,
/// `token` identifies the subscription for [`AudioManager::unsubscribe_pcm`],
/// and `dropped` counts backpressure drops.
pub struct PcmSubscription {
    pub receiver: Receiver<PcmChunk>,
    pub format: AudioFormat,
    pub token: PcmToken,
    pub dropped: Arc<AtomicU64>,
    /// Samples lost to drops. The consumer must insert this much silence; see
    /// `AudioPipe::start` in the ffmpeg subprocess.
    pub lost_samples: Arc<AtomicU64>,
}

/// Sends a chunk of raw interleaved PCM to every passthrough subscriber.
///
/// Never blocks, since this runs on the real-time capture callback. On a full
/// channel the chunk is dropped and its sample count added to `lost_samples`.
/// PCM is timed only by sample count, so consumers must replace lost samples
/// with silence or everything after the drop plays early. Disconnected
/// subscribers are skipped.
fn fan_out_pcm(subs: &[PcmSubscriber], samples: &[f32]) {
    for sub in subs {
        match sub.sender.try_send(PcmChunk {
            samples: samples.to_vec(),
        }) {
            Ok(()) | Err(crossbeam_channel::TrySendError::Disconnected(_)) => {}
            Err(crossbeam_channel::TrySendError::Full(_)) => {
                sub.dropped.fetch_add(1, Ordering::Relaxed);
                sub.lost_samples
                    .fetch_add(samples.len() as u64, Ordering::Relaxed);
            }
        }
    }
}

/// Samples per channel per buffer: 256 at 48 kHz ≈ 5.3 ms latency.
pub const AUDIO_BUFFER_SIZE: usize = 256;
/// FFT size: 2048 at 48 kHz gives 23 Hz bins, enough to separate bass.
pub const FFT_SIZE: usize = 2048;
/// Hop between overlapping analysis frames.
const FFT_HOP: usize = AUDIO_BUFFER_SIZE;
/// Band (Hz) whose spectral flux drives onsets: the kick, not hats, snare or
/// voice.
const ONSET_BAND_HZ: (f32, f32) = (30.0, 150.0);
/// Minimum seconds between onsets, so one kick's attack, spread over several
/// hops by the 2048-sample frame, fires once.
const MIN_ONSET_INTERVAL: f32 = 0.05;
/// Window for the adaptive onset threshold (median of recent onset strength).
const ONSET_MEDIAN_WINDOW: usize = 8;
/// Onset fires when onset strength exceeds median * this + offset.
const ONSET_THRESHOLD_MULTIPLIER: f32 = 1.5;
/// Minimum onset strength for an onset. Onset strength is flux relative to the
/// band's recent level, so this doesn't depend on input gain.
const ONSET_THRESHOLD_OFFSET: f32 = 0.1;
/// RMS level below which input counts as silent (-60 dBFS).
const SILENCE_LEVEL: f32 = 1e-3;
/// Seconds of silence, or without an onset, before tempo tracking resets.
const SILENCE_RESET_SECS: f32 = 2.0;
/// Seconds of onset strength kept for tempo estimation.
const TEMPO_WINDOW_SECS: f32 = 8.0;
/// Seconds of onset strength needed before the first tempo estimate.
const TEMPO_MIN_SECS: f32 = 4.0;
/// Seconds between tempo estimates.
const TEMPO_UPDATE_SECS: f32 = 0.5;
/// Tempo range (BPM) searched for the beat period, before folding into the
/// user's [`TempoRange`].
const TEMPO_SEARCH_BPM: (f32, f32) = (40.0, 320.0);
/// Multiples of a candidate period the comb sums over the autocorrelation.
const TEMPO_COMB_HARMONICS: usize = 4;
/// Minimum comb score, relative to a perfectly periodic envelope, for an
/// estimate to count.
const TEMPO_MIN_CONFIDENCE: f32 = 0.2;
/// Estimates within this relative distance agree.
const TEMPO_TOLERANCE: f32 = 0.04;
/// Agreeing estimates needed before a different tempo replaces the current one.
const TEMPO_CONFIRMATIONS: u32 = 3;
/// How far each agreeing estimate moves the current tempo toward it.
const TEMPO_SMOOTHING: f32 = 0.3;

/// Tempo range (BPM) that detected tempos are folded into by octaves, set from
/// the clock popover. `max` is at least twice `min`, so every tempo has an
/// octave inside it.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct TempoRange {
    pub min: f32,
    pub max: f32,
}

impl Default for TempoRange {
    fn default() -> Self {
        Self {
            min: 70.0,
            max: 240.0,
        }
    }
}

impl TempoRange {
    /// Bounds either end can take: the tempos the estimator searches.
    pub const LIMITS: (f32, f32) = TEMPO_SEARCH_BPM;

    /// A range within [`Self::LIMITS`], widened to an octave by moving `max`
    /// up. Non-finite ends fall back to the default.
    pub fn new(min: f32, max: f32) -> Self {
        let default = Self::default();
        let min = if min.is_finite() { min } else { default.min };
        let max = if max.is_finite() { max } else { default.max };
        let min = min.clamp(Self::LIMITS.0, Self::LIMITS.1 / 2.0);
        let max = max.clamp(2.0 * min, Self::LIMITS.1);
        Self { min, max }
    }

    /// Doubles or halves a positive `bpm` into the range.
    fn fold(self, mut bpm: f32) -> f32 {
        while bpm < self.min {
            bpm *= 2.0;
        }
        while bpm > self.max {
            bpm /= 2.0;
        }
        bpm
    }

    /// Both ends packed into one word, for the analysis threads.
    pub(crate) fn to_bits(self) -> u64 {
        (u64::from(self.min.to_bits()) << 32) | u64::from(self.max.to_bits())
    }

    pub(crate) fn from_bits(bits: u64) -> Self {
        Self {
            min: f32::from_bits((bits >> 32) as u32),
            max: f32::from_bits(bits as u32),
        }
    }
}

/// A detected audio input device.
#[derive(Debug, Clone)]
pub struct AudioDeviceInfo {
    pub id: AudioSourceId,
    pub name: String,
}

/// Audio analysis for the render thread. The sample arrays are shared, so
/// cloning costs two refcount bumps.
#[derive(Clone)]
pub struct AudioData {
    /// Raw waveform, -1.0 to 1.0.
    pub waveform: std::sync::Arc<[f32]>,
    /// FFT magnitude spectrum, normalized 0.0 to 1.0.
    pub fft: std::sync::Arc<[f32]>,
    /// RMS level, 0.0 to 1.0.
    pub level: f32,
    /// Detected BPM (if available)
    pub bpm: Option<f32>,
    /// Seconds since the last beat of the tracked beat grid while `bpm` is
    /// known, else since the last onset.
    pub time_since_beat: f32,
    /// Hz; needed to convert FFT bins to frequencies.
    pub sample_rate: f32,
}

impl Default for AudioData {
    fn default() -> Self {
        Self {
            waveform: vec![0.0; AUDIO_BUFFER_SIZE].into(),
            fft: vec![0.0; FFT_SIZE / 2].into(),
            level: 0.0,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        }
    }
}

impl AudioData {
    /// Hz per FFT bin, from the FFT data length.
    fn bin_width(&self) -> f32 {
        let fft_size = self.fft.len() * 2;
        self.sample_rate / fft_size as f32
    }

    /// Energy in a frequency range (Hz). `bass`, `mid`, and `treble` are presets
    /// over this.
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
        // RMS energy, then a dB-based perceptual mapping.
        let rms = (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt();
        if rms < 1e-6 {
            return 0.0;
        }
        let db = 20.0 * rms.log10();
        ((db + 60.0) / 60.0).clamp(0.0, 1.0)
    }

    /// Bass level, ~20-250 Hz.
    pub fn bass(&self) -> f32 {
        self.energy_in_range(20.0, 250.0)
    }

    /// Mid level, ~250-2000 Hz.
    pub fn mid(&self) -> f32 {
        self.energy_in_range(250.0, 2000.0)
    }

    /// Treble level, ~2000 Hz and up.
    pub fn treble(&self) -> f32 {
        self.energy_in_range(2000.0, 20000.0)
    }

    /// Beat phase, 0.0 to 1.0, where 0.0 is on the beat. While `bpm` is known
    /// this follows the analysis's beat grid, which onsets only nudge, so a
    /// stray onset doesn't reset it.
    pub fn beat_phase(&self) -> f32 {
        if let Some(bpm) = self.bpm {
            let beat_duration = 60.0 / bpm;
            (self.time_since_beat / beat_duration).fract()
        } else {
            0.0
        }
    }
}

/// Copies the finite values into a scratch buffer for median selection.
fn finite_values(values: &[f32]) -> Vec<f32> {
    // Non-short-circuiting, so the common all-finite case stays a memcpy.
    if values.iter().fold(true, |ok, v| ok & v.is_finite()) {
        values.to_vec()
    } else {
        values.iter().copied().filter(|v| v.is_finite()).collect()
    }
}

/// Comparison for values known to be finite.
fn cmp_finite(a: f32, b: f32) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Adaptive onset threshold for a window of spectral flux values:
/// `median(flux_history) * ONSET_THRESHOLD_MULTIPLIER + ONSET_THRESHOLD_OFFSET`.
/// The median uses quickselect (`select_nth_unstable_by`). Non-finite values
/// are skipped, so the comparison is a total order.
pub fn compute_onset_threshold(flux_history: &[f32]) -> f32 {
    let mut buf: Vec<f32> = finite_values(flux_history);
    if buf.is_empty() {
        return ONSET_THRESHOLD_OFFSET;
    }
    let mid = buf.len() / 2;
    buf.select_nth_unstable_by(mid, |a, b| cmp_finite(*a, *b));
    buf[mid] * ONSET_THRESHOLD_MULTIPLIER + ONSET_THRESHOLD_OFFSET
}

/// Estimates BPM from an onset strength envelope sampled at `hop_rate` Hz.
///
/// Scores each candidate period in [`TEMPO_SEARCH_BPM`] by summing the
/// envelope's autocorrelation at its first [`TEMPO_COMB_HARMONICS`] multiples.
/// Missing or stray onsets only lower some terms, the true period beats its
/// double (fewer multiples fit the window) and its half (every other multiple
/// misses). The best candidate is folded by octaves into `range`.
/// `None` without a clear periodicity. Non-finite values count as zero.
pub fn estimate_bpm(envelope: &[f32], hop_rate: f32, range: TempoRange) -> Option<f32> {
    const STEP_BPM: f32 = 0.5;
    let n = envelope.len();
    if n < 2 || !(hop_rate.is_finite() && hop_rate > 0.0) {
        return None;
    }
    let finite = |v: &f32| if v.is_finite() { *v } else { 0.0 };
    let mean = envelope.iter().map(finite).sum::<f32>() / n as f32;
    let centered: Vec<f32> = envelope.iter().map(|v| finite(v) - mean).collect();

    let period = |bpm: f32| hop_rate * 60.0 / bpm;
    let max_lag =
        ((TEMPO_COMB_HARMONICS as f32 * period(TEMPO_SEARCH_BPM.0)).ceil() as usize + 1).min(n - 1);
    let acf: Vec<f32> = (0..=max_lag)
        .map(|lag| {
            centered[..n - lag]
                .iter()
                .zip(&centered[lag..])
                .map(|(a, b)| a * b)
                .sum::<f32>()
        })
        .collect();
    if !(acf[0].is_finite() && acf[0] > 0.0) {
        return None;
    }
    // Linear interpolation; lags past the window contribute nothing.
    let acf_at = |lag: f32| {
        let i = lag as usize;
        if i >= max_lag {
            return 0.0;
        }
        let t = lag - i as f32;
        acf[i] * (1.0 - t) + acf[i + 1] * t
    };
    let comb = |bpm: f32| {
        (1..=TEMPO_COMB_HARMONICS)
            .map(|k| acf_at(k as f32 * period(bpm)))
            .sum::<f32>()
    };
    let candidate = |i: usize| TEMPO_SEARCH_BPM.0 + i as f32 * STEP_BPM;

    let steps = ((TEMPO_SEARCH_BPM.1 - TEMPO_SEARCH_BPM.0) / STEP_BPM) as usize;
    let scores: Vec<f32> = (0..=steps).map(|i| comb(candidate(i))).collect();
    let (best, &score) = scores
        .iter()
        .enumerate()
        .max_by(|a, b| cmp_finite(*a.1, *b.1))?;
    if score / (TEMPO_COMB_HARMONICS as f32 * acf[0]) < TEMPO_MIN_CONFIDENCE {
        return None;
    }
    // Parabolic refinement between grid steps.
    let offset = if best > 0 && best < steps {
        let (l, r) = (scores[best - 1], scores[best + 1]);
        let curvature = l - 2.0 * score + r;
        if curvature < 0.0 {
            (0.5 * (l - r) / curvature).clamp(-0.5, 0.5)
        } else {
            0.0
        }
    } else {
        0.0
    };
    Some(range.fold(candidate(best) + offset * STEP_BPM))
}

/// An open audio source and its capture stream.
struct ActiveAudioSource {
    _stream: cpal::Stream,
    receiver: Receiver<AudioData>,
    /// Latest polled data, cached between polls.
    pub latest: AudioData,
    /// Native sample rate in Hz.
    sample_rate_hz: u32,
    /// Native channel count.
    channels: u16,
    /// Passthrough subscribers, swapped lock-free so the capture callback never
    /// takes a lock.
    pcm_subs: Arc<ArcSwap<Vec<PcmSubscriber>>>,
}

/// Audio device enumeration and input streams.
///
/// A device is open only while it has at least one holder: a modulation ref
/// (`mod_refs`, reconciled each frame by
/// [`set_modulation_refs`](Self::set_modulation_refs)), a PCM passthrough
/// subscriber (`pcm_subs` on each source), or a manual pin (`manual_pins`, via
/// [`open_source`](Self::open_source)). A device with no holder is closed.
pub struct AudioManager {
    /// Detected input devices, refreshed on scan.
    devices: Vec<AudioDeviceInfo>,
    active: HashMap<AudioSourceId, ActiveAudioSource>,
    /// Default input id (OS default matched by name, else first), cached at
    /// scan time. Resolves `AudioBand { source_id: None }`.
    default_source_id: Option<AudioSourceId>,
    /// Devices pinned open by `open_source`.
    manual_pins: HashSet<AudioSourceId>,
    /// Devices referenced by `AudioBand` modulators, as last reconciled.
    mod_refs: HashSet<AudioSourceId>,
    /// [`TempoRange`] bits, read by every source's analysis thread.
    tempo_range: Arc<AtomicU64>,
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
            tempo_range: Arc::new(AtomicU64::new(TempoRange::default().to_bits())),
        };
        // Enumerate and cache the default input, but open nothing: capture
        // starts when a holder appears.
        mgr.scan_devices();
        mgr
    }

    /// Picks the default input: the OS default matched by name if present,
    /// else the first device. `None` with no inputs. Pure, for testing without
    /// audio hardware.
    fn pick_default_input(
        default_name: Option<&str>,
        devices: &[AudioDeviceInfo],
    ) -> Option<AudioSourceId> {
        if let Some(name) = default_name
            && let Some(dev) = devices.iter().find(|d| d.name == name)
        {
            return Some(dev.id);
        }
        devices.first().map(|d| d.id)
    }

    /// Scans input devices and caches the default.
    pub fn scan_devices(&mut self) {
        let host = cpal::default_host();
        self.devices = host
            .input_devices()
            .map(|devs| {
                devs.enumerate()
                    .map(|(i, d)| {
                        let name = d.description().map_or_else(
                            |_| format!("Audio Input {i}"),
                            |desc| desc.name().to_string(),
                        );
                        AudioDeviceInfo {
                            id: i as AudioSourceId,
                            name,
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Match the OS default by name, so the user's mic/interface wins over a
        // silent BlackHole loopback that enumerates first. Done here, off the
        // per-frame path.
        let default_name = host
            .default_input_device()
            .and_then(|d| d.description().ok().map(|desc| desc.name().to_string()));
        self.default_source_id = Self::pick_default_input(default_name.as_deref(), &self.devices);
        log::info!("Audio scan: found {} input device(s)", self.devices.len());
        for dev in &self.devices {
            log::info!("  Audio {}: {}", dev.id, dev.name);
        }
    }

    /// Tempo range detected tempos are folded into.
    pub fn tempo_range(&self) -> TempoRange {
        TempoRange::from_bits(self.tempo_range.load(Ordering::Relaxed))
    }

    /// Sets the tempo range for every source, open now or later. Tempo
    /// tracking restarts in the new range.
    pub fn set_tempo_range(&mut self, range: TempoRange) {
        let range = TempoRange::new(range.min, range.max);
        self.tempo_range.store(range.to_bits(), Ordering::Relaxed);
    }

    pub fn devices(&self) -> &[AudioDeviceInfo] {
        &self.devices
    }

    /// Default input id, which resolves `AudioBand { source_id: None }`.
    pub fn default_source_id(&self) -> Option<AudioSourceId> {
        self.default_source_id
    }

    /// Pins a source open regardless of other holders and starts capture if
    /// needed. Used by `POST /audio/sources/{id}/open` and `ToggleAudioSource`.
    /// Release with [`close_source`](Self::close_source).
    ///
    /// # Errors
    ///
    /// Returns an error if input devices can't be enumerated, no device exists
    /// at `id`, the device has no default input config, its sample format isn't
    /// `f32`/`i16`/`u16`, or the cpal stream fails to build or start.
    pub fn open_source(&mut self, id: AudioSourceId) -> Result<()> {
        self.manual_pins.insert(id);
        self.ensure_open(id)
    }

    /// Starts capture if not already active. Registers no holder; callers
    /// manage holder sets.
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
            .map_or_else(|_| format!("Audio {id}"), |desc| desc.name().to_string());
        log::info!("Opening audio source {id}: {dev_name}");

        let config = device
            .default_input_config()
            .context("Failed to get default audio input config")?;
        let sample_rate = config.sample_rate() as f32;
        let sample_rate_hz = config.sample_rate() as u32;
        let channels = config.channels();
        log::info!("Audio config for '{dev_name}': {config:?}");

        let (sender, receiver) = bounded::<AudioData>(16);
        let pcm_subs = Arc::new(ArcSwap::from_pointee(Vec::new()));

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => Self::build_stream::<f32>(
                &device,
                &config.into(),
                sender,
                sample_rate,
                pcm_subs.clone(),
                self.tempo_range.clone(),
            )?,
            cpal::SampleFormat::I16 => Self::build_stream::<i16>(
                &device,
                &config.into(),
                sender,
                sample_rate,
                pcm_subs.clone(),
                self.tempo_range.clone(),
            )?,
            cpal::SampleFormat::U16 => Self::build_stream::<u16>(
                &device,
                &config.into(),
                sender,
                sample_rate,
                pcm_subs.clone(),
                self.tempo_range.clone(),
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

    /// Subscribes to raw PCM passthrough for a source, opening it if needed.
    /// Returns a receiver and the device's native [`AudioFormat`] for ffmpeg
    /// input args. Analysis and every subscriber share one hardware clock.
    /// `None` if the source can't be opened.
    pub fn subscribe_pcm(&mut self, id: AudioSourceId) -> Option<PcmSubscription> {
        if !self.active.contains_key(&id)
            && let Err(e) = self.ensure_open(id)
        {
            log::warn!("subscribe_pcm: failed to open audio source {id}: {e}");
            return None;
        }
        let source = self.active.get(&id)?;
        let format = AudioFormat {
            sample_rate: source.sample_rate_hz,
            channels: source.channels,
        };
        let (sender, receiver) = bounded::<PcmChunk>(PCM_CHANNEL_CAPACITY);
        let token = NEXT_PCM_TOKEN.fetch_add(1, Ordering::Relaxed);
        let dropped = Arc::new(AtomicU64::new(0));
        let lost_samples = Arc::new(AtomicU64::new(0));
        let sub = PcmSubscriber {
            token,
            sender,
            dropped: dropped.clone(),
            lost_samples: lost_samples.clone(),
        };
        source.pcm_subs.rcu(|cur| {
            let mut next = Vec::with_capacity(cur.len() + 1);
            next.extend(cur.iter().cloned());
            next.push(sub.clone());
            next
        });
        log::info!("PCM passthrough subscriber {token} added to source {id}");
        Some(PcmSubscription {
            receiver,
            format,
            token,
            dropped,
            lost_samples,
        })
    }

    /// Removes a PCM passthrough subscriber when its output stops, and closes
    /// the device if nothing else holds it.
    pub fn unsubscribe_pcm(&mut self, id: AudioSourceId, token: PcmToken) {
        if let Some(source) = self.active.get(&id) {
            source.pcm_subs.rcu(|cur| {
                cur.iter()
                    .filter(|s| s.token != token)
                    .cloned()
                    .collect::<Vec<_>>()
            });
            log::info!("PCM passthrough subscriber {token} removed from source {id}");
        }
        self.close_if_orphaned(id);
    }

    /// Releases a manual pin and closes the device if nothing else holds it.
    pub fn close_source(&mut self, id: AudioSourceId) {
        self.manual_pins.remove(&id);
        self.close_if_orphaned(id);
    }

    /// Whether a source has any holder: manual pin, modulation ref, or PCM
    /// passthrough subscriber.
    fn has_holder(&self, id: AudioSourceId) -> bool {
        self.manual_pins.contains(&id)
            || self.mod_refs.contains(&id)
            || self
                .active
                .get(&id)
                .is_some_and(|s| !s.pcm_subs.load().is_empty())
    }

    /// Stops the `cpal` stream for a source with no holder. No-op if closed or
    /// still held.
    fn close_if_orphaned(&mut self, id: AudioSourceId) {
        if self.has_holder(id) {
            return;
        }
        if self.active.remove(&id).is_some() {
            log::info!("Closed audio source {id} (no remaining holders)");
        }
    }

    /// Reconciles the devices referenced by `AudioBand` modulators. Call each
    /// frame with the needed device ids (`None` already resolved to the
    /// default). Opens new ones and closes dropped ones no longer held by a pin
    /// or passthrough subscriber.
    pub fn set_modulation_refs(&mut self, needed: &BTreeSet<AudioSourceId>) {
        // Leaving the set: drop the ref, then close if nothing holds it.
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
        // Entering the set: add the ref, then open.
        for &id in needed {
            if (self.mod_refs.insert(id) || !self.active.contains_key(&id))
                && let Err(e) = self.ensure_open(id)
            {
                log::warn!("set_modulation_refs: failed to open audio source {id}: {e}");
                self.mod_refs.remove(&id);
            }
        }
    }

    /// Resolves `AudioBand` device selections to the set of device ids to open.
    /// `None` resolves to `default`, or is dropped with no default input. Pure,
    /// for testing without audio hardware.
    pub fn needed_from_bands(
        bands: &[Option<AudioSourceId>],
        default: Option<AudioSourceId>,
    ) -> BTreeSet<AudioSourceId> {
        bands.iter().filter_map(|sel| sel.or(default)).collect()
    }

    /// Polls every active source. Call once per frame.
    pub fn poll(&mut self) {
        for source in self.active.values_mut() {
            while let Ok(data) = source.receiver.try_recv() {
                source.latest = data;
            }
        }
    }

    /// Latest `AudioData` for a source.
    pub fn get_data(&self, id: AudioSourceId) -> Option<&AudioData> {
        self.active.get(&id).map(|s| &s.latest)
    }

    /// First active source's data, or a default.
    pub fn get_primary_data(&self) -> &AudioData {
        static DEFAULT: std::sync::LazyLock<AudioData> =
            std::sync::LazyLock::new(AudioData::default);
        self.active
            .values()
            .next()
            .map_or_else(|| &*DEFAULT, |s| &s.latest)
    }

    /// Each active source's latest analysis.
    pub fn active_data(&self) -> impl Iterator<Item = (AudioSourceId, &AudioData)> {
        self.active.iter().map(|(id, source)| (*id, &source.latest))
    }

    pub fn active_source_ids(&self) -> Vec<AudioSourceId> {
        self.active.keys().copied().collect()
    }

    pub fn has_active_source(&self) -> bool {
        !self.active.is_empty()
    }

    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        sender: Sender<AudioData>,
        sample_rate: f32,
        pcm_subs: Arc<ArcSwap<Vec<PcmSubscriber>>>,
        tempo_range: Arc<AtomicU64>,
    ) -> Result<cpal::Stream>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let mut capture = analysis::CaptureState::new(
            config.channels as usize,
            sample_rate,
            sender,
            pcm_subs,
            tempo_range,
        )
        .context("Failed to start audio analysis")?;
        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| capture.process(data),
            |err| log::error!("Audio stream error: {err}"),
            None,
        )?;

        Ok(stream)
    }
}

/// Textures for ISF audio inputs.
pub struct AudioTextures {
    /// ISF "audio" input.
    pub waveform_texture: wgpu::Texture,
    pub waveform_view: wgpu::TextureView,

    /// ISF "audioFFT" input.
    pub fft_texture: wgpu::Texture,
    pub fft_view: wgpu::TextureView,
}

impl AudioTextures {
    pub fn new(device: &wgpu::Device) -> Self {
        // 1D-like textures: width = buffer size, height = 1.
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

    /// Uploads new audio data to the textures.
    pub fn update(&self, queue: &wgpu::Queue, audio_data: &AudioData) {
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
        assert!(window[0].abs() < 1e-6, "Hann window start should be ~0");
        assert!(
            window[FFT_SIZE - 1].abs() < 1e-6,
            "Hann window end should be ~0"
        );
        let mid = window[FFT_SIZE / 2];
        assert!(
            (mid - 1.0).abs() < 0.01,
            "Hann window midpoint should be ~1.0, got {mid}"
        );
        for i in 0..FFT_SIZE / 2 {
            assert!(
                (window[i] - window[FFT_SIZE - 1 - i]).abs() < 1e-6,
                "Hann window should be symmetric at index {i}"
            );
        }
    }

    #[test]
    fn bin_resolution_at_48khz() {
        let data = AudioData {
            fft: vec![0.0; FFT_SIZE / 2].into(),
            sample_rate: 48000.0,
            ..AudioData::default()
        };
        let bw = data.bin_width();
        // 48000 / 2048 = 23.4375 Hz/bin
        assert!(
            (bw - 23.4375).abs() < 0.01,
            "Bin width should be ~23.4Hz, got {bw}"
        );
        // Bass (20-250 Hz) spans ~10 bins.
        let bass_bins = (250.0 / bw).ceil() as usize - (20.0 / bw).floor() as usize;
        assert!(
            bass_bins >= 9,
            "Bass range should span >=9 bins, got {bass_bins}"
        );
    }

    #[test]
    fn spectral_flux_detects_onset_not_steady_state() {
        // Identical frames: zero flux.
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

        // Sharp increase: positive flux.
        let frame_b: Vec<f32> = vec![0.5; FFT_SIZE / 2];
        let flux_onset: f32 = frame_b
            .iter()
            .zip(frame_a.iter())
            .map(|(c, p)| (c - p).max(0.0))
            .sum();
        assert!(flux_onset > 0.0, "Onset spectral flux should be positive");

        // Energy drop: zero flux (half-wave rectified).
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
    fn onset_threshold_survives_non_finite_flux() {
        // Drivers can deliver NaN/Inf; the audio thread must not panic.
        let flux = [0.1, f32::NAN, 0.3, f32::INFINITY, 0.2];
        let _ = compute_onset_threshold(&flux);
    }

    /// Hop rate at 48 kHz.
    const HOP_RATE: f32 = 48_000.0 / FFT_HOP as f32;

    /// An 8 s onset envelope with a short pulse at each time in `beats`
    /// (seconds) of the given strength.
    fn pulse_envelope(beats: impl IntoIterator<Item = (f32, f32)>) -> Vec<f32> {
        let mut env = vec![0.0; (TEMPO_WINDOW_SECS * HOP_RATE) as usize];
        for (t, strength) in beats {
            let i = (t * HOP_RATE).round() as usize;
            for (j, w) in [0.5, 1.0, 0.5].into_iter().enumerate() {
                if let Some(v) = env.get_mut(i + j) {
                    *v += strength * w;
                }
            }
        }
        env
    }

    fn beat_times(bpm: f32) -> impl Iterator<Item = f32> {
        (0..)
            .map(move |i| i as f32 * 60.0 / bpm)
            .take_while(|t| *t < 8.0)
    }

    #[test]
    fn bpm_estimate_survives_non_finite_envelope() {
        let mut env = pulse_envelope(beat_times(120.0).map(|t| (t, 1.0)));
        env[3] = f32::NAN;
        env[40] = f32::NEG_INFINITY;
        let range = TempoRange::default();
        let bpm = estimate_bpm(&env, HOP_RATE, range).expect("still periodic");
        assert!((bpm - 120.0).abs() < 1.0, "got {bpm}");
        assert_eq!(estimate_bpm(&[f32::NAN; 64], HOP_RATE, range), None);
        assert_eq!(estimate_bpm(&env, f32::NAN, range), None);
    }

    #[test]
    fn bpm_estimate_steady_beats() {
        for truth in [75.0, 100.0, 128.0, 140.0, 174.0, 200.0, 230.0] {
            let env = pulse_envelope(beat_times(truth).map(|t| (t, 1.0)));
            let bpm = estimate_bpm(&env, HOP_RATE, TempoRange::default()).expect("periodic");
            assert!((bpm - truth).abs() < 1.0, "{truth} BPM: got {bpm}");
        }
    }

    /// Missing beats and weaker off-beat onsets don't move the estimate,
    /// including to its double or half.
    #[test]
    fn bpm_estimate_tolerates_missing_and_stray_onsets() {
        let truth = 128.0;
        let beats = beat_times(truth)
            .enumerate()
            .filter(|(i, _)| i % 5 != 3)
            .map(|(_, t)| (t, 1.0));
        let strays = beat_times(truth)
            .step_by(3)
            .map(|t| (t + 30.0 / truth, 0.4));
        let env = pulse_envelope(beats.chain(strays));
        let bpm = estimate_bpm(&env, HOP_RATE, TempoRange::default()).expect("periodic");
        assert!((bpm - truth).abs() < 1.0, "got {bpm}");
    }

    /// A kick with three softer bass notes per beat, on the other 16ths, gives
    /// the kick's tempo, not its double.
    #[test]
    fn bpm_estimate_kick_and_rolling_bass() {
        for truth in [145.0, 190.0, 210.0] {
            let kicks = beat_times(truth).map(|t| (t, 1.0));
            let bass = beat_times(truth)
                .flat_map(|t| (1..4).map(move |i| (t + i as f32 * 15.0 / truth, 0.5)));
            let env = pulse_envelope(kicks.chain(bass));
            let bpm = estimate_bpm(&env, HOP_RATE, TempoRange::default()).expect("periodic");
            assert!((bpm - truth).abs() < 1.0, "{truth} BPM: got {bpm}");
        }
    }

    /// A period outside the range folds into it by octaves.
    #[test]
    fn bpm_estimate_folds_into_range() {
        // Beats at 50 BPM report 100. At 220 BPM, 220 by default and 110 in
        // 70-180.
        for (beats, range, want) in [
            (50.0, TempoRange::default(), 100.0),
            (220.0, TempoRange::default(), 220.0),
            (220.0, TempoRange::new(70.0, 180.0), 110.0),
        ] {
            let env = pulse_envelope(beat_times(beats).map(|t| (t, 1.0)));
            let bpm = estimate_bpm(&env, HOP_RATE, range).expect("periodic");
            assert!((bpm - want).abs() < 1.0, "{beats} BPM beats: got {bpm}");
        }
        let range = TempoRange::default();
        assert!((range.fold(512.0) - 128.0).abs() < 1e-3);
        assert!((range.fold(30.0) - 120.0).abs() < 1e-3);
        assert!((range.fold(200.0) - 200.0).abs() < 1e-3);
    }

    /// A range spans at least an octave within the limits, so folding always
    /// ends, and survives packing for the analysis threads.
    #[test]
    fn tempo_range_spans_an_octave() {
        assert_eq!(
            TempoRange::new(150.0, 200.0),
            TempoRange {
                min: 150.0,
                max: 300.0
            }
        );
        assert_eq!(
            TempoRange::new(10.0, 1000.0),
            TempoRange {
                min: 40.0,
                max: 320.0
            }
        );
        assert_eq!(
            TempoRange::new(300.0, 0.0),
            TempoRange {
                min: 160.0,
                max: 320.0
            }
        );
        assert_eq!(
            TempoRange::new(f32::NAN, f32::INFINITY),
            TempoRange::default()
        );
        let range = TempoRange::new(87.5, 210.0);
        assert_eq!(TempoRange::from_bits(range.to_bits()), range);
    }

    #[test]
    fn bpm_estimate_none_without_periodicity() {
        let range = TempoRange::default();
        assert_eq!(estimate_bpm(&vec![0.0; 1500], HOP_RATE, range), None);
        // Deterministic noise.
        let mut x = 0x2545_f491_u32;
        let noise: Vec<f32> = (0..1500)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                (x >> 8) as f32 / (1 << 24) as f32
            })
            .collect();
        assert_eq!(estimate_bpm(&noise, HOP_RATE, range), None);
    }

    #[test]
    fn energy_in_range_with_new_fft_size() {
        // At 48 kHz, 2048-point bins are ~23.4 Hz. Energy only in bins 2-4 (~47-94 Hz).
        let mut fft = vec![0.0_f32; FFT_SIZE / 2];
        fft[2] = 0.5;
        fft[3] = 0.5;
        fft[4] = 0.5;

        let data = AudioData {
            fft: fft.into(),
            sample_rate: 48000.0,
            waveform: vec![0.0; AUDIO_BUFFER_SIZE].into(),
            level: 0.0,
            bpm: None,
            time_since_beat: 0.0,
        };

        let energy = data.energy_in_range(40.0, 100.0);
        assert!(energy > 0.0, "Should detect energy in 40-100Hz range");

        let energy_high = data.energy_in_range(5000.0, 10000.0);
        assert!(
            energy_high < 1e-6,
            "Should detect no energy in 5k-10kHz range"
        );
    }

    // ── Frequency edge cases ─────────────────────────────────────────────

    #[test]
    fn chaos_energy_in_range_nan_freq_low() {
        let data = AudioData {
            waveform: vec![0.0; 128].into(),
            fft: vec![0.5; 1024].into(),
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(f32::NAN, 1000.0);
        // NaN / bw → NaN; floor as usize saturates to 0; .min(len-1) → 0. No panic.
        assert!(
            val.is_finite() || val == 0.0,
            "NaN freq_low should not crash"
        );
    }

    #[test]
    fn chaos_energy_in_range_nan_freq_high() {
        let data = AudioData {
            waveform: vec![0.0; 128].into(),
            fft: vec![0.5; 1024].into(),
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
            waveform: vec![0.0; 128].into(),
            fft: vec![0.5; 1024].into(),
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
            waveform: vec![0.0; 128].into(),
            fft: vec![0.5; 1024].into(),
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(-1000.0, -500.0);
        // Negative / bw → negative; floor as usize saturates to 0.
        assert!(
            val >= 0.0,
            "negative freq should not produce negative energy"
        );
    }

    #[test]
    fn chaos_energy_in_range_infinity() {
        let data = AudioData {
            waveform: vec![0.0; 128].into(),
            fft: vec![0.5; 1024].into(),
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
            waveform: vec![0.0; 128].into(),
            fft: vec![0.5; 1024].into(),
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
            waveform: vec![0.0; 128].into(),
            fft: vec![0.5; 1024].into(),
            level: 0.5,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 0.0,
        };
        let val = data.energy_in_range(100.0, 1000.0);
        // bin_width is 0, caught by the bw <= 0.0 guard.
        assert_eq!(val, 0.0, "zero sample rate should return 0.0");
    }

    #[test]
    fn chaos_energy_in_range_empty_fft() {
        let data = AudioData {
            waveform: vec![].into(),
            fft: vec![].into(),
            level: 0.0,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(20.0, 20000.0);
        assert_eq!(val, 0.0, "empty FFT should return 0.0");
    }

    // ── PCM passthrough tap ────────────────────────────────────────────

    /// A subscriber plus its receiver, drop counter and lost-sample counter.
    fn make_sub(cap: usize) -> (PcmSubscriber, Receiver<PcmChunk>, Arc<AtomicU64>) {
        let (sub, rx, dropped, _) = make_sub_counted(cap);
        (sub, rx, dropped)
    }

    fn make_sub_counted(
        cap: usize,
    ) -> (
        PcmSubscriber,
        Receiver<PcmChunk>,
        Arc<AtomicU64>,
        Arc<AtomicU64>,
    ) {
        let (sender, receiver) = bounded::<PcmChunk>(cap);
        let dropped = Arc::new(AtomicU64::new(0));
        let lost_samples = Arc::new(AtomicU64::new(0));
        let sub = PcmSubscriber {
            token: NEXT_PCM_TOKEN.fetch_add(1, Ordering::Relaxed),
            sender,
            dropped: dropped.clone(),
            lost_samples: lost_samples.clone(),
        };
        (sub, receiver, dropped, lost_samples)
    }

    #[test]
    fn pcm_fan_out_delivers_to_all_subscribers() {
        let (first, rx1, _) = make_sub(4);
        let (second, rx2, _) = make_sub(4);
        let subs = vec![first, second];
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

    /// A drop records the samples lost, not just the event. Consumers time PCM
    /// by sample count and chunk lengths vary, so they need the sample total to
    /// fill the gap.
    #[test]
    fn pcm_fan_out_records_how_many_samples_were_lost() {
        // Capacity 2, never drained: 5 sends of 8 samples → 2 buffered, 3
        // dropped, 24 samples owed.
        let (sub, _rx, dropped, lost) = make_sub_counted(2);
        let subs = vec![sub];

        for _ in 0..5 {
            fan_out_pcm(&subs, &[0.0; 8]);
        }

        assert_eq!(dropped.load(Ordering::Relaxed), 3);
        assert_eq!(lost.load(Ordering::Relaxed), 24);
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
            waveform: vec![0.0; 2].into(),
            fft: vec![1.0].into(),
            level: 1.0,
            bpm: None,
            time_since_beat: 0.0,
            sample_rate: 48000.0,
        };
        let val = data.energy_in_range(0.0, 48000.0);
        // Single-bin FFT must not panic.
        assert!(val.is_finite());
    }
}
