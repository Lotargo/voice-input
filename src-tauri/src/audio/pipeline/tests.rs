use super::*;
use crate::records::Records;
use std::cell::{Cell, RefCell};

#[derive(Default)]
struct FakeRecognizer {
    samples: Vec<i16>,
    tail: String,
}
impl Recognizer for FakeRecognizer {
    fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        self.samples.extend_from_slice(samples);
        Ok(false)
    }
    fn result(&mut self) -> String {
        String::new()
    }
    fn partial(&mut self) -> String {
        String::new()
    }
    fn finish(&mut self) -> String {
        std::mem::take(&mut self.tail)
    }
    fn reset(&mut self) {
        self.tail.clear();
    }
}

#[test]
fn manual_next_flushes_before_the_boundary_and_clear_discards_the_old_hypothesis() {
    let settings = SplitSettings::new();
    let mut pipeline = Pipeline::new(FakeRecognizer::default());
    let mut events = Vec::new();
    let mut emit = |event| match event {
        AudioEvent::Final(text) => events.push(text),
        AudioEvent::Boundary(action) => events.push(format!("{action:?}")),
        _ => {}
    };
    pipeline.recognizer.tail = "хвост предыдущей записи".into();
    pipeline.push(&[123; 17], &settings, &mut emit).unwrap();
    pipeline.boundary(RecordAction::New, &mut emit).unwrap();
    pipeline.recognizer.tail = "отменённая гипотеза".into();
    pipeline.push(&[234; 9], &settings, &mut emit).unwrap();
    pipeline.boundary(RecordAction::Clear, &mut emit).unwrap();
    pipeline.recognizer.tail = "новый вариант".into();
    pipeline.push(&[345; 257], &settings, &mut emit).unwrap();
    pipeline.finish(&mut emit).unwrap();
    assert_eq!(
        events,
        ["хвост предыдущей записи", "New", "Clear", "новый вариант"]
    );
    assert_eq!(
        pipeline.recognizer.samples,
        [vec![123; 17], vec![345; 257]].concat()
    );
}

#[test]
fn native_vosk_and_vad_keep_pauses_together_and_split_only_after_speech_resumes() {
    let root = std::env::var_os("VOICE_INPUT_SMOKE_ROOT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources"));
    let speech: Vec<i16> = include_bytes!("../../../tests/fixtures/voice-ru.pcm")
        .chunks_exact(2)
        .map(|bytes| i16::from_le_bytes([bytes[0], bytes[1]]))
        .collect();
    for automatic in [false, true] {
        let recognizer = Vosk::open(
            &root.join("libvosk.dll"),
            &root.join("models/vosk-model-small-ru-0.22"),
        )
        .unwrap();
        let mut pipeline = Pipeline::new(recognizer);
        let settings = SplitSettings::new();
        settings.set(automatic, 3).unwrap();
        let records = RefCell::new(Records::new());
        let boundaries = Cell::new(0);
        let segment = Cell::new(0);
        let mut emit = |event| match event {
            AudioEvent::Final(text) => {
                segment.set(segment.get() + 1);
                records
                    .borrow_mut()
                    .append(format!("1:{}", segment.get()), text);
            }
            AudioEvent::Boundary(RecordAction::Automatic) => {
                boundaries.set(boundaries.get() + 1);
                records.borrow_mut().next();
            }
            _ => {}
        };
        pipeline
            .push(&vec![0; 16_000 * 5], &settings, &mut emit)
            .unwrap();
        assert_eq!(boundaries.get(), 0, "initial silence must not make records");
        for chunk in speech.chunks(731) {
            pipeline.push(chunk, &settings, &mut emit).unwrap();
        }
        pipeline.push(&vec![0; 3200], &settings, &mut emit).unwrap();
        for chunk in speech.chunks(317) {
            pipeline.push(chunk, &settings, &mut emit).unwrap();
        }
        assert_eq!(
            boundaries.get(),
            0,
            "long speech and short pauses stay together"
        );
        pipeline
            .push(&vec![0; 16_000 * 4], &settings, &mut emit)
            .unwrap();
        assert_eq!(
            boundaries.get(),
            0,
            "silence seals, but does not open an empty record"
        );
        let first = records.borrow().snapshot().current.text;
        assert!(
            !first.is_empty(),
            "fixture must produce real Russian recognition"
        );
        for chunk in speech.chunks(521) {
            pipeline.push(chunk, &settings, &mut emit).unwrap();
        }
        pipeline.finish(&mut emit).unwrap();
        let snapshot = records.borrow().snapshot();
        assert_eq!(boundaries.get(), usize::from(automatic));
        assert_eq!(snapshot.history.len(), usize::from(automatic));
        assert!(!snapshot.current.text.is_empty());
        if automatic {
            assert_eq!(snapshot.history[0].text, first);
        } else {
            assert!(snapshot.current.text.len() > first.len());
        }
    }
}
