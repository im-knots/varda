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
/// Beat intervals kept for BPM estimation.
const BPM_HISTORY_SIZE: usize = 16;
/// Minimum seconds between beats, to avoid double triggers.
const MIN_BEAT_INTERVAL: f32 = 0.2; // Max ~300 BPM
/// Seconds without a beat before BPM tracking resets.
const MAX_BEAT_INTERVAL: f32 = 2.0; // Min ~30 BPM
/// Window for the adaptive onset threshold (median of recent spectral flux).
const ONSET_MEDIAN_WINDOW: usize = 8;
/// Onset fires when spectral flux exceeds median * this + offset.
const ONSET_THRESHOLD_MULTIPLIER: f32 = 1.5;
/// Minimum spectral flux for an onset, so silence doesn't trigger.
const ONSET_THRESHOLD_OFFSET: f32 = 0.01;
/// Beat intervals more than 15% from the median are rejected.
const TEMPO_TOLERANCE: f32 = 0.15;

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
    /// Seconds since the last beat.
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

    /// Beat phase, 0.0 to 1.0, where 0.0 is on the beat.
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

/// Estimates BPM from beat intervals, rejecting outliers around the median.
/// `None` with fewer than 4 intervals or a BPM outside 30-300. The median uses
/// quickselect; non-finite intervals are skipped.
pub fn estimate_bpm(beat_intervals: &[f32]) -> Option<f32> {
    let mut buf: Vec<f32> = finite_values(beat_intervals);
    if buf.len() < 4 {
        return None;
    }
    let mid = buf.len() / 2;
    buf.select_nth_unstable_by(mid, |a, b| cmp_finite(*a, *b));
    let median = buf[mid];
    // Filter outliers and average in one pass.
    let (sum, count) = buf.iter().fold((0.0f32, 0u32), |(s, c), &iv| {
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
    ) -> Result<cpal::Stream>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let mut capture =
            analysis::CaptureState::new(config.channels as usize, sample_rate, sender, pcm_subs)
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

    #[test]
    fn bpm_estimate_survives_non_finite_intervals() {
        let intervals = [0.5, f32::NAN, 0.5, 0.5, f32::NEG_INFINITY, 0.5];
        let _ = estimate_bpm(&intervals);
    }

    #[test]
    fn bpm_outlier_rejection() {
        // Mostly ~0.5 s (120 BPM), one outlier.
        let intervals: Vec<f32> = vec![0.50, 0.51, 0.49, 0.50, 0.52, 0.48, 1.2, 0.50];
        let mut sorted = intervals.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let median = sorted[sorted.len() / 2];

        let stable: Vec<f32> = intervals
            .iter()
            .filter(|&&iv| (iv - median).abs() / median < TEMPO_TOLERANCE)
            .copied()
            .collect();

        assert!(
            !stable.contains(&1.2),
            "Outlier interval should be rejected"
        );
        assert!(stable.len() >= 6, "Most intervals should survive filtering");

        let avg = stable.iter().sum::<f32>() / stable.len() as f32;
        let bpm = 60.0 / avg;
        assert!((bpm - 120.0).abs() < 5.0, "BPM should be ~120, got {bpm}");
    }

    #[test]
    fn bpm_stability_consistent_beats() {
        // Steady 128 BPM (0.46875 s).
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
            "BPM should be exactly ~128, got {bpm}"
        );
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
