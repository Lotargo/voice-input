//! Audio-time silence clock: never runs before speech and never makes empty records.
use serde::Serialize;
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct SplitOptions {
    pub enabled: bool,
    pub silence_seconds: u32,
}

pub struct SplitSettings(AtomicU32);

impl SplitSettings {
    pub fn new() -> Self {
        Self(AtomicU32::new(3))
    }
    pub fn get(&self) -> SplitOptions {
        let bits = self.0.load(Ordering::Acquire);
        SplitOptions {
            enabled: bits & 256 != 0,
            silence_seconds: bits & 255,
        }
    }
    pub fn set(&self, enabled: bool, seconds: u32) -> Result<SplitOptions, String> {
        if !(1..=60).contains(&seconds) {
            return Err("Интервал тишины должен быть от 1 до 60 секунд".into());
        }
        self.0
            .store(seconds | if enabled { 256 } else { 0 }, Ordering::Release);
        Ok(self.get())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Decision {
    Continue,
    Seal,
    Next,
}

#[derive(Default)]
pub(super) struct SilenceClock {
    samples: u64,
    last_voice_end: Option<u64>,
    sealed: bool,
}

impl SilenceClock {
    pub fn push(&mut self, voice: bool, count: usize, options: SplitOptions) -> Decision {
        let previous_end = self.samples;
        self.samples += count as u64;
        if !options.enabled {
            self.sealed = false;
        }
        let gap = previous_end.saturating_sub(self.last_voice_end.unwrap_or(previous_end));
        let threshold = options.silence_seconds as u64 * 16_000;
        if voice {
            let next = options.enabled && self.sealed && gap >= threshold;
            self.last_voice_end = Some(self.samples);
            self.sealed = false;
            return if next {
                Decision::Next
            } else {
                Decision::Continue
            };
        }
        if options.enabled
            && !self.sealed
            && self
                .last_voice_end
                .is_some_and(|end| self.samples.saturating_sub(end) >= threshold)
        {
            self.sealed = true;
            return Decision::Seal;
        }
        Decision::Continue
    }

    pub fn recognized(&mut self) -> bool {
        let next = self.sealed;
        if next || self.last_voice_end.is_none() {
            self.last_voice_end = Some(self.samples);
        }
        self.sealed = false;
        next
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const AUTO: SplitOptions = SplitOptions {
        enabled: true,
        silence_seconds: 3,
    };

    #[test]
    fn initial_silence_and_continuous_speech_never_open_a_new_record() {
        let mut clock = SilenceClock::default();
        assert_eq!(clock.push(false, 16_000 * 600, AUTO), Decision::Continue);
        for _ in 0..1000 {
            assert_eq!(clock.push(true, 256, AUTO), Decision::Continue);
        }
        assert_eq!(clock.push(false, 16_000 * 2, AUTO), Decision::Continue);
        assert_eq!(clock.push(true, 256, AUTO), Decision::Continue);
    }

    #[test]
    fn deadline_starts_at_last_voice_end_and_only_next_speech_advances_once() {
        for seconds in [1, 3, 60] {
            let options = SplitOptions {
                silence_seconds: seconds,
                ..AUTO
            };
            let mut clock = SilenceClock::default();
            clock.push(true, 16_000 * 10, options);
            assert_eq!(
                clock.push(false, 16_000 * seconds as usize - 1, options),
                Decision::Continue
            );
            assert_eq!(clock.push(false, 1, options), Decision::Seal);
            assert_eq!(clock.push(false, 16_000 * 120, options), Decision::Continue);
            assert_eq!(clock.push(true, 256, options), Decision::Next);
            assert_eq!(clock.push(true, 256, options), Decision::Continue);
        }
    }

    #[test]
    fn manual_mode_reset_and_disabling_auto_cancel_pending_splits() {
        let mut clock = SilenceClock::default();
        let manual = SplitOptions {
            enabled: false,
            ..AUTO
        };
        clock.push(true, 256, manual);
        assert_eq!(clock.push(false, 16_000 * 70, manual), Decision::Continue);
        assert_eq!(clock.push(true, 256, manual), Decision::Continue);
        assert_eq!(clock.push(false, 16_000 * 3, AUTO), Decision::Seal);
        clock.push(false, 256, manual);
        assert_eq!(clock.push(true, 256, manual), Decision::Continue);
        clock.reset();
        assert_eq!(clock.push(false, 16_000 * 80, AUTO), Decision::Continue);
    }

    #[test]
    fn settings_reject_out_of_range_values_without_changing_the_saved_value() {
        let settings = SplitSettings::new();
        assert_eq!(
            settings.get(),
            SplitOptions {
                enabled: false,
                silence_seconds: 3
            }
        );
        settings.set(true, 60).unwrap();
        assert!(settings.set(true, 0).is_err());
        assert!(settings.set(true, 61).is_err());
        assert_eq!(
            settings.get(),
            SplitOptions {
                enabled: true,
                silence_seconds: 60
            }
        );
    }
}
