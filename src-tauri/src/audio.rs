use std::{
    path::Path,
    sync::{atomic::{AtomicBool, Ordering}, mpsc::{self, SyncSender, TrySendError}, Arc},
    time::{Duration, Instant},
};
use cpal::{traits::{DeviceTrait, HostTrait, StreamTrait}, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig};
use crate::vosk::Vosk;

pub enum AudioEvent {
    Partial(String),
    Final(String),
    AutoStopped,
}

/// Incremental linear sample rate conversion. Retains phase across callback boundaries.
/// Low-complexity baseline; a band-limited converter is a future accuracy improvement.
struct Resampler {
    source_per_output: f64,
    next: f64,
    index: u64,
    previous: f32,
}

impl Resampler {
    fn new(source_rate: u32) -> Self {
        Self { source_per_output: source_rate as f64 / 16_000.0, next: 0.0, index: 0, previous: 0.0 }
    }

    fn push(&mut self, mono: &[f32]) -> Vec<i16> {
        let mut result = Vec::new();
        for &sample in mono {
            let i = self.index as f64;
            while self.next <= i {
                let interpolated = if self.index == 0 { sample } else {
                    let t = (self.next - (i - 1.0)).clamp(0.0, 1.0) as f32;
                    self.previous + (sample - self.previous) * t
                };
                result.push((interpolated.clamp(-1.0, 1.0) * 32767.0) as i16);
                self.next += self.source_per_output;
            }
            self.previous = sample;
            self.index += 1;
        }
        result
    }
}

fn make_stream<T>(device: &cpal::Device, config: &StreamConfig, sender: SyncSender<Vec<i16>>,
                  overflow: Arc<AtomicBool>) -> Result<Stream, String>
where T: SizedSample + Sample + Send + 'static,
      f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let mut converter = Resampler::new(config.sample_rate.0);
    device.build_input_stream(
        config,
        move |samples: &[T], _| {
            let mut mono = Vec::with_capacity(samples.len() / channels);
            for frame in samples.chunks_exact(channels) {
                let sample = frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
                mono.push(sample);
            }
            let converted = converter.push(&mono);
            if !converted.is_empty() {
                match sender.try_send(converted) {
                    Err(TrySendError::Full(_)) => overflow.store(true, Ordering::Relaxed),
                    _ => {}
                }
            }
        },
        move |e| eprintln!("Audio capture error: {e}"),
        None,
    ).map_err(|e| format!("Не удалось открыть микрофон: {e}"))
}

pub fn record_session<F>(model_dir: &Path, vosk_dll: &Path, stop: Arc<AtomicBool>,
                         auto_stop: bool, mut event: F) -> Result<(), String>
where F: FnMut(AudioEvent) {
    let mut recognizer = Vosk::open(vosk_dll, model_dir)?;
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("Микрофон не найден")?;
    let supported = device.default_input_config().map_err(|e| format!("Формат микрофона: {e}"))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let (tx, rx) = mpsc::sync_channel::<Vec<i16>>(32);
    let overflow = Arc::new(AtomicBool::new(false));
    let stream = match sample_format {
        SampleFormat::F32 => make_stream::<f32>(&device, &config, tx, overflow.clone())?,
        SampleFormat::I16 => make_stream::<i16>(&device, &config, tx, overflow.clone())?,
        SampleFormat::U16 => make_stream::<u16>(&device, &config, tx, overflow.clone())?,
        _ => return Err(format!("Формат аудио {sample_format:?} пока не поддерживается")),
    };
    stream.play().map_err(|e| format!("Запуск микрофона: {e}"))?;
    let mut last_partial = String::new();
    let mut partial_at = Instant::now() - Duration::from_secs(1);
    let mut stopped_automatically = false;
    while !stop.load(Ordering::Acquire) {
        let samples = match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(samples) => samples,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        if recognizer.accept(&samples)? {
            let text = recognizer.result();
            last_partial.clear();
            if !text.trim().is_empty() {
                event(AudioEvent::Final(text));
                if auto_stop {
                    stopped_automatically = true;
                    break;
                }
            }
        } else if partial_at.elapsed() > Duration::from_millis(160) {
            let text = recognizer.partial();
            if text != last_partial {
                last_partial = text.clone();
                event(AudioEvent::Partial(text));
            }
            partial_at = Instant::now();
        }
        if overflow.load(Ordering::Relaxed) {
            return Err("Аудио не успевает обрабатываться: переполнение очереди".into());
        }
    }
    drop(stream); // Capture stops BEFORE flushing queued audio blocks.
    while let Ok(samples) = rx.try_recv() {
        if recognizer.accept(&samples)? {
            let text = recognizer.result();
            if !text.trim().is_empty() { event(AudioEvent::Final(text)); }
        }
    }
    let last = recognizer.finish();
    if !last.trim().is_empty() { event(AudioEvent::Final(last)); }
    if stopped_automatically { event(AudioEvent::AutoStopped); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Resampler;
    #[test]
    fn resamples_48k_to_16k_across_chunks() {
        let mut converter = Resampler::new(48_000);
        let a = converter.push(&vec![0.5; 480]);
        let b = converter.push(&vec![0.5; 480]);
        assert_eq!(a.len() + b.len(), 320);
        assert!(a.iter().chain(b.iter()).all(|x| *x > 16000));
    }
}
