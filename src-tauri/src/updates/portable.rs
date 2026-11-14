//! Portable updates use the signed application itself as a short-lived helper.
//! Only files present in the package are replaced; failures restore the originals.
use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Read},
    os::windows::{fs::MetadataExt, process::CommandExt},
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Manager};
use windows::{
    core::PWSTR,
    Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, WaitForSingleObject, PROCESS_NAME_WIN32,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE,
        },
    },
};

const APP_ID: &str = "dev.lotargo.voiceinput";
const HELPER_ARG: &str = "--apply-portable-update";
const MAX_PACKAGE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Plan {
    app_id: String,
    parent: u32,
    executable: PathBuf,
    version: String,
    signature: String,
}
#[derive(Deserialize)]
struct Marker {
    app_id: String,
    kind: String,
    version: String,
}

pub(super) fn is_portable(root: &Path) -> bool {
    read_marker(&root.join("portable.json")).is_ok()
}

fn read_marker(path: &Path) -> Result<Marker, String> {
    let text = fs::read(path).map_err(|e| e.to_string())?;
    let marker: Marker = serde_json::from_slice(&text).map_err(|e| e.to_string())?;
    if marker.app_id != APP_ID || marker.kind != "portable" {
        return Err("Неверный маркер portable".into());
    }
    semver::Version::parse(&marker.version).map_err(|e| e.to_string())?;
    Ok(marker)
}

fn cache(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_cache_dir()
        .map(|path| path.join("updates"))
        .map_err(|e| e.to_string())
}

pub(super) fn last_result(app: &AppHandle) -> Option<String> {
    let path = cache(app).ok()?.join("last-result.json");
    let result: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    result.get("message")?.as_str().map(str::to_owned)
}

pub(super) fn launch(
    app: &AppHandle,
    version: &str,
    signature: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let root = cache(app)?;
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let work = root.join(format!("update-{}-{nonce}", std::process::id()));
    fs::create_dir(&work).map_err(|e| e.to_string())?;
    let executable = dunce::canonicalize(std::env::current_exe().map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    let plan = Plan {
        app_id: APP_ID.into(),
        parent: std::process::id(),
        executable: executable.clone(),
        version: version.into(),
        signature: signature.into(),
    };
    fs::write(work.join("package.zip"), bytes).map_err(|e| e.to_string())?;
    fs::write(work.join("plan.json"), serde_json::to_vec(&plan).unwrap())
        .map_err(|e| e.to_string())?;
    let helper = work.join("update-helper.exe");
    fs::copy(&executable, &helper).map_err(|e| e.to_string())?;
    let mut process = Command::new(helper)
        .arg(HELPER_ARG)
        .creation_flags(0x08000000)
        .current_dir(&work)
        .spawn()
        .map_err(|e| e.to_string())?;
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        if work.join("ready").is_file() {
            return Ok(());
        }
        if process.try_wait().map_err(|e| e.to_string())?.is_some() {
            let reason = fs::read_to_string(work.join("error.txt"))
                .unwrap_or_else(|_| "Помощник обновления завершился до подготовки".into());
            return Err(reason);
        }
        if Instant::now() >= deadline {
            // This is our own child, before handoff. The app and its files stay intact.
            let _ = process.kill();
            let _ = process.wait();
            return Err(
                "Превышено время подготовки обновления; приложение продолжает работать".into(),
            );
        }
        thread::sleep(Duration::from_millis(100));
    }
}

pub fn run_helper_if_requested() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new(HELPER_ARG)) {
        return None;
    }
    let work = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))?;
    let result = apply(&work);
    if let Err(error) = result {
        let _ = fs::write(work.join("error.txt"), &error);
        write_result(&work, &format!("Обновление не установлено: {error}"));
        Some(1)
    } else {
        Some(0)
    }
}

fn verify_package(bytes: &[u8], signature: &str, version: &str) -> Result<(), String> {
    let config: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    let key = config["plugins"]["updater"]["pubkey"]
        .as_str()
        .ok_or("Нет ключа обновлений")?;
    verify_with_key(bytes, signature, version, key)
}

