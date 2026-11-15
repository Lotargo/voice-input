use super::*;
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::atomic::AtomicUsize,
};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "voice-model-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        assert!(!root.exists());
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn catalog(&self, spec: ModelSpec) -> Catalog {
        Catalog::with_specs(
            self.0.join("models"),
            self.0.join("config/models.json"),
            vec![],
            vec![spec],
        )
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn spec(url: String, data: &[u8]) -> ModelSpec {
    ModelSpec {
        id: "test".into(),
        name: "Test".into(),
        engine: "gigaam".into(),
        description: String::new(),
        license: "MIT".into(),
        source: String::new(),
        revision: "fixed-revision".into(),
        files: vec![ModelFile {
            name: "encoder.onnx".into(),
            url,
            size: data.len() as u64,
            sha256: format!("{:x}", Sha256::digest(data)),
        }],
    }
}
fn request(stream: &mut std::net::TcpStream) -> String {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        let mut one = [0];
        assert_eq!(stream.read(&mut one).unwrap(), 1);
        bytes.push(one[0]);
    }
    String::from_utf8(bytes).unwrap()
}
fn quiet() -> Notify {
    Arc::new(|_| {})
}
fn wait(catalog: &Catalog) {
    let started = Instant::now();
    while catalog.busy() {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn disconnect_and_recreated_catalog_resume_exact_saved_byte_without_redownload() {
    let temp = Temp::new();
    let data = vec![37; 32768];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let model = spec(
        format!("http://{}/model", listener.local_addr().unwrap()),
        &data,
    );
    let payload = data.clone();
    let server = std::thread::spawn(move || {
        let (mut first, _) = listener.accept().unwrap();
        assert!(!request(&mut first).to_lowercase().contains("range:"));
        write!(
            first,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .unwrap();
        first.write_all(&payload[..8192]).unwrap();
        std::thread::sleep(Duration::from_millis(150));
        drop(first);
        let (mut second, _) = listener.accept().unwrap();
        assert!(request(&mut second)
            .to_lowercase()
            .contains("range: bytes=8192-"));
        write!(second, "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes 8192-{}/{}\r\nConnection: close\r\n\r\n", payload.len() - 8192, payload.len() - 1, payload.len()).unwrap();
        second.write_all(&payload[8192..]).unwrap();
    });
    let first = temp.catalog(model.clone());
    assert!(first.download(&model, &AtomicU8::new(0), &quiet()).is_err());
    assert_eq!(first.transferred(&model), 8192);
    drop(first);
    let restored = temp.catalog(model.clone());
    assert_eq!(restored.view().entries[0].state, "paused");
    restored
        .download(&model, &AtomicU8::new(0), &quiet())
        .unwrap();
    server.join().unwrap();
    assert!(restored.installed(&model));
    assert_eq!(
        fs::read(restored.root.join("test/encoder.onnx")).unwrap(),
        data
    );
}
#[test]
fn ignored_range_preserves_prefix_and_does_not_install_or_truncate() {
    let temp = Temp::new();
    let data = vec![7; 100];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let model = spec(
        format!("http://{}/model", listener.local_addr().unwrap()),
        &data,
    );
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        request(&mut stream);
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\nConnection: close\r\n\r\n"
        );
    });
    let catalog = temp.catalog(model.clone());
    fs::create_dir_all(catalog.stage("test")).unwrap();
    let part = catalog.stage("test").join("encoder.onnx.part");
    fs::write(&part, &data[..30]).unwrap();
    assert!(catalog
        .download(&model, &AtomicU8::new(0), &quiet())
        .is_err());
    server.join().unwrap();
    assert_eq!(fs::read(part).unwrap(), &data[..30]);
    assert!(!catalog.installed(&model));
}
#[test]
fn completed_partial_is_verified_locally_and_tamper_never_becomes_installed() {
    let temp = Temp::new();
    let model = spec("http://127.0.0.1:1/no-network".into(), &[1, 2, 3]);
    let catalog = temp.catalog(model.clone());
    fs::create_dir_all(catalog.stage("test")).unwrap();
    let part = catalog.stage("test").join("encoder.onnx.part");
    fs::write(&part, [1, 2, 4]).unwrap();
    assert!(catalog
        .download(&model, &AtomicU8::new(0), &quiet())
        .unwrap_err()
        .contains("Контрольная сумма"));
    assert!(!catalog.installed(&model));
    fs::write(part, [1, 2, 3]).unwrap();
    catalog
        .download(&model, &AtomicU8::new(0), &quiet())
        .unwrap();
    assert!(catalog.installed(&model));
}
#[test]
fn persisted_primary_survives_restart_and_missing_model_never_falls_back() {
    let temp = Temp::new();
    let model = spec("unused".into(), &[1]);
    let catalog = temp.catalog(model.clone());
    fs::create_dir_all(catalog.stage("test")).unwrap();
    fs::write(catalog.stage("test").join("encoder.onnx.part"), [1]).unwrap();
    catalog
        .download(&model, &AtomicU8::new(0), &quiet())
        .unwrap();
    let path = catalog.root.join("test");
    catalog.select(path.clone()).unwrap();
    assert!(catalog.remove("test").unwrap_err().contains("основную"));
    drop(catalog);
    let restored = temp.catalog(model);
    assert_eq!(restored.selection().unwrap().path, path);
    fs::remove_file(path.join("encoder.onnx")).unwrap();
    assert!(restored.selection().is_err());
    assert!(restored.view().selected.is_some());
}
#[test]
fn cancel_removes_partial_files_only_and_arbitrary_ids_are_rejected() {
    let temp = Temp::new();
    let model = spec("unused".into(), &[1]);
    let catalog = temp.catalog(model);
    fs::create_dir_all(catalog.stage("test")).unwrap();
    fs::write(catalog.stage("test").join("encoder.onnx.part"), [1]).unwrap();
    fs::write(catalog.root.join("unrelated.txt"), "keep").unwrap();
    catalog.interrupt("test", true).unwrap();
    assert!(!catalog.stage("test").exists());
    assert!(catalog.root.join("unrelated.txt").exists());
    assert!(catalog.remove("../..").is_err());
    assert!(catalog.spec("bundled-0").is_err());
}
#[test]
fn pause_is_acknowledged_after_io_and_keeps_resume_data() {
    let temp = Temp::new();
    let data = vec![5; 1024 * 1024];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let model = spec(
        format!("http://{}/model", listener.local_addr().unwrap()),
        &data,
    );
    let (release, released) = std::sync::mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            data.len()
        )
        .unwrap();
        stream.write_all(&data[..65536]).unwrap();
        released.recv_timeout(Duration::from_secs(5)).unwrap();
        let _ = stream.write_all(&data[65536..]);
    });
    let catalog = Arc::new(temp.catalog(model.clone()));
    catalog.start("test", quiet()).unwrap();
    let started = Instant::now();
    while catalog.transferred(&model) == 0 {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    catalog.interrupt("test", false).unwrap();
    release.send(()).unwrap();
    wait(&catalog);
    server.join().unwrap();
    assert_eq!(catalog.view().entries[0].state, "paused");
    assert!(catalog.transferred(&model) > 0);
    assert!(!catalog.installed(&model));
}
#[test]
fn response_ranges_and_curated_paths_are_checked() {
    let mut headers = header::HeaderMap::new();
    headers.insert(header::CONTENT_RANGE, "bytes 9-99/100".parse().unwrap());
    assert!(validate_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 100).is_err());
    headers.insert(header::CONTENT_RANGE, "bytes 10-99/100".parse().unwrap());
    assert!(validate_response(StatusCode::PARTIAL_CONTENT, &headers, 10, 100).is_ok());
    for spec in specifications() {
        assert!(spec
            .id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-'));
        assert!(!spec.files.is_empty());
        for file in spec.files {
            assert!(!file.name.contains(['/', '\\']));
            assert_eq!(file.sha256.len(), 64);
            assert!(file.url.starts_with("https://huggingface.co/"));
            assert!(file.url.contains(&spec.revision));
        }
    }
}

