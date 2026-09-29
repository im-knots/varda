//! One input source's capture callback and audio analysis: FFT, level,
//! spectral-flux onsets, and BPM, one 256-sample hop at a time.

use std::sync::Arc;
use std::time::Instant;

use arc_swap::ArcSwap;
use cpal::Sample;
use crossbeam_channel::Sender;
use rustfft::{Fft, FftPlanner, num_complex::Complex};

use super::{
    AudioData, BPM_HISTORY_SIZE, FFT_HOP, FFT_SIZE, MAX_BEAT_INTERVAL, MIN_BEAT_INTERVAL,
    ONSET_MEDIAN_WINDOW, PcmSubscriber, compute_onset_threshold, estimate_bpm, fan_out_pcm,
};

/// Analysis state carried between hops.
pub(crate) struct HopAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    hann_window: Vec<f32>,
    /// The last `FFT_SIZE` samples, written in hops, for overlapping frames.
    ring_buffer: Vec<f32>,
    ring_write_pos: usize,
    prev_fft_magnitudes: Arc<[f32]>,
    flux_history: Vec<f32>,
    fft_input: Vec<Complex<f32>>,
    last_beat_time: Instant,
    beat_intervals: Vec<f32>,
    current_bpm: Option<f32>,
    sample_rate: f32,
}

impl HopAnalyzer {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let mut fft_planner = FftPlanner::new();
        Self {
            fft: fft_planner.plan_fft_forward(FFT_SIZE),
            // Hann window, to reduce spectral leakage.
            hann_window: (0..FFT_SIZE)
                .map(|i| {
                    0.5 * (1.0
                        - (2.0 * std::f32::consts::PI * i as f32 / (FFT_SIZE - 1) as f32).cos())
                })
                .collect(),
            ring_buffer: vec![0.0; FFT_SIZE],
            ring_write_pos: 0,
            prev_fft_magnitudes: vec![0.0; FFT_SIZE / 2].into(),
            flux_history: Vec::with_capacity(ONSET_MEDIAN_WINDOW + 1),
            fft_input: vec![Complex::new(0.0, 0.0); FFT_SIZE],
            last_beat_time: Instant::now(),
            beat_intervals: Vec::with_capacity(BPM_HISTORY_SIZE),
            current_bpm: None,
            sample_rate,
        }
    }

    /// Analyzes one hop of `FFT_HOP` mono samples.
    pub(crate) fn analyze(&mut self, hop: &[f32]) -> AudioData {
        const NOISE_FLOOR: f32 = 1e-4;

        let waveform: Arc<[f32]> = hop.into();
        let level = (waveform.iter().map(|s| s * s).sum::<f32>() / FFT_HOP as f32).sqrt();

        for (i, &s) in waveform.iter().enumerate() {
            self.ring_buffer[(self.ring_write_pos + i) % FFT_SIZE] = s;
        }
        self.ring_write_pos = (self.ring_write_pos + FFT_HOP) % FFT_SIZE;

        // Linearize the 2048-sample frame and apply the window.
        for i in 0..FFT_SIZE {
            let idx = (self.ring_write_pos + i) % FFT_SIZE;
            self.fft_input[i] = Complex::new(self.ring_buffer[idx] * self.hann_window[i], 0.0);
        }
        self.fft.process(&mut self.fft_input);

        // Magnitude scaling with a noise floor.
        let scale = 2.0 / FFT_SIZE as f32;
        let fft_magnitudes: Arc<[f32]> = self.fft_input[..FFT_SIZE / 2]
            .iter()
            .map(|c: &Complex<f32>| {
                let mag = c.norm() * scale;
                if mag < NOISE_FLOOR { 0.0 } else { mag }
            })
            .collect();

        // Spectral-flux onset detection.
        let spectral_flux: f32 = fft_magnitudes
            .iter()
            .zip(self.prev_fft_magnitudes.iter())
            .map(|(curr, prev)| (curr - prev).max(0.0))
            .sum();

        self.flux_history.push(spectral_flux);
        if self.flux_history.len() > ONSET_MEDIAN_WINDOW {
            self.flux_history.remove(0);
        }
        let onset_threshold = compute_onset_threshold(&self.flux_history);

        let now = Instant::now();
        let elapsed = now.duration_since(self.last_beat_time).as_secs_f32();
        let is_onset = spectral_flux > onset_threshold && elapsed > MIN_BEAT_INTERVAL;
        self.prev_fft_magnitudes = Arc::clone(&fft_magnitudes);

        // BPM estimate with outlier rejection.
        if is_onset {
            if elapsed < MAX_BEAT_INTERVAL {
                self.beat_intervals.push(elapsed);
                if self.beat_intervals.len() > BPM_HISTORY_SIZE {
                    self.beat_intervals.remove(0);
                }
                if let Some(bpm) = estimate_bpm(&self.beat_intervals) {
                    self.current_bpm = Some(bpm);
                }
            } else {
                self.beat_intervals.clear();
                self.current_bpm = None;
            }
            self.last_beat_time = now;
        }

        AudioData {
            waveform,
            fft: fft_magnitudes,
            level,
            bpm: self.current_bpm,
            time_since_beat: now.duration_since(self.last_beat_time).as_secs_f32(),
            sample_rate: self.sample_rate,
        }
    }
}

