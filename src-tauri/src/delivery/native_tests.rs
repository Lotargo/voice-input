use super::{
    has_writable_text, unicode_inputs, Delivery, UIAutomation, KEYEVENTF_KEYUP, KEYEVENTF_UNICODE,
};
use std::{sync::mpsc, thread, time::Duration};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, GetWindowTextW,
            PostMessageW, SendMessageW, SetWindowTextW, ES_MULTILINE, ES_READONLY, MSG,
            WINDOW_STYLE, WM_APP, WM_CHAR, WS_CHILD, WS_POPUP,
        },
    },
};

struct NativeEdits {
    parent: isize,
    writable: isize,
    readonly: isize,
    multiline: isize,
    thread: Option<thread::JoinHandle<()>>,
}

impl NativeEdits {
    fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = thread::spawn(move || {
            // Hidden native controls: no focus changes, keystrokes or clipboard access.
            unsafe {
                let parent = CreateWindowExW(
                    Default::default(),
                    w!("STATIC"),
                    w!("Voice Input test"),
                    WS_POPUP,
                    0,
                    0,
                    400,
                    200,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
                let create_edit = |extra| {
                    CreateWindowExW(
                        Default::default(),
                        w!("EDIT"),
                        w!("test"),
                        WS_CHILD | WINDOW_STYLE(extra),
                        0,
                        0,
                        300,
                        40,
                        Some(parent),
                        None,
                        None,
                        None,
                    )
                    .unwrap()
                };
                let writable = create_edit(0);
                let readonly = create_edit(ES_READONLY as u32);
                let multiline = create_edit(ES_MULTILINE as u32);
                SetWindowTextW(multiline, w!("")).unwrap();
                tx.send((
                    parent.0 as isize,
                    writable.0 as isize,
                    readonly.0 as isize,
                    multiline.0 as isize,
                ))
                .unwrap();
                let mut message = MSG::default();
                while GetMessageW(&mut message, None, 0, 0).0 > 0 {
                    if message.message == WM_APP {
                        break;
                    }
                    DispatchMessageW(&message);
                }
                DestroyWindow(parent).unwrap();
            }
        });
        let (parent, writable, readonly, multiline) =
            rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            parent,
            writable,
            readonly,
            multiline,
            thread: Some(thread),
        }
    }
}

impl Drop for NativeEdits {
    fn drop(&mut self) {
        unsafe {
            let _ = PostMessageW(Some(HWND(self.parent as _)), WM_APP, WPARAM(0), LPARAM(0));
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn native_readonly_controls_are_not_writable_targets() {
    let controls = NativeEdits::new();
    let automation = UIAutomation::new().unwrap();
    let element = |handle: isize| {
        automation
            .element_from_handle(HWND(handle as _).into())
            .unwrap()
    };
    assert!(has_writable_text(&element(controls.writable)));
    assert!(!has_writable_text(&element(controls.readonly)));
    assert!(!has_writable_text(&element(controls.parent)));
}

#[test]
fn queued_phrases_insert_native_lines_with_unicode_characters_and_no_enter_key() {
    let controls = NativeEdits::new();
    let delivery = Delivery::new();
    delivery.enqueue_pending(1, 1, "Я тут подумал".into());
    delivery.enqueue_pending(1, 2, "надо бы сходить прогуляться".into());
    let text: String = delivery
        .snapshot()
        .pending
        .into_iter()
        .map(|part| part.text)
        .collect();
    assert_eq!(text, "Я тут подумал\nнадо бы сходить прогуляться");
    for input in unicode_inputs(&text) {
        let key = unsafe { input.Anonymous.ki };
        assert_eq!(
            key.wVk.0, 0,
            "no virtual key, especially VK_RETURN, is sent"
        );
        assert!(key.dwFlags.contains(KEYEVENTF_UNICODE));
        assert_ne!(key.wScan, 13, "line feeds must not become Enter characters");
        if !key.dwFlags.contains(KEYEVENTF_KEYUP) {
            // Replay the WM_CHAR produced by a Unicode packet into our hidden
            // control. No real focus, SendInput or system clipboard is used.
            unsafe {
                SendMessageW(
                    HWND(controls.multiline as _),
                    WM_CHAR,
                    Some(WPARAM(key.wScan as usize)),
                    Some(LPARAM(0)),
                );
            }
        }
    }
    let mut buffer = [0u16; 256];
    let length = unsafe { GetWindowTextW(HWND(controls.multiline as _), &mut buffer) };
    assert_eq!(
        String::from_utf16(&buffer[..length as usize]).unwrap(),
        text.replace('\n', "\r\n")
    );
}