#[test]
fn verification_is_visible_cancellable_and_resumes_without_network_after_restart() {
    let temp = Temp::new();
    let data = vec![9; 1024 * 1024];
    let mut model = spec("http://127.0.0.1:1/must-not-download".into(), &data);
    let mut second = model.files[0].clone();
    second.name = "decoder.onnx".into();
    model.files.push(second);
    let catalog = Arc::new(temp.catalog(model.clone()));
    fs::create_dir_all(catalog.stage("test")).unwrap();
    for file in &model.files {
        fs::write(
            catalog.stage("test").join(format!("{}.part", file.name)),
            &data,
        )
        .unwrap();
    }
    assert_eq!(catalog.view().entries[0].state, "verification_pending");
    assert!(catalog.select(catalog.root.join("test")).is_err());
    let weak = Arc::downgrade(&catalog);
    let observed = Arc::new(AtomicUsize::new(0));
    let checked = observed.clone();
    let expected_size = model.size();
    catalog
        .start(
            "test",
            Arc::new(move |view| {
                let entry = &view.entries[0];
                if entry.state == "verifying" {
                    assert_eq!(entry.downloaded, expected_size);
                    assert!(!entry.installed);
                    let catalog = weak.upgrade().unwrap();
                    assert!(catalog.select(catalog.root.join("test")).is_err());
                    if entry.verified > 0 {
                        checked.fetch_add(1, Ordering::Relaxed);
                        catalog.interrupt("test", false).unwrap();
                    }
                }
            }),
        )
        .unwrap();
    wait(&catalog);
    assert!(observed.load(Ordering::Relaxed) > 0);
    assert_eq!(catalog.view().entries[0].state, "paused");
    assert_eq!(catalog.transferred(&model), model.size());
    assert!(!catalog.installed(&model));
    drop(catalog);
    let restored = Arc::new(temp.catalog(model.clone()));
    assert_eq!(restored.view().entries[0].state, "verification_pending");
    restored.start("test", quiet()).unwrap();
    wait(&restored);
    assert!(restored.installed(&model));
    // A delayed pause message cannot hide the committed, verified model.
    restored
        .transfers
        .lock()
        .unwrap()
        .messages
        .insert("test".into(), ("paused".into(), "stale".into()));
    let entry = &restored.view().entries[0];
    assert_eq!(entry.state, "installed");
    assert_eq!(entry.verified, model.size());
    assert!(entry.installed);
    restored.select(restored.root.join("test")).unwrap();
}

