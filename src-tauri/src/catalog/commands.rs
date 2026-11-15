use super::View;
use crate::AppRuntime;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn model_catalog(state: State<'_, Arc<AppRuntime>>) -> View {
    state.models.view()
}

#[tauri::command]
pub fn migrate_model_selection(
    state: State<'_, Arc<AppRuntime>>,
    path: String,
) -> Result<View, String> {
    let recorder = state.recorder.lock().unwrap();
    if !recorder.recording && !recorder.updating {
        state.models.migrate_selection(path.into())?;
    }
    Ok(state.models.view())
}

#[tauri::command]
pub fn download_model(
    app: AppHandle,
    state: State<'_, Arc<AppRuntime>>,
    id: String,
) -> Result<View, String> {
    let recorder = state.recorder.lock().unwrap();
    if recorder.updating {
        return Err("Дождитесь завершения обновления".into());
    }
    state.models.start(
        &id,
        Arc::new(move |view| {
            let _ = app.emit("model-catalog", view);
        }),
    )?;
    Ok(state.models.view())
}
#[tauri::command]
pub fn pause_model_download(state: State<'_, Arc<AppRuntime>>, id: String) -> Result<View, String> {
    state.models.interrupt(&id, false)?;
    Ok(state.models.view())
}
#[tauri::command]
pub fn cancel_model_download(
    state: State<'_, Arc<AppRuntime>>,
    id: String,
) -> Result<View, String> {
    state.models.interrupt(&id, true)?;
    Ok(state.models.view())
}
#[tauri::command]
pub fn delete_model(state: State<'_, Arc<AppRuntime>>, id: String) -> Result<View, String> {
    let recorder = state.recorder.lock().unwrap();
    if recorder.recording || recorder.updating {
        return Err(
            "Остановите запись и дождитесь завершения обновления перед удалением модели".into(),
        );
    }
    state.models.remove(&id)?;
    Ok(state.models.view())
}
