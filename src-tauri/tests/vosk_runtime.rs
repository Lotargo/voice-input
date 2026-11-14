#[allow(dead_code)]
#[path = "../src/vosk.rs"]
mod vosk;

fn resources() -> std::path::PathBuf {
    std::env::var_os("VOICE_INPUT_SMOKE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources"))
}

#[test]
fn bundled_vosk_loads_russian_model_and_processes_silence() {
    let resources = resources();
    let mut recognizer = vosk::Vosk::open(
        &resources.join("libvosk.dll"),
        &resources.join("models/vosk-model-small-ru-0.22"),
    )
    .expect("Run scripts/prepare-assets.ps1 before the native runtime smoke test");
    for _ in 0..10 {
        if recognizer.accept(&[0; 1600]).unwrap() {
            assert!(recognizer.result().is_empty());
        }
    }
    assert!(recognizer.finish().is_empty());
}

#[test]
fn canonical_windows_model_path_loads() {
    let resources = resources();
    let model = resources
        .join("models/vosk-model-small-ru-0.22")
        .canonicalize()
        .unwrap();
    let dll = resources.join("libvosk.dll").canonicalize().unwrap();
    eprintln!("Canonical model path: {}", model.display());
    vosk::Vosk::open(&dll, &model).unwrap_or_else(|error| panic!("{}: {error}", model.display()));
}
