//! Audio analysis values for the modulation engine.

/// Audio analysis values for a single source, passed to modulation engine.
#[derive(Debug, Clone)]
pub struct AudioSourceValues {
    pub fft: std::sync::Arc<[f32]>,
    pub level: f32,
    pub sample_rate: f32,
}

impl AudioSourceValues {
    /// Compute energy in a frequency range from the FFT data.
    /// Returns a perceptually-scaled value in roughly 0.0–1.0 range
    /// suitable for driving modulation (dB-based mapping).
    pub fn energy_in_range(&self, freq_low: f32, freq_high: f32) -> f32 {
        if self.fft.is_empty() || self.sample_rate <= 0.0 {
            return 0.0;
        }
        let fft_size = self.fft.len() * 2;
        let bin_width = self.sample_rate / fft_size as f32;
        let bin_low = ((freq_low / bin_width).floor() as usize).min(self.fft.len() - 1);
        let bin_high = ((freq_high / bin_width).ceil() as usize).min(self.fft.len());
        if bin_high <= bin_low {
            return 0.0;
        }
        let slice = &self.fft[bin_low..bin_high];
        let rms = (slice.iter().map(|v| v * v).sum::<f32>() / slice.len() as f32).sqrt();
        if rms < 1e-6 {
            return 0.0;
        }
        let db = 20.0 * rms.log10();
        ((db + 60.0) / 60.0).clamp(0.0, 1.0)
    }
}

/// All audio source data for the current frame.
#[derive(Debug, Clone, Default)]
pub struct AudioValues {
    /// Per-source audio data, keyed by `AudioSourceId`.
    pub sources: std::collections::HashMap<crate::audio::AudioSourceId, AudioSourceValues>,
}

impl AudioValues {
    /// This frame's modulation inputs from each active source's latest data.
    pub fn collect<'a>(
        sources: impl IntoIterator<Item = (crate::audio::AudioSourceId, &'a crate::audio::AudioData)>,
    ) -> Self {
        Self {
            sources: sources
                .into_iter()
                .map(|(id, data)| {
                    (
                        id,
                        AudioSourceValues {
                            fft: std::sync::Arc::clone(&data.fft),
                            level: data.level,
                            sample_rate: data.sample_rate,
                        },
                    )
                })
                .collect(),
        }
    }

    /// The first source's data.
    pub fn primary(&self) -> Option<&AudioSourceValues> {
        self.sources
            .iter()
            .min_by_key(|(id, _)| **id)
            .map(|(_, v)| v)
    }
}

/// Analyzer scalar values from all decks for the current frame, read by
/// `ModulationSource::Analyzer`.
#[derive(Debug, Default)]
pub struct AnalyzerValues {
    entries: Vec<AnalyzerValueEntry>,
}

#[derive(Debug)]
struct AnalyzerValueEntry {
    deck_id: String,
    analyzer_type: String,
    output_name: String,
    value: f32,
}

impl AnalyzerValues {
    /// Look up a scalar analyzer output. Returns 0.0 if not found.
    pub fn get(&self, deck_id: &str, analyzer_type: &str, output_name: &str) -> f32 {
        self.entries
            .iter()
            .find(|e| {
                e.deck_id == deck_id
                    && e.analyzer_type == analyzer_type
                    && e.output_name == output_name
            })
            .map_or(0.0, |e| e.value)
    }

    /// Add a scalar value entry.
    pub fn insert(
        &mut self,
        deck_id: String,
        analyzer_type: String,
        output_name: String,
        value: f32,
    ) {
        self.entries.push(AnalyzerValueEntry {
            deck_id,
            analyzer_type,
            output_name,
            value,
        });
    }

    /// Clear all entries, keeping the allocation.
    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::AudioValues;
    use crate::audio::AudioData;

    /// A frame's modulation inputs share each source's spectrum instead of copying it.
    #[test]
    fn collected_values_share_each_sources_spectrum() {
        let data = AudioData::default();
        let values = AudioValues::collect([(3, &data)]);
        assert!(std::sync::Arc::ptr_eq(&values.sources[&3].fft, &data.fft));
        assert!(std::sync::Arc::ptr_eq(
            &data.clone().waveform,
            &data.waveform
        ));
    }
}
