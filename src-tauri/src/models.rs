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
    #[cfg(debug_assertions)]
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd.join("src-tauri"));
    }
    roots
}

pub fn models(app: &AppHandle) -> Vec<LocalModel> {
    bundled_from_roots(&candidate_roots(app))
}

fn bundled_from_roots(roots: &[PathBuf]) -> Vec<LocalModel> {
    for root in roots {
        for base in [root.join("resources/models"), root.join("models")] {
            let path = base.join("vosk-model-small-ru-0.22");
            if looks_like_model(&path) {
                return vec![LocalModel {
                    name: "Vosk Small RU".into(),
                    path: path.to_string_lossy().into_owned(),
                }];
            }
        }
    }
    Vec::new()
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
    fn repeated_bundle_locations_produce_one_curated_vosk_and_ignore_other_folders() {
        let root = std::env::temp_dir().join(format!("voice-bundled-paths-{}", std::process::id()));
        assert!(!root.exists());
        let roots = [root.join("installed"), root.join("development")];
        for base in &roots {
            for model in ["vosk-model-small-ru-0.22", "another-vosk"] {
                for part in ["am", "conf"] {
                    std::fs::create_dir_all(base.join("resources/models").join(model).join(part))
                        .unwrap();
                }
            }
        }
        let found = bundled_from_roots(&roots);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name, "Vosk Small RU");
        assert!(Path::new(&found[0].path).starts_with(&roots[0]));
        std::fs::remove_dir_all(root).unwrap();
    }
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