#[test]
fn all_files_download_before_verification_and_availability_is_committed_last() {
    let temp = Temp::new();
    let data = vec![13; 65536];
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut model = spec(
        format!("http://{}/file", listener.local_addr().unwrap()),
        &data,
    );
    let mut second = model.files[0].clone();
    second.name = "decoder.onnx".into();
    model.files.push(second);
    let server = std::thread::spawn(move || {
        for _ in 0..2 {
            let (mut stream, _) = listener.accept().unwrap();
            request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                data.len()
            )
            .unwrap();
            stream.write_all(&data).unwrap();
        }
    });
    let catalog = Arc::new(temp.catalog(model.clone()));
    let states = Arc::new(Mutex::new(Vec::new()));
    let events = states.clone();
    let total = model.size();
    catalog
        .start(
            "test",
            Arc::new(move |view| {
                let entry = &view.entries[0];
                if entry.state == "verifying" {
                    assert_eq!(entry.downloaded, total);
                    assert!(!entry.installed);
                }
                if entry.installed {
                    assert_eq!(entry.verified, total);
                }
                events.lock().unwrap().push(entry.state.clone());
            }),
        )
        .unwrap();
    wait(&catalog);
    server.join().unwrap();
    let started = Instant::now();
    while !states.lock().unwrap().iter().any(|s| s == "installed") {
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(5));
    }
    let states = states.lock().unwrap();
    assert!(states.iter().any(|s| s == "downloading"));
    assert!(states.iter().any(|s| s == "verifying"));
    assert_eq!(states.last().unwrap(), "installed");
}
