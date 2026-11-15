//! Native CPU recognizers. The shared pipeline owns VAD, records and delivery.
use crate::{audio::pipeline::Recognizer, catalog::Selection, vosk::Vosk};
use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OnlineRecognizer, OnlineRecognizerConfig,
    OnlineStream,
};
use std::path::Path;

pub fn open(
    selection: &Selection,
    runtime_file: &Path,
) -> Result<Box<dyn Recognizer + Send>, String> {
    if selection.engine == "vosk" {
        return open_with_threads(selection, runtime_file, 2);
    }
    // For native engines the caller resolves the CPU profile itself; no Vosk DLL is needed.
    let profile = runtime_file;
    if !profile.is_file() {
        return Err("Не найдены параметры CPU runtime: models/CPU-Execution.conf".into());
    }
    let logical = std::thread::available_parallelism().map_or(2, |n| n.get());
    open_configured(
        selection,
        runtime_file,
        cpu_threads(logical),
        &format!("cpu:{}", profile.to_string_lossy()),
    )
}
pub(crate) fn cpu_threads(logical: usize) -> i32 {
    // Keep headroom for capture/DSP and the foreground editor. Do not use every logical core.
    logical.div_ceil(2).clamp(1, 4) as i32
}
pub(crate) fn open_with_threads(
    selection: &Selection,
    runtime_file: &Path,
    threads: i32,
) -> Result<Box<dyn Recognizer + Send>, String> {
    open_configured(selection, runtime_file, threads, "cpu")
}
pub(crate) fn open_configured(
    selection: &Selection,
    runtime_file: &Path,
    threads: i32,
    provider: &str,
) -> Result<Box<dyn Recognizer + Send>, String> {
    if !(1..=8).contains(&threads) {
        return Err("Для CPU требуется от 1 до 8 потоков".into());
    }
    match selection.engine.as_str() {
        "vosk" => Ok(Box::new(Vosk::open(runtime_file, &selection.path)?)),
        "gigaam" => Ok(Box::new(Giga::open(&selection.path, threads, provider)?)),
        "nemotron" => Ok(Box::new(Nemotron::open(
            &selection.path,
            threads,
            provider,
        )?)),
        _ => Err("Неизвестный движок распознавания".into()),
    }
}
fn filename(root: &Path, name: &str) -> Option<String> {
    Some(root.join(name).to_string_lossy().into_owned())
}
pub struct Giga {
    decoder: OfflineRecognizer,
    samples: Vec<f32>,
    speech: bool,
    heard: bool,
    silence: usize,
    ready: String,
}
impl Giga {
    fn open(root: &Path, threads: i32, provider: &str) -> Result<Self, String> {
        crate::native_runtime::ensure_offline()?;
        let mut config = OfflineRecognizerConfig::default();
        config.model_config.transducer.encoder = filename(root, "encoder.int8.onnx");
        config.model_config.transducer.decoder = filename(root, "decoder.onnx");
        config.model_config.transducer.joiner = filename(root, "joiner.onnx");
        config.model_config.tokens = filename(root, "tokens.txt");
        config.model_config.model_type = Some("nemo_transducer".into());
        config.model_config.provider = Some(provider.into());
        config.model_config.num_threads = threads;
        config.decoding_method = Some("greedy_search".into());
        let decoder =
            OfflineRecognizer::create(&config).ok_or("GigaAM не смог загрузить модель")?;
        Ok(Self {
            decoder,
            samples: Vec::new(),
            speech: false,
            heard: false,
            silence: 0,
            ready: String::new(),
        })
    }
    fn decode(&mut self) -> Result<String, String> {
        if !self.heard {
            self.samples.clear();
            return Ok(String::new());
        }
        let stream = self.decoder.create_stream();
        stream.accept_waveform(16_000, &self.samples);
        self.decoder.decode(&stream);
        let result = stream
            .get_result()
            .ok_or("GigaAM не вернул результат")?
            .text;
        self.samples.clear();
        self.heard = false;
        self.silence = 0;
        Ok(result.trim().to_owned())
    }
}
impl Recognizer for Giga {
    fn speech(&mut self, speech: bool) {
        self.speech = speech;
    }
    fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        self.samples
            .extend(samples.iter().map(|v| *v as f32 / 32768.0));
        if self.speech {
            self.heard = true;
            self.silence = 0;
        } else {
            self.silence += samples.len();
        }
        // Keep 250 ms of pre-roll, but never decode unbounded initial silence.
        if !self.heard && self.samples.len() > 4000 {
            self.samples.drain(..self.samples.len() - 4000);
        }
        if self.heard && (self.silence >= 11_200 || self.samples.len() >= 30 * 16_000) {
            self.ready = self.decode()?;
            return Ok(true);
        }
        Ok(false)
    }
    fn result(&mut self) -> String {
        std::mem::take(&mut self.ready)
    }
    fn partial(&mut self) -> String {
        String::new()
    }
    fn finish(&mut self) -> String {
        self.decode().unwrap_or_default()
    }
    fn finish_checked(&mut self) -> Result<String, String> {
        self.decode()
    }
    fn reset(&mut self) {
        self.samples.clear();
        self.heard = false;
        self.silence = 0;
        self.ready.clear();
        self.speech = false;
    }
}

