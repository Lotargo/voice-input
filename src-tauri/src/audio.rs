use crate::vosk::Vosk;
mod pipeline;
mod processing;
pub mod settings;
mod silence;
mod visualizer;
use cpal::{
    traits::{DeviceTrait, StreamTrait},
    FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
};
use pipeline::Pipeline;
use processing::{Captured, Processor};
pub use settings::{InputOptions, InputView, Preferences};
pub use silence::{SplitOptions, SplitSettings};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
pub use visualizer::AudioLevels;

pub enum AudioEvent {
    Started(String),
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
    Pcm(Captured),
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

fn make_stream<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    sender: SyncSender<CaptureInput>,
    overflow: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
) -> Result<Stream, String>
where
    T: SizedSample + Sample + Send + 'static,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    device.build_input_stream(config.clone(), move |samples:&[T],_| {
        let mut mono = Vec::with_capacity(samples.len()/channels);
        let mut peak = 0.0_f32; let mut clipped = 0;
        for frame in samples.chunks_exact(channels) {
            let mut sum = 0.0;
            for sample in frame {
                let value = f32::from_sample(*sample);
                let value = if value.is_finite() {value} else {0.0};
                peak = peak.max(value.abs());
                if value.abs() >= 0.985 { clipped += 1; }
                sum += value;
            }
            mono.push((sum/channels as f32).clamp(-1.0,1.0));
        }
        if !mono.is_empty() {
            if let Err(TrySendError::Full(_)) = sender.try_send(CaptureInput::Pcm(Captured { mono, peak:peak.min(1.0), clipped, samples:samples.len() })) {
                overflow.store(true, Ordering::Relaxed);
            }
        }
    }, move |e| {
        eprintln!("Audio capture error: {e}");
        *error.lock().unwrap() = Some("Микрофон отключён или недоступен. Распознанный текст остаётся в текущей записи.".into());
    }, None).map_err(|e| format!("Не удалось открыть микрофон: {e}"))
}