fn verify_with_key(bytes: &[u8], signature: &str, version: &str, key: &str) -> Result<(), String> {
    let decode = |text: &str| -> Result<String, String> {
        String::from_utf8(STANDARD.decode(text).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())
    };
    let key = PublicKey::decode(&decode(key)?).map_err(|e| e.to_string())?;
    let signature = Signature::decode(&decode(signature)?).map_err(|e| e.to_string())?;
    key.verify(bytes, &signature, true)
        .map_err(|e| format!("Подпись пакета: {e}"))?;
    let signed = signature
        .trusted_comment()
        .split('\t')
        .find_map(|part| part.strip_prefix("version:"))
        .ok_or("Подпись не содержит версии")?;
    if semver::Version::parse(signed).map_err(|e| e.to_string())?
        != semver::Version::parse(version).map_err(|e| e.to_string())?
    {
        return Err("Версия пакета не совпадает с подписанной".into());
    }
    Ok(())
}

// ZIP paths use '/' on all platforms. Deny Windows aliases, ADS, device names and links.
fn package_path(name: &str) -> Result<PathBuf, String> {
    let name = name
        .strip_prefix("Voice-Input/")
        .ok_or("В архиве нет корня Voice-Input/")?;
    let mut path = PathBuf::new();
    for part in name.trim_end_matches('/').split('/') {
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with(['.', ' '])
            || part
                .chars()
                .any(|c| c.is_control() || "\\:<>\"|?*".contains(c))
        {
            return Err("Недопустимый путь в обновлении".into());
        }
        let stem = part.split('.').next().unwrap().to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err("Системное имя в архиве обновления".into());
        }
        path.push(part);
    }
    let first = path
        .components()
        .next()
        .ok_or("Пустой путь")?
        .as_os_str()
        .to_string_lossy();
    if first != "models"
        && !matches!(
            first.as_ref(),
            "voice-input.exe"
                | "portable.json"
                | "libvosk.dll"
                | "libgcc_s_seh-1.dll"
                | "libstdc++-6.dll"
                | "libwinpthread-1.dll"
        )
    {
        return Err("В архиве есть файл вне состава portable".into());
    }
    if first != "models" && path.components().count() != 1 {
        return Err("Некорректный путь файла portable".into());
    }
    Ok(path)
}

fn no_links(root: &Path, relative: &Path) -> Result<(), String> {
    let mut path = root.to_path_buf();
    for part in std::iter::once(None).chain(relative.components().map(Some)) {
        if let Some(part) = part {
            path.push(part);
        }
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_attributes() & 0x400 != 0 => {
                return Err("Путь обновления содержит ссылку Windows".into())
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(())
}

fn extract(bytes: &[u8], stage: &Path, version: &str) -> Result<Vec<PathBuf>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if archive.len() > 10000 {
        return Err("Слишком много файлов в архиве".into());
    }
    let mut paths = HashSet::new();
    let mut files = Vec::new();
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
        let name = entry.name().map_err(|e| e.to_string())?;
        if name == "Voice-Input/" && entry.is_dir() {
            continue;
        }
        let path = package_path(&name)?;
        if entry
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err("Ссылка в архиве".into());
        }
        if !paths.insert(path.to_string_lossy().to_lowercase()) {
            return Err("Повтор пути в архиве".into());
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or("Слишком большой архив")?;
        if expanded > MAX_EXPANDED {
            return Err("Распакованное обновление превышает 1 ГБ".into());
        }
        if entry.is_dir() {
            fs::create_dir_all(stage.join(&path)).map_err(|e| e.to_string())?;
            continue;
        }
        let destination = stage.join(&path);
        fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(destination)
            .map_err(|e| e.to_string())?;
        std::io::copy(&mut entry, &mut file).map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        files.push(path);
    }
    for required in [
        "voice-input.exe",
        "portable.json",
        "libvosk.dll",
        "libgcc_s_seh-1.dll",
        "libstdc++-6.dll",
        "libwinpthread-1.dll",
    ] {
        if !files.contains(&PathBuf::from(required)) {
            return Err(format!("Нет обязательного файла {required}"));
        }
    }
    if read_marker(&stage.join("portable.json"))?.version != version {
        return Err("Версия portable.json не совпадает с релизом".into());
    }
    if !files
        .iter()
        .any(|path| path.starts_with("models") && path.ends_with("final.mdl"))
    {
        return Err("В portable-пакете отсутствует модель Vosk".into());
    }
    Ok(files)
}

