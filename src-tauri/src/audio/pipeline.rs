use super::{
    silence::{Decision, SilenceClock},
    AudioEvent, RecordAction, SplitSettings,
};
use crate::vosk::Vosk;
use earshot::Detector;

pub(super) trait Recognizer {
    fn accept(&mut self, samples: &[i16]) -> Result<bool, String>;
    fn result(&mut self) -> String;
    fn partial(&mut self) -> String;
    fn finish(&mut self) -> String;
    fn reset(&mut self);
}

impl Recognizer for Vosk {
    fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        self.accept(samples)
    }
    fn result(&mut self) -> String {
        self.result()
    }
    fn partial(&mut self) -> String {
        self.partial()
    }
    fn finish(&mut self) -> String {
        self.finish()
    }
    fn reset(&mut self) {
        self.reset();
    }
}

pub(super) struct Pipeline<R: Recognizer> {
    recognizer: R,
    detector: Detector,
    clock: SilenceClock,
    frame: [i16; 256],
    filled: usize,
    partial_samples: usize,
    last_partial: String,
}

impl<R: Recognizer> Pipeline<R> {
    pub fn new(recognizer: R) -> Self {
        Self {
            recognizer,
            detector: Detector::default(),
            clock: SilenceClock::default(),
            frame: [0; 256],
            filled: 0,
            partial_samples: 0,
            last_partial: String::new(),
        }
    }

    fn final_text(&mut self, text: String, event: &mut impl FnMut(AudioEvent)) {
        self.last_partial.clear();
        if !text.trim().is_empty() {
            event(AudioEvent::Final(text));
        }
        event(AudioEvent::Partial(String::new()));
    }

    pub fn push(
        &mut self,
        samples: &[i16],
        settings: &SplitSettings,
        event: &mut impl FnMut(AudioEvent),
    ) -> Result<(), String> {
        for &sample in samples {
            self.frame[self.filled] = sample;
            self.filled += 1;
            if self.filled != 256 {
                continue;
            }
            self.filled = 0;
            let score = self.detector.predict_i16(&self.frame);
            match self.clock.push(score >= 0.5, 256, settings.get()) {
                Decision::Seal => {
                    let text = self.recognizer.finish();
                    self.final_text(text, event);
                    self.recognizer.reset();
                }
                Decision::Next => event(AudioEvent::Boundary(RecordAction::Automatic)),
                Decision::Continue => {}
            }
            if self.recognizer.accept(&self.frame)? {
                let text = self.recognizer.result();
                if !text.trim().is_empty() && self.clock.recognized() {
                    event(AudioEvent::Boundary(RecordAction::Automatic));
                }
                self.final_text(text, event);
            } else {
                self.partial_samples += 256;
                if self.partial_samples >= 2560 {
                    self.partial_samples = 0;
                    let text = self.recognizer.partial();
                    if text != self.last_partial {
                        if !text.trim().is_empty() && self.clock.recognized() {
                            event(AudioEvent::Boundary(RecordAction::Automatic));
                        }
                        self.last_partial = text.clone();
                        event(AudioEvent::Partial(text));
                    }
                }
            }
        }
        Ok(())
    }

    fn finish_tail(&mut self, event: &mut impl FnMut(AudioEvent)) -> Result<(), String> {
        if self.filled > 0 {
            if self.recognizer.accept(&self.frame[..self.filled])? {
                let text = self.recognizer.result();
                self.final_text(text, event);
            }
            self.filled = 0;
        }
        let text = self.recognizer.finish();
        self.final_text(text, event);
        Ok(())
    }

    pub fn boundary(
        &mut self,
        action: RecordAction,
        event: &mut impl FnMut(AudioEvent),
    ) -> Result<(), String> {
        if action == RecordAction::New {
            self.finish_tail(event)?;
        }
        self.filled = 0;
        self.recognizer.reset();
        self.detector.reset();
        self.clock.reset();
        self.last_partial.clear();
        self.partial_samples = 0;
        event(AudioEvent::Boundary(action));
        Ok(())
    }

    pub fn finish(&mut self, event: &mut impl FnMut(AudioEvent)) -> Result<(), String> {
        self.finish_tail(event)
    }
}

#[cfg(test)]
mod tests;
