#![cfg_attr(not(debug_assertions), allow(dead_code))]
#[cfg(not(windows))]
compile_error!("Initial MVP supports Windows 10/11 only");

mod audio;
mod delivery;
mod hotkey;
mod models;
mod records;
mod vosk;

use serde::Serialize;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, State, WindowEvent,
};

use audio::AudioEvent;
use delivery::{Delivery, DeliveryStatus};
use models::LocalModel;

struct RecordingState {
    recording: bool,
    session: u64,
    stop: Option<Arc<AtomicBool>>,
    model: Option<PathBuf>,
    control: Option<audio::CaptureControl>,
}

struct AppRuntime {
    recorder: Mutex<RecordingState>,
    delivery: Arc<Delivery>,
    records: Mutex<records::Records>,
    split: Arc<audio::SplitSettings>,
}

impl AppRuntime {
    fn new() -> Self {
        Self {
            recorder: Mutex::new(RecordingState {
                recording: false,
                session: 0,
                stop: None,
                model: None,
                control: None,
            }),
            delivery: Arc::new(Delivery::new()),
            records: Mutex::new(records::Records::new()),
            split: Arc::new(audio::SplitSettings::new()),
        }
    }

    fn begin(self: &Arc<Self>, app: AppHandle) -> Result<(), String> {
        let (session, stop, model, input) = {
            let mut recorder = self.recorder.lock().unwrap();
            if recorder.recording {
                return Err("Запись уже запущена".into());
            }
            let selected = recorder
                .model
                .clone()
                .or_else(|| {
                    models::models(&app)
                        .first()
                        .map(|model| PathBuf::from(&model.path))
                })
                .ok_or("Сначала установите локальную модель Vosk")?;
            if !models::looks_like_model(&selected) {
                return Err("Папка выбранной модели не содержит am/ и conf/".into());
            }
            let dll = models::dll_path(&app)?;
            // Validate runtime dependencies BEFORE setting the session to 'recording'.
            if !dll.is_file() {
                return Err("libvosk.dll не найден".into());
            }
            recorder.recording = true;
            recorder.session += 1;
            let stop = Arc::new(AtomicBool::new(false));
            recorder.stop = Some(stop.clone());
            let (control, input) = audio::input_channel();
            recorder.control = Some(control);
            (recorder.session, stop, selected, input)
        };
        let runtime = self.clone();
        std::thread::spawn(move || {
            let _ = app.emit(
                "recording-status",
                RecordingView {
                    recording: true,
                    session,
                    message: "Загрузка модели и запуск микрофона".into(),
                },
            );
            let dll = match models::dll_path(&app) {
                Ok(value) => value,
                Err(e) => {
                    runtime.end_session(&app, session, Some(e));
                    return;
                }
            };
            let mut segment: u32 = 0;
            let result = audio::record_session(
                &model,
                &dll,
                stop,
                input,
                runtime.split.clone(),
                |ev| match ev {
                    AudioEvent::Started => {
                        let _ = app.emit(
                            "recording-status",
                            RecordingView {
                                recording: true,
                                session,
                                message: "Запись идёт".into(),
                            },
                        );
                    }
                    AudioEvent::Partial(text) => {
                        let mut records = runtime.records.lock().unwrap();
                        records.partial(text.clone());
                        let (record_id, generation, version) = records.identity();
                        let _ = app.emit(
                            "transcription-partial",
                            TranscriptView {
                                session,
                                text,
                                record_id,
                                generation,
                                version,
                            },
                        );
                    }
                    AudioEvent::Levels(levels) => {
                        let _ = app.emit("audio-levels", AudioLevelsView { session, levels });
                    }
                    AudioEvent::Final(text) => {
                        segment += 1;
                        let mut records = runtime.records.lock().unwrap();
                        let id = format!("{session}:{segment}");
                        if !records.append(id.clone(), text.clone()) {
                            return;
                        }
                        let (record_id, generation, version) = records.identity();
                        let _ = runtime.delivery.enqueue_record(
                            &app,
                            session,
                            segment,
                            record_id,
                            generation,
                            text.clone(),
                        );
                        let _ = app.emit(
                            "transcription-final",
                            FinalTranscriptView {
                                session,
                                id,
                                text,
                                record_id,
                                generation,
                                version,
                            },
                        );
                    }
                    AudioEvent::Boundary(action) => {
                        runtime.change_record(&app, action);
                    }
                },
                |record_id, generation| {
                    let identity = runtime.records.lock().unwrap().identity();
                    identity.0 == record_id && identity.1 == generation
                },
            );
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
                current.control = None;
            }
        }
        let _ = app.emit(
            "recording-status",
            RecordingView {
                recording: false,
                session,
                message: error.unwrap_or_else(|| "Запись завершена".into()),
            },
        );
    }

