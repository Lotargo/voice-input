//! Local CPU processing outside the capture callback. Source peaks precede all filters.
use super::{visualizer::AudioAnalyzer, AudioLevels};
use nnnoiseless::DenoiseState;
use std::f64::consts::PI;

pub struct Captured {
    pub mono: Vec<f32>,
    pub peak: f32,
    pub clipped: usize,
    pub samples: usize,
}

/// Stateful conversion; the FIR prevents high-frequency input folding into speech.
pub struct Resampler {
    step: f64,
    next: f64,
    index: u64,
    previous: f32,
    coefficients: Vec<f32>,
    history: Vec<f32>,
    cursor: usize,
}
impl Resampler {
    pub fn new(input: u32, output: u32) -> Self {
        let coefficients = if output < input {
            let cutoff = 0.47 * output as f64 / input as f64;
            let mut taps: Vec<f32> = (0..63)
                .map(|i| {
                    let x = i as f64 - 31.0;
                    let sinc = if x == 0.0 {
                        2.0 * cutoff
                    } else {
                        (2.0 * PI * cutoff * x).sin() / (PI * x)
                    };
                    (sinc * (0.54 - 0.46 * (2.0 * PI * i as f64 / 62.0).cos())) as f32
                })
                .collect();
            let sum: f32 = taps.iter().sum();
            taps.iter_mut().for_each(|x| *x /= sum);
            taps
        } else {
            Vec::new()
        };
        Self {
            step: input as f64 / output as f64,
            next: 0.0,
            index: 0,
            previous: 0.0,
            history: vec![0.0; coefficients.len()],
            coefficients,
            cursor: 0,
        }
    }
    pub fn push(&mut self, input: &[f32]) -> Vec<f32> {
        let mut output = Vec::new();
        for &raw in input {
            let mut sample = if raw.is_finite() {
                raw.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            if !self.coefficients.is_empty() {
                self.history[self.cursor] = sample;
                sample = self
                    .coefficients
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        c * self.history
                            [(self.cursor + self.history.len() - i) % self.history.len()]
                    })
                    .sum();
                self.cursor = (self.cursor + 1) % self.history.len();
            }
            let index = self.index as f64;
            while self.next <= index {
                let value = if self.index == 0 {
                    sample
                } else {
                    self.previous
                        + (sample - self.previous)
                            * (self.next - (index - 1.0)).clamp(0.0, 1.0) as f32
                };
                output.push(value);
                self.next += self.step;
            }
            self.previous = sample;
            self.index += 1;
        }
        output
    }
}

/// A gentle 100 Hz high-pass targets low fan/desk rumble that can resemble pitch.
struct HighPass {
    b: [f32; 3],
    a: [f32; 2],
    x: [f32; 2],
    y: [f32; 2],
}
impl HighPass {
    fn new() -> Self {
        let w = 2.0 * std::f32::consts::PI * 100.0 / 48000.0;
        let alpha = w.sin() / (2.0 * std::f32::consts::FRAC_1_SQRT_2);
        let a0 = 1.0 + alpha;
        Self {
            b: [
                (1.0 + w.cos()) / (2.0 * a0),
                -(1.0 + w.cos()) / a0,
                (1.0 + w.cos()) / (2.0 * a0),
            ],
            a: [-2.0 * w.cos() / a0, (1.0 - alpha) / a0],
            x: [0.0; 2],
            y: [0.0; 2],
        }
    }
    fn push(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.b[1] * self.x[0] + self.b[2] * self.x[1]
            - self.a[0] * self.y[0]
            - self.a[1] * self.y[1];
        self.x = [x, self.x[0]];
        self.y = [y, self.y[0]];
        y
    }
}

