use super::*;
use std::{collections::HashSet, sync::atomic::AtomicUsize};

#[derive(Default)]
struct FakeRegistrar {
    registered: Mutex<HashSet<Shortcut>>,
    blocked: Mutex<HashSet<Shortcut>>,
    fail_unregister: AtomicBool,
    calls: AtomicUsize,
}

impl Registrar for FakeRegistrar {
    fn register(&self, shortcut: Shortcut) -> Result<(), String> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        if self.blocked.lock().unwrap().contains(&shortcut) {
            return Err("already registered by another process".into());
        }
        self.registered.lock().unwrap().insert(shortcut);
        Ok(())
    }
    fn unregister(&self, shortcut: Shortcut) -> Result<(), String> {
        if self.fail_unregister.load(Ordering::Relaxed) {
            return Err("unregister failed".into());
        }
        self.registered.lock().unwrap().remove(&shortcut);
        Ok(())
    }
}

fn service() -> (Hotkeys, FakeRegistrar) {
    let keys = Hotkeys::load(Err("test has no config path".into()));
    let registrar = FakeRegistrar::default();
    keys.start_with(&registrar);
    (keys, registrar)
}

#[test]
fn validates_combinations_and_round_trips_windows_labels() {
    for value in [
        "Ctrl+Alt+Space",
        "Alt+Shift+R",
        "Win+Ctrl+9",
        "F8",
        "Ctrl+PageDown",
    ] {
        let key = parse(value).unwrap();
        assert_eq!(key, parse(&label(key)).unwrap());
    }
    assert_eq!(
        parse(" ctrl + shift + f9 ").unwrap(),
        parse("Shift+Control+F9").unwrap()
    );
    for invalid in [
        "", "R", "Shift+R", "Ctrl", "Ctrl+R+T", "Ctrl++R", "Ctrl+F99", "Ctrl+Р",
    ] {
        assert!(parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn conflict_preserves_old_registration_and_does_not_persist() {
    let (keys, registrar) = service();
    let old = parse(DEFAULT_SHORTCUT).unwrap();
    registrar
        .blocked
        .lock()
        .unwrap()
        .insert(parse("Ctrl+Shift+F9").unwrap());
    let result = keys.set_with(&registrar, "Ctrl+Shift+F9", |_| {
        panic!("must not persist a conflict")
    });
    assert!(result.unwrap_err().contains("занято"));
    assert!(registrar.registered.lock().unwrap().contains(&old));
    assert_eq!(keys.snapshot().shortcut, DEFAULT_SHORTCUT);
    assert!(matches!(
        keys.event(old, ShortcutState::Pressed),
        Action::Toggle
    ));
}

#[test]
fn persistence_failure_removes_candidate_and_keeps_old_binding() {
    let (keys, registrar) = service();
    assert!(keys
        .set_with(&registrar, "Ctrl+Shift+F9", |_| Err("disk failure".into()))
        .is_err());
    assert_eq!(
        *registrar.registered.lock().unwrap(),
        HashSet::from([parse(DEFAULT_SHORTCUT).unwrap()])
    );
    assert_eq!(keys.snapshot().shortcut, DEFAULT_SHORTCUT);
    assert!(keys.snapshot().active);
}

#[test]
fn successful_rebind_releases_old_and_ignores_staged_and_retired_events() {
    let (keys, registrar) = service();
    let old = parse(DEFAULT_SHORTCUT).unwrap();
    let next = parse("Ctrl+Shift+F9").unwrap();
    keys.set_with(&registrar, "Ctrl+Shift+F9", |candidate| {
        assert_eq!(candidate, next);
        assert!(matches!(
            keys.event(candidate, ShortcutState::Pressed),
            Action::Ignore
        ));
        Ok(())
    })
    .unwrap();
    assert_eq!(*registrar.registered.lock().unwrap(), HashSet::from([next]));
    assert!(matches!(
        keys.event(old, ShortcutState::Pressed),
        Action::Ignore
    ));
    assert!(matches!(
        keys.event(next, ShortcutState::Pressed),
        Action::Toggle
    ));
    assert!(matches!(
        keys.event(next, ShortcutState::Pressed),
        Action::Ignore
    ));
    assert!(matches!(
        keys.event(next, ShortcutState::Released),
        Action::Ignore
    ));
    assert!(matches!(
        keys.event(next, ShortcutState::Pressed),
        Action::Toggle
    ));
}

#[test]
fn capturing_current_binding_does_not_toggle_or_repeat_after_capture_ends() {
    let (keys, _) = service();
    let shortcut = parse(DEFAULT_SHORTCUT).unwrap();
    keys.set_capture(true);
    assert!(matches!(
        keys.event(shortcut, ShortcutState::Pressed),
        Action::Capture
    ));
    keys.set_capture(false);
    assert!(matches!(
        keys.event(shortcut, ShortcutState::Pressed),
        Action::Ignore
    ));
    keys.event(shortcut, ShortcutState::Released);
    assert!(matches!(
        keys.event(shortcut, ShortcutState::Pressed),
        Action::Toggle
    ));
}

#[test]
fn unavailable_startup_binding_can_be_replaced_without_stopping_the_app() {
    let keys = Hotkeys::load(Err("no path".into()));
    let registrar = FakeRegistrar::default();
    let old = parse(DEFAULT_SHORTCUT).unwrap();
    registrar.blocked.lock().unwrap().insert(old);
    keys.start_with(&registrar);
    assert!(!keys.snapshot().active);
    assert!(keys.snapshot().error.unwrap().contains("недоступно"));
    assert!(matches!(
        keys.event(old, ShortcutState::Pressed),
        Action::Ignore
    ));
    let view = keys
        .set_with(&registrar, "Ctrl+Shift+F9", |_| Ok(()))
        .unwrap();
    assert!(view.active);
    assert!(view.error.is_none());
}

#[test]
fn applying_same_binding_is_idempotent_and_still_persists() {
    let (keys, registrar) = service();
    let calls = registrar.calls.load(Ordering::Relaxed);
    let mut persisted = false;
    keys.set_with(&registrar, DEFAULT_SHORTCUT, |_| {
        persisted = true;
        Ok(())
    })
    .unwrap();
    assert!(persisted);
    assert_eq!(registrar.calls.load(Ordering::Relaxed), calls);
}

#[test]
fn retired_shortcut_never_toggles_when_os_cleanup_fails() {
    let (keys, registrar) = service();
    registrar.fail_unregister.store(true, Ordering::Relaxed);
    let view = keys
        .set_with(&registrar, "Ctrl+Shift+F9", |_| Ok(()))
        .unwrap();
    assert!(view.active);
    assert!(view.error.is_some());
    assert!(matches!(
        keys.event(parse(DEFAULT_SHORTCUT).unwrap(), ShortcutState::Pressed),
        Action::Ignore
    ));
    assert!(matches!(
        keys.event(parse("Ctrl+Shift+F9").unwrap(), ShortcutState::Pressed),
        Action::Toggle
    ));
}

#[test]
fn atomic_save_replaces_existing_config_and_reload_restores_binding() {
    let directory = std::env::temp_dir().join(format!(
        "voice-input-hotkey-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let path = directory.join("hotkey.json");
    let keys = Hotkeys::load(Ok(path.clone()));
    assert_eq!(keys.snapshot().shortcut, DEFAULT_SHORTCUT);
    assert!(keys.snapshot().error.is_none());
    save(&path, parse("Ctrl+Shift+F9").unwrap()).unwrap();
    assert_eq!(
        Hotkeys::load(Ok(path.clone())).snapshot().shortcut,
        "Ctrl+Shift+F9"
    );
    save(&path, parse(DEFAULT_SHORTCUT).unwrap()).unwrap();
    assert_eq!(
        Hotkeys::load(Ok(path.clone())).snapshot().shortcut,
        DEFAULT_SHORTCUT
    );
    fs::write(&path, "{broken}").unwrap();
    let recovered = Hotkeys::load(Ok(path.clone())).snapshot();
    assert_eq!(recovered.shortcut, DEFAULT_SHORTCUT);
    assert!(recovered.error.is_some());
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
    fs::remove_file(path).unwrap();
    fs::remove_dir(directory).unwrap();
}
