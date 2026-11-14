//! Persistent global shortcut. Failed registration never removes the working binding.
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Mutex,
    },
};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

#[cfg(test)]
mod tests;

pub const DEFAULT_SHORTCUT: &str = "Ctrl+Alt+Space";

#[derive(Clone, Debug, Serialize)]
pub struct HotkeyView {
    pub shortcut: String,
    pub default_shortcut: String,
    pub active: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize)]
struct Config {
    shortcut: String,
}

struct Binding {
    shortcut: Shortcut,
    active: bool,
    error: Option<String>,
}

pub struct Hotkeys {
    binding: Mutex<Binding>,
    path: Result<PathBuf, String>,
    active_id: AtomicU32,
    active: AtomicBool,
    capturing: AtomicBool,
    pressed: AtomicBool,
}

pub enum Action {
    Ignore,
    Toggle,
    Capture,
}

trait Registrar {
    fn register(&self, shortcut: Shortcut) -> Result<(), String>;
    fn unregister(&self, shortcut: Shortcut) -> Result<(), String>;
}

struct NativeRegistrar<'a>(&'a AppHandle);

impl Registrar for NativeRegistrar<'_> {
    fn register(&self, shortcut: Shortcut) -> Result<(), String> {
        let manager = self.0.global_shortcut();
        if manager.is_registered(shortcut) {
            return Ok(());
        }
        manager.register(shortcut).map_err(|e| e.to_string())
    }

    fn unregister(&self, shortcut: Shortcut) -> Result<(), String> {
        self.0
            .global_shortcut()
            .unregister(shortcut)
            .map_err(|e| e.to_string())
    }
}

pub fn parse(value: &str) -> Result<Shortcut, String> {
    if value.len() > 96 {
        return Err("Слишком длинное сочетание клавиш".into());
    }
    let normalized = value
        .split('+')
        .map(|part| match part.trim().to_ascii_lowercase().as_str() {
            "win" | "windows" | "meta" => "Super".to_owned(),
            _ => part.trim().to_owned(),
        })
        .collect::<Vec<_>>()
        .join("+");
    let shortcut: Shortcut = normalized
        .parse()
        .map_err(|_| "Укажите модификаторы и одну клавишу, например Ctrl+Shift+F9".to_owned())?;
    let function_key = matches!(
        shortcut.key,
        Code::F1
            | Code::F2
            | Code::F3
            | Code::F4
            | Code::F5
            | Code::F6
            | Code::F7
            | Code::F8
            | Code::F9
            | Code::F10
            | Code::F11
            | Code::F12
            | Code::F13
            | Code::F14
            | Code::F15
            | Code::F16
            | Code::F17
            | Code::F18
            | Code::F19
            | Code::F20
            | Code::F21
            | Code::F22
            | Code::F23
            | Code::F24
    );
    if !function_key
        && !shortcut
            .mods
            .intersects(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SUPER)
    {
        return Err(
            "Добавьте Ctrl, Alt или Win. Клавиши F1–F24 можно использовать отдельно.".into(),
        );
    }
    Ok(shortcut)
}

pub fn label(shortcut: Shortcut) -> String {
    let mut parts = Vec::new();
    for (modifier, name) in [
        (Modifiers::CONTROL, "Ctrl"),
        (Modifiers::ALT, "Alt"),
        (Modifiers::SHIFT, "Shift"),
        (Modifiers::SUPER, "Win"),
    ] {
        if shortcut.mods.contains(modifier) {
            parts.push(name.to_owned());
        }
    }
    let key = shortcut.key.to_string();
    parts.push(
        key.strip_prefix("Key")
            .or_else(|| key.strip_prefix("Digit"))
            .unwrap_or(&key)
            .to_owned(),
    );
    parts.join("+")
}

