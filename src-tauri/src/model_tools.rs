//! Explicit local QA entry points, excluded from normal builds.
use crate::{
    audio::{self, pipeline::Pipeline},
    catalog::{Catalog, Selection},
    recognizers,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
pub fn download(id: &str, root: &Path, crash_after: Option<u64>) -> Result<(), String> {
    let catalog = Arc::new(Catalog::load(
        root.join("models"),
        root.join("models.json"),
        vec![],
    ));
    let model_id = id.to_owned();
    catalog.start(
        id,
        Arc::new(move |view| {
            if let Some(entry) = view.entries.iter().find(|m| m.id == model_id) {
                println!(
                    "{} {} / {} {}",
                    entry.id, entry.downloaded, entry.size, entry.state
                );
                if crash_after
                    .is_some_and(|bytes| entry.downloaded >= bytes && entry.state == "downloading")
                {
                    std::process::exit(88);
                }
            }
        }),
    )?;
    while catalog.busy() {
        std::thread::sleep(Duration::from_millis(50));
    }
    let view = catalog.view();
    let entry = view.entries.iter().find(|m| m.id == id).unwrap();
    if !entry.installed {
        return Err(entry.message.clone());
    }
    Ok(())
}
pub fn transcribe(id: &str, root: &Path, file: &Path) -> Result<serde_json::Value, String> {
    transcribe_with_threads(id, root, file, None, false)
}
pub fn transcribe_with_threads(
    id: &str,
    root: &Path,
    file: &Path,
    threads: Option<i32>,
    no_spin: bool,
) -> Result<serde_json::Value, String> {
    let spec = crate::catalog::specifications()
        .into_iter()
        .find(|s| s.id == id)
        .ok_or("Неизвестная модель")?;
    let selection = Selection {
        path: root.join("models").join(id),
        engine: spec.engine,
        name: spec.name,
    };
    let (samples, rate) = if file.extension().is_some_and(|e| e == "pcm") {
        let bytes = std::fs::read(file).map_err(|e| e.to_string())?;
        (
            bytes
                .chunks_exact(2)
                .map(|s| i16::from_le_bytes([s[0], s[1]]))
                .collect::<Vec<_>>(),
            16_000,
        )
    } else {
        let wave =
            sherpa_onnx::Wave::read(&file.to_string_lossy()).ok_or("Не удалось открыть WAV")?;
        (
            wave.samples()
                .iter()
                .map(|v| (v.clamp(-1.0, 1.0) * 32767.0) as i16)
                .collect(),
            wave.sample_rate(),
        )
    };
    if rate != 16_000 {
        return Err("Для QA нужен PCM/WAV mono 16 kHz".into());
    }
    let started = Instant::now();
    let recognizer = match (threads, no_spin) {
        (Some(threads), true) => recognizers::open_configured(
            &selection,
            Path::new(""),
            threads,
            concat!(
                "cpu:",
                env!("CARGO_MANIFEST_DIR"),
                "/resources/models/CPU-Execution.conf"
            ),
        )?,
        (Some(threads), false) => {
            recognizers::open_with_threads(&selection, Path::new(""), threads)?
        }
        (None, _) => {
            let runtime_file = crate::models::runtime_from_roots(
                &[env!("CARGO_MANIFEST_DIR").into()],
                &selection.engine,
            )?;
            recognizers::open(&selection, &runtime_file)?
        }
    };
    let loaded = started.elapsed();
    let mut pipeline = Pipeline::new(recognizer);
    let settings = audio::SplitSettings::new();
    let mut finals = Vec::new();
    let mut partials = 0;
    let mut receive = |event| match event {
        audio::AudioEvent::Final(text) => finals.push(text),
        audio::AudioEvent::Partial(text) if !text.is_empty() => partials += 1,
        _ => {}
    };
    for frame in samples.chunks(1600) {
        pipeline.push(frame, &settings, &mut receive)?;
    }
    pipeline.finish(&mut receive)?;
    let decoded = started.elapsed() - loaded;
    Ok(
        serde_json::json!({ "model": id, "automatic": threads.is_none(), "threads": threads.unwrap_or_else(|| recognizers::cpu_threads(std::thread::available_parallelism().map_or(2, |n| n.get()))), "no_spin": no_spin || threads.is_none(), "audio_seconds": samples.len() as f64 / 16000.0, "load_seconds": loaded.as_secs_f64(), "decode_seconds": decoded.as_secs_f64(), "text": finals.join("\n"), "partial_updates": partials }),
    )
}
