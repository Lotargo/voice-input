use crate::vosk::Vosk;
mod pipeline;
mod silence;
mod visualizer;
use cpal::{
    traits::{DeviceTrait, HostTrait, StreamTrait},
    FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
};
use pipeline::Pipeline;
pub use silence::{SplitOptions, SplitSettings};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
        Arc,
    },
    time::Duration,
};
pub use visualizer::AudioLevels;

pub enum AudioEvent {
    Started,
    Levels(AudioLevels),
    Partial(String),
    Final(String),
    Boundary(RecordAction),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordAction {
    New,
    Clear,
    Automatic,
}

enum CaptureInput {
    Pcm(Vec<i16>),
    Boundary(RecordAction, u64, u64, Sender<Result<(), String>>),
}

#[derive(Clone)]
pub struct CaptureControl {
    sender: SyncSender<CaptureInput>,
}

pub struct InputPipe {
    sender: SyncSender<CaptureInput>,
    receiver: Receiver<CaptureInput>,
}

pub fn input_channel() -> (CaptureControl, InputPipe) {
    let (sender, receiver) = mpsc::sync_channel(64);
    (
        CaptureControl {
            sender: sender.clone(),
        },
        InputPipe { sender, receiver },
    )
}

impl CaptureControl {
    pub fn request(
        &self,
        action: RecordAction,
        record_id: u64,
        generation: u64,
    ) -> Result<Receiver<Result<(), String>>, String> {
        let (reply, received) = mpsc::channel();
        self.sender
            .try_send(CaptureInput::Boundary(action, record_id, generation, reply))
            .map_err(|_| "Запись занята или уже останавливается. Повторите действие.".to_owned())?;
        Ok(received)
    }
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
        Self {
            source_per_output: source_rate as f64 / 16_000.0,
            next: 0.0,
            index: 0,
            previous: 0.0,
        }
    }

    fn push(&mut self, mono: &[f32]) -> Vec<i16> {
        let mut result = Vec::new();
        for &sample in mono {
            let i = self.index as f64;
            while self.next <= i {
                let interpolated = if self.index == 0 {
                    sample
                } else {
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

fn make_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    sender: SyncSender<CaptureInput>,
    overflow: Arc<AtomicBool>,
) -> Result<Stream, String>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let mut converter = Resampler::new(config.sample_rate.0);
    device
        .build_input_stream(
            config,
            move |samples: &[T], _| {
                let mut mono = Vec::with_capacity(samples.len() / channels);
                for frame in samples.chunks_exact(channels) {
                    let sample =
                        frame.iter().map(|s| f32::from_sample(*s)).sum::<f32>() / channels as f32;
                    mono.push(sample);
                }
                let converted = converter.push(&mono);
                if !converted.is_empty() {
                    match sender.try_send(CaptureInput::Pcm(converted)) {
                        Err(TrySendError::Full(_)) => overflow.store(true, Ordering::Relaxed),
                        _ => {}
                    }
                }
            },
            move |e| eprintln!("Audio capture error: {e}"),
            None,
        )
        .map_err(|e| format!("Не удалось открыть микрофон: {e}"))
}

pub fn record_session<F>(
    model_dir: &Path,
    vosk_dll: &Path,
    stop: Arc<AtomicBool>,
    input: InputPipe,
    settings: Arc<SplitSettings>,
    mut event: F,
    is_current: impl Fn(u64, u64) -> bool,
) -> Result<(), String>
where
    F: FnMut(AudioEvent),
{
    let recognizer = Vosk::open(vosk_dll, model_dir)?;
    let host = cpal::default_host();
    let device = host.default_input_device().ok_or("Микрофон не найден")?;
    let supported = device
        .default_input_config()
        .map_err(|e| format!("Формат микрофона: {e}"))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let InputPipe {
        sender: tx,
        receiver: rx,
    } = input;
    let overflow = Arc::new(AtomicBool::new(false));
    let stream = match sample_format {
        SampleFormat::F32 => make_stream::<f32>(&device, &config, tx, overflow.clone())?,
        SampleFormat::I16 => make_stream::<i16>(&device, &config, tx, overflow.clone())?,
        SampleFormat::U16 => make_stream::<u16>(&device, &config, tx, overflow.clone())?,
        _ => {
            return Err(format!(
                "Формат аудио {sample_format:?} пока не поддерживается"
            ))
        }
    };
    let mut pipeline = Pipeline::new(recognizer);
    stream
        .play()
        .map_err(|e| format!("Запуск микрофона: {e}"))?;
    event(AudioEvent::Started);
    let mut process = |input: CaptureInput| -> Result<(), String> {
        match input {
            CaptureInput::Pcm(samples) => pipeline.push(&samples, &settings, &mut event),
            CaptureInput::Boundary(action, record_id, generation, reply) => {
                if !is_current(record_id, generation) {
                    let _ = reply.send(Err(
                        "Запись уже изменилась. Повторите действие для текущей записи.".into(),
                    ));
                    return Ok(());
                }
                let result = pipeline.boundary(action, &mut event);
                let _ = reply.send(result.clone());
                result
            }
        }
    };
    while !stop.load(Ordering::Acquire) {
        let input = match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(input) => input,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => break,
        };
        process(input)?;
        if overflow.load(Ordering::Relaxed) {
            return Err("Аудио не успевает обрабатываться: переполнение очереди".into());
        }
    }
    drop(stream); // Capture stops BEFORE flushing queued audio blocks.
    while let Ok(input) = rx.try_recv() {
        process(input)?;
    }
    drop(process);
    pipeline.finish(&mut event)?;
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