fn save(path: &Path, shortcut: Shortcut) -> Result<(), String> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("Не найден каталог настроек")?;
    fs::create_dir_all(parent).map_err(|e| format!("Не удалось создать каталог настроек: {e}"))?;
    let temporary = parent.join(format!(
        ".hotkey-{}-{}.tmp",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let data = serde_json::to_vec_pretty(&Config {
            shortcut: label(shortcut),
        })?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        // On Windows rename replaces the destination file atomically.
        fs::rename(&temporary, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result.map_err(|e| format!("Не удалось сохранить горячую клавишу: {e}"))
}

impl Hotkeys {
    pub fn load(path: Result<PathBuf, String>) -> Self {
        let loaded = match &path {
            Ok(path) => match fs::read_to_string(path) {
                Ok(data) => serde_json::from_str::<Config>(&data)
                    .map_err(|e| e.to_string())
                    .and_then(|config| parse(&config.shortcut)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => parse(DEFAULT_SHORTCUT),
                Err(e) => Err(e.to_string()),
            },
            Err(error) => Err(error.clone()),
        };
        let (shortcut, error) = match loaded {
            Ok(shortcut) => (shortcut, None),
            Err(error) => (
                parse(DEFAULT_SHORTCUT).unwrap(),
                Some(format!(
                    "Настройка не прочитана; выбрано стандартное сочетание. {error}"
                )),
            ),
        };
        Self {
            binding: Mutex::new(Binding {
                shortcut,
                active: false,
                error,
            }),
            path,
            active_id: AtomicU32::new(shortcut.id()),
            active: AtomicBool::new(false),
            capturing: AtomicBool::new(false),
            pressed: AtomicBool::new(false),
        }
    }

    pub fn start(&self, app: &AppHandle) {
        self.start_with(&NativeRegistrar(app));
    }

    fn start_with(&self, registrar: &impl Registrar) {
        let mut binding = self.binding.lock().unwrap();
        match registrar.register(binding.shortcut) {
            Ok(()) => {
                binding.active = true;
                self.active.store(true, Ordering::Release);
            }
            Err(error) => {
                binding.error = Some(format!(
                    "Сочетание {} недоступно: оно занято или зарезервировано Windows. Выберите другое. {error}",
                    label(binding.shortcut)
                ));
            }
        }
    }

    pub fn snapshot(&self) -> HotkeyView {
        Self::view(&self.binding.lock().unwrap())
    }

    fn view(binding: &Binding) -> HotkeyView {
        HotkeyView {
            shortcut: label(binding.shortcut),
            default_shortcut: DEFAULT_SHORTCUT.into(),
            active: binding.active,
            error: binding.error.clone(),
        }
    }

    pub fn set(&self, app: &AppHandle, value: &str) -> Result<HotkeyView, String> {
        self.set_with(&NativeRegistrar(app), value, |shortcut| {
            let path = self.path.as_ref().map_err(Clone::clone)?;
            save(path, shortcut)
        })
    }

    fn set_with(
        &self,
        registrar: &impl Registrar,
        value: &str,
        persist: impl FnOnce(Shortcut) -> Result<(), String>,
    ) -> Result<HotkeyView, String> {
        let next = parse(value)?;
        let mut binding = self.binding.lock().unwrap();
        let same_active = binding.active && binding.shortcut == next;
        if !same_active {
            registrar.register(next).map_err(|_| {
                "Сочетание занято другим приложением или зарезервировано Windows. Выберите другое; прежняя настройка сохранена.".to_owned()
            })?;
        }
        if let Err(error) = persist(next) {
            if !same_active {
                let _ = registrar.unregister(next);
            }
            return Err(error);
        }
        let old = binding.shortcut;
        let old_active = binding.active;
        binding.shortcut = next;
        binding.active = true;
        binding.error = None;
        // The callback only reads atomics: registering on the main thread must
        // never wait on this mutex. Staged/retired shortcut events are ignored.
        self.active_id.store(next.id(), Ordering::Release);
        self.active.store(true, Ordering::Release);
        if old != next {
            self.pressed.store(false, Ordering::Release);
        }
        if old != next && old_active {
            if let Err(error) = registrar.unregister(old) {
                binding.error = Some(format!(
                    "Новое сочетание работает. Предыдущее будет освобождено после выхода из приложения: {error}"
                ));
            }
        }
        Ok(Self::view(&binding))
    }

    pub fn set_capture(&self, enabled: bool) {
        self.capturing.store(enabled, Ordering::Release);
    }

    pub fn event(&self, shortcut: Shortcut, state: ShortcutState) -> Action {
        if shortcut.id() != self.active_id.load(Ordering::Acquire)
            || !self.active.load(Ordering::Acquire)
        {
            return Action::Ignore;
        }
        if state == ShortcutState::Released {
            self.pressed.store(false, Ordering::Release);
            return Action::Ignore;
        }
        if self.pressed.swap(true, Ordering::AcqRel) {
            return Action::Ignore;
        }
        if self.capturing.load(Ordering::Acquire) {
            Action::Capture
        } else {
            Action::Toggle
        }
    }
}

#[tauri::command]
pub async fn hotkey_status(
    state: tauri::State<'_, std::sync::Arc<Hotkeys>>,
) -> Result<HotkeyView, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || state.snapshot())
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_hotkey(
    app: AppHandle,
    state: tauri::State<'_, std::sync::Arc<Hotkeys>>,
    shortcut: String,
) -> Result<HotkeyView, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let view = state.set(&app, &shortcut)?;
        let _ = app.emit("hotkey-status", &view);
        Ok(view)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn set_hotkey_capture(
    app: AppHandle,
    state: tauri::State<'_, std::sync::Arc<Hotkeys>>,
    enabled: bool,
) -> Result<(), String> {
    if enabled {
        let focused = app
            .get_webview_window("main")
            .and_then(|window| window.is_focused().ok())
            .unwrap_or(false);
        if !focused {
            return Err("Для записи сочетания откройте окно Voice Input".into());
        }
    }
    state.set_capture(enabled);
    Ok(())
}