struct Process(HANDLE);
impl Drop for Process {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
fn parent_process(pid: u32, expected: &Path) -> Result<Process, String> {
    let process = Process(
        unsafe {
            OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE,
                false,
                pid,
            )
        }
        .map_err(|e| e.to_string())?,
    );
    let mut buffer = vec![0_u16; 32768];
    let mut length = buffer.len() as u32;
    unsafe {
        QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        )
    }
    .map_err(|e| e.to_string())?;
    let actual = dunce::canonicalize(PathBuf::from(String::from_utf16_lossy(
        &buffer[..length as usize],
    )))
    .map_err(|e| e.to_string())?;
    if actual != expected {
        return Err("Процесс для обновления не соответствует приложению".into());
    }
    Ok(process)
}

fn write_result(work: &Path, message: &str) {
    if let Some(root) = work.parent() {
        let _ = fs::write(
            root.join("last-result.json"),
            serde_json::to_vec(&serde_json::json!({ "message": message })).unwrap(),
        );
    }
}

fn restart(executable: &Path) -> Result<(), String> {
    Command::new(executable)
        .current_dir(executable.parent().unwrap())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn apply(work: &Path) -> Result<(), String> {
    let plan: Plan =
        serde_json::from_slice(&fs::read(work.join("plan.json")).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    if plan.app_id != APP_ID
        || plan.executable.file_name().and_then(|s| s.to_str()) != Some("voice-input.exe")
    {
        return Err("Некорректный план обновления".into());
    }
    if semver::Version::parse(&plan.version).map_err(|e| e.to_string())?
        <= semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap()
    {
        return Err("Обновление не новее текущей версии".into());
    }
    let target = plan.executable.parent().ok_or("Нет папки приложения")?;
    if !is_portable(target) {
        return Err("Обновляемая папка не является portable".into());
    }
    let parent = parent_process(plan.parent, &plan.executable)?;
    let package = work.join("package.zip");
    if fs::metadata(&package).map_err(|e| e.to_string())?.len() > MAX_PACKAGE {
        return Err("Обновление превышает 512 МБ".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(package)
        .map_err(|e| e.to_string())?
        .take(MAX_PACKAGE + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_PACKAGE {
        return Err("Слишком большой пакет".into());
    }
    verify_package(&bytes, &plan.signature, &plan.version)?;
    let stage = work.join("stage");
    fs::create_dir(&stage).map_err(|e| e.to_string())?;
    let files = extract(&bytes, &stage, &plan.version)?;
    drop(bytes);
    for file in &files {
        no_links(target, file)?;
    }
    // Prove the portable root is writable before asking the original app to exit.
    let probe = target.join(format!(".voice-input-update-{}", std::process::id()));
    fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
        .map_err(|e| format!("Нет прав записи в portable-папку: {e}"))?;
    fs::remove_file(probe).map_err(|e| e.to_string())?;
    fs::write(work.join("ready"), b"ready").map_err(|e| e.to_string())?;
    if unsafe { WaitForSingleObject(parent.0, 120000) } != WAIT_OBJECT_0 {
        return Err("Приложение не завершилось за 120 секунд".into());
    }
    let backup = work.join("backup");
    let mut applied = Vec::new();
    let result = replace_files(target, &stage, &backup, &files, &mut applied).and_then(|_| {
        write_result(work, &format!("Portable обновлена до {}", plan.version));
        restart(&plan.executable)
    });
    if let Err(error) = result {
        let rollback = restore_files(target, &backup, &applied);
        let message = match rollback {
            Ok(()) => format!("{error}. Прежние файлы восстановлены."),
            Err(rollback) => format!(
                "{error}. Не удалось полностью восстановить файлы: {rollback}. Резервная копия: {}",
                backup.display()
            ),
        };
        write_result(work, &format!("Обновление не установлено: {message}"));
        let _ = restart(&plan.executable);
        return Err(message);
    }
    // Keep backups for recovery. They contain application files, never transcripts.
    Ok(())
}

fn replace_files(
    target: &Path,
    stage: &Path,
    backup: &Path,
    files: &[PathBuf],
    applied: &mut Vec<(PathBuf, bool)>,
) -> Result<(), String> {
    for file in files {
        no_links(target, file)?;
        let destination = target.join(file);
        fs::create_dir_all(destination.parent().unwrap()).map_err(|e| e.to_string())?;
        let existed = destination.try_exists().map_err(|e| e.to_string())?;
        if existed {
            let old = backup.join(file);
            fs::create_dir_all(old.parent().unwrap()).map_err(|e| e.to_string())?;
            fs::copy(&destination, old).map_err(|e| e.to_string())?;
        }
        // Copy to the same volume, then replace by rename. Never truncate the old EXE.
        let temporary =
            destination.with_extension(format!("voiceinput-update-{}", std::process::id()));
        if temporary.exists() {
            return Err("Временный файл обновления уже существует".into());
        }
        fs::copy(stage.join(file), &temporary).map_err(|e| e.to_string())?;
        let mut error = None;
        for _ in 0..30 {
            match fs::rename(&temporary, &destination) {
                Ok(()) => {
                    error = None;
                    break;
                }
                Err(e) => {
                    error = Some(e);
                    thread::sleep(Duration::from_millis(100));
                }
            }
        }
        if let Some(error) = error {
            let _ = fs::remove_file(temporary);
            return Err(error.to_string());
        }
        applied.push((file.clone(), existed));
    }
    Ok(())
}

fn restore_files(target: &Path, backup: &Path, applied: &[(PathBuf, bool)]) -> Result<(), String> {
    let mut errors = Vec::new();
    for (file, existed) in applied.iter().rev() {
        let result = if *existed {
            fs::copy(backup.join(file), target.join(file)).map(|_| ())
        } else {
            match fs::remove_file(target.join(file)) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                other => other,
            }
        };
        if let Err(e) = result {
            errors.push(format!("{}: {e}", file.display()));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_reject_tampering_and_a_false_announced_version() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/update-signature.json"))
                .unwrap();
        let payload = fixture["payload"].as_str().unwrap().as_bytes();
        let signature = fixture["signature"].as_str().unwrap();
        let public_key = fixture["public_key"].as_str().unwrap();
        assert!(verify_with_key(payload, signature, "0.2.1", public_key).is_ok());
        assert!(verify_with_key(b"Modified bytes", signature, "0.2.1", public_key).is_err());
        assert!(verify_with_key(payload, signature, "0.3.0", public_key).is_err());
        assert!(verify_with_key(payload, "not a signature", "0.2.1", public_key).is_err());
    }
    #[test]
    fn rejects_zip_slip_and_windows_aliases() {
        for path in [
            "../voice-input.exe",
            "Voice-Input/../voice-input.exe",
            "Voice-Input/models/../../secret",
            "Voice-Input/models/C:/data",
            "Voice-Input/models/test.",
            "Voice-Input/models/CON.txt",
            "Voice-Input/models/a\\b",
            "Voice-Input/other.exe",
        ] {
            assert!(package_path(path).is_err(), "{path}");
        }
        assert_eq!(
            package_path("Voice-Input/models/ru/am/final.mdl").unwrap(),
            PathBuf::from("models/ru/am/final.mdl")
        );
    }
    #[test]
    fn rollback_restores_existing_files_and_removes_new_ones() {
        let work = std::env::temp_dir().join(format!(
            "voice-input-rollback-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let target = work.join("target");
        let stage = work.join("stage");
        let backup = work.join("backup");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::write(target.join("voice-input.exe"), b"old").unwrap();
        fs::write(target.join("personal.txt"), b"keep").unwrap();
        fs::write(stage.join("voice-input.exe"), b"new").unwrap();
        fs::write(stage.join("libvosk.dll"), b"new dll").unwrap();
        let mut applied = Vec::new();
        assert!(replace_files(
            &target,
            &stage,
            &backup,
            &[
                "voice-input.exe".into(),
                "libvosk.dll".into(),
                "missing".into()
            ],
            &mut applied
        )
        .is_err());
        restore_files(&target, &backup, &applied).unwrap();
        assert_eq!(fs::read(target.join("voice-input.exe")).unwrap(), b"old");
        assert!(!target.join("libvosk.dll").exists());
        assert_eq!(fs::read(target.join("personal.txt")).unwrap(), b"keep");
        fs::remove_dir_all(work).unwrap();
    }
}
