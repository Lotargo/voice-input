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

pub fn runtime_path(app: &AppHandle, engine: &str) -> Result<PathBuf, String> {
    runtime_from_roots(&candidate_roots(app), engine)
}

pub(crate) fn runtime_from_roots(roots: &[PathBuf], engine: &str) -> Result<PathBuf, String> {
    let file = match engine {
        "vosk" => "libvosk.dll",
        "gigaam" | "nemotron" => "models/CPU-Execution.conf",
        _ => return Err("Неизвестный движок распознавания".into()),
    };
    for root in roots {
        for path in [root.join("resources").join(file), root.join(file)] {
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    Err(format!(
        "Не найден файл runtime {file}. Восстановите файлы приложения."
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_profile_is_found_in_installed_and_portable_layouts_without_vosk_dll() {
        let root = std::env::temp_dir().join(format!("voice-runtime-paths-{}", std::process::id()));
        assert!(!root.exists());
        for layout in ["installed/resources", "portable"] {
            let base = root.join(layout);
            std::fs::create_dir_all(base.join("models")).unwrap();
            std::fs::write(
                base.join("models/CPU-Execution.conf"),
                include_bytes!("../resources/models/CPU-Execution.conf"),
            )
            .unwrap();
        }
        for layout in ["installed", "portable"] {
            let roots = [root.join(layout)];
            for engine in ["gigaam", "nemotron"] {
                let profile = runtime_from_roots(&roots, engine).unwrap();
                assert!(profile.is_file());
                assert!(profile.ends_with("CPU-Execution.conf"));
            }
            assert!(runtime_from_roots(&roots, "vosk").is_err());
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
