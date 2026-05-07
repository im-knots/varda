//! Audio analysis values for the modulation engine.

/// Audio analysis values for a single source, passed to modulation engine.
#[derive(Debug, Clone)]
pub struct AudioSourceValues {
    pub fft: Vec<f32>,
    pub level: f32,
    pub sample_rate: f32,
}

impl AudioSourceValues {
    /// Compute energy in a frequency range from the FFT data.
    /// Returns a perceptually-scaled value in roughly 0.0–1.0 range
    /// suitable for driving modulation (dB-based mapping).
    pub fn energy_in_range(&self, freq_low: f32, freq_high: f32) -> f32 {
        if self.fft.is_empty() || self.sample_rate <= 0.0 { return 0.0; }
        let fft_size = self.fft.len() * 2;
        let bin_width = self.sample_rate / fft_size as f32;
        let bin_low = ((freq_low / bin_width).floor() as usize).min(self.fft.len() - 1);
        let bin_high = ((freq_high / bin_width).ceil() as usize).min(self.fft.len());
        if bin_high <= bin_low { return 0.0; }
        let slice = &self.fft[bin_low..bin_high];
        let rms = (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt();
        if rms < 1e-6 { return 0.0; }
        let db = 20.0 * rms.log10();
        ((db + 60.0) / 60.0).clamp(0.0, 1.0)
    }
}

/// All audio source data for the current frame.
#[derive(Debug, Clone, Default)]
pub struct AudioValues {
    /// Per-source audio data, keyed by AudioSourceId.
    pub sources: std::collections::HashMap<crate::audio::AudioSourceId, AudioSourceValues>,
}

impl AudioValues {
    /// Get the first/primary source's data (convenience).
    pub fn primary(&self) -> Option<&AudioSourceValues> {
        self.sources.iter().min_by_key(|(id, _)| **id).map(|(_, v)| v)
    }
}