pub struct Nemotron {
    // Drop the stream before its owning recognizer.
    stream: OnlineStream,
    decoder: OnlineRecognizer,
    received: bool,
}
impl Nemotron {
    fn open(root: &Path, threads: i32, provider: &str) -> Result<Self, String> {
        crate::native_runtime::ensure_offline()?;
        let mut config = OnlineRecognizerConfig::default();
        config.model_config.transducer.encoder = filename(root, "encoder.int8.onnx");
        config.model_config.transducer.decoder = filename(root, "decoder.int8.onnx");
        config.model_config.transducer.joiner = filename(root, "joiner.int8.onnx");
        config.model_config.tokens = filename(root, "tokens.txt");
        config.model_config.provider = Some(provider.into());
        config.model_config.num_threads = threads;
        config.decoding_method = Some("greedy_search".into());
        config.enable_endpoint = true;
        config.rule1_min_trailing_silence = 2.4;
        config.rule2_min_trailing_silence = 0.8;
        config.rule3_min_utterance_length = 30.0;
        let decoder =
            OnlineRecognizer::create(&config).ok_or("Nemotron не смог загрузить модель")?;
        let stream = decoder.create_stream();
        stream.set_option("language", "ru-RU");
        Ok(Self {
            stream,
            decoder,
            received: false,
        })
    }
    fn decode_ready(&self) {
        while self.decoder.is_ready(&self.stream) {
            self.decoder.decode(&self.stream);
        }
    }
    fn text(&self) -> String {
        self.decoder
            .get_result(&self.stream)
            .map_or_else(String::new, |r| r.text.trim().to_owned())
    }
}
impl Recognizer for Nemotron {
    fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        let samples = samples
            .iter()
            .map(|v| *v as f32 / 32768.0)
            .collect::<Vec<_>>();
        self.stream.accept_waveform(16_000, &samples);
        self.received = true;
        self.decode_ready();
        Ok(self.decoder.is_endpoint(&self.stream))
    }
    fn result(&mut self) -> String {
        let text = self.text();
        self.reset();
        text
    }
    fn partial(&mut self) -> String {
        self.text()
    }
    fn finish(&mut self) -> String {
        if !self.received {
            return String::new();
        }
        // Explicit input completion flushes the 560 ms encoder's trailing context.
        self.stream.accept_waveform(16_000, &[0.0; 9600]);
        self.stream.input_finished();
        self.decode_ready();
        let text = self.text();
        self.reset();
        text
    }
    fn reset(&mut self) {
        self.stream = self.decoder.create_stream();
        self.stream.set_option("language", "ru-RU");
        self.received = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_budget_is_bounded_and_leaves_headroom() {
        for (logical, expected) in [(1, 1), (2, 1), (4, 2), (8, 4), (12, 4), (64, 4)] {
            assert_eq!(cpu_threads(logical), expected);
        }
    }
}

#[cfg(test)]
mod release_tests {
    use super::*;
    #[repr(C)]
    struct Counters {
        cb: u32,
        faults: u32,
        values: [usize; 9],
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, size: u32) -> i32;
    }
    fn private_bytes() -> usize {
        let mut counters = Counters {
            cb: std::mem::size_of::<Counters>() as u32,
            faults: 0,
            values: [0; 9],
        };
        assert_ne!(
            unsafe {
                K32GetProcessMemoryInfo(-1, &mut counters, std::mem::size_of::<Counters>() as u32)
            },
            0
        );
        counters.values[8]
    }
    #[test]
    #[ignore = "Explicit native memory QA; requires installed catalog models"]
    fn native_models_release_memory_across_repeated_sessions() {
        let resources = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
        let local = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
            .join("dev.lotargo.voiceinput/models");
        for (id, engine, root, runtime) in [
            (
                "vosk-model-small-ru-0.22",
                "vosk",
                resources.join("models/vosk-model-small-ru-0.22"),
                resources.join("libvosk.dll"),
            ),
            (
                "gigaam-v3-e2e",
                "gigaam",
                local.join("gigaam-v3-e2e"),
                resources.join("models/CPU-Execution.conf"),
            ),
            (
                "nemotron-3-5-ru",
                "nemotron",
                local.join("nemotron-3-5-ru"),
                resources.join("models/CPU-Execution.conf"),
            ),
        ] {
            assert!(root.is_dir(), "Missing QA model {id}");
            for cycle in 1..=3 {
                let before = private_bytes();
                let selection = Selection {
                    path: root.clone(),
                    engine: engine.into(),
                    name: id.into(),
                };
                let mut model = open(&selection, &runtime).unwrap();
                let loaded = private_bytes();
                model.speech(true);
                model.accept(&[0; 1600]).unwrap();
                model.finish_checked().unwrap();
                drop(model);
                let released = private_bytes();
                println!("{id} cycle={cycle} private_bytes before={before} loaded={loaded} released={released}");
                assert!(
                    released < loaded,
                    "Model memory did not decrease on drop: {id}"
                );
            }
        }
    }
}
