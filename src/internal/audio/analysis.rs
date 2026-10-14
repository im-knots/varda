//! One input source's capture callback and audio analysis: FFT, level,
//! low-band spectral-flux onsets, BPM and beat phase, one 256-sample hop at a
//! time.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use arc_swap::ArcSwap;
use cpal::Sample;
use crossbeam_channel::Sender;
use rustfft::{Fft, FftPlanner, num_complex::Complex};

use super::{
    AudioData, FFT_HOP, FFT_SIZE, MIN_ONSET_INTERVAL, ONSET_BAND_HZ, ONSET_MEDIAN_WINDOW,
    PcmSubscriber, SILENCE_LEVEL, SILENCE_RESET_SECS, TEMPO_CONFIRMATIONS, TEMPO_MIN_SECS,
    TEMPO_SMOOTHING, TEMPO_TOLERANCE, TEMPO_UPDATE_SECS, TEMPO_WINDOW_SECS, TempoRange,
    compute_onset_threshold, estimate_bpm, fan_out_pcm,
};

/// Seconds the band's level average takes to follow a change, for onset
/// strength normalization.
const BAND_LEVEL_SECS: f32 = 1.0;
/// Seconds from a kick's start to its detected onset, as a fraction of the
/// frame: flux builds up as the attack moves into the Hann-windowed frame.
const ONSET_LATENCY_FRAMES: f64 = 0.25;
/// Seconds after an onset over which its strength peaks, taken as its
/// strength when picking each beat's strongest onset. Below
/// [`MIN_ONSET_INTERVAL`], so peaks don't overlap.
const ONSET_PEAK_SECS: f64 = 0.04;
/// Seconds of onsets kept for the beat grid: more than a beat and a half at
/// the slowest tempo.
const ONSET_HISTORY_SECS: f64 = 3.0;
/// Distance (beats) from a predicted beat within which a beat's strongest
/// onset counts as on the beat.
const PHASE_LOCK_BEATS: f64 = 0.125;
/// How far each on-beat strongest onset pulls the beat grid toward itself.
const PHASE_CORRECTION: f64 = 0.25;
/// Consecutive beats whose strongest onset is off the grid after which the
/// grid snaps to the latest of them.
const PHASE_RELOCK_BEATS: u32 = 4;

/// Tempo with hysteresis: a tempo that disagrees with the current one, such as
/// its double, replaces it only after [`TEMPO_CONFIRMATIONS`] agreeing
/// estimates.
#[derive(Default)]
struct TempoTracker {
    bpm: Option<f32>,
    /// A disagreeing tempo, averaged over its agreeing estimates so far.
    pending: Option<(f32, u32)>,
}

impl TempoTracker {
    /// Feeds one estimate. True when the tempo was replaced, not just refined.
    fn update(&mut self, estimate: f32) -> bool {
        let agrees = |a: f32, b: f32| (a - b).abs() <= TEMPO_TOLERANCE * b;
        if let Some(bpm) = self.bpm
            && agrees(estimate, bpm)
        {
            self.bpm = Some(bpm + (estimate - bpm) * TEMPO_SMOOTHING);
            self.pending = None;
            return false;
        }
        let (tempo, count) = match self.pending {
            Some((tempo, count)) if agrees(estimate, tempo) => {
                let count = count + 1;
                (tempo + (estimate - tempo) / count as f32, count)
            }
            _ => (estimate, 1),
        };
        if count >= TEMPO_CONFIRMATIONS {
            self.bpm = Some(tempo);
            self.pending = None;
            true
        } else {
            self.pending = Some((tempo, count));
            false
        }
    }
}

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
    sample_rate: f32,
    /// Hops analyzed. Time is sample-based, `hop_index * FFT_HOP / sample_rate`,
    /// so device buffers arriving in bursts add no jitter.
    hop_index: u64,
    /// FFT bins of [`ONSET_BAND_HZ`].
    onset_bins: std::ops::RangeInclusive<usize>,
    /// Slow average of the onset band's magnitude sum.
    band_level: f32,
    /// Onset strength per hop over the last [`TEMPO_WINDOW_SECS`].
    envelope: VecDeque<f32>,
    envelope_capacity: usize,
    /// Hops until the next tempo estimate.
    hops_to_estimate: usize,
    /// Time of the latest onset, corrected for detection latency.
    last_onset: Option<f64>,
    /// Onsets over the last [`ONSET_HISTORY_SECS`]: time and peak strength.
    onsets: VecDeque<(f64, f32)>,
    /// Hops left over which the latest onset's peak strength is tracked.
    peak_hops_left: usize,
    /// Seconds of input below [`SILENCE_LEVEL`].
    quiet_secs: f32,
    tempo: TempoTracker,
    tempo_range: TempoRange,
    /// Time of the latest beat on the tracked grid, kept within one beat of
    /// now so tempo refinements barely move the phase.
    beat_anchor: f64,
    /// The latest predicted beat whose strongest onset was checked.
    judged_beat: f64,
    off_beat_beats: u32,
}

