//! Audio input and analysis for audio-reactive shaders

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use crossbeam_channel::{bounded, Receiver, Sender};
use rustfft::{num_complex::Complex, FftPlanner};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

/// Opaque audio source identifier.
pub type AudioSourceId = u32;

/// Audio buffer size (samples per channel) — 256 @ 48kHz ≈ 5.3ms latency
pub const AUDIO_BUFFER_SIZE: usize = 256;
/// FFT size — must be >= AUDIO_BUFFER_SIZE, zero-padded if larger for resolution
pub const FFT_SIZE: usize = 512;
/// Number of beat intervals to average for BPM calculation
const BPM_HISTORY_SIZE: usize = 8;
/// Minimum time between beats (in seconds) to avoid double-triggering
const MIN_BEAT_INTERVAL: f32 = 0.2;  // Max ~300 BPM
/// Maximum time between beats before we reset BPM tracking
const MAX_BEAT_INTERVAL: f32 = 2.0;  // Min ~30 BPM

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
    /// Hz width of each FFT bin
    fn bin_width(&self) -> f32 {
        self.sample_rate / FFT_SIZE as f32
    }

    /// Get energy in an arbitrary frequency range (Hz).
    /// This is the core method — bass/mid/treble are just presets on top of it.
    pub fn energy_in_range(&self, freq_low: f32, freq_high: f32) -> f32 {
        if self.fft.is_empty() { return 0.0; }
        let bw = self.bin_width();
        if bw <= 0.0 { return 0.0; }
        let bin_low = ((freq_low / bw).floor() as usize).min(self.fft.len() - 1);
        let bin_high = ((freq_high / bw).ceil() as usize).min(self.fft.len());
        if bin_high <= bin_low { return 0.0; }
        let slice = &self.fft[bin_low..bin_high];
        // RMS energy then dB-based perceptual mapping
        let rms = (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt();
        if rms < 1e-6 { return 0.0; }
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

/// An active audio source with its own capture stream.
struct ActiveAudioSource {
    _stream: cpal::Stream,
    receiver: Receiver<AudioData>,
    /// Latest polled data (cached between polls)
    pub latest: AudioData,
}

/// Manages audio device enumeration and multiple simultaneous audio input streams.
pub struct AudioManager {
    /// Detected audio input devices (refreshed on scan).
    devices: Vec<AudioDeviceInfo>,
    /// Active audio sources keyed by AudioSourceId.
    active: HashMap<AudioSourceId, ActiveAudioSource>,
}

impl AudioManager {
    pub fn new() -> Self {
        let mut mgr = Self {
            devices: Vec::new(),
            active: HashMap::new(),
        };
        mgr.scan_devices();
        // Auto-open the default device if available
        if let Some(dev) = mgr.devices.first() {
            let id = dev.id;
            if let Err(e) = mgr.open_source(id) {
                log::warn!("Failed to auto-open default audio device: {}", e);
            }
        }
        mgr
    }

    /// Scan for available audio input devices.
    pub fn scan_devices(&mut self) {
        let host = cpal::default_host();
        self.devices = host.input_devices()
            .map(|devs| {
                devs.enumerate().filter_map(|(i, d)| {
                    let name = d.description()
                        .map(|desc| desc.name().to_string())
                        .unwrap_or_else(|_| format!("Audio Input {}", i));
                    Some(AudioDeviceInfo { id: i as AudioSourceId, name })
                }).collect()
            })
            .unwrap_or_default();
        log::info!("Audio scan: found {} input device(s)", self.devices.len());
        for dev in &self.devices {
            log::info!("  Audio {}: {}", dev.id, dev.name);
        }
    }

    /// Get the list of detected audio input devices.
    pub fn devices(&self) -> &[AudioDeviceInfo] {
        &self.devices
    }

    /// Open an audio source and start capturing.
    pub fn open_source(&mut self, id: AudioSourceId) -> Result<()> {
        if self.active.contains_key(&id) {
            return Ok(()); // Already open
        }

        let host = cpal::default_host();
        let device = host.input_devices()
            .context("Failed to enumerate audio devices")?
            .nth(id as usize)
            .context("Audio device not found")?;

        let dev_name = device.description()
            .map(|desc| desc.name().to_string())
            .unwrap_or_else(|_| format!("Audio {}", id));
        log::info!("Opening audio source {}: {}", id, dev_name);

        let config = device.default_input_config()
            .context("Failed to get default audio input config")?;
        let sample_rate = config.sample_rate() as f32;
        log::info!("Audio config for '{}': {:?}", dev_name, config);

        let (sender, receiver) = bounded::<AudioData>(16);

        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => Self::build_stream::<f32>(&device, &config.into(), sender, sample_rate)?,
            cpal::SampleFormat::I16 => Self::build_stream::<i16>(&device, &config.into(), sender, sample_rate)?,
            cpal::SampleFormat::U16 => Self::build_stream::<u16>(&device, &config.into(), sender, sample_rate)?,
            _ => anyhow::bail!("Unsupported sample format"),
        };

        stream.play().context("Failed to start audio stream")?;

        self.active.insert(id, ActiveAudioSource {
            _stream: stream,
            receiver,
            latest: AudioData { sample_rate, ..AudioData::default() },
        });

        Ok(())
    }

    /// Close an audio source.
    pub fn close_source(&mut self, id: AudioSourceId) {
        if self.active.remove(&id).is_some() {
            log::info!("Closed audio source {}", id);
        }
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
        self.active.values().next().map(|s| &s.latest).unwrap_or_else(|| {
            static DEFAULT: std::sync::LazyLock<AudioData> = std::sync::LazyLock::new(AudioData::default);
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
    ) -> Result<cpal::Stream>
    where
        T: cpal::Sample + cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let channels = config.channels as usize;
        let mut buffer: Vec<f32> = Vec::with_capacity(AUDIO_BUFFER_SIZE);
        let mut fft_planner = FftPlanner::new();
        let fft = Arc::new(fft_planner.plan_fft_forward(FFT_SIZE));
        let buf_size = AUDIO_BUFFER_SIZE;

        // BPM detection state
        let mut bass_history: Vec<f32> = vec![0.0; 8];
        let mut last_beat_time = Instant::now();
        let mut beat_intervals: Vec<f32> = Vec::with_capacity(BPM_HISTORY_SIZE);
        let mut current_bpm: Option<f32> = None;

        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                for chunk in data.chunks(channels) {
                    let sample: f32 = chunk.iter()
                        .map(|s| <f32 as Sample>::from_sample(*s))
                        .sum::<f32>() / channels as f32;
                    buffer.push(sample);
                }

                if buffer.len() >= buf_size {
                    let waveform: Vec<f32> = buffer.drain(..AUDIO_BUFFER_SIZE).collect();
                    let level = (waveform.iter().map(|s| s * s).sum::<f32>() / AUDIO_BUFFER_SIZE as f32).sqrt();

                    let mut fft_buffer: Vec<Complex<f32>> = waveform.iter()
                        .map(|&s| Complex::new(s, 0.0))
                        .collect();
                    fft_buffer.resize(FFT_SIZE, Complex::new(0.0, 0.0));
                    fft.process(&mut fft_buffer);

                    // Absolute magnitude scaling: divide by FFT_SIZE for correct amplitude,
                    // then apply a noise floor so silence reads as zero.
                    const NOISE_FLOOR: f32 = 1e-4;
                    let scale = 2.0 / FFT_SIZE as f32; // 2x because we only keep positive half
                    let fft_magnitudes: Vec<f32> = fft_buffer[..FFT_SIZE/2].iter()
                        .map(|c: &Complex<f32>| {
                            let mag = c.norm() * scale;
                            if mag < NOISE_FLOOR { 0.0 } else { mag }
                        })
                        .collect();

                    // BPM detection
                    let bass_energy = if fft_magnitudes.len() >= 3 {
                        (fft_magnitudes[0] + fft_magnitudes[1] + fft_magnitudes[2]) / 3.0
                    } else { 0.0 };

                    bass_history.remove(0);
                    bass_history.push(bass_energy);
                    let avg_bass: f32 = bass_history.iter().sum::<f32>() / bass_history.len() as f32;
                    let beat_threshold = avg_bass * 1.4 + 0.05;
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_beat_time).as_secs_f32();

                    if bass_energy > beat_threshold && elapsed > MIN_BEAT_INTERVAL {
                        if elapsed < MAX_BEAT_INTERVAL {
                            beat_intervals.push(elapsed);
                            if beat_intervals.len() > BPM_HISTORY_SIZE {
                                beat_intervals.remove(0);
                            }
                            if beat_intervals.len() >= 2 {
                                let avg_interval: f32 = beat_intervals.iter().sum::<f32>()
                                    / beat_intervals.len() as f32;
                                let bpm = 60.0 / avg_interval;
                                if (30.0..=300.0).contains(&bpm) {
                                    current_bpm = Some(bpm);
                                }
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

