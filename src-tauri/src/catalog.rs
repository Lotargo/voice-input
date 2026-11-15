//! Curated, explicitly requested downloads. No network access during discovery/startup.
pub mod commands;
use reqwest::{header, StatusCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, AtomicU8, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelFile {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub sha256: String,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ModelSpec {
    pub id: String,
    pub name: String,
    pub engine: String,
    pub description: String,
    pub license: String,
    pub source: String,
    pub revision: String,
    pub files: Vec<ModelFile>,
}
impl ModelSpec {
    pub fn size(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}
pub fn specifications() -> Vec<ModelSpec> {
    serde_json::from_str(include_str!("../model-catalog.json")).expect("validated model catalog")
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Selection {
    pub path: PathBuf,
    pub engine: String,
    pub name: String,
}
#[derive(Clone, Serialize)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub license: String,
    pub engine: String,
    pub size: u64,
    pub downloaded: u64,
    pub verified: u64,
    pub state: String,
    pub message: String,
    pub installed: bool,
    pub bundled: bool,
    pub primary: bool,
    pub path: String,
}
#[derive(Clone, Serialize)]
pub struct View {
    pub version: u64,
    pub entries: Vec<Entry>,
    pub selected: Option<Selection>,
    pub error: Option<String>,
    pub active: bool,
    pub selection_saved: bool,
}
#[derive(Default)]
struct Transfers {
    active: Option<(String, Arc<AtomicU8>)>,
    messages: BTreeMap<String, (String, String)>,
    verified: BTreeMap<String, u64>,
}
pub struct Catalog {
    root: PathBuf,
    settings_path: PathBuf,
    specs: Vec<ModelSpec>,
    builtin: Vec<Selection>,
    selected: Mutex<Option<Selection>>,
    load_error: Mutex<Option<String>>,
    transfers: Mutex<Transfers>,
    version: AtomicU64,
}
type Notify = Arc<dyn Fn(View) + Send + Sync>;

pub fn atomic_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("Нет папки настроек")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let mut file = File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(tmp, path).map_err(|e| e.to_string())
}

fn ordinary(path: &Path) -> bool {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return false;
    };
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    !meta.file_type().is_symlink()
}
fn file_size(path: &Path) -> u64 {
    if ordinary(path) {
        fs::metadata(path)
            .ok()
            .filter(|m| m.is_file())
            .map_or(0, |m| m.len())
    } else {
        0
    }
}
fn sha256(
    path: &Path,
    signal: &AtomicU8,
    mut progress: impl FnMut(u64),
) -> Result<Option<String>, String> {
    if !ordinary(path) {
        return Err("Недопустимый файл модели".into());
    }
    let mut reader = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 128 * 1024];
    let mut checked = 0;
    loop {
        if signal.load(Ordering::Acquire) != 0 {
            return Ok(None);
        }
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
        checked += count as u64;
        progress(checked);
    }
    Ok(Some(format!("{:x}", hash.finalize())))
}