pub struct Processor {
    source_rate: u32,
    input: Resampler,
    output: Resampler,
    denoiser: Option<Box<DenoiseState<'static>>>,
    high_pass: HighPass,
    frame: [f32; 480],
    filled: usize,
    analyzer: AudioAnalyzer,
    raw_peak: f32,
    raw_clipped: usize,
    raw_samples: usize,
    raw_energy: f64,
    raw_mono_count: usize,
}
impl Processor {
    pub fn new(source_rate: u32, denoise: bool) -> Self {
        Self {
            source_rate,
            input: Resampler::new(source_rate, if denoise { 48000 } else { 16000 }),
            output: Resampler::new(48000, 16000),
            denoiser: denoise.then(DenoiseState::new),
            high_pass: HighPass::new(),
            frame: [0.0; 480],
            filled: 0,
            analyzer: AudioAnalyzer::new(),
            raw_peak: 0.0,
            raw_clipped: 0,
            raw_samples: 0,
            raw_energy: 0.0,
            raw_mono_count: 0,
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new(self.source_rate, self.denoiser.is_some());
    }
    fn convert(&mut self, input: &[f32]) -> Vec<i16> {
        let samples = self.input.push(input);
        let mut result = Vec::new();
        if self.denoiser.is_none() {
            result.extend(samples.into_iter().map(pcm));
        } else {
            for sample in samples {
                self.frame[self.filled] = self.high_pass.push(sample) * 32768.0;
                self.filled += 1;
                if self.filled == 480 {
                    let mut cleaned = [0.0; 480];
                    self.denoiser
                        .as_mut()
                        .unwrap()
                        .process_frame(&mut cleaned, &self.frame);
                    cleaned.iter_mut().for_each(|v| *v /= 32768.0);
                    result.extend(self.output.push(&cleaned).into_iter().map(pcm));
                    self.filled = 0;
                }
            }
        }
        result
    }
    pub fn push(&mut self, captured: Captured) -> (Vec<i16>, Option<AudioLevels>) {
        self.raw_peak = self.raw_peak.max(captured.peak);
        self.raw_clipped += captured.clipped;
        self.raw_samples += captured.samples;
        self.raw_energy += captured
            .mono
            .iter()
            .map(|x| (*x as f64).powi(2))
            .sum::<f64>();
        self.raw_mono_count += captured.mono.len();
        let samples = self.convert(&captured.mono);
        let levels = self.analyzer.push(&samples).map(|mut level| {
            level.input_peak = self.raw_peak;
            level.clipped_ratio = self.raw_clipped as f32 / self.raw_samples.max(1) as f32;
            level.raw_rms = (self.raw_energy / self.raw_mono_count.max(1) as f64).sqrt() as f32;
            self.raw_peak = 0.0;
            self.raw_clipped = 0;
            self.raw_samples = 0;
            self.raw_energy = 0.0;
            self.raw_mono_count = 0;
            level
        });
        (samples, levels)
    }
    pub fn finish(&mut self) -> Vec<i16> {
        // Drain filter history, the partial 10 ms frame and RNNoise's delayed output.
        let mut result = self.convert(&[0.0; 63]);
        if let Some(denoiser) = &mut self.denoiser {
            if self.filled > 0 {
                self.frame[self.filled..].fill(0.0);
                let mut tail = [0.0; 480];
                denoiser.process_frame(&mut tail, &self.frame);
                tail.iter_mut().for_each(|v| *v /= 32768.0);
                result.extend(self.output.push(&tail).into_iter().map(pcm));
            }
            let mut tail = [0.0; 480];
            denoiser.process_frame(&mut tail, &[0.0; 480]);
            tail.iter_mut().for_each(|v| *v /= 32768.0);
            result.extend(self.output.push(&tail).into_iter().map(pcm));
            result.extend(self.output.push(&[0.0; 63]).into_iter().map(pcm));
        }
        result
    }
}
fn pcm(value: f32) -> i16 {
    if value.is_finite() {
        (value.clamp(-1.0, 1.0) * 32767.0) as i16
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fan_noise_is_reduced_and_real_vosk_still_recognizes_the_speech() {
        let speech: Vec<f32> = include_bytes!("../../tests/fixtures/voice-ru.pcm")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
            .collect();
        let speech = Resampler::new(16000, 48000).push(&speech);
        let mut input = vec![0.0; 48000];
        input.extend(speech);
        input.extend(vec![0.0; 48000]);
        let mut seed = 17_u32;
        for (i, sample) in input.iter_mut().enumerate() {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            let air = ((seed >> 8) as f32 / 16777216.0 - 0.5) * 0.018;
            let hum = (2.0 * std::f32::consts::PI * 90.0 * i as f32 / 48000.0).sin() * 0.018;
            *sample = (*sample + air + hum).clamp(-1.0, 1.0);
        }
        let root = std::env::var_os("VOICE_INPUT_SMOKE_ROOT")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources"));
        let mut rms_values = Vec::new();
        let mut word_counts = Vec::new();
        for enabled in [false, true] {
            let mut processor = Processor::new(48000, enabled);
            let mut output = Vec::new();
            for chunk in input.chunks(1733) {
                let captured = Captured {
                    mono: chunk.to_vec(),
                    peak: chunk.iter().map(|x| x.abs()).fold(0.0, f32::max),
                    clipped: 0,
                    samples: chunk.len(),
                };
                output.extend(processor.push(captured).0);
            }
            output.extend(processor.finish());
            let rms = (output[8000..14000]
                .iter()
                .map(|x| (*x as f64 / 32768.0).powi(2))
                .sum::<f64>()
                / 6000.0)
                .sqrt();
            rms_values.push(rms);
            let mut recognizer = crate::vosk::Vosk::open(
                &root.join("libvosk.dll"),
                &root.join("models/vosk-model-small-ru-0.22"),
            )
            .unwrap();
            let mut text = String::new();
            for chunk in output.chunks(512) {
                if recognizer.accept(chunk).unwrap() {
                    text.push_str(&recognizer.result());
                    text.push(' ');
                }
            }
            text.push_str(&recognizer.finish());
            let count = ["проверка", "голосового", "ввода", "один", "два", "три"]
                .iter()
                .filter(|word| text.split_whitespace().any(|v| &v == *word))
                .count();
            word_counts.push(count);
            eprintln!("Noise suppression {enabled}: noise RMS {rms:.6}; recognized: {text}");
        }
        assert!(
            rms_values[1] < rms_values[0] * 0.7,
            "Fan noise reduction too small: {rms_values:?}"
        );
        assert!(
            word_counts[1] >= 5 && word_counts[1] + 1 >= word_counts[0],
            "Speech lost: {word_counts:?}"
        );
    }
    #[test]
    fn resampling_is_chunk_independent_and_filters_out_of_band_tones() {
        let signal: Vec<_> = (0..48000)
            .map(|i| (2.0 * std::f32::consts::PI * 12000.0 * i as f32 / 48000.0).sin() * 0.5)
            .collect();
        let full = Resampler::new(48000, 16000).push(&signal);
        let mut converter = Resampler::new(48000, 16000);
        let chunks: Vec<_> = signal.chunks(137).flat_map(|c| converter.push(c)).collect();
        assert_eq!(full, chunks);
        assert_eq!(full.len(), 16000);
        let rms = (full[100..].iter().map(|x| x * x).sum::<f32>() / 15900.0).sqrt();
        assert!(rms < 0.01, "Out-of-band RMS: {rms}");
    }
    #[test]
    fn denoising_does_not_hide_raw_clipping() {
        let mut processor = Processor::new(48000, true);
        let (_, levels) = processor.push(Captured {
            mono: vec![1.0; 4800],
            peak: 1.0,
            clipped: 4800,
            samples: 4800,
        });
        let levels = levels.unwrap();
        assert_eq!(levels.input_peak, 1.0);
        assert_eq!(levels.clipped_ratio, 1.0);
        assert_eq!(levels.raw_rms, 1.0);
        assert!(levels.rms < levels.raw_rms);
    }
}
