use cpal::traits::{DeviceTrait, HostTrait};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct InputOptions {
    pub device_id: Option<String>,
    pub device_name: Option<String>,
    pub denoise: bool,
}

#[derive(Clone, Serialize)]
pub struct InputDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

pub fn devices() -> Result<Vec<InputDevice>, String> {
    let host = cpal::default_host();
    let default = host.default_input_device().and_then(|d| d.id().ok());
    let mut result = Vec::new();
    for device in host
        .input_devices()
        .map_err(|e| format!("Список микрофонов: {e}"))?
    {
        let Ok(id) = device.id() else { continue };
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "Микрофон".into());
        result.push(InputDevice {
            is_default: default.as_ref() == Some(&id),
            id: id.to_string(),
            name,
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
    Ok(result)
}

pub fn resolve(options: &InputOptions) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    if let Some(id) = &options.device_id {
        let id: cpal::DeviceId = id
            .parse()
            .map_err(|_| "Сохранённый микрофон недоступен. Выберите устройство заново.")?;
        host.device_by_id(&id).ok_or_else(|| {
            format!(
                "Микрофон «{}» отключён или недоступен. Подключите его или выберите другой.",
                options.device_name.as_deref().unwrap_or("выбранный")
            )
        })
    } else {
        host.default_input_device()
            .ok_or_else(|| "Микрофон Windows по умолчанию не найден".into())
    }
}

pub fn connected(id: &str) -> Result<bool, String> {
    let host = cpal::default_host();
    Ok(host
        .input_devices()
        .map_err(|e| format!("Не удалось проверить микрофон: {e}"))?
        .any(|device| device.id().is_ok_and(|value| value.to_string() == id)))
}

#[derive(Serialize)]
pub struct InputView {
    pub options: InputOptions,
    pub devices: Vec<InputDevice>,
    pub name: String,
    pub available: bool,
    pub error: Option<String>,
}

pub struct Preferences {
    pub options: InputOptions,
    path: Result<PathBuf, String>,
    error: Option<String>,
}

impl Preferences {
    pub fn load(path: Result<PathBuf, String>) -> Self {
        let mut result = Self {
            options: InputOptions::default(),
            path,
            error: None,
        };
        match &result.path {
            Ok(path) => match fs::read(path) {
                Ok(bytes) => match serde_json::from_slice(&bytes) {
                    Ok(options) => result.options = options,
                    Err(e) => {
                        result.error = Some(format!("Не удалось прочитать настройки звука: {e}"))
                    }
                },
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => result.error = Some(format!("Не удалось прочитать настройки звука: {e}")),
            },
            Err(e) => result.error = Some(e.clone()),
        }
        result
    }

    pub fn view(&self) -> Result<InputView, String> {
        let devices = devices()?;
        let selected = if let Some(id) = &self.options.device_id {
            devices.iter().find(|d| &d.id == id)
        } else {
            devices.iter().find(|d| d.is_default)
        };
        let name = selected
            .map(|d| d.name.clone())
            .or_else(|| self.options.device_name.clone())
            .unwrap_or_else(|| "Микрофон не найден".into());
        Ok(InputView {
            options: self.options.clone(),
            devices: devices.clone(),
            name,
            available: selected.is_some(),
            error: self.error.clone(),
        })
    }

    pub fn capture_options(&self) -> Result<InputOptions, String> {
        if self.error.is_some() {
            return Err("Настройки звука не прочитаны. Выберите микрофон в параметрах и сохраните настройку перед захватом звука.".into());
        }
        Ok(self.options.clone())
    }

    pub fn save(&mut self, device_id: Option<String>, denoise: bool) -> Result<InputView, String> {
        let devices = devices()?;
        let device_name = if let Some(id) = &device_id {
            Some(
                devices
                    .iter()
                    .find(|d| &d.id == id)
                    .ok_or("Выбранный микрофон уже отключён. Обновите список.")?
                    .name
                    .clone(),
            )
        } else {
            None
        };
        let next = InputOptions {
            device_id,
            device_name,
            denoise,
        };
        let path = self.path.as_ref().map_err(Clone::clone)?;
        save_file(path, &next)?;
        self.options = next;
        self.error = None;
        let selected = if let Some(id) = &self.options.device_id {
            devices.iter().find(|d| &d.id == id)
        } else {
            devices.iter().find(|d| d.is_default)
        };
        let name = selected
            .map(|d| d.name.clone())
            .or_else(|| self.options.device_name.clone())
            .unwrap_or_else(|| "Микрофон не найден".into());
        let available = selected.is_some();
        Ok(InputView {
            options: self.options.clone(),
            devices,
            name,
            available,
            error: None,
        })
    }
}

fn save_file(path: &std::path::Path, options: &InputOptions) -> Result<(), String> {
    static NONCE: AtomicU64 = AtomicU64::new(0);
    let parent = path.parent().ok_or("Нет каталога настроек звука")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let temporary = parent.join(format!(
        "audio-{}-{}.tmp",
        std::process::id(),
        NONCE.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(&serde_json::to_vec(options).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, path)
            .map_err(|e| format!("Не удалось сохранить настройки звука: {e}"))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_an_opaque_device_id_and_noise_choice_without_using_an_index() {
        let root = std::env::temp_dir().join(format!("voice-input-audio-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("audio.json");
        let options = InputOptions {
            device_id: Some("wasapi:opaque-device-id".into()),
            device_name: Some("USB microphone".into()),
            denoise: true,
        };
        save_file(&path, &options).unwrap();
        let read = Preferences::load(Ok(path.clone()));
        assert_eq!(read.options.device_id, options.device_id);
        assert!(read.options.denoise);
        save_file(&path, &InputOptions::default()).unwrap();
        assert!(Preferences::load(Ok(path.clone()))
            .options
            .device_id
            .is_none());
        fs::remove_file(path).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
