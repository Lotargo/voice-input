use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Clone, Serialize)]
pub struct LocalModel {
    pub name: String,
    pub path: String,
}

fn candidate_roots(app: &AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(root) = app.path().resource_dir() {
        roots.push(root);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("src-tauri"));
    }
    if let Ok(local) = app.path().app_local_data_dir() {
        roots.push(local);
    }
    roots
}

pub fn models(app: &AppHandle) -> Vec<LocalModel> {
    let mut out = Vec::<LocalModel>::new();
    for root in candidate_roots(app) {
        for base in [root.join("resources/models"), root.join("models")] {
            let Ok(entries) = std::fs::read_dir(base) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if !looks_like_model(&path) {
                    continue;
                }
                let text = path.to_string_lossy().into_owned();
                if out.iter().any(|m| m.path == text) {
                    continue;
                }
                out.push(LocalModel {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    path: text,
                });
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

pub fn looks_like_model(path: &Path) -> bool {
    path.is_dir() && path.join("am").is_dir() && path.join("conf").is_dir()
}

pub fn dll_path(app: &AppHandle) -> Result<PathBuf, String> {
    for root in candidate_roots(app) {
        for path in [root.join("resources/libvosk.dll"), root.join("libvosk.dll")] {
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    Err("Не найден libvosk.dll. Запустите scripts/prepare-assets.ps1".into())
}
