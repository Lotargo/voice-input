//! Best-effort, at-most-once keyboard delivery. No system clipboard modification.
//! Windows cannot prove that a foreign editor accepted an input sequence; uncertain
//! sends are NEVER automatically retried.
use serde::Serialize;
use std::{
    collections::{HashSet, VecDeque},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter};
use uiautomation::{
    core::{UIAutomation, UIElement},
    patterns::{UITextPattern, UIValuePattern},
    types::{ControlType, Point, TextAttribute},
    variants::Value,
};
use windows::Win32::{
    Foundation::POINT,
    UI::{
        Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
            KEYEVENTF_UNICODE, VIRTUAL_KEY,
        },
        WindowsAndMessaging::{GetCursorPos, GetForegroundWindow, GetWindowThreadProcessId},
    },
};

#[derive(Clone, Debug, Serialize)]
pub struct PendingText {
    pub id: String,
    pub text: String,
    pub record_id: u64,
    pub generation: u64,
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
    last_record: Option<(u64, u64)>,
}

pub struct Delivery {
    inner: Mutex<QueueState>,
    hover_enabled: AtomicBool,
    stopped: AtomicBool,
}

impl Delivery {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(QueueState {
                pending: VecDeque::new(),
                uncertain: VecDeque::new(),
                seen: HashSet::new(),
                inserted_segments: 0,
                last_message: "Готов к диктовке".into(),
                last_session: None,
                last_record: None,
            }),
            hover_enabled: AtomicBool::new(true),
            stopped: AtomicBool::new(false),
        }
    }

    pub fn shutdown(&self) {
        self.stopped.store(true, Ordering::Release);
    }

    pub fn enqueue_record(
        &self,
        app: &AppHandle,
        session: u64,
        segment: u32,
        record_id: u64,
        generation: u64,
        text: String,
    ) -> bool {
        let inserted = self.enqueue_record_pending(session, segment, record_id, generation, text);
        if inserted {
            self.notify(app);
        }
        inserted
    }

    #[cfg(test)]
    fn enqueue_pending(&self, session: u64, segment: u32, text: String) -> bool {
        self.enqueue_record_pending(session, segment, session, 0, text)
    }

    fn enqueue_record_pending(
        &self,
        session: u64,
        segment: u32,
        record_id: u64,
        generation: u64,
        text: String,
    ) -> bool {
        let text = text.trim().to_owned();
        if text.is_empty() {
            return false;
        }
        let id = format!("{session}:{segment}");
        {
            let mut state = self.inner.lock().unwrap();
            if !state.seen.insert(id.clone()) {
                return false;
            }
            // A completed phrase after a pause starts a new line, including when
            // the microphone resumes the same logical record in a later session.
            let prefix = if state.last_session == Some(session)
                || state.last_record == Some((record_id, generation))
            {
                "\n"
            } else {
                ""
            };
            state.last_session = Some(session);
            state.last_record = Some((record_id, generation));
            state.pending.push_back(PendingText {
                id,
                text: format!("{prefix}{text}"),
                record_id,
                generation,
            });
            state.last_message = "Фрагмент ожидает подходящее поле ввода".into();
        }
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

    pub fn take_pending(&self, app: &AppHandle, ids: &[String]) -> usize {
        let count = self.take_pending_inner(ids);
        if count > 0 {
            self.notify(app);
        }
        count
    }

    fn take_pending_inner(&self, ids: &[String]) -> usize {
        let ids: HashSet<&str> = ids.iter().map(String::as_str).collect();
        let mut state = self.inner.lock().unwrap();
        let before = state.pending.len();
        state.pending.retain(|item| !ids.contains(item.id.as_str()));
        let count = before - state.pending.len();
        if count > 0 {
            state.last_message = "Текст оставлен для ручной правки и вставки".into();
        }
        // Keep `seen` and uncertain attempts intact: a handover never authorizes
        // another automatic attempt, even when copying to the clipboard fails.
        count
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
        self.record_attempt(text, result);
        self.notify(app);
    }

    fn record_attempt(&self, text: PendingText, result: Result<(), String>) {
        let mut state = self.inner.lock().unwrap();
        match result {
            Ok(()) => {
                state.inserted_segments += 1;
                state.last_message =
                    "Ввод передан Windows; отправьте сообщение самостоятельно".into();
            }
            Err(err) => {
                state.uncertain.push_back(text);
                state.last_message =
                    format!("Автоввод не подтверждён: {err}. Автоповтора не будет");
            }
        }
    }

    fn deliver_one(&self, app: &AppHandle, automation: &UIAutomation, target: &UIElement) {
        if self.stopped.load(Ordering::Acquire) {
            return;
        }
        // Revalidate as close to the attempt as possible. If the field became
        // unavailable before dequeueing, its text still belongs in the queue.
        let Ok(focused) = automation.get_focused_element() else {
            return;
        };
        if !editable(&focused)
            || !automation
                .compare_elements(target, &focused)
                .unwrap_or(false)
        {
            return;
        }
        let Some(item) = self.pop_attempt() else {
            return;
        };
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
            while !self.stopped.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(180));
                if !self.has_pending() {
                    hover = None;
                    continue;
                }
                // Priority: an already focused editable control. Hover only if none exists.
                if let Ok(target) = automation.get_focused_element() {
                    if editable(&target) {
                        if let Ok(verify) = automation.get_focused_element() {
                            if automation
                                .compare_elements(&target, &verify)
                                .unwrap_or(false)
                            {
                                self.deliver_one(&app, &automation, &target);
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
                let Ok(point) = mouse_position() else {
                    continue;
                };
                let Ok(target) = automation.element_from_point(point) else {
                    hover = None;
                    continue;
                };
                if !editable(&target) {
                    hover = None;
                    continue;
                }
                let Ok(id) = target.get_runtime_id() else {
                    hover = None;
                    continue;
                };
                let ready = match &hover {
                    Some((previous, started)) if *previous == id => {
                        started.elapsed() >= Duration::from_millis(700)
                    }
                    _ => {
                        hover = Some((id, Instant::now()));
                        false
                    }
                };
                if !ready {
                    continue;
                }
                // UIA set_focus can fail on browser contenteditables. Never send if it does.
                if target.set_focus().is_ok() {
                    thread::sleep(Duration::from_millis(60));
                    if let Ok(focused) = automation.get_focused_element() {
                        if editable(&focused)
                            && automation
                                .compare_elements(&target, &focused)
                                .unwrap_or(false)
                        {
                            self.deliver_one(&app, &automation, &target);
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
    if hwnd.0.is_null() {
        return false;
    }
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == std::process::id()
}

fn editable(element: &UIElement) -> bool {
    // WebView2 renderers run in a different process; excluding the root Tauri
    // process ID alone would accidentally allow injection into our own settings.
    if our_window_is_foreground() {
        return false;
    }
    let Ok(process_id) = element.get_process_id() else {
        return false;
    };
    if process_id == std::process::id() {
        return false;
    }
    matches!(
        element.get_control_type(),
        Ok(ControlType::Edit | ControlType::Document)
    ) && element.is_enabled().unwrap_or(false)
        && !element.is_password().unwrap_or(true)
        && !element.is_offscreen().unwrap_or(true)
        && element.is_keyboard_focusable().unwrap_or(false)
        && has_writable_text(element)
}

fn has_writable_text(element: &UIElement) -> bool {
    if let Ok(value) = element.get_pattern::<UIValuePattern>() {
        return matches!(value.is_readonly(), Ok(false));
    }
    // Rich editors often expose TextPattern without ValuePattern. The presence
    // of TextPattern or a Document control alone does not prove editability.
    let Ok(text) = element.get_pattern::<UITextPattern>() else {
        return false;
    };
    let Ok(range) = text.get_document_range() else {
        return false;
    };
    let Ok(readonly) = range.get_attribute_value(TextAttribute::IsReadOnly) else {
        return false;
    };
    matches!(readonly.get_value(), Ok(Value::BOOL(false)))
}

fn mouse_position() -> Result<Point, String> {
    let mut point = POINT::default();
    unsafe {
        GetCursorPos(&mut point).map_err(|e| e.to_string())?;
    }
    Ok(Point::new(point.x, point.y))
}

fn unicode_inputs(text: &str) -> Vec<INPUT> {
    // Windows receives paired keydown/keyup Unicode VK_PACKET events. No Ctrl+V,
    // clipboard replacement, Enter, mouse click, or submission is performed.
    // Line feeds stay Unicode text; do not turn them into VK_RETURN or CR.
    let mut inputs = Vec::with_capacity(text.len() * 2);
    for unit in text.encode_utf16() {
        for released in [false, true] {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: unit,
                        dwFlags: if released {
                            KEYEVENTF_UNICODE | KEYEVENTF_KEYUP
                        } else {
                            KEYEVENTF_UNICODE
                        },
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    inputs
}

fn send_unicode(text: &str) -> Result<(), String> {
    let inputs = unicode_inputs(text);
    if inputs.is_empty() {
        return Ok(());
    }
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == inputs.len() {
        Ok(())
    } else {
        Err(format!(
            "SendInput обработал {sent}/{} событий; результат неопределён",
            inputs.len()
        ))
    }
}

#[cfg(test)]
mod native_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_queue() {
        let d = Delivery::new();
        assert!(!d.has_pending());
        assert_eq!(d.snapshot().inserted_segments, 0);
    }

    #[test]
    fn identical_text_with_distinct_segment_ids_is_not_dropped() {
        let d = Delivery::new();
        assert!(d.enqueue_pending(1, 1, "да".into()));
        assert!(d.enqueue_pending(1, 2, "да".into()));
        assert!(d.enqueue_pending(2, 1, "да".into()));
        let text: Vec<_> = d.snapshot().pending.into_iter().map(|p| p.text).collect();
        assert_eq!(text, ["да", "\nда", "да"]);
        assert!(!d.enqueue_pending(1, 1, "да".into()));
    }

    #[test]
    fn uncertain_attempt_is_removed_and_never_requeued() {
        let d = Delivery::new();
        assert!(d.enqueue_pending(1, 1, "текст".into()));
        let item = d.pop_attempt().unwrap();
        assert!(!d.has_pending());
        d.record_attempt(item, Err("partial SendInput".into()));
        assert!(!d.enqueue_pending(1, 1, "текст".into()));
        assert!(d.pop_attempt().is_none());
        assert_eq!(d.snapshot().uncertain.len(), 1);
        assert_eq!(d.snapshot().inserted_segments, 0);
    }

    #[test]
    fn resumed_record_queues_the_same_line_breaks_after_an_attempt_and_a_new_session() {
        let d = Delivery::new();
        assert!(d.enqueue_record_pending(1, 1, 7, 0, "я тут подумал".into()));
        let first = d.pop_attempt().unwrap();
        assert_eq!(first.text, "я тут подумал");
        d.record_attempt(first, Ok(()));
        assert!(d.enqueue_record_pending(1, 2, 7, 0, "надо бы сходить прогуляться".into()));
        assert!(d.enqueue_record_pending(2, 1, 7, 0, "после включения микрофона".into()));
        let text: String = d
            .snapshot()
            .pending
            .into_iter()
            .map(|part| part.text)
            .collect();
        assert_eq!(
            text,
            "\nнадо бы сходить прогуляться\nпосле включения микрофона"
        );
    }

    #[test]
    fn manual_handover_removes_only_selected_pending_ids_and_keeps_them_seen() {
        let d = Delivery::new();
        assert!(d.enqueue_pending(1, 1, "первый".into()));
        assert!(d.enqueue_pending(2, 1, "второй".into()));
        assert_eq!(d.take_pending_inner(&["1:1".into(), "missing".into()]), 1);
        assert!(!d.enqueue_pending(1, 1, "исправленный".into()));
        assert_eq!(d.pop_attempt().unwrap().id, "2:1");
        assert!(d.pop_attempt().is_none());
    }

    #[test]
    fn manual_handover_never_changes_uncertain_attempts() {
        let d = Delivery::new();
        assert!(d.enqueue_pending(1, 1, "первый".into()));
        let item = d.pop_attempt().unwrap();
        d.record_attempt(item, Err("partial SendInput".into()));
        assert_eq!(d.take_pending_inner(&["1:1".into()]), 0);
        assert_eq!(d.snapshot().uncertain.len(), 1);
        assert!(!d.enqueue_pending(1, 1, "исправленный".into()));
        assert!(d.pop_attempt().is_none());
    }

    #[test]
    fn simultaneous_consumers_can_only_attempt_a_segment_once() {
        let d = Arc::new(Delivery::new());
        assert!(d.enqueue_pending(1, 1, "текст".into()));
        let consumers: Vec<_> = (0..8)
            .map(|_| {
                let d = d.clone();
                thread::spawn(move || d.pop_attempt().is_some())
            })
            .collect();
        let attempts = consumers
            .into_iter()
            .map(|c| c.join().unwrap() as usize)
            .sum::<usize>();
        assert_eq!(attempts, 1);
        assert!(!d.enqueue_pending(1, 1, "текст".into()));
    }
}