fn feed<R: pipeline::Recognizer>(
    samples: &[i16],
    pipeline: &mut Option<Pipeline<R>>,
    settings: &SplitSettings,
    event: &mut impl FnMut(AudioEvent),
) -> Result<(), String> {
    if let Some(pipeline) = pipeline {
        pipeline.push(samples, settings, event)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub fn record_session<F>(
    model_dir: &Path,
    vosk_dll: &Path,
    stop: Arc<AtomicBool>,
    input: InputPipe,
    settings: Arc<SplitSettings>,
    options: InputOptions,
    testing: bool,
    mut event: F,
    is_current: impl Fn(u64, u64) -> bool,
) -> Result<(), String>
where
    F: FnMut(AudioEvent),
{
    let device = settings::resolve(&options)?;
    let device_id = device
        .id()
        .map_err(|e| format!("Идентификатор микрофона: {e}"))?
        .to_string();
    let name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "Микрофон".into());
    let supported = device
        .default_input_config()
        .map_err(|e| format!("Формат микрофона: {e}"))?;
    let sample_format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let pipeline = if testing {
        None
    } else {
        Some(Pipeline::new(Vosk::open(vosk_dll, model_dir)?))
    };
    if stop.load(Ordering::Acquire) {
        return Ok(());
    }
    let InputPipe {
        sender: tx,
        receiver: rx,
    } = input;
    let overflow = Arc::new(AtomicBool::new(false));
    let stream_error = Arc::new(Mutex::new(None));
    macro_rules! stream {
        ($t:ty) => {
            make_stream::<$t>(&device, &config, tx, overflow.clone(), stream_error.clone())?
        };
    }
    let stream = match sample_format {
        SampleFormat::F32 => stream!(f32),
        SampleFormat::I16 => stream!(i16),
        SampleFormat::U16 => stream!(u16),
        SampleFormat::I8 => stream!(i8),
        SampleFormat::U8 => stream!(u8),
        SampleFormat::I32 => stream!(i32),
        SampleFormat::U32 => stream!(u32),
        SampleFormat::F64 => stream!(f64),
        _ => {
            return Err(format!(
                "Формат аудио {sample_format:?} пока не поддерживается"
            ))
        }
    };
    let processor = Processor::new(config.sample_rate, options.denoise);
    stream
        .play()
        .map_err(|e| format!("Запуск микрофона: {e}"))?;
    event(AudioEvent::Started(name));
    consume(
        stream,
        rx,
        stop,
        stream_error,
        overflow,
        processor,
        pipeline,
        settings,
        testing,
        Some(device_id),
        event,
        is_current,
    )
}

#[allow(clippy::too_many_arguments)]
fn consume<R: pipeline::Recognizer, S>(
    stream: S,
    rx: Receiver<CaptureInput>,
    stop: Arc<AtomicBool>,
    stream_error: Arc<Mutex<Option<String>>>,
    overflow: Arc<AtomicBool>,
    mut processor: Processor,
    mut pipeline: Option<Pipeline<R>>,
    settings: Arc<SplitSettings>,
    testing: bool,
    device_id: Option<String>,
    mut event: impl FnMut(AudioEvent),
    is_current: impl Fn(u64, u64) -> bool,
) -> Result<(), String> {
    let started = Instant::now();
    let mut process = |input: CaptureInput| -> Result<(), String> {
        match input {
            CaptureInput::Pcm(samples) => {
                let (samples, levels) = processor.push(samples);
                if let Some(levels) = levels {
                    event(AudioEvent::Levels(levels));
                }
                feed(&samples, &mut pipeline, &settings, &mut event)
            }
            CaptureInput::Boundary(action, record_id, generation, reply) => {
                if !is_current(record_id, generation) {
                    let _ = reply.send(Err(
                        "Запись уже изменилась. Повторите действие для текущей записи.".into(),
                    ));
                    return Ok(());
                }
                let result = (|| {
                    if action == RecordAction::New {
                        feed(&processor.finish(), &mut pipeline, &settings, &mut event)?;
                    }
                    processor.reset();
                    if let Some(pipeline) = &mut pipeline {
                        pipeline.boundary(action, &mut event)?;
                    }
                    Ok(())
                })();
                let _ = reply.send(result.clone());
                result
            }
        }
    };
    // An error is retained even if the PCM queue is full. Drain accepted PCM before finalizing.
    let mut failure = None;
    let mut last_received = Instant::now();
    let mut last_device_check = Instant::now();
    while !stop.load(Ordering::Acquire) {
        if testing && started.elapsed() >= Duration::from_secs(20) {
            break;
        }
        if let Some(error) = stream_error.lock().unwrap().take() {
            failure = Some(error);
            break;
        }
        if last_device_check.elapsed() >= Duration::from_secs(1) {
            if let Some(id) = &device_id {
                match settings::connected(id) {
                    Ok(true) => {}
                    Ok(false) => {
                        failure = Some(
                            "Микрофон отключён. Распознанный текст остаётся в текущей записи."
                                .into(),
                        );
                        break;
                    }
                    Err(error) => {
                        failure = Some(error);
                        break;
                    }
                }
            }
            last_device_check = Instant::now();
        }
        if overflow.load(Ordering::Relaxed) {
            failure = Some("Аудио не успевает обрабатываться: переполнение очереди".into());
            break;
        }
        match rx.recv_timeout(Duration::from_millis(80)) {
            Ok(input) => {
                if matches!(&input, CaptureInput::Pcm(_)) {
                    last_received = Instant::now();
                }
                if let Err(error) = process(input) {
                    failure = Some(error);
                    break;
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_received.elapsed() > Duration::from_secs(3) {
                    failure = Some("Микрофон перестал передавать звук. Распознанный текст остаётся в текущей записи.".into());
                    break;
                }
            }
            Err(_) => {
                failure = Some("Поток микрофона завершился".into());
                break;
            }
        }
    }
    drop(stream);
    while let Ok(input) = rx.try_recv() {
        if let Err(error) = process(input) {
            failure.get_or_insert(error);
        }
    }
    drop(process);
    feed(&processor.finish(), &mut pipeline, &settings, &mut event)?;
    if let Some(pipeline) = &mut pipeline {
        pipeline.finish(&mut event)?;
    }
    failure.map_or(Ok(()), Err)
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Counting(usize);
    impl pipeline::Recognizer for Counting {
        fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
            self.0 += samples.len();
            Ok(false)
        }
        fn result(&mut self) -> String {
            String::new()
        }
        fn partial(&mut self) -> String {
            String::new()
        }
        fn finish(&mut self) -> String {
            self.0.to_string()
        }
        fn reset(&mut self) {
            self.0 = 0;
        }
    }
    struct FakeStream(Arc<AtomicBool>);
    impl Drop for FakeStream {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    #[test]
    fn disconnection_drains_accepted_pcm_and_finalizes_once_after_capture_stops() {
        let mut transcripts = Vec::new();
        for disconnected in [false, true] {
            let (tx, rx) = mpsc::sync_channel(64);
            for _ in 0..3 {
                tx.send(CaptureInput::Pcm(Captured {
                    mono: vec![0.25; 4800],
                    peak: 0.25,
                    clipped: 0,
                    samples: 4800,
                }))
                .unwrap();
            }
            drop(tx);
            let dropped = Arc::new(AtomicBool::new(false));
            let mut finals = Vec::new();
            let error = if disconnected {
                Some("device disconnected".into())
            } else {
                None
            };
            let result = consume(
                FakeStream(dropped.clone()),
                rx,
                Arc::new(AtomicBool::new(!disconnected)),
                Arc::new(Mutex::new(error)),
                Arc::new(AtomicBool::new(false)),
                Processor::new(48000, false),
                Some(Pipeline::new(Counting(0))),
                Arc::new(SplitSettings::new()),
                false,
                None,
                |event| {
                    if let AudioEvent::Final(text) = event {
                        assert!(dropped.load(Ordering::Acquire));
                        finals.push(text);
                    }
                },
                |_, _| true,
            );
            assert_eq!(result.is_err(), disconnected);
            assert_eq!(finals.len(), 1);
            assert!(finals[0].parse::<usize>().unwrap() >= 4800);
            transcripts.push(finals);
        }
        assert_eq!(transcripts[0], transcripts[1]);
    }
    #[test]
    fn microphone_test_cannot_produce_transcripts_or_record_boundaries() {
        let (tx, rx) = mpsc::sync_channel(64);
        tx.send(CaptureInput::Pcm(Captured {
            mono: vec![0.25; 4800],
            peak: 0.25,
            clipped: 0,
            samples: 4800,
        }))
        .unwrap();
        drop(tx);
        let mut events = Vec::new();
        consume::<Counting, _>(
            (),
            rx,
            Arc::new(AtomicBool::new(true)),
            Arc::new(Mutex::new(None)),
            Arc::new(AtomicBool::new(false)),
            Processor::new(48000, false),
            None,
            Arc::new(SplitSettings::new()),
            true,
            None,
            |event| events.push(event),
            |_, _| true,
        )
        .unwrap();
        assert!(!events.is_empty());
        assert!(events
            .iter()
            .all(|event| matches!(event, AudioEvent::Levels(_))));
    }
}
