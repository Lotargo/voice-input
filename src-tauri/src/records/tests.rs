use super::*;

#[test]
fn endpoints_and_microphone_sessions_append_to_one_logical_record() {
    let mut records = Records::new();
    for (id, text) in [
        ("1:1", "первая фраза"),
        ("1:2", "после паузы"),
        ("2:1", "после включения микрофона"),
    ] {
        assert!(records.append(id.into(), text.into()));
    }
    let snapshot = records.snapshot();
    assert_eq!(snapshot.current.id, 1);
    assert_eq!(
        snapshot.current.text,
        "первая фраза после паузы после включения микрофона"
    );
    assert_eq!(snapshot.current.fragments.len(), 3);
    assert!(snapshot.history.is_empty());
}

#[test]
fn new_record_keeps_completed_text_and_clear_only_resets_current_generation() {
    let mut records = Records::new();
    records.append("1:1".into(), "первое сообщение".into());
    records.next();
    records.append("1:2".into(), "передумал".into());
    records.partial("незаконченная фраза".into());
    assert_eq!(records.clear(), ["1:2"]);
    let snapshot = records.snapshot();
    assert_eq!(snapshot.current.id, 2);
    assert_eq!(snapshot.current.generation, 1);
    assert!(snapshot.current.text.is_empty() && snapshot.current.partial.is_empty());
    assert_eq!(snapshot.history[0].text, "первое сообщение");
    assert!(!records.append("1:2".into(), "передумал".into()));
    assert!(records.append("1:3".into(), "новый вариант".into()));
}

#[test]
fn manual_edits_preserve_new_final_segments_that_arrive_during_the_request() {
    let mut records = Records::new();
    records.append("1:1".into(), "ашипка".into());
    records.append("2:1".into(), "продолжение".into());
    let edited = records
        .edit(1, 0, "Исправленный текст.".into(), &["1:1".into()])
        .unwrap();
    assert_eq!(edited.text, "Исправленный текст. продолжение");
    records.next();
    let history_edit = records
        .edit(1, 0, "Поздняя правка".into(), &["1:1".into(), "2:1".into()])
        .unwrap();
    assert_eq!(history_edit.text, "Поздняя правка");
    assert!(records.snapshot().current.text.is_empty());
}

#[test]
fn a_late_edit_cannot_restore_a_cleared_record() {
    let mut records = Records::new();
    records.append("1:1".into(), "старый текст".into());
    records.clear();
    assert!(records
        .edit(1, 0, "запоздалая правка".into(), &["1:1".into()])
        .is_err());
    assert!(records.snapshot().current.text.is_empty());
}
