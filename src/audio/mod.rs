//! Audio input and analysis for audio-reactive shaders

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::Sample;
use crossbeam_channel::{bounded, Receiver, Sender};
use rustfft::{num_complex::Complex, FftPlanner};
use std::sync::Arc;
use std::time::Instant;

/// Audio buffer size (samples per channel)
pub const AUDIO_BUFFER_SIZE: usize = 512;
/// FFT size
pub const FFT_SIZE: usize = 512;
/// Number of beat intervals to average for BPM calculation
const BPM_HISTORY_SIZE: usize = 8;
/// Minimum time between beats (in seconds) to avoid double-triggering
const MIN_BEAT_INTERVAL: f32 = 0.2;  // Max ~300 BPM
/// Maximum time between beats before we reset BPM tracking
const MAX_BEAT_INTERVAL: f32 = 2.0;  // Min ~30 BPM

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
}

impl Default for AudioData {
    fn default() -> Self {
        Self {
            waveform: vec![0.0; AUDIO_BUFFER_SIZE],
            fft: vec![0.0; FFT_SIZE / 2],
            level: 0.0,
            bpm: None,
            time_since_beat: 0.0,
        }
    }
}

impl AudioData {
    /// Get bass level (low frequencies, ~20-250Hz)
    /// For 48kHz sample rate with 512 FFT, each bin is ~94Hz
    /// Bass: bins 0-2 (roughly 0-280Hz)
    pub fn bass(&self) -> f32 {
        if self.fft.len() < 3 { return 0.0; }
        (self.fft[0] + self.fft[1] + self.fft[2]) / 3.0
    }

    /// Get mid level (mid frequencies, ~250-2000Hz)
    /// Mid: bins 3-21 (roughly 280-2000Hz)
    pub fn mid(&self) -> f32 {
        if self.fft.len() < 22 { return 0.0; }
        self.fft[3..22].iter().sum::<f32>() / 19.0
    }

    /// Get treble level (high frequencies, ~2000Hz+)
    /// Treble: bins 22+ (roughly 2000Hz+)
    pub fn treble(&self) -> f32 {
        if self.fft.len() < 23 { return 0.0; }
        let slice = &self.fft[22..];
        if slice.is_empty() { return 0.0; }
        slice.iter().sum::<f32>() / slice.len() as f32
    }

    /// Get beat phase (0.0 to 1.0, where 0.0 is on the beat)
    /// This can be used for tempo-synced animations
    pub fn beat_phase(&self) -> f32 {
        if let Some(bpm) = self.bpm {
            let beat_duration = 60.0 / bpm;
            (self.time_since_beat / beat_duration).fract()
        } else {
            0.0
        }
    }
}

/// Audio input manager
pub struct AudioInput {
    _stream: cpal::Stream,
    receiver: Receiver<AudioData>,
}

impl AudioInput {
    /// Create a new audio input from the default input device
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host.default_input_device()
            .context("No audio input device available")?;
        
        if let Ok(desc) = device.description() {
            log::info!("Using audio input device: {}", desc.name());
        } else {
            log::info!("Using audio input device: Unknown");
        }
        
        let config = device.default_input_config()
            .context("Failed to get default audio input config")?;
        
        log::info!("Audio config: {:?}", config);
        
        let (sender, receiver) = bounded::<AudioData>(4);
        
        let stream = match config.sample_format() {
            cpal::SampleFormat::F32 => Self::build_stream::<f32>(&device, &config.into(), sender)?,
            cpal::SampleFormat::I16 => Self::build_stream::<i16>(&device, &config.into(), sender)?,
            cpal::SampleFormat::U16 => Self::build_stream::<u16>(&device, &config.into(), sender)?,
            _ => anyhow::bail!("Unsupported sample format"),
        };
        
        stream.play().context("Failed to start audio stream")?;
        
