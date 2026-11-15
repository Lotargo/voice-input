//! One Windows user profile, shared by installed and portable application copies.
//! Keep this identity stable: neither the executable directory nor packaging chooses it.
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

pub const APP_ID: &str = "dev.lotargo.voiceinput";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserStorage {
    pub config: PathBuf,
    pub local: PathBuf,
}

impl UserStorage {
    pub fn resolve(app: &AppHandle) -> tauri::Result<Self> {
        Ok(Self::from_base(
            &app.path().config_dir()?,
            &app.path().local_data_dir()?,
        ))
    }

    pub(crate) fn from_base(roaming: &Path, local: &Path) -> Self {
        Self {
            config: roaming.join(APP_ID),
            local: local.join(APP_ID),
        }
    }

    pub fn models(&self) -> PathBuf {
        self.local.join("models")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_identity_matches_both_release_formats_and_uninstaller() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], APP_ID);
        // The NSIS data-retention choice operates on APPDATA/LOCALAPPDATA + BUNDLEID.
        let paths = UserStorage::from_base(Path::new("Roaming"), Path::new("Local"));
        assert_eq!(paths.config, Path::new("Roaming").join(APP_ID));
        assert_eq!(
            paths.models(),
            Path::new("Local").join(APP_ID).join("models")
        );
        assert!(include_str!("../../scripts/build-windows.ps1")
            .contains("app_id = 'dev.lotargo.voiceinput'"));
    }
}