    fn stop(&self) {
        let state = self.recorder.lock().unwrap();
        if let Some(flag) = &state.stop {
            flag.store(true, Ordering::Release);
        }
    }

    fn change_record(&self, app: &AppHandle, action: audio::RecordAction) -> records::Snapshot {
        let mut records = self.records.lock().unwrap();
        if action == audio::RecordAction::Clear {
            let ids = records.clear();
            self.delivery.take_pending(app, &ids);
        } else {
            records.next();
        }
        let snapshot = records.snapshot();
        let _ = app.emit("record-changed", &snapshot);
        snapshot
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
            recording: state.recording,
            session: state.session,
            message: if state.recording {
                "Запись идёт"
            } else {
                "Ожидание"
            }
            .into(),
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
    record_id: u64,
    generation: u64,
    version: u64,
}

#[derive(Clone, Serialize)]
struct FinalTranscriptView {
    session: u64,
    id: String,
    text: String,
    record_id: u64,
    generation: u64,
    version: u64,
}

#[derive(Clone, Serialize)]
struct AudioLevelsView {
    session: u64,
    #[serde(flatten)]
    levels: audio::AudioLevels,
}

#[tauri::command]
fn available_models(app: AppHandle) -> Vec<LocalModel> {
    models::models(&app)
}

#[tauri::command]
fn select_model(state: State<'_, Arc<AppRuntime>>, path: String) -> Result<(), String> {
    let selected = PathBuf::from(path);
    if !models::looks_like_model(&selected) {
        return Err("Папка модели должна содержать am/ и conf/".into());
    }
    let mut state = state.recorder.lock().unwrap();
    if state.recording {
        return Err("Нельзя переключать модель во время записи".into());
    }
    state.model = Some(selected);
    Ok(())
}

#[tauri::command]
fn set_record_options(
    state: State<'_, Arc<AppRuntime>>,
    enabled: bool,
    silence_seconds: u32,
) -> Result<audio::SplitOptions, String> {
    state.split.set(enabled, silence_seconds)
}

#[tauri::command]
fn record_snapshot(state: State<'_, Arc<AppRuntime>>) -> records::Snapshot {
    state.records.lock().unwrap().snapshot()
}

#[tauri::command]
fn edit_record(
    app: AppHandle,
    state: State<'_, Arc<AppRuntime>>,
    record_id: u64,
    generation: u64,
    text: String,
    base_ids: Vec<String>,
) -> Result<records::Record, String> {
    let mut records = state.records.lock().unwrap();
    let edited = records.edit(record_id, generation, text, &base_ids)?;
    state.delivery.take_pending(&app, &base_ids);
    Ok(edited)
}

#[tauri::command]
async fn change_record(
    app: AppHandle,
    state: State<'_, Arc<AppRuntime>>,
    clear: bool,
    record_id: u64,
    generation: u64,
    session: u64,
) -> Result<records::Snapshot, String> {
    let state = state.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let action = if clear {
            audio::RecordAction::Clear
        } else {
            audio::RecordAction::New
        };
        let recorder = state.recorder.lock().unwrap();
        if recorder.session != session {
            return Err("Диктовка уже перезапущена. Повторите действие для текущей записи.".into());
        }
        let control = recorder.control.clone();
        if let Some(control) = control {
            let reply = control.request(action, record_id, generation)?;
            drop(recorder);
            // The same FIFO orders audio and controls. Wait for acknowledgement;
            // disconnected workers report failure instead of replaying a control.
            reply
                .recv()
                .map_err(|_| "Микрофон остановлен до обработки команды".to_owned())??;
            Ok(state.records.lock().unwrap().snapshot())
        } else {
            let identity = state.records.lock().unwrap().identity();
            if identity.0 != record_id || identity.1 != generation {
                return Err("Запись уже изменилась. Повторите действие для текущей записи.".into());
            }
            // Serialize an idle transition with begin(), before any worker starts.
            let snapshot = state.change_record(&app, action);
            drop(recorder);
            Ok(snapshot)
        }
    })
    .await
    .map_err(|e| e.to_string())?
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
fn end_recording(state: State<'_, Arc<AppRuntime>>) {
    state.stop();
}