        Ok(Self {
            _stream: stream,
            receiver,
        })
    }
    
    fn build_stream<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        sender: Sender<AudioData>,
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
        let mut bass_history: Vec<f32> = vec![0.0; 8];  // Rolling history for threshold
        let mut last_beat_time = Instant::now();
        let mut beat_intervals: Vec<f32> = Vec::with_capacity(BPM_HISTORY_SIZE);
        let mut current_bpm: Option<f32> = None;

        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // Convert samples to f32 and mix to mono
                for chunk in data.chunks(channels) {
                    let sample: f32 = chunk.iter()
                        .map(|s| <f32 as Sample>::from_sample(*s))
                        .sum::<f32>() / channels as f32;
                    buffer.push(sample);
                }

                // When we have enough samples, send audio data
                if buffer.len() >= buf_size {
                    let waveform: Vec<f32> = buffer.drain(..AUDIO_BUFFER_SIZE).collect();

                    // Compute RMS level
                    let level = (waveform.iter().map(|s| s * s).sum::<f32>() / AUDIO_BUFFER_SIZE as f32).sqrt();

                    // Compute FFT
                    let mut fft_buffer: Vec<Complex<f32>> = waveform.iter()
                        .map(|&s| Complex::new(s, 0.0))
                        .collect();
                    fft_buffer.resize(FFT_SIZE, Complex::new(0.0, 0.0));
                    fft.process(&mut fft_buffer);

                    // Get magnitude spectrum (first half only)
                    let max_mag = fft_buffer[1..FFT_SIZE/2].iter()
                        .map(|c: &Complex<f32>| c.norm())
                        .fold(0.0f32, |a: f32, b: f32| a.max(b))
                        .max(0.001); // Avoid division by zero

                    let fft_magnitudes: Vec<f32> = fft_buffer[..FFT_SIZE/2].iter()
                        .map(|c: &Complex<f32>| (c.norm() / max_mag).min(1.0))
                        .collect();

                    // === BPM Detection ===
                    // Calculate bass energy (first 3 bins ~0-280Hz)
                    let bass_energy = if fft_magnitudes.len() >= 3 {
                        (fft_magnitudes[0] + fft_magnitudes[1] + fft_magnitudes[2]) / 3.0
                    } else {
                        0.0
                    };

                    // Add to rolling history and calculate average
                    bass_history.remove(0);
                    bass_history.push(bass_energy);
                    let avg_bass: f32 = bass_history.iter().sum::<f32>() / bass_history.len() as f32;

                    // Beat detection: check if current bass exceeds threshold
                    let beat_threshold = avg_bass * 1.4 + 0.05; // 40% above average + min floor
                    let now = Instant::now();
                    let elapsed = now.duration_since(last_beat_time).as_secs_f32();

                    if bass_energy > beat_threshold && elapsed > MIN_BEAT_INTERVAL {
                        // Beat detected!
                        if elapsed < MAX_BEAT_INTERVAL {
                            beat_intervals.push(elapsed);
                            if beat_intervals.len() > BPM_HISTORY_SIZE {
                                beat_intervals.remove(0);
                            }

                            // Calculate BPM from average interval
                            if beat_intervals.len() >= 2 {
                                let avg_interval: f32 = beat_intervals.iter().sum::<f32>()
                                    / beat_intervals.len() as f32;
                                let bpm = 60.0 / avg_interval;
                                // Constrain to reasonable range
                                if (30.0..=300.0).contains(&bpm) {
                                    current_bpm = Some(bpm);
                                }
                            }
                        } else {
                            // Too long since last beat, reset
                            beat_intervals.clear();
                            current_bpm = None;
                        }
                        last_beat_time = now;
                    }

                    // Calculate time since last beat
                    let time_since_beat = now.duration_since(last_beat_time).as_secs_f32();

                    let data = AudioData {
                        waveform,
                        fft: fft_magnitudes,
                        level,
                        bpm: current_bpm,
                        time_since_beat,
                    };

                    // Send (non-blocking, drop if receiver is full)
                    let _ = sender.try_send(data);
                }
            },
            |err| log::error!("Audio stream error: {}", err),
            None,
        )?;

        Ok(stream)
    }
    
    /// Get the latest audio data (non-blocking)
    pub fn get_latest(&self) -> Option<AudioData> {
        // Drain all pending and return the latest
        let mut latest = None;
        while let Ok(data) = self.receiver.try_recv() {
            latest = Some(data);
        }
        latest
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

