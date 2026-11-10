//! Best-effort, at-most-once keyboard delivery. No system clipboard modification.
//! Windows cannot prove that a foreign editor accepted an input sequence; uncertain
//! sends are NEVER automatically retried.
use std::{
    collections::{HashSet, VecDeque},
    sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}},
    thread,
    time::{Duration, Instant},
};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use uiautomation::{core::{UIAutomation, UIElement}, types::{ControlType, Point}};
use windows::Win32::{
    Foundation::POINT,
    UI::{
        Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
                                     KEYEVENTF_UNICODE, SendInput, VIRTUAL_KEY},
        WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId},
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct PendingText {
    pub id: String,
    pub text: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct DeliveryStatus {
    pub pending: Vec<PendingText>,
    pub uncertain: Vec<PendingText>,
    pub inserted_segments: u64,
    pub hover_enabled: bool,
    pub last_message: String,
}

struct QueueState {
    pending: VecDeque<PendingText>,
    uncertain: VecDeque<PendingText>,
    seen: HashSet<String>,
    inserted_segments: u64,
    last_message: String,
    last_session: Option<u64>,
}

pub struct Delivery {
    inner: Mutex<QueueState>,
    hover_enabled: AtomicBool,
}

impl Delivery {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(QueueState {
                pending: VecDeque::new(), uncertain: VecDeque::new(),
                seen: HashSet::new(), inserted_segments: 0,
                last_message: "Готов к диктовке".into(), last_session: None,
            }),
            hover_enabled: AtomicBool::new(true),
        }
    }

    pub fn enqueue(&self, app: &AppHandle, session: u64, segment: u32, text: String) -> bool {
        let text = text.trim().to_owned();
        if text.is_empty() { return false; }
        let id = format!("{session}:{segment}");
        {
            let mut state = self.inner.lock().unwrap();
            if !state.seen.insert(id.clone()) { return false; }
            // Insert a separator between STT endpoint chunks within the same session.
            let prefix = if state.last_session == Some(session) { " " } else { "" };
            state.last_session = Some(session);
            state.pending.push_back(PendingText { id, text: format!("{prefix}{text}") });
            state.last_message = "Фрагмент ожидает подходящее поле ввода".into();
        }
        self.notify(app);
        true
    }

    pub fn snapshot(&self) -> DeliveryStatus {
        let state = self.inner.lock().unwrap();
        DeliveryStatus {
            pending: state.pending.iter().cloned().collect(),
            uncertain: state.uncertain.iter().cloned().collect(),
            inserted_segments: state.inserted_segments,
            hover_enabled: self.hover_enabled.load(Ordering::Relaxed),
            last_message: state.last_message.clone(),
        }
    }

    pub fn set_hover(&self, app: &AppHandle, enabled: bool) {
        self.hover_enabled.store(enabled, Ordering::Release);
        self.notify(app);
    }

    pub fn discard(&self, app: &AppHandle, id: &str) {
        let mut state = self.inner.lock().unwrap();
        state.pending.retain(|p| p.id != id);
        state.uncertain.retain(|p| p.id != id);
        state.last_message = "Фрагмент удалён пользователем".into();
        drop(state);
        self.notify(app);
    }

    fn notify(&self, app: &AppHandle) {
        let _ = app.emit("delivery-status", self.snapshot());
    }

    fn has_pending(&self) -> bool {
        !self.inner.lock().unwrap().pending.is_empty()
    }

    fn pop_attempt(&self) -> Option<PendingText> {
        // Remove BEFORE trying SendInput. A crash/callback race cannot trigger a second
        // automatic send for the same segment in this process.
        self.inner.lock().unwrap().pending.pop_front()
    }

    fn finish_attempt(&self, app: &AppHandle, text: PendingText, result: Result<(), String>) {
        let mut state = self.inner.lock().unwrap();
        match result {
            Ok(()) => {
                state.inserted_segments += 1;
                state.last_message = "Ввод передан Windows; отправьте сообщение самостоятельно".into();
            }
            Err(err) => {
                state.uncertain.push_back(text);
                state.last_message = format!("Автоввод не подтверждён: {err}. Автоповтора не будет");
            }
        }
        drop(state);
        self.notify(app);
    }

    fn deliver_one(&self, app: &AppHandle) {
        let Some(item) = self.pop_attempt() else { return };
        let result = send_unicode(&item.text);
        self.finish_attempt(app, item, result);
    }

    pub fn spawn_monitor(self: Arc<Self>, app: AppHandle) {
        thread::spawn(move || {
            let automation = match UIAutomation::new() {
                Ok(value) => value,
                Err(err) => {
                    let mut inner = self.inner.lock().unwrap();
                    inner.last_message = format!("Windows UI Automation недоступен: {err}");
                    drop(inner);
                    self.notify(&app);
                    return;
                }
            };
            let mut hover: Option<(Vec<i32>, Instant)> = None;
            loop {
                thread::sleep(Duration::from_millis(180));
                if !self.has_pending() {
                    hover = None;
                    continue;
                }
                // Priority: an already focused editable control. Hover only if none exists.
                if let Ok(target) = automation.get_focused_element() {
                    if editable(&target) {
                        if let Ok(verify) = automation.get_focused_element() {
                            if automation.compare_elements(&target, &verify).unwrap_or(false) {
                                self.deliver_one(&app);
                                hover = None;
                                continue;
                            }
                        }
                    }
                }
                if !self.hover_enabled.load(Ordering::Acquire) {
                    hover = None;
                    continue;
                }
                let Ok(point) = mouse_position() else { continue };
                let Ok(target) = automation.element_from_point(point) else { hover = None; continue };
                if !editable(&target) { hover = None; continue; }
                let Ok(id) = target.get_runtime_id() else { hover = None; continue };
                let ready = match &hover {
                    Some((previous, started)) if *previous == id => started.elapsed() >= Duration::from_millis(700),
                    _ => { hover = Some((id, Instant::now())); false }
                };
                if !ready { continue; }
                // UIA set_focus can fail on browser contenteditables. Never send if it does.
                if target.set_focus().is_ok() {
                    thread::sleep(Duration::from_millis(60));
                    if let Ok(focused) = automation.get_focused_element() {
                        if editable(&focused) && automation.compare_elements(&target, &focused).unwrap_or(false) {
                            self.deliver_one(&app);
                        }
                    }
                }
                hover = None;
            }
        });
    }
}