impl HopAnalyzer {
    pub(crate) fn new(sample_rate: f32) -> Self {
        let mut fft_planner = FftPlanner::new();
        let bin_width = sample_rate / FFT_SIZE as f32;
        let band_bin = |hz: f32| ((hz / bin_width).round() as usize).clamp(1, FFT_SIZE / 2 - 1);
        let envelope_capacity = (TEMPO_WINDOW_SECS * sample_rate / FFT_HOP as f32) as usize;
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
            sample_rate,
            hop_index: 0,
            onset_bins: band_bin(ONSET_BAND_HZ.0)..=band_bin(ONSET_BAND_HZ.1),
            band_level: 0.0,
            envelope: VecDeque::with_capacity(envelope_capacity + 1),
            envelope_capacity,
            hops_to_estimate: 0,
            last_onset: None,
            onsets: VecDeque::new(),
            peak_hops_left: 0,
            quiet_secs: 0.0,
            tempo: TempoTracker::default(),
            tempo_range: TempoRange::default(),
            beat_anchor: 0.0,
            judged_beat: f64::NEG_INFINITY,
            off_beat_beats: 0,
        }
    }

    /// Sets the range tempos are folded into. A different range restarts tempo
    /// tracking, so the next estimate lands in it.
    pub(crate) fn set_tempo_range(&mut self, range: TempoRange) {
        let range = TempoRange::new(range.min, range.max);
        if range != self.tempo_range {
            self.tempo_range = range;
            self.tempo = TempoTracker::default();
            self.hops_to_estimate = 0;
        }
    }

    /// Hops per second.
    fn hop_rate(&self) -> f32 {
        self.sample_rate / FFT_HOP as f32
    }

    /// Seconds from a kick's start to its detected onset.
    fn onset_latency(&self) -> f64 {
        ONSET_LATENCY_FRAMES * FFT_SIZE as f64 / f64::from(self.sample_rate)
    }

    /// Time of the strongest onset in `from..to`.
    fn strongest_onset(&self, from: f64, to: f64) -> Option<f64> {
        self.onsets
            .iter()
            .filter(|(t, _)| (from..to).contains(t))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|&(t, _)| t)
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

        // Sample-based time at the end of this hop.
        self.hop_index += 1;
        let hop_secs = FFT_HOP as f32 / self.sample_rate;
        let now = self.hop_index as f64 * f64::from(hop_secs);

        // Onset strength: spectral flux over the kick band, relative to the
        // band's level so it doesn't depend on input gain.
        let band = self.onset_bins.clone();
        let (flux, band_sum) = fft_magnitudes[band.clone()]
            .iter()
            .zip(&self.prev_fft_magnitudes[band])
            .fold((0.0, 0.0), |(flux, sum), (curr, prev)| {
                (flux + (curr - prev).max(0.0), sum + curr)
            });
        self.prev_fft_magnitudes = Arc::clone(&fft_magnitudes);
        self.band_level += (band_sum - self.band_level) * (hop_secs / BAND_LEVEL_SECS).min(1.0);
        let strength = if level < SILENCE_LEVEL || !flux.is_finite() {
            0.0
        } else {
            flux / (self.band_level.max(band_sum) + f32::EPSILON)
        };

        self.flux_history.push(strength);
        if self.flux_history.len() > ONSET_MEDIAN_WINDOW {
            self.flux_history.remove(0);
        }
        let onset_time = now - self.onset_latency();
        let onset = (strength > compute_onset_threshold(&self.flux_history)
            && self
                .last_onset
                .is_none_or(|t| onset_time - t > f64::from(MIN_ONSET_INTERVAL)))
        .then_some(onset_time);
        if let Some(t) = onset {
            self.last_onset = onset;
            self.onsets.push_back((t, strength));
            self.peak_hops_left = (ONSET_PEAK_SECS * f64::from(self.hop_rate())) as usize;
        } else if self.peak_hops_left > 0 {
            self.peak_hops_left -= 1;
            if let Some((_, peak)) = self.onsets.back_mut() {
                *peak = peak.max(strength);
            }
        }
        while self
            .onsets
            .front()
            .is_some_and(|(t, _)| now - t > ONSET_HISTORY_SECS)
        {
            self.onsets.pop_front();
        }

        self.envelope.push_back(strength);
        if self.envelope.len() > self.envelope_capacity {
            self.envelope.pop_front();
        }

        // Reset after silence or a lack of onsets, so the tempo doesn't freeze.
        self.quiet_secs = if level < SILENCE_LEVEL {
            self.quiet_secs + hop_secs
        } else {
            0.0
        };
        let onsets_stopped = self
            .last_onset
            .is_none_or(|t| now - t >= f64::from(SILENCE_RESET_SECS));
        if self.quiet_secs >= SILENCE_RESET_SECS || onsets_stopped {
            self.tempo = TempoTracker::default();
            self.envelope.clear();
            self.hops_to_estimate = 0;
        }

        self.update_tempo(now);
        self.update_phase(now);

        let time_since_beat = match self.tempo.bpm {
            Some(_) => now - self.beat_anchor,
            None => self.last_onset.map_or(0.0, |t| now - t),
        };
        AudioData {
            waveform,
            fft: fft_magnitudes,
            level,
            bpm: self.tempo.bpm,
            time_since_beat: time_since_beat as f32,
            sample_rate: self.sample_rate,
        }
    }

    /// Estimates the tempo every [`TEMPO_UPDATE_SECS`] once the envelope holds
    /// [`TEMPO_MIN_SECS`]. A replaced tempo puts the beat grid on the strongest
    /// onset of the last beat.
    fn update_tempo(&mut self, now: f64) {
        if self.hops_to_estimate > 0 {
            self.hops_to_estimate -= 1;
            return;
        }
        let hop_rate = self.hop_rate();
        if (self.envelope.len() as f32) < TEMPO_MIN_SECS * hop_rate {
            return;
        }
        self.hops_to_estimate = (TEMPO_UPDATE_SECS * hop_rate) as usize;
        if let Some(estimate) =
            estimate_bpm(self.envelope.make_contiguous(), hop_rate, self.tempo_range)
            && self.tempo.update(estimate)
            && let Some(bpm) = self.tempo.bpm
            && let Some(onset) = self
                .strongest_onset(now - 60.0 / f64::from(bpm), now)
                .or(self.last_onset)
        {
            self.beat_anchor = onset;
            self.judged_beat = onset;
            self.off_beat_beats = 0;
        }
    }

    /// Beat grid as a simple PLL on each beat's strongest onset, so a rolling
    /// bassline's notes between kicks don't move it. Once a beat's window,
    /// half a beat either side, has passed, its strongest onset pulls the grid
    /// toward itself if near the beat. Off the beat it is ignored unless that
    /// keeps happening, in which case the grid snaps to it.
    fn update_phase(&mut self, now: f64) {
        let Some(bpm) = self.tempo.bpm else {
            return;
        };
        let period = 60.0 / f64::from(bpm);
        // Onsets up to here have their peak strength.
        let settled = now - self.onset_latency() - ONSET_PEAK_SECS;
        // The latest predicted beat whose window has settled.
        let beat = self.beat_anchor
            + ((settled - 0.5 * period - self.beat_anchor) / period).floor() * period;
        if beat > self.judged_beat + 0.5 * period {
            self.judged_beat = beat;
            if let Some(onset) = self.strongest_onset(beat - 0.5 * period, beat + 0.5 * period) {
                // Distance from the predicted beat, in beats.
                let error = (onset - beat) / period;
                if error.abs() < PHASE_LOCK_BEATS {
                    self.beat_anchor += error * period * PHASE_CORRECTION;
                    self.off_beat_beats = 0;
                } else {
                    self.off_beat_beats += 1;
                    if self.off_beat_beats >= PHASE_RELOCK_BEATS {
                        self.beat_anchor = onset;
                        self.judged_beat = onset;
                        self.off_beat_beats = 0;
                    }
                }
            }
        }
        // Keep the anchor on the latest beat at or before now.
        self.beat_anchor += ((now - self.beat_anchor) / period).floor() * period;
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
    /// Starts the analysis thread, which publishes to `sender`, folds tempos
    /// into the [`TempoRange`] packed in `tempo_range`, and exits when this is
    /// dropped with the stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the analysis thread cannot be spawned.
    pub(crate) fn new(
        channels: usize,
        sample_rate: f32,
        sender: Sender<AudioData>,
        pcm_subs: Arc<ArcSwap<Vec<PcmSubscriber>>>,
        tempo_range: Arc<AtomicU64>,
    ) -> std::io::Result<Self> {
        let (hops, received) = crossbeam_channel::bounded::<[f32; FFT_HOP]>(HOP_QUEUE);
        std::thread::Builder::new()
            .name("audio-analysis".into())
            .spawn(move || {
                let mut analyzer = HopAnalyzer::new(sample_rate);
                while let Ok(hop) = received.recv() {
                    analyzer.set_tempo_range(TempoRange::from_bits(
                        tempo_range.load(Ordering::Relaxed),
                    ));
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
    use super::{CaptureState, FFT_HOP, HopAnalyzer, TempoRange, TempoTracker};
    use crate::audio::AudioData;
    use std::f32::consts::TAU;
    use std::sync::Arc;
    use std::sync::atomic::AtomicU64;

    const RATE: f32 = 48_000.0;

    /// Hats per beat, or none.
    #[derive(Clone, Copy, Debug)]
    enum Hats {
        None,
        Eighths,
        Sixteenths,
    }

    /// Deterministic white noise in -1..1.
    struct Noise(u32);

    impl Noise {
        fn next(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 17;
            self.0 ^= self.0 << 5;
            (self.0 >> 8) as f32 / (1 << 23) as f32 - 1.0
        }
    }

    /// Kick: a sine sweeping 155 Hz down to 45 Hz with an exponential decay.
    fn kick(out: &mut [f32]) {
        let mut phase = 0.0;
        for (i, v) in out.iter_mut().take((0.4 * RATE) as usize).enumerate() {
            let t = i as f32 / RATE;
            phase += TAU * (45.0 + 110.0 * (-t / 0.03).exp()) / RATE;
            *v += 0.8 * (-t / 0.12).exp() * phase.sin();
        }
    }

    /// Hat: short high-passed (first-difference) noise.
    fn hat(out: &mut [f32], noise: &mut Noise) {
        let mut prev = 0.0;
        for (i, v) in out.iter_mut().take((0.05 * RATE) as usize).enumerate() {
            let n = noise.next();
            *v += 0.25 * (-(i as f32) / RATE / 0.015).exp() * (n - prev);
            prev = n;
        }
    }

    /// Snare: noise plus a 190 Hz sine.
    fn snare(out: &mut [f32], noise: &mut Noise) {
        for (i, v) in out.iter_mut().take((0.25 * RATE) as usize).enumerate() {
            let t = i as f32 / RATE;
            *v += 0.35 * (-t / 0.06).exp() * noise.next()
                + 0.4 * (-t / 0.08).exp() * (TAU * 190.0 * t).sin();
        }
    }

    /// Bass note: a 55 Hz sine with a fast attack and a short decay, as in a
    /// rolling psytrance bassline. Softer than the kick.
    fn bass(out: &mut [f32]) {
        for (i, v) in out.iter_mut().take((0.15 * RATE) as usize).enumerate() {
            let t = i as f32 / RATE;
            let attack = (t / 0.002).min(1.0);
            *v += 0.4 * attack * (-t / 0.04).exp() * (TAU * 55.0 * t).sin();
        }
    }

    /// `secs` of a kick on each beat and a bass note on each of the three
    /// other 16ths, at `bpm`.
    fn rolling_bass(bpm: f32, secs: f32) -> Vec<f32> {
        let beat = 60.0 / bpm;
        let mut out = vec![0.0; (secs * RATE) as usize];
        let end = out.len();
        let start = |t: f32| ((t * RATE) as usize).min(end);
        for b in 0..(secs / beat).ceil() as usize {
            let t = b as f32 * beat;
            kick(&mut out[start(t)..end]);
            for sixteenth in 1..4 {
                bass(&mut out[start(t + beat * sixteenth as f32 / 4.0)..end]);
            }
        }
        out
    }

    /// `secs` of a four-on-the-floor kick at `bpm`, with optional snare on
    /// beats 2 and 4 and hats, followed by `silence` seconds.
    fn pattern(bpm: f32, secs: f32, snares: bool, hats: Hats, silence: f32) -> Vec<f32> {
        let beat = 60.0 / bpm;
        let mut out = vec![0.0; ((secs + silence) * RATE) as usize];
        let end = (secs * RATE) as usize;
        let start = |t: f32| (t * RATE) as usize;
        let mut noise = Noise(0x9e37_79b9);
        let beats = (secs / beat).ceil() as usize;
        for b in 0..beats {
            let t = b as f32 * beat;
            kick(&mut out[start(t)..end]);
            if snares && b % 2 == 1 {
                snare(&mut out[start(t)..end], &mut noise);
            }
            let per_beat = match hats {
                Hats::None => 0,
                Hats::Eighths => 2,
                Hats::Sixteenths => 4,
            };
            for h in 0..per_beat {
                let at = start(t + beat * h as f32 / per_beat as f32);
                if at < end {
                    hat(&mut out[at..end], &mut noise);
                }
            }
        }
        out
    }

    /// Runs `signal` through a fresh analyzer: each hop's output and the
    /// analyzer state after it.
    fn run(signal: &[f32], mut each: impl FnMut(usize, &AudioData, &HopAnalyzer)) {
        let mut analyzer = HopAnalyzer::new(RATE);
        for (i, hop) in signal.as_chunks::<FFT_HOP>().0.iter().enumerate() {
            let data = analyzer.analyze(hop);
            each(i, &data, &analyzer);
        }
    }

    fn hop_secs() -> f32 {
        FFT_HOP as f32 / RATE
    }

    /// BPM reported at every hop.
    fn bpm_track(signal: &[f32]) -> Vec<Option<f32>> {
        let mut track = Vec::new();
        run(signal, |_, data, _| track.push(data.bpm));
        track
    }

    /// The BPM is within ±2 of the truth from 10 s on, and never near its
    /// double or half, for kick, kick + snare, and kick + 8th or 16th hats.
    #[test]
    fn bpm_matches_synthetic_rhythms() {
        for bpm in [100.0, 128.0, 140.0] {
            for (snares, hats) in [
                (false, Hats::None),
                (true, Hats::None),
                (false, Hats::Eighths),
                (false, Hats::Sixteenths),
            ] {
                let case = format!("{bpm} BPM, snare {snares}, hats {hats:?}");
                let track = bpm_track(&pattern(bpm, 16.0, snares, hats, 0.0));
                for (i, got) in track.iter().enumerate() {
                    let t = i as f32 * hop_secs();
                    if let Some(got) = got {
                        assert!(
                            (got / bpm - 1.0).abs() < 0.25,
                            "{case}: {got} at {t:.2} s is an octave error"
                        );
                    }
                    if t >= 10.0 {
                        let got = got.unwrap_or_else(|| panic!("{case}: no BPM at {t:.2} s"));
                        assert!((got - bpm).abs() <= 2.0, "{case}: {got} at {t:.2} s");
                    }
                }
            }
        }
    }

    /// BPM clears within 3 s of the music stopping, and the clock falls back
    /// to inactive with it.
    #[test]
    fn bpm_clears_after_silence() {
        let signal = pattern(128.0, 15.0, false, Hats::None, 5.0);
        let mut clock = crate::clock::ClockManager::new();
        let mut last_bpm_at = 0.0;
        run(&signal, |i, data, _| {
            let t = i as f32 * hop_secs();
            if data.bpm.is_some() {
                last_bpm_at = t;
            }
            if (14.0..15.0).contains(&t) {
                assert!(data.bpm.is_some(), "BPM while playing at {t:.2} s");
            }
            clock.update_audio(data.bpm, data.beat_phase());
            clock.update();
        });
        assert!(last_bpm_at <= 18.0, "BPM still set at {last_bpm_at:.2} s");
        assert!(!clock.state().active);
    }

    /// The same signal at ×0.05 and ×1 gives the same onsets and BPM.
    #[test]
    fn analysis_does_not_depend_on_gain() {
        let loud = pattern(128.0, 12.0, true, Hats::Sixteenths, 0.0);
        let quiet: Vec<f32> = loud.iter().map(|v| v * 0.05).collect();
        let collect = |signal: &[f32]| {
            let mut onsets = Vec::new();
            let mut bpm = None;
            run(signal, |_, data, analyzer| {
                if analyzer.last_onset != onsets.last().copied() {
                    onsets.extend(analyzer.last_onset);
                }
                bpm = data.bpm;
            });
            (onsets, bpm)
        };
        let (loud_onsets, loud_bpm) = collect(&loud);
        let (quiet_onsets, quiet_bpm) = collect(&quiet);
        assert_eq!(loud_onsets, quiet_onsets);
        let (loud_bpm, quiet_bpm) = (loud_bpm.expect("loud BPM"), quiet_bpm.expect("quiet BPM"));
        assert!(
            (loud_bpm - quiet_bpm).abs() < 0.5,
            "{loud_bpm} vs {quiet_bpm}"
        );
    }

    /// From 10 s on, the beat phase is near 0 when each kick of `signal`, a
    /// kick on every beat at `bpm`, starts.
    fn assert_phase_zero_on_kicks(case: &str, bpm: f32, signal: &[f32]) {
        let beat = 60.0 / bpm;
        let mut kicks = 0;
        run(signal, |i, data, _| {
            // Phase at the end of the hop holding a kick's first sample.
            let start = i as f32 * hop_secs();
            let end = start + hop_secs();
            let kick = (end / beat).floor() * beat;
            if start < 10.0 || kick < start {
                return;
            }
            let phase = data.beat_phase() - (end - kick) / beat;
            let error = (phase + 0.5).rem_euclid(1.0) - 0.5;
            assert!(
                error.abs() < 0.06,
                "{case}: phase off by {error:.3} at {start:.2} s"
            );
            kicks += 1;
        });
        assert!(kicks >= 8, "{case}: only {kicks} kicks checked");
    }

    /// On a kick alone, the beat phase is near 0 when each kick starts.
    #[test]
    fn beat_phase_is_zero_on_kicks() {
        for bpm in [100.0, 128.0, 140.0] {
            let signal = pattern(bpm, 16.0, false, Hats::None, 0.0);
            assert_phase_zero_on_kicks(&format!("{bpm} BPM"), bpm, &signal);
        }
    }

    /// With a rolling bassline in the kick band, the BPM is the kick's, not a
    /// 16th's, from fast psytrance to hitech, and the beat grid stays on the
    /// kick rather than a bass note.
    #[test]
    fn kick_and_rolling_bass() {
        for bpm in [145.0, 190.0, 210.0] {
            let signal = rolling_bass(bpm, 16.0);
            let case = format!("{bpm} BPM rolling bass");
            for (i, got) in bpm_track(&signal).iter().enumerate() {
                let t = i as f32 * hop_secs();
                if t >= 10.0 {
                    let got = got.unwrap_or_else(|| panic!("{case}: no BPM at {t:.2} s"));
                    assert!((got - bpm).abs() <= 2.0, "{case}: {got} at {t:.2} s");
                }
            }
            assert_phase_zero_on_kicks(&case, bpm, &signal);
        }
    }

    /// A tempo above the range is folded into it, and setting the range again
    /// unfolds it.
    #[test]
    fn tempo_range_folds_the_bpm() {
        let signal = pattern(200.0, 16.0, false, Hats::None, 0.0);
        let mut analyzer = HopAnalyzer::new(RATE);
        analyzer.set_tempo_range(TempoRange::new(70.0, 180.0));
        let hops = signal.as_chunks::<FFT_HOP>().0;
        let (first, second) = hops.split_at(hops.len() / 2);
        let mut bpm = None;
        for hop in first {
            bpm = analyzer.analyze(hop).bpm;
        }
        let folded = bpm.expect("BPM in the narrow range");
        assert!((folded - 100.0).abs() <= 2.0, "folded to {folded}");
        analyzer.set_tempo_range(TempoRange::default());
        for hop in second {
            bpm = analyzer.analyze(hop).bpm;
        }
        let unfolded = bpm.expect("BPM in the default range");
        assert!((unfolded - 200.0).abs() <= 2.0, "unfolded to {unfolded}");
    }

    /// A single estimate at the double tempo doesn't replace the tempo; three
    /// agreeing ones do.
    #[test]
    fn tempo_needs_agreeing_estimates_to_change() {
        let mut tempo = TempoTracker::default();
        for _ in 0..3 {
            tempo.update(128.0);
        }
        assert_eq!(tempo.bpm, Some(128.0));
        tempo.update(256.0);
        tempo.update(129.0);
        tempo.update(256.0);
        assert!((tempo.bpm.unwrap() - 128.3).abs() < 0.1);
        for _ in 0..3 {
            tempo.update(140.0);
        }
        assert_eq!(tempo.bpm, Some(140.0));
    }

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
        let tempo_range = Arc::new(AtomicU64::new(TempoRange::default().to_bits()));
        let mut capture = CaptureState::new(2, 48_000.0, sender, Arc::default(), tempo_range)
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
