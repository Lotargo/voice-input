use super::{has_writable_text, UIAutomation};
use std::{sync::mpsc, thread, time::Duration};
use windows::{
    core::w,
    Win32::{
        Foundation::{HWND, LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            CreateWindowExW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW,
            ES_READONLY, MSG, WINDOW_STYLE, WM_APP, WS_CHILD, WS_POPUP,
        },
    },
};

struct NativeEdits {
    parent: isize,
    writable: isize,
    readonly: isize,
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
                tx.send((parent.0 as isize, writable.0 as isize, readonly.0 as isize))
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
        let (parent, writable, readonly) = rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            parent,
            writable,
            readonly,
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
