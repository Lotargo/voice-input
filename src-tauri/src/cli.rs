//! Local, request/response control. No sockets, HTTP, automatic launch or retries.
use crate::{show_window, AppRuntime};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tauri::{AppHandle, Manager};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::windows::named_pipe::{ClientOptions, ServerOptions},
};

const LIMIT: u64 = 16 * 1024;
const TIMEOUT: Duration = Duration::from_secs(120);

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub command: String,
    #[serde(default)]
    pub argument: Option<String>,
}

fn pipe_name() -> Result<String, String> {
    use sha2::{Digest, Sha256};
    let profile = std::env::var("APPDATA").map_err(|_| "APPDATA is unavailable")?;
    let hash = Sha256::digest(profile.to_lowercase().as_bytes());
    Ok(format!(r"\\.\pipe\dev.lotargo.voiceinput-{:x}", hash))
}

pub(crate) fn serve(app: AppHandle) -> Result<(), String> {
    let name = pipe_name()?;
    // Windows default DACL; remote clients are explicitly rejected. Reserving the
    // first instance fails closed instead of connecting to an existing server.
    let server = {
        let _guard = tauri::async_runtime::handle().inner().enter();
        ServerOptions::new()
            .first_pipe_instance(true)
            .reject_remote_clients(true)
            .create(&name)
            .map_err(|e| format!("CLI pipe: {e}"))?
    };
    tauri::async_runtime::spawn(async move {
        let mut server = server;
        loop {
            if server.connect().await.is_err() {
                break;
            }
            // Reserve the next instance before closing this one to avoid a gap.
            let next = match ServerOptions::new()
                .reject_remote_clients(true)
                .create(&name)
            {
                Ok(next) => next,
                Err(error) => {
                    eprintln!("CLI pipe: {error}");
                    break;
                }
            };
            let mut line = String::new();
            let read = tokio::time::timeout(
                Duration::from_secs(2),
                BufReader::new((&mut server).take(LIMIT + 1)).read_line(&mut line),
            )
            .await;
            let response = if matches!(read, Ok(Ok(n)) if n > 0 && n <= LIMIT as usize)
                && line.ends_with('\n')
            {
                match serde_json::from_str::<Request>(&line) {
                    Ok(request) => dispatch(app.clone(), request).await,
                    Err(error) => Err(error.to_string()),
                }
            } else {
                Err("Request must be a JSON line of at most 16 KiB (2 s read timeout)".into())
            };
            let quit = response
                .as_ref()
                .ok()
                .is_some_and(|v| v["quitting"] == true);
            let response = match response {
                Ok(result) => json!({"ok":true,"result":result}),
                Err(error) => json!({"ok":false,"error":error}),
            };
            let mut bytes = serde_json::to_vec(&response).unwrap();
            bytes.push(b'\n');
            let _ = tokio::time::timeout(Duration::from_secs(2), server.write_all(&bytes)).await;
            drop(server);
            if quit {
                app.exit(0);
                break;
            }
            server = next;
        }
    });
    Ok(())
}

async fn dispatch(app: AppHandle, request: Request) -> Result<Value, String> {
    let runtime = app.state::<Arc<AppRuntime>>().inner().clone();
    if runtime.closing.load(std::sync::atomic::Ordering::Acquire) {
        return Err("Application is shutting down".into());
    }
    if request.command != "select-model" && request.argument.is_some() {
        return Err("This command accepts no argument".into());
    }
    match request.command.as_str() {
        "status" => Ok(
            json!({"pid": std::process::id(), "version":env!("CARGO_PKG_VERSION"), "recording":runtime.view(), "delivery":runtime.delivery.snapshot()}),
        ),
        "records" => Ok(json!(runtime.records.lock().unwrap().snapshot())),
        "models" => Ok(json!(runtime.models.view())),
        "select-model" => {
            let id = request
                .argument
                .ok_or("select-model requires a catalog ID")?;
            let recorder = runtime.recorder.lock().unwrap();
            if recorder.recording || recorder.updating {
                return Err("Stop capture/update before selecting a model".into());
            }
            let entry = runtime
                .models
                .view()
                .entries
                .into_iter()
                .find(|entry| entry.id == id && entry.installed)
                .ok_or("Installed catalog model not found")?;
            runtime.models.select(entry.path.into())?;
            Ok(json!({"selected":id}))
        }
        "start" | "mic-test" => {
            runtime.begin(app, request.command == "mic-test")?;
            Ok(json!({"accepted":true,"recording":runtime.view()}))
        }
        "stop" => {
            tauri::async_runtime::spawn_blocking(move || runtime.stop_and_wait())
                .await
                .map_err(|e| e.to_string())??;
            Ok(json!({"stopped":true,"model_released":true}))
        }
        "new-record" | "clear-record" => {
            let identity = runtime.records.lock().unwrap().identity();
            let session = runtime.view().session;
            let snapshot = crate::change_record(
                app.clone(),
                app.state(),
                request.command == "clear-record",
                identity.0,
                identity.1,
                session,
            )
            .await?;
            Ok(json!(snapshot))
        }
        "show" => {
            show_window(&app);
            Ok(json!({"visible":true}))
        }
        "hide" => {
            if runtime.view().testing {
                runtime.stop();
            }
            if let Some(window) = app.get_webview_window("main") {
                window.hide().map_err(|e| e.to_string())?;
            }
            Ok(json!({"visible":false}))
        }
        "quit" => {
            if runtime.recorder.lock().unwrap().updating {
                return Err("Wait for update installation".into());
            }
            tauri::async_runtime::spawn_blocking(move || runtime.shutdown())
                .await
                .map_err(|e| e.to_string())?;
            Ok(json!({"quitting":true,"model_released":true}))
        }
        _ => Err("Unknown command; run voice-input-cli --help".into()),
    }
}

/// One attempt only: a timeout/disconnect may mean a command was already applied.
pub async fn request(request: Request) -> Result<Value, String> {
    tokio::time::timeout(TIMEOUT, async {
        let mut pipe = ClientOptions::new()
            .open(pipe_name()?)
            .map_err(|e| format!("Cannot connect to Voice Input (start the app first): {e}"))?;
        let mut bytes = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
        bytes.push(b'\n');
        pipe.write_all(&bytes).await.map_err(|e| e.to_string())?;
        let mut response = String::new();
        BufReader::new(pipe.take(16 * 1024 * 1024))
            .read_line(&mut response)
            .await
            .map_err(|e| e.to_string())?;
        if !response.ends_with('\n') {
            return Err(
                "Incomplete response; command outcome is unknown. Do not automatically retry."
                    .into(),
            );
        }
        serde_json::from_str(&response).map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "Timeout; command outcome is unknown. Do not automatically retry.".to_owned())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_rejects_unknown_fields_and_non_string_arguments() {
        assert!(serde_json::from_str::<Request>(r#"{"command":"stop","retry":true}"#).is_err());
        assert!(
            serde_json::from_str::<Request>(r#"{"command":"select-model","argument":3}"#).is_err()
        );
        let request: Request = serde_json::from_str(r#"{"command":"status"}"#).unwrap();
        assert_eq!(request.command, "status");
        assert!(request.argument.is_none());
    }
}
