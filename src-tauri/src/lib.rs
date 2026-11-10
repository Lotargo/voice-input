#![cfg_attr(not(debug_assertions), allow(dead_code))]
#[cfg(not(windows))]
compile_error!("Initial MVP supports Windows 10/11 only");

mod audio;
mod delivery;
mod models;
mod vosk;

use std::{path::PathBuf, sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex}};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State, WindowEvent, menu::{Menu, MenuItem}, tray::TrayIconBuilder};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use audio::AudioEvent;
use delivery::{Delivery, DeliveryStatus};
use models::LocalModel;

struct RecordingState {
    recording: bool,
    session: u64,
    stop: Option<Arc<AtomicBool>>,
    model: Option<PathBuf>,
    auto_stop: bool,
}

struct AppRuntime {
    recorder: Mutex<RecordingState>,
    delivery: Arc<Delivery>,
}

impl AppRuntime {
    fn new() -> Self {
        Self {
            recorder: Mutex::new(RecordingState {
                recording: false, session: 0, stop: None, model: None, auto_stop: false,
            }),
            delivery: Arc::new(Delivery::new()),
        }
    }

    fn begin(self: &Arc<Self>, app: AppHandle) -> Result<(), String> {
        let (session, stop, model, auto_stop) = {
            let mut recorder = self.recorder.lock().unwrap();
            if recorder.recording { return Err("Запись уже запущена".into()); }
            let selected = recorder.model.clone()
                .or_else(|| models::models(&app).first().map(|model| PathBuf::from(&model.path)))
                .ok_or("Сначала установите локальную модель Vosk")?;
            if !models::looks_like_model(&selected) {
                return Err("Папка выбранной модели не содержит am/ и conf/".into());
            }
            let dll = models::dll_path(&app)?;
            // Validate runtime dependencies BEFORE setting the session to 'recording'.
            if !dll.is_file() { return Err("libvosk.dll не найден".into()); }
            recorder.recording = true;
            recorder.session += 1;
            let stop = Arc::new(AtomicBool::new(false));
            recorder.stop = Some(stop.clone());
            (recorder.session, stop, selected, recorder.auto_stop)
        };
        let runtime = self.clone();
        std::thread::spawn(move || {
            let _ = app.emit("recording-status", RecordingView {
                recording: true, session, message: "Загрузка модели и запуск микрофона".into(),
            });
            let dll = match models::dll_path(&app) {
                Ok(value) => value,
                Err(e) => {
                    runtime.end_session(&app, session, Some(e));
                    return;
                }
            };
            let mut segment: u32 = 0;
            let result = audio::record_session(&model, &dll, stop, auto_stop, |ev| match ev {
                AudioEvent::Partial(text) => {
                    let _ = app.emit("transcription-partial", TranscriptView { session, text });
                }
                AudioEvent::Final(text) => {
                    segment += 1;
                    let _ = runtime.delivery.enqueue(&app, session, segment, text.clone());
                    let _ = app.emit("transcription-final", TranscriptView { session, text });
                }
                AudioEvent::AutoStopped => {
                    let _ = app.emit("transcription-partial", TranscriptView {
                        session, text: String::new(),
                    });
                }
            });
            runtime.end_session(&app, session, result.err());
        });
        Ok(())
    }

    fn end_session(&self, app: &AppHandle, session: u64, error: Option<String>) {
        {
            let mut current = self.recorder.lock().unwrap();
            if current.session == session {
                current.recording = false;
                current.stop = None;
            }
        }
        let _ = app.emit("recording-status", RecordingView {
            recording: false, session,
            message: error.unwrap_or_else(|| "Запись завершена".into()),
        });
    }

    fn stop(&self) {
        let state = self.recorder.lock().unwrap();
        if let Some(flag) = &state.stop { flag.store(true, Ordering::Release); }
    }

    fn toggle(self: &Arc<Self>, app: AppHandle) -> Result<(), String> {
        if self.recorder.lock().unwrap().recording {
            self.stop();
            Ok(())
        } else {
            self.begin(app)
        }
    }

    fn view(&self) -> RecordingView {
        let state = self.recorder.lock().unwrap();
        RecordingView {
            recording: state.recording, session: state.session,
            message: if state.recording { "Запись идёт" } else { "Ожидание" }.into(),
        }
    }
}

#[derive(Clone, Serialize)]
struct RecordingView {
    recording: bool,
    session: u64,
    message: String,
}

#[derive(Clone, Serialize)]
struct TranscriptView {
    session: u64,
    text: String,
}

#[tauri::command]
fn available_models(app: AppHandle) -> Vec<LocalModel> { models::models(&app) }

#[tauri::command]
fn select_model(state: State<'_, Arc<AppRuntime>>, path: String) -> Result<(), String> {
    let selected = PathBuf::from(path);
    if !models::looks_like_model(&selected) {
        return Err("Папка модели должна содержать am/ и conf/".into());
    }
    let mut state = state.recorder.lock().unwrap();
    if state.recording { return Err("Нельзя переключать модель во время записи".into()); }
    state.model = Some(selected);
    Ok(())
}

#[tauri::command]
fn set_auto_stop(state: State<'_, Arc<AppRuntime>>, enabled: bool) {
    state.recorder.lock().unwrap().auto_stop = enabled;
}

#[tauri::command]
fn set_hover_delivery(app: AppHandle, state: State<'_, Arc<AppRuntime>>, enabled: bool) {
    state.delivery.set_hover(&app, enabled);
}

#[tauri::command]
fn begin_recording(app: AppHandle, state: State<'_, Arc<AppRuntime>>) -> Result<(), String> {
    state.begin(app)
}

#[tauri::command]
fn end_recording(state: State<'_, Arc<AppRuntime>>) { state.stop(); }

#[tauri::command]
fn recording_status(state: State<'_, Arc<AppRuntime>>) -> RecordingView { state.view() }

#[tauri::command]
fn delivery_status(state: State<'_, Arc<AppRuntime>>) -> DeliveryStatus { state.delivery.snapshot() }

#[tauri::command]
fn discard_pending(app: AppHandle, state: State<'_, Arc<AppRuntime>>, id: String) {
    state.delivery.discard(&app, &id);
}

fn show_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let runtime = Arc::new(AppRuntime::new());
            runtime.delivery.clone().spawn_monitor(app.handle().clone());
            app.manage(runtime);

            // Fixed hotkey for v0.1: Ctrl + Alt + Space. No window activation.
            let hotkey = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::Space);
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new().with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        let state = app.state::<Arc<AppRuntime>>();
                        if let Err(error) = state.toggle(app.clone()) {
                            let _ = app.emit("recording-status", RecordingView {
                                recording: false, session: state.view().session, message: error,
                            });
                        }
                    }
                }).build()
            )?;
            app.global_shortcut().register(hotkey)?;

            let show = MenuItem::with_id(app, "show", "Показать Voice Input", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().ok_or("Не настроена иконка приложения")?.clone())
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => show_window(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                        show_window(tray.app_handle());
                    }
                })
                .build(app)?;
            std::mem::forget(tray); // Kept alive by the Tauri tray manager.
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            available_models, select_model, set_auto_stop, set_hover_delivery,
            begin_recording, end_recording, recording_status, delivery_status, discard_pending,
        ])
        .run(tauri::generate_context!())
        .expect("Не удалось запустить Voice Input");
}