impl Catalog {
    pub fn load(root: PathBuf, settings_path: PathBuf, builtin: Vec<Selection>) -> Self {
        Self::with_specs(root, settings_path, builtin, specifications())
    }
    fn with_specs(
        root: PathBuf,
        settings_path: PathBuf,
        builtin: Vec<Selection>,
        specs: Vec<ModelSpec>,
    ) -> Self {
        let (selected, error) = match fs::read(&settings_path) {
            Ok(bytes) => match serde_json::from_slice::<Selection>(&bytes) {
                Ok(value) => (Some(value), None),
                Err(e) => (
                    None,
                    Some(format!(
                        "Настройки основной модели не прочитаны: {e}. Выберите модель заново."
                    )),
                ),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (builtin.first().cloned(), None),
            Err(e) => (
                None,
                Some(format!("Настройки основной модели недоступны: {e}")),
            ),
        };
        Self {
            root,
            settings_path,
            specs,
            builtin,
            selected: Mutex::new(selected),
            load_error: Mutex::new(error),
            transfers: Mutex::new(Transfers::default()),
            version: AtomicU64::new(0),
        }
    }
    fn spec(&self, id: &str) -> Result<ModelSpec, String> {
        self.specs
            .iter()
            .find(|m| m.id == id)
            .cloned()
            .ok_or_else(|| "Модель отсутствует в каталоге".into())
    }
    fn stage(&self, id: &str) -> PathBuf {
        self.root.join(".downloads").join(id)
    }
    fn installed(&self, spec: &ModelSpec) -> bool {
        let root = self.root.join(&spec.id);
        ordinary(&root)
            && fs::read_to_string(root.join("complete.json"))
                .ok()
                .as_deref()
                == Some(&spec.revision)
            && spec.files.iter().all(|file| {
                ordinary(&root.join(&file.name)) && file_size(&root.join(&file.name)) == file.size
            })
    }
    fn transferred(&self, spec: &ModelSpec) -> u64 {
        let stage = self.stage(&spec.id);
        spec.files
            .iter()
            .map(|file| {
                file_size(&stage.join(&file.name))
                    .max(file_size(&stage.join(format!("{}.part", file.name))))
                    .min(file.size)
            })
            .sum()
    }
    fn safe_directory(&self, directory: &Path) -> Result<(), String> {
        // All callers use a compiled catalog ID, never a UI-supplied filesystem path.
        for part in directory
            .strip_prefix(&self.root)
            .map_err(|_| "Папка вне каталога моделей")?
            .components()
        {
            if !matches!(part, std::path::Component::Normal(_)) {
                return Err("Недопустимый путь модели".into());
            }
        }
        let mut current = self.root.clone();
        if current.exists() && !ordinary(&current) {
            return Err("Папка моделей не должна быть ссылкой".into());
        }
        for part in directory.strip_prefix(&self.root).unwrap().components() {
            current.push(part);
            if current.exists() && !ordinary(&current) {
                return Err("Папка загрузки не должна быть ссылкой".into());
            }
        }
        Ok(())
    }
    pub fn view(&self) -> View {
        let selected = self.selected.lock().unwrap().clone();
        let transfer = self.transfers.lock().unwrap();
        let mut entries = self
            .builtin
            .iter()
            .enumerate()
            .map(|(i, model)| Entry {
                id: format!("bundled-{i}"),
                name: model.name.clone(),
                description:
                    "Базовая русская модель. Входит в приложение, работает на CPU без пунктуации."
                        .into(),
                license: "Apache-2.0".into(),
                engine: "vosk".into(),
                size: 0,
                downloaded: 0,
                verified: 0,
                state: "installed".into(),
                message: String::new(),
                installed: true,
                bundled: true,
                primary: selected.as_ref().is_some_and(|s| s.path == model.path),
                path: model.path.to_string_lossy().into_owned(),
            })
            .collect::<Vec<_>>();
        for spec in &self.specs {
            let active = transfer
                .active
                .as_ref()
                .is_some_and(|(id, _)| id == &spec.id);
            let committed = self.installed(spec);
            let installed = committed && !active;
            let downloaded = if committed {
                spec.size()
            } else {
                self.transferred(spec)
            };
            let default = if installed {
                "installed"
            } else if self.root.join(&spec.id).exists() {
                "damaged"
            } else if downloaded == spec.size() {
                "verification_pending"
            } else if downloaded > 0 {
                "paused"
            } else {
                "available"
            };
            let (state, message) = if installed {
                ("installed".into(), "Модель загружена и проверена".into())
            } else if active {
                transfer
                    .messages
                    .get(&spec.id)
                    .cloned()
                    .unwrap_or_else(|| (default.into(), String::new()))
            } else {
                transfer
                    .messages
                    .get(&spec.id)
                    .filter(|(state, _)| state == "paused")
                    .cloned()
                    .unwrap_or_else(|| {
                        (
                            default.into(),
                            if default == "verification_pending" {
                                "Все файлы скачаны. Нажмите «Проверить», чтобы завершить установку."
                                    .into()
                            } else {
                                String::new()
                            },
                        )
                    })
            };
            entries.push(Entry {
                id: spec.id.clone(),
                name: spec.name.clone(),
                description: spec.description.clone(),
                license: spec.license.clone(),
                engine: spec.engine.clone(),
                size: spec.size(),
                downloaded,
                verified: if installed {
                    spec.size()
                } else {
                    transfer
                        .verified
                        .get(&spec.id)
                        .copied()
                        .unwrap_or(0)
                        .min(spec.size())
                },
                state,
                message,
                installed,
                bundled: false,
                primary: selected
                    .as_ref()
                    .is_some_and(|s| s.path == self.root.join(&spec.id)),
                path: self.root.join(&spec.id).to_string_lossy().into_owned(),
            });
        }
        // A custom Vosk path remains available and is never owned/deleted by the catalog.
        if let Some(model) = &selected {
            if !entries.iter().any(|e| Path::new(&e.path) == model.path) {
                entries.push(Entry {
                    id: "custom".into(),
                    name: model.name.clone(),
                    description: "Пользовательская папка. Управление файлами остаётся за вами."
                        .into(),
                    license: String::new(),
                    engine: model.engine.clone(),
                    size: 0,
                    downloaded: 0,
                    verified: 0,
                    state: if model.path.exists() {
                        "installed"
                    } else {
                        "missing"
                    }
                    .into(),
                    message: String::new(),
                    installed: model.path.exists(),
                    bundled: true,
                    primary: true,
                    path: model.path.to_string_lossy().into_owned(),
                });
            }
        }
        View {
            version: self.version.fetch_add(1, Ordering::Relaxed),
            entries,
            selected,
            error: self.load_error.lock().unwrap().clone(),
            active: transfer.active.is_some(),
            selection_saved: self.settings_path.exists(),
        }
    }
    pub fn selection(&self) -> Result<Selection, String> {
        if let Some(error) = self.load_error.lock().unwrap().clone() {
            return Err(error);
        }
        let selected = self
            .selected
            .lock()
            .unwrap()
            .clone()
            .ok_or("Выберите основную модель в каталоге")?;
        if selected.engine == "vosk" {
            if !crate::models::looks_like_model(&selected.path) {
                return Err(
                    "Основная модель недоступна. Выберите или загрузите модель в каталоге.".into(),
                );
            }
        } else {
            let spec = self
                .specs
                .iter()
                .find(|m| self.root.join(&m.id) == selected.path && m.engine == selected.engine)
                .ok_or("Неизвестная основная модель")?;
            if !self.installed(spec) {
                return Err("Основная модель не установлена. Продолжите загрузку или выберите другую модель.".into());
            }
        }
        Ok(selected)
    }
    pub fn select(&self, path: PathBuf) -> Result<(), String> {
        if self
            .transfers
            .lock()
            .unwrap()
            .active
            .as_ref()
            .is_some_and(|(id, _)| self.root.join(id) == path)
        {
            return Err("Дождитесь завершения проверки модели".into());
        }
        let selection =
            if let Some(spec) = self.specs.iter().find(|s| self.root.join(&s.id) == path) {
                if !self.installed(spec) {
                    return Err("Сначала завершите загрузку модели".into());
                }
                Selection {
                    path,
                    engine: spec.engine.clone(),
                    name: spec.name.clone(),
                }
            } else {
                if !crate::models::looks_like_model(&path) {
                    return Err("Папка Vosk должна содержать am/ и conf/".into());
                }
                Selection {
                    name: self
                        .builtin
                        .iter()
                        .find(|s| s.path == path)
                        .map(|s| s.name.clone())
                        .unwrap_or_else(|| "Пользовательская Vosk".into()),
                    path,
                    engine: "vosk".into(),
                }
            };
        let mut selected = self.selected.lock().unwrap();
        atomic_json(&self.settings_path, &selection)?;
        *selected = Some(selection);
        *self.load_error.lock().unwrap() = None;
        Ok(())
    }
    pub fn migrate_selection(&self, path: PathBuf) -> Result<(), String> {
        if self.settings_path.exists() || !path.is_absolute() {
            return Ok(());
        }
        let mut selected = self.selected.lock().unwrap();
        let selection = Selection {
            name: self
                .builtin
                .iter()
                .find(|s| s.path == path)
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "Пользовательская Vosk".into()),
            path,
            engine: "vosk".into(),
        };
        atomic_json(&self.settings_path, &selection)?;
        *selected = Some(selection);
        Ok(())
    }
    pub fn busy(&self) -> bool {
        self.transfers.lock().unwrap().active.is_some()
    }
    pub fn start(self: &Arc<Self>, id: &str, notify: Notify) -> Result<(), String> {
        let spec = self.spec(id)?;
        if self.installed(&spec) {
            return Err("Модель уже установлена".into());
        }
        if self.root.join(id).exists() {
            return Err("Файлы модели повреждены или неполны. Удалите её через каталог перед повторной загрузкой.".into());
        }
        self.safe_directory(&self.stage(id))?;
        let signal = Arc::new(AtomicU8::new(0));
        {
            let mut transfer = self.transfers.lock().unwrap();
            if transfer.active.is_some() {
                return Err("Сначала приостановите текущую загрузку".into());
            }
            transfer.active = Some((id.into(), signal.clone()));
            transfer.verified.insert(id.into(), 0);
            transfer.messages.insert(
                id.into(),
                (
                    if self.transferred(&spec) == spec.size() {
                        "verifying"
                    } else {
                        "downloading"
                    }
                    .into(),
                    String::new(),
                ),
            );
        }
        notify(self.view());
        let catalog = self.clone();
        std::thread::spawn(move || {
            let result = catalog.download(&spec, &signal, &notify);
            let mode = signal.load(Ordering::Acquire);
            let (state, message) = if catalog.installed(&spec) {
                ("installed", "Модель загружена и проверена".into())
            } else if mode == 2 {
                match catalog.remove_stage(&spec.id) {
                    Ok(()) => ("available", String::new()),
                    Err(e) => ("paused", e),
                }
            } else if mode == 1 {
                (
                    "paused",
                    "Загрузка приостановлена. Скачанная часть сохранена.".into(),
                )
            } else {
                match result {
                    Ok(()) => (
                        "paused",
                        "Файлы сохранены. Продолжите проверку модели.".into(),
                    ),
                    Err(error) => (
                        "paused",
                        format!("{error}. Скачанная часть сохранена; нажмите «Продолжить»."),
                    ),
                }
            };
            {
                let mut transfer = catalog.transfers.lock().unwrap();
                transfer.active = None;
                transfer
                    .messages
                    .insert(spec.id.clone(), (state.into(), message));
            }
            notify(catalog.view());
        });
        Ok(())
    }
    pub fn interrupt(&self, id: &str, cancel: bool) -> Result<(), String> {
        self.spec(id)?;
        let mut transfer = self.transfers.lock().unwrap();
        if let Some((active, signal)) = &transfer.active {
            if active != id {
                return Err("Эта модель сейчас не загружается".into());
            }
            signal.fetch_max(if cancel { 2 } else { 1 }, Ordering::AcqRel);
            transfer.messages.insert(
                id.into(),
                (
                    if cancel { "cancelling" } else { "pausing" }.into(),
                    "Завершаем текущую операцию…".into(),
                ),
            );
        } else if cancel {
            self.remove_stage(id)?;
            transfer.messages.remove(id);
            transfer.verified.remove(id);
        }
        Ok(())
    }
    fn remove_stage(&self, id: &str) -> Result<(), String> {
        let stage = self.stage(id);
        self.safe_directory(&stage)?;
        if stage.exists() {
            fs::remove_dir_all(stage).map_err(|e| e.to_string())?;
        }
        Ok(())
    }
    pub fn remove(&self, id: &str) -> Result<(), String> {
        let spec = self.spec(id)?;
        let transfer = self.transfers.lock().unwrap();
        if transfer.active.is_some() {
            return Err("Дождитесь остановки загрузки".into());
        }
        if self
            .selected
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|s| s.path == self.root.join(id))
        {
            return Err("Сначала назначьте другую основную модель".into());
        }
        let target = self.root.join(&spec.id);
        self.safe_directory(&target)?;
        if target.exists() {
            fs::remove_dir_all(target).map_err(|e| e.to_string())?;
        }
        self.remove_stage(id)?;
        drop(transfer);
        self.transfers.lock().unwrap().messages.remove(id);
        self.transfers.lock().unwrap().verified.remove(id);
        Ok(())
    }
    fn download(&self, spec: &ModelSpec, signal: &AtomicU8, notify: &Notify) -> Result<(), String> {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?
            .block_on(self.download_async(spec, signal, notify))
    }
    async fn download_async(
        &self,
        spec: &ModelSpec,
        signal: &AtomicU8,
        notify: &Notify,
    ) -> Result<(), String> {
        let stage = self.stage(&spec.id);
        self.safe_directory(&stage)?;
        fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
        let builder = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(Duration::from_secs(20))
            .user_agent(concat!("VoiceInput/", env!("CARGO_PKG_VERSION")));
        #[cfg(test)]
        let builder = builder.no_proxy();
        // No whole-file deadline: large weights may take hours on a slow connection.
        let client = builder.build().map_err(|e| e.to_string())?;
        for file in &spec.files {
            if signal.load(Ordering::Acquire) != 0 {
                return Ok(());
            }
            let complete = stage.join(&file.name);
            let part = stage.join(format!("{}.part", file.name));
            if complete.exists() {
                if file_size(&complete) == file.size {
                    continue;
                }
                fs::remove_file(&complete).map_err(|e| e.to_string())?;
            }
            if part.exists() && !ordinary(&part) {
                return Err("Недопустимый файл загрузки".into());
            }
            let offset = file_size(&part);
            if offset > file.size {
                return Err("Размер сохранённой части превышает размер файла".into());
            }
            if offset < file.size {
                let mut request = client
                    .get(&file.url)
                    .header(header::ACCEPT_ENCODING, "identity");
                if offset > 0 {
                    request = request.header(header::RANGE, format!("bytes={offset}-"));
                }
                let Some(mut response) = network(signal, request.send()).await? else {
                    return Ok(());
                };
                validate_response(response.status(), response.headers(), offset, file.size)?;
                let mut output = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&part)
                    .map_err(|e| e.to_string())?;
                let mut count = offset;
                let mut last = Instant::now();
                loop {
                    if signal.load(Ordering::Acquire) != 0 {
                        output.sync_all().map_err(|e| e.to_string())?;
                        return Ok(());
                    }
                    let Some(chunk) = network(signal, response.chunk()).await? else {
                        output.sync_all().map_err(|e| e.to_string())?;
                        return Ok(());
                    };
                    let Some(chunk) = chunk else {
                        break;
                    };
                    if count + chunk.len() as u64 > file.size {
                        return Err("Сервер прислал файл другого размера".into());
                    }
                    output
                        .write_all(&chunk)
                        .map_err(|e| format!("Не удалось записать загрузку: {e}"))?;
                    count += chunk.len() as u64;
                    if last.elapsed() >= Duration::from_millis(250) {
                        output.sync_data().map_err(|e| e.to_string())?;
                        notify(self.view());
                        last = Instant::now();
                    }
                }
                output.sync_all().map_err(|e| e.to_string())?;
                drop(output);
                if count != file.size {
                    return Err("Соединение закрылось до конца файла".into());
                }
            }
            notify(self.view());
        }
        if signal.load(Ordering::Acquire) != 0 {
            return Ok(());
        }
        {
            let mut transfer = self.transfers.lock().unwrap();
            transfer.messages.insert(
                spec.id.clone(),
                (
                    "verifying".into(),
                    "Все файлы скачаны. Проверяем их целостность; модель пока недоступна.".into(),
                ),
            );
            transfer.verified.insert(spec.id.clone(), 0);
        }
        notify(self.view());
        let mut verified = 0;
        for file in &spec.files {
            let complete = stage.join(&file.name);
            let part = stage.join(format!("{}.part", file.name));
            let source = if complete.exists() { &complete } else { &part };
            let mut last = Instant::now();
            let digest = sha256(source, signal, |count| {
                if last.elapsed() >= Duration::from_millis(100) || count == file.size {
                    self.transfers
                        .lock()
                        .unwrap()
                        .verified
                        .insert(spec.id.clone(), verified + count);
                    notify(self.view());
                    last = Instant::now();
                }
            })?;
            let Some(digest) = digest else {
                return Ok(());
            };
            if digest != file.sha256 {
                fs::remove_file(source).map_err(|e| e.to_string())?;
                return Err(format!(
                    "Контрольная сумма {} не совпала; остальные файлы сохранены",
                    file.name
                ));
            }
            verified += file.size;
            if source == &part {
                fs::rename(part, complete).map_err(|e| e.to_string())?;
            }
        }
        if signal.load(Ordering::Acquire) != 0 {
            return Ok(());
        }
        let mut marker = File::create(stage.join("complete.json")).map_err(|e| e.to_string())?;
        marker
            .write_all(spec.revision.as_bytes())
            .map_err(|e| e.to_string())?;
        marker.sync_all().map_err(|e| e.to_string())?;
        drop(marker);
        let target = self.root.join(&spec.id);
        self.safe_directory(&target)?;
        if target.exists() {
            return Err(
                "Папка модели уже существует; удалите повреждённую модель через каталог".into(),
            );
        }
        let _commit = self.transfers.lock().unwrap();
        if signal.load(Ordering::Acquire) != 0 {
            return Ok(());
        }
        fs::rename(stage, target).map_err(|e| e.to_string())
    }
}

