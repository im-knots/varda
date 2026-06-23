//! Trivial brightness analyzer — validates analyzer plumbing.
//!
//! Computes average brightness, contrast, and dominant RGB channel from
//! an RGBA input frame using pure CPU arithmetic. No ML dependencies.

use std::collections::HashMap;

use super::traits::{Analyzer, AnalyzerInput, AnalyzerSchema, AnalyzerSnapshot, ScalarOutputDef};

/// Pixel stride default — skip every N-th pixel for speed.
const DEFAULT_SAMPLE_STRIDE: usize = 4;

/// Luminance weights (Rec.709).
const LUM_R: f32 = 0.2126;
const LUM_G: f32 = 0.7152;
const LUM_B: f32 = 0.0722;

pub(crate) struct BrightnessAnalyzer {
    sample_stride: usize,
}

impl BrightnessAnalyzer {
    pub(crate) fn new() -> Self {
        Self {
            sample_stride: DEFAULT_SAMPLE_STRIDE,
        }
    }
}

impl Analyzer for BrightnessAnalyzer {
    fn analyzer_type(&self) -> &str {
        "brightness"
    }

    fn output_schema(&self) -> AnalyzerSchema {
        AnalyzerSchema {
            scalars: vec![
                ScalarOutputDef {
                    name: "brightness".into(),
                    description: "Average luminance (Rec.709)".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.1,
                },
                ScalarOutputDef {
                    name: "contrast".into(),
                    description: "Standard deviation of luminance".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.1,
                },
                ScalarOutputDef {
                    name: "red".into(),
                    description: "Average red channel".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.1,
                },
                ScalarOutputDef {
                    name: "green".into(),
                    description: "Average green channel".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.1,
                },
                ScalarOutputDef {
                    name: "blue".into(),
                    description: "Average blue channel".into(),
                    range: (0.0, 1.0),
                    default: 0.0,
                    default_smoothing: 0.1,
                },
            ],
            textures: Vec::new(),
        }
    }

    fn init(&mut self, options: &serde_json::Value) -> anyhow::Result<()> {
        if let Some(stride) = options.get("sample_stride").and_then(|v| v.as_u64()) {
            let stride = stride.max(1) as usize;
            log::info!("BrightnessAnalyzer: sample_stride set to {stride}");
            self.sample_stride = stride;
        }
        Ok(())
    }

    fn analyze(&mut self, input: &AnalyzerInput) -> anyhow::Result<AnalyzerSnapshot> {
        let pixels = input.width as usize * input.height as usize;
        let stride = self.sample_stride;

        let mut sum_r: f64 = 0.0;
        let mut sum_g: f64 = 0.0;
        let mut sum_b: f64 = 0.0;
        let mut sum_lum: f64 = 0.0;
        let mut sum_lum_sq: f64 = 0.0;
        let mut count: usize = 0;

        let mut i = 0;
        while i < pixels {
            let off = i * 4;
            let r = input.frame[off] as f64 / 255.0;
            let g = input.frame[off + 1] as f64 / 255.0;
            let b = input.frame[off + 2] as f64 / 255.0;

            let lum = LUM_R as f64 * r + LUM_G as f64 * g + LUM_B as f64 * b;

            sum_r += r;
            sum_g += g;
            sum_b += b;
            sum_lum += lum;
            sum_lum_sq += lum * lum;
            count += 1;

            i += stride;
        }

        let (brightness, contrast, red, green, blue) = if count > 0 {
            let n = count as f64;
            let mean_lum = sum_lum / n;
            let variance = (sum_lum_sq / n - mean_lum * mean_lum).max(0.0);
            let stddev = variance.sqrt();
            (
                mean_lum as f32,
                stddev as f32,
                (sum_r / n) as f32,
                (sum_g / n) as f32,
                (sum_b / n) as f32,
            )
        } else {
            (0.0, 0.0, 0.0, 0.0, 0.0)
        };

        let mut scalars = HashMap::with_capacity(5);
        scalars.insert("brightness".into(), brightness);
        scalars.insert("contrast".into(), contrast);
        scalars.insert("red".into(), red);
        scalars.insert("green".into(), green);
        scalars.insert("blue".into(), blue);

        Ok(AnalyzerSnapshot {
            scalars,
            textures: HashMap::new(),
            timestamp: input.timestamp,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn make_frame(width: u32, height: u32, r: u8, g: u8, b: u8, a: u8) -> Vec<u8> {
        let pixels = (width * height) as usize;
        let mut buf = Vec::with_capacity(pixels * 4);
        for _ in 0..pixels {
            buf.extend_from_slice(&[r, g, b, a]);
        }
        buf
    }

    fn make_input(width: u32, height: u32, r: u8, g: u8, b: u8) -> AnalyzerInput {
        AnalyzerInput {
            frame: make_frame(width, height, r, g, b, 255),
            width,
            height,
            timestamp: Instant::now(),
        }
    }

    #[test]
    fn schema_has_correct_outputs() {
        let analyzer = BrightnessAnalyzer::new();
        let schema = analyzer.output_schema();

        let names: Vec<&str> = schema.scalars.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["brightness", "contrast", "red", "green", "blue"]);
        assert!(schema.textures.is_empty());

        for s in &schema.scalars {
            assert_eq!(s.range, (0.0, 1.0));
        }
    }

    #[test]
    fn all_white_brightness_is_one() {
        let mut analyzer = BrightnessAnalyzer::new();
        analyzer.sample_stride = 1;
        let input = make_input(8, 8, 255, 255, 255);
        let snap = analyzer.analyze(&input).unwrap();

        let b = snap.scalars["brightness"];
        assert!((b - 1.0).abs() < 1e-4, "expected ~1.0, got {b}");
        assert!(
            snap.scalars["contrast"].abs() < 1e-4,
            "expected ~0 contrast"
        );
    }

    #[test]
    fn all_black_brightness_is_zero() {
        let mut analyzer = BrightnessAnalyzer::new();
        analyzer.sample_stride = 1;
        let input = make_input(8, 8, 0, 0, 0);
        let snap = analyzer.analyze(&input).unwrap();

        let b = snap.scalars["brightness"];
        assert!(b.abs() < 1e-4, "expected ~0.0, got {b}");
    }

    #[test]
    fn known_color_rgb_values() {
        let mut analyzer = BrightnessAnalyzer::new();
        analyzer.sample_stride = 1;
        // Pure red frame
        let input = make_input(4, 4, 255, 0, 0);
        let snap = analyzer.analyze(&input).unwrap();

        assert!((snap.scalars["red"] - 1.0).abs() < 1e-4);
        assert!(snap.scalars["green"].abs() < 1e-4);
        assert!(snap.scalars["blue"].abs() < 1e-4);

        // Luminance of pure red = 0.2126
        let b = snap.scalars["brightness"];
        assert!((b - 0.2126).abs() < 1e-3, "expected ~0.2126, got {b}");
    }
}
