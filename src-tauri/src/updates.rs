//! Explicit, signed GitHub Releases updates. No background network requests.
mod portable;

use crate::AppRuntime;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

pub use portable::run_helper_if_requested;

const REPOSITORY: &str = "https://github.com/Lotargo/voice-input";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Portable,
    Installed,
    Development,
}

fn mode() -> Mode {
    let Ok(exe) = std::env::current_exe() else {
        return Mode::Development;
    };
    let Some(root) = exe.parent() else {
        return Mode::Development;
    };
    if portable::is_portable(root) {
        Mode::Portable
    } else if root.join("uninstall.exe").is_file() {
        Mode::Installed
    } else {
        Mode::Development
    }
}

#[derive(Default)]
pub struct Updates {
    busy: AtomicBool,
    available: Mutex<Option<Update>>,
}

struct Busy<'a>(&'a AtomicBool);
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
impl Updates {
    fn acquire(&self) -> Result<Busy<'_>, String> {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .map_err(|_| "Обновление уже проверяется или загружается".to_owned())?;
        Ok(Busy(&self.busy))
    }
}

#[derive(Serialize)]
pub struct Info {
    version: &'static str,
    mode: Mode,
    supported: bool,
    last_result: Option<String>,
}

#[tauri::command]
pub fn update_info(app: AppHandle) -> Info {
    let mode = mode();
    Info {
        version: VERSION,
        mode,
        supported: mode != Mode::Development,
        last_result: portable::last_result(&app),
    }
}

#[derive(Serialize)]
pub struct Available {
    version: String,
    notes: String,
}

fn validate_release(update: &Update, mode: Mode) -> Result<(), String> {
    let version = semver::Version::parse(&update.version).map_err(|e| e.to_string())?;
    if version <= semver::Version::parse(VERSION).unwrap()
        || !version.pre.is_empty()
        || !version.build.is_empty()
    {
        return Err("Релиз должен иметь более новую стабильную версию".into());
    }
    let name = if mode == Mode::Portable {
        "Voice-Input-Portable-windows-x64.zip"
    } else {
        "Voice-Input-Setup-windows-x64.exe"
    };
    let expected = format!("{REPOSITORY}/releases/download/v{version}/{name}");
    if update.download_url.as_str() != expected {
        return Err("Пакет обновления не принадлежит ожидаемому релизу Voice Input".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn check_updates(
    app: AppHandle,
    state: State<'_, Updates>,
) -> Result<Option<Available>, String> {
    let _busy = state.acquire()?;
    *state.available.lock().unwrap() = None;
    let mode = mode();
    if mode == Mode::Development {
        return Err("Проверка доступна в установленной или portable-сборке из релиза".into());
    }
    let manifest = if mode == Mode::Portable {
        "latest-portable.json"
    } else {
        "latest.json"
    };
    let endpoint = format!("{REPOSITORY}/releases/latest/download/{manifest}");
    let updater = app
        .updater_builder()
        .endpoints(vec![endpoint.parse().map_err(|e| format!("{e}"))?])
        .map_err(|e| e.to_string())?
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| {
        format!("Не удалось проверить GitHub Releases. Проверьте соединение и повторите: {e}")
    })?;
    if let Some(mut update) = update {
        validate_release(&update, mode)?;
        update.timeout = Some(Duration::from_secs(600));
        let result = Available {
            version: update.version.clone(),
            notes: update.body.clone().unwrap_or_default(),
        };
        *state.available.lock().unwrap() = Some(update);
        Ok(Some(result))
    } else {
        Ok(None)
    }
}

#[derive(Clone, Serialize)]
struct Progress {
    phase: &'static str,
    downloaded: u64,
    total: Option<u64>,
}

struct RecordingGate(Arc<AppRuntime>, bool);
impl Drop for RecordingGate {
    fn drop(&mut self) {
        if !self.1 {
            self.0.recorder.lock().unwrap().updating = false;
        }
    }
}

#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    updates: State<'_, Updates>,
    runtime: State<'_, Arc<AppRuntime>>,
    version: String,
) -> Result<(), String> {
    let _busy = updates.acquire()?;
    let update = updates
        .available
        .lock()
        .unwrap()
        .clone()
        .filter(|update| update.version == version)
        .ok_or("Сначала проверьте доступную версию обновления")?;
    let mode = mode();
    if mode == Mode::Development {
        return Err("Установка обновления доступна только в релизной сборке".into());
    }
    validate_release(&update, mode)?;
    {
        // Serialize this with begin(), including starts from the global shortcut.
        let mut recording = runtime.recorder.lock().unwrap();
        if recording.recording {
            return Err("Перед обновлением завершите диктовку".into());
        }
        recording.updating = true;
    }
    let mut recording_gate = RecordingGate(runtime.inner().clone(), false);
    let mut downloaded = 0_u64;
    let mut emitted = Instant::now() - Duration::from_secs(1);
    let bytes = update
        .download(
            |chunk, total| {
                downloaded += chunk as u64;
                if emitted.elapsed() >= Duration::from_millis(150) {
                    let _ = app.emit(
                        "update-progress",
                        Progress {
                            phase: "downloading",
                            downloaded,
                            total,
                        },
                    );
                    emitted = Instant::now();
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Загрузка или проверка подписи не удалась: {e}"))?;
    let _ = app.emit(
        "update-progress",
        Progress {
            phase: "installing",
            downloaded,
            total: Some(bytes.len() as u64),
        },
    );
    if mode == Mode::Portable {
        let app_clone = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            portable::launch(&app_clone, &update.version, &update.signature, &bytes)
        })
        .await
        .map_err(|e| e.to_string())??;
        recording_gate.1 = true; // Keep the native hotkey blocked until the process exits.
        app.exit(0);
        Ok(())
    } else {
        // Tauri verifies again, invokes NSIS in passive mode, exits, and restarts the app.
        update
            .restart_after_install(true)
            .install(bytes)
            .map_err(|e| format!("Не удалось запустить установщик: {e}"))
    }
}