async fn network<T>(
    signal: &AtomicU8,
    operation: impl std::future::Future<Output = Result<T, reqwest::Error>>,
) -> Result<Option<T>, String> {
    tokio::pin!(operation);
    loop {
        tokio::select! {
            result = &mut operation => return result.map(Some).map_err(|_| "Соединение прервано или временно недоступно".into()),
            _ = tokio::time::sleep(Duration::from_millis(100)) => if signal.load(Ordering::Acquire) != 0 { return Ok(None); },
        }
    }
}

fn validate_response(
    status: StatusCode,
    headers: &header::HeaderMap,
    offset: u64,
    size: u64,
) -> Result<(), String> {
    if status == StatusCode::PARTIAL_CONTENT {
        let range = headers
            .get(header::CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            .ok_or("Сервер не подтвердил диапазон загрузки")?;
        let (interval, total) = range
            .strip_prefix("bytes ")
            .and_then(|r| r.split_once('/'))
            .ok_or("Некорректный диапазон загрузки")?;
        let (start, end) = interval
            .split_once('-')
            .ok_or("Некорректный диапазон загрузки")?;
        if start.parse::<u64>().ok() != Some(offset)
            || total.parse::<u64>().ok() != Some(size)
            || end.parse::<u64>().ok() != Some(size - 1)
        {
            return Err("Сервер прислал другой диапазон; сохранённая часть не изменена".into());
        }
    } else if status != StatusCode::OK || offset > 0 {
        return Err(if status == StatusCode::OK {
            "Сервер не поддержал продолжение; сохранённая часть не изменена".into()
        } else {
            format!("Сервер временно недоступен (HTTP {})", status.as_u16())
        });
    }
    if headers
        .get(header::CONTENT_ENCODING)
        .and_then(|h| h.to_str().ok())
        .is_some_and(|v| v != "identity")
    {
        return Err("Сервер изменил кодировку файла".into());
    }
    if let Some(length) = headers.get(header::CONTENT_LENGTH) {
        if length.to_str().ok().and_then(|v| v.parse::<u64>().ok()) != Some(size - offset) {
            return Err("Сервер сообщил другой размер файла".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