#[tauri::command]
fn recording_status(state: State<'_, Arc<AppRuntime>>) -> RecordingView {
    state.view()
}

#[tauri::command]
fn delivery_status(state: State<'_, Arc<AppRuntime>>) -> DeliveryStatus {
    state.delivery.snapshot()
}

#[tauri::command]
fn discard_pending(app: AppHandle, state: State<'_, Arc<AppRuntime>>, id: String) {
    state.delivery.discard(&app, &id);
}

#[tauri::command]
fn take_pending(app: AppHandle, state: State<'_, Arc<AppRuntime>>, ids: Vec<String>) -> usize {
    state.delivery.take_pending(&app, &ids)
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

            let hotkeys = Arc::new(hotkey::Hotkeys::load(
                app.path()
                    .app_config_dir()
                    .map(|directory| directory.join("hotkey.json"))
                    .map_err(|error| error.to_string()),
            ));
            app.manage(hotkeys.clone());
            app.handle().plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_handler(|app, shortcut, event| {
                        let hotkeys = app.state::<Arc<hotkey::Hotkeys>>();
                        match hotkeys.event(*shortcut, event.state()) {
                            hotkey::Action::Ignore => {}
                            hotkey::Action::Capture => {
                                let _ = app.emit("hotkey-captured", hotkey::label(*shortcut));
                            }
                            hotkey::Action::Toggle => {
                                let state = app.state::<Arc<AppRuntime>>();
                                if let Err(error) = state.toggle(app.clone()) {
                                    let _ = app.emit(
                                        "recording-status",
                                        RecordingView {
                                            recording: false,
                                            session: state.view().session,
                                            message: error,
                                        },
                                    );
                                }
                            }
                        }
                    })
                    .build(),
            )?;
            // A conflict keeps the app usable so the binding can be changed.
            hotkeys.start(app.handle());

            let show = MenuItem::with_id(app, "show", "Показать Voice Input", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            let tray = TrayIconBuilder::new()
                .icon(tauri::include_image!("icons/tray.png"))
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
            if matches!(
                event,
                WindowEvent::Focused(false) | WindowEvent::CloseRequested { .. }
            ) {
                if let Some(hotkeys) = window.try_state::<Arc<hotkey::Hotkeys>>() {
                    hotkeys.set_capture(false);
                }
            }
            if let WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            available_models,
            select_model,
            set_record_options,
            record_snapshot,
            edit_record,
            change_record,
            set_hover_delivery,
            begin_recording,
            end_recording,
            recording_status,
            delivery_status,
            discard_pending,
            take_pending,
            hotkey::hotkey_status,
            hotkey::set_hotkey,
            hotkey::set_hotkey_capture,
        ])
        .run(tauri::generate_context!())
        .expect("Не удалось запустить Voice Input");
}