fn our_window_is_foreground() -> bool {
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() { return false; }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == std::process::id()
}

fn editable(element: &UIElement) -> bool {
    // WebView2 renderers run in a different process; excluding the root Tauri
    // process ID alone would accidentally allow injection into our own settings.
    if our_window_is_foreground() { return false; }
    if element.get_process_id().unwrap_or(-1) == std::process::id() as i32 { return false; }
    matches!(element.get_control_type(), Ok(ControlType::Edit | ControlType::Document))
        && element.is_enabled().unwrap_or(false)
        && !element.is_password().unwrap_or(true)
        && !element.is_offscreen().unwrap_or(true)
        && element.is_keyboard_focusable().unwrap_or(false)
}

fn mouse_position() -> Result<Point, String> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point).map_err(|e| e.to_string())?; }
    Ok(Point::new(point.x, point.y))
}

fn send_unicode(text: &str) -> Result<(), String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.is_empty() { return Ok(()); }
    // Windows receives paired keydown/keyup Unicode VK_PACKET events. No Ctrl+V,
    // clipboard replacement, Enter, mouse click, or submission is performed.
    let mut inputs = Vec::with_capacity(units.len() * 2);
    for unit in units {
        for released in [false, true] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 { ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0), wScan: unit,
                    dwFlags: if released { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE },
                    time: 0, dwExtraInfo: 0,
                } },
            });
        }
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!("SendInput обработал {sent}/{} событий; результат неопределён", inputs.len()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_queue() {
        let d = Delivery::new();
        assert!(!d.has_pending());
        assert_eq!(d.snapshot().inserted_segments, 0);
    }
}