/// Hops the capture callback may queue ahead of the analysis thread (about
/// 170 ms at 48 kHz). When full, hops are dropped so the callback never blocks.
const HOP_QUEUE: usize = 32;

/// Capture callback state: tees raw PCM to passthrough subscribers, mixes to
/// mono, and sends each full hop to the analysis thread. Never waits, and
/// allocates only when a passthrough is subscribed.
pub(crate) struct CaptureState {
    channels: usize,
    hop: [f32; FFT_HOP],
    filled: usize,
    hops: Sender<[f32; FFT_HOP]>,
    pcm_subs: Arc<ArcSwap<Vec<PcmSubscriber>>>,
}

impl CaptureState {
    /// Starts the analysis thread, which publishes to `sender` and exits when
    /// this is dropped with the stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the analysis thread cannot be spawned.
    pub(crate) fn new(
        channels: usize,
        sample_rate: f32,
        sender: Sender<AudioData>,
        pcm_subs: Arc<ArcSwap<Vec<PcmSubscriber>>>,
    ) -> std::io::Result<Self> {
        let (hops, received) = crossbeam_channel::bounded::<[f32; FFT_HOP]>(HOP_QUEUE);
        std::thread::Builder::new()
            .name("audio-analysis".into())
            .spawn(move || {
                let mut analyzer = HopAnalyzer::new(sample_rate);
                while let Ok(hop) = received.recv() {
                    let _ = sender.try_send(analyzer.analyze(&hop));
                }
            })?;
        Ok(Self {
            channels,
            hop: [0.0; FFT_HOP],
            filled: 0,
            hops,
            pcm_subs,
        })
    }

    /// Handles one buffer of interleaved device samples.
    pub(crate) fn process<T>(&mut self, data: &[T])
    where
        T: cpal::Sample,
        f32: cpal::FromSample<T>,
    {
        // Forward raw interleaved PCM to every passthrough subscriber. Skipped
        // when there are none, so the video-only path costs nothing.
        let subs = self.pcm_subs.load();
        if !subs.is_empty() {
            let pcm: Vec<f32> = data
                .iter()
                .map(|s| <f32 as Sample>::from_sample(*s))
                .collect();
            fan_out_pcm(&subs, &pcm);
        }

        for chunk in data.chunks(self.channels) {
            let sample: f32 = chunk
                .iter()
                .map(|s| <f32 as Sample>::from_sample(*s))
                .sum::<f32>()
                / self.channels as f32;
            self.hop[self.filled] = sample;
            self.filled += 1;
            if self.filled == FFT_HOP {
                // A full queue means the analysis is behind; this hop is lost.
                let _ = self.hops.try_send(self.hop);
                self.filled = 0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CaptureState, FFT_HOP, HopAnalyzer};

    /// Each published hop carries the same waveform, spectrum, and level as
    /// analyzing that hop directly. Device buffers don't align with hops, so
    /// this also covers hop assembly across buffers.
    #[test]
    fn the_analysis_thread_publishes_what_direct_analysis_gives() {
        const HOPS: usize = 24;
        // Stereo, both channels equal, so the mono mix is exact: two tones and
        // a burst halfway through, for onsets.
        let signal: Vec<f32> = (0..HOPS * FFT_HOP)
            .flat_map(|i| {
                let t = i as f32 / 48_000.0;
                let burst = if (HOPS / 2 * FFT_HOP..HOPS / 2 * FFT_HOP + 512).contains(&i) {
                    0.6
                } else {
                    0.0
                };
                let v = 0.3 * (t * 220.0 * std::f32::consts::TAU).sin()
                    + 0.2 * (t * 3_000.0 * std::f32::consts::TAU).sin()
                    + burst;
                [v, v]
            })
            .collect();

        let (sender, published) = crossbeam_channel::bounded(HOPS);
        let mut capture = CaptureState::new(2, 48_000.0, sender, std::sync::Arc::default())
            .expect("analysis thread");
        for buffer in signal.chunks(2 * 100) {
            capture.process(buffer);
        }

        let mut direct = HopAnalyzer::new(48_000.0);
        // Both channels are equal, so the callback's mix is the left channel.
        let mono: Vec<f32> = signal.iter().step_by(2).copied().collect();
        for (i, hop) in mono.as_chunks::<FFT_HOP>().0.iter().enumerate() {
            let want = direct.analyze(hop);
            let got = published
                .recv_timeout(std::time::Duration::from_secs(5))
                .expect("every hop is analyzed");
            assert_eq!(got.waveform, want.waveform, "hop {i} waveform");
            assert_eq!(got.fft, want.fft, "hop {i} spectrum");
            assert_eq!(got.level.to_bits(), want.level.to_bits(), "hop {i} level");
        }
    }
}
