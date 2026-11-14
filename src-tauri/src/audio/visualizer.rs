//! Read-only analysis of the same 16 kHz PCM sent to Vosk. No audio leaves Rust.
use rustfft::{num_complex::Complex, Fft, FftPlanner};
use serde::Serialize;
use std::{f32::consts::TAU, sync::Arc};

const WINDOW: usize = 1024;
const RATE: f32 = 16_000.0;
const EDGES: [f32; 9] = [
    60.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 3500.0, 5500.0, 8000.0,
];

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct AudioLevels {
    pub rms: f32,
    pub peak: f32,
    /// Absolute digital peak and fraction of near-rail PCM samples, before DC removal.
    pub input_peak: f32,
    pub clipped_ratio: f32,
    /// Relative spectral energy, not independently normalized loudness.
    pub bands: [f32; 8],
}

pub(super) struct AudioAnalyzer {
    samples: [f32; WINDOW],
    filled: usize,
    window: [f32; WINDOW],
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    fft: Arc<dyn Fft<f32>>,
}

impl AudioAnalyzer {
    pub fn new() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(WINDOW);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        Self {
            samples: [0.0; WINDOW],
            filled: 0,
            window: std::array::from_fn(|i| {
                0.5 - 0.5 * (TAU * i as f32 / (WINDOW - 1) as f32).cos()
            }),
            spectrum: vec![Complex::default(); WINDOW],
            scratch,
            fft,
        }
    }

    pub fn push(&mut self, samples: &[i16]) -> Option<AudioLevels> {
        let mut latest = None;
        for &sample in samples {
            self.samples[self.filled] = sample as f32 / 32768.0;
            self.filled += 1;
            if self.filled == WINDOW {
                latest = Some(self.analyze());
                self.filled = 0;
            }
        }
        latest
    }

    fn analyze(&mut self) -> AudioLevels {
        let mean = self.samples.iter().sum::<f32>() / WINDOW as f32;
        let mut squares = 0.0;
        let mut peak = 0.0f32;
        let mut input_peak = 0.0f32;
        let mut clipped = 0;
        for (index, &sample) in self.samples.iter().enumerate() {
            input_peak = input_peak.max(sample.abs());
            if sample.abs() >= 0.985 {
                clipped += 1;
            }
            let ac = sample - mean;
            squares += ac * ac;
            peak = peak.max(ac.abs());
            self.spectrum[index] = Complex::new(ac * self.window[index], 0.0);
        }
        let rms = (squares / WINDOW as f32).sqrt().clamp(0.0, 1.0);
        let mut bands = [0.0; 8];
        if rms > 0.000001 {
            self.fft
                .process_with_scratch(&mut self.spectrum, &mut self.scratch);
            let mut total = 0.0;
            for bin in 1..=WINDOW / 2 {
                let power = self.spectrum[bin].norm_sqr();
                total += power;
                let hz = bin as f32 * RATE / WINDOW as f32;
                if let Some(band) = EDGES
                    .windows(2)
                    .position(|edge| hz >= edge[0] && hz < edge[1])
                {
                    bands[band] += power;
                }
            }
            if total > f32::EPSILON {
                for band in &mut bands {
                    *band = (*band / total).sqrt().clamp(0.0, 1.0);
                }
            }
        }
        AudioLevels {
            rms,
            peak: peak.clamp(0.0, 1.0),
            input_peak,
            clipped_ratio: clipped as f32 / WINDOW as f32,
            bands,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f32, amplitude: f32) -> Vec<i16> {
        (0..WINDOW)
            .map(|i| ((TAU * hz * i as f32 / RATE).sin() * amplitude * 32767.0) as i16)
            .collect()
    }

    #[test]
    fn silence_and_dc_offset_have_no_visual_energy() {
        let mut analyzer = AudioAnalyzer::new();
        for value in [0, 4000] {
            let levels = analyzer.push(&[value; WINDOW]).unwrap();
            assert_eq!(levels.rms, 0.0);
            assert_eq!(levels.peak, 0.0);
            assert_eq!(levels.bands, [0.0; 8]);
        }
    }

    #[test]
    fn higher_volume_increases_rms_without_rewriting_the_spectrum() {
        let mut analyzer = AudioAnalyzer::new();
        let quiet = analyzer.push(&tone(1500.0, 0.05)).unwrap();
        let loud = analyzer.push(&tone(1500.0, 0.25)).unwrap();
        assert!((loud.rms / quiet.rms - 5.0).abs() < 0.02);
        assert!((loud.rms - 0.25 / 2.0f32.sqrt()).abs() < 0.0001);
        assert!(loud.peak > quiet.peak * 4.9);
        for (a, b) in quiet.bands.iter().zip(loud.bands) {
            assert!((a - b).abs() < 0.01);
        }
    }

    #[test]
    fn digital_peak_and_near_rail_samples_detect_overload_without_changing_pcm() {
        let normal = tone(1500.0, 0.4);
        let normal_levels = AudioAnalyzer::new().push(&normal).unwrap();
        assert!(normal_levels.input_peak < 0.401);
        assert_eq!(normal_levels.clipped_ratio, 0.0);
        let clipped: Vec<i16> = (0..WINDOW)
            .map(|index| if index % 2 == 0 { i16::MAX } else { i16::MIN })
            .collect();
        let original = clipped.clone();
        let levels = AudioAnalyzer::new().push(&clipped).unwrap();
        assert_eq!(levels.input_peak, 1.0);
        assert_eq!(levels.clipped_ratio, 1.0);
        assert_eq!(clipped, original);
        let mut sparse = vec![0; WINDOW];
        sparse[10] = i16::MAX;
        let sparse_levels = AudioAnalyzer::new().push(&sparse).unwrap();
        assert_eq!(sparse_levels.clipped_ratio, 1.0 / WINDOW as f32);
    }

    #[test]
    fn frequency_changes_move_energy_to_the_expected_band() {
        let mut analyzer = AudioAnalyzer::new();
        for (hz, expected) in [(156.25, 1), (1500.0, 4), (6000.0, 7)] {
            let levels = analyzer.push(&tone(hz, 0.25)).unwrap();
            let dominant = levels
                .bands
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap()
                .0;
            assert_eq!(dominant, expected);
            assert!(levels.bands[expected] > 0.98);
        }
    }

    #[test]
    fn analysis_is_chunk_independent_and_never_changes_the_pcm() {
        let samples = tone(1500.0, 0.2);
        let original = samples.clone();
        let expected = AudioAnalyzer::new().push(&samples).unwrap();
        let mut analyzer = AudioAnalyzer::new();
        assert!(analyzer.push(&samples[..500]).is_none());
        let actual = analyzer.push(&samples[500..]).unwrap();
        assert_eq!(actual.rms, expected.rms);
        assert_eq!(actual.bands, expected.bands);
        assert_eq!(samples, original);
    }

    #[test]
    fn visual_updates_are_bounded_by_the_pcm_window_rate() {
        let mut analyzer = AudioAnalyzer::new();
        let mut events = 0;
        for _ in 0..1000 {
            events += usize::from(analyzer.push(&[0; 16]).is_some());
        }
        assert_eq!(events, 15); // 16,000 samples / 1,024, not one IPC event per callback.
    }
}
