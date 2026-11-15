//! A bounded decoder FIFO keeps expensive phrase inference off the DSP/level worker.
//! PCM, flush and reset commands retain their original order.
use super::pipeline::Recognizer;
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver, Sender, SyncSender, TrySendError},
    thread::JoinHandle,
};

enum Input {
    Pcm(Vec<i16>, bool),
    Finish(Sender<Result<String, String>>),
    Reset(Sender<()>),
}
enum Output {
    Final(String),
    Partial(String),
    Error(String),
}
pub(super) struct Decoder {
    sender: Option<SyncSender<Input>>,
    receiver: Receiver<Output>,
    thread: Option<JoinHandle<()>>,
    ready: VecDeque<String>,
    partial: String,
    error: Option<String>,
    speech: bool,
}
impl Decoder {
    pub fn new(mut recognizer: Box<dyn Recognizer + Send>) -> Result<Self, String> {
        // 2048 * 256 samples: about 33 s / 1 MiB of PCM, with a hard bound.
        let (sender, incoming) = mpsc::sync_channel(2048);
        let (outgoing, receiver) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("speech-decoder".into())
            .spawn(move || {
                let mut partial_samples = 0;
                let mut last_partial = String::new();
                while let Ok(input) = incoming.recv() {
                    match input {
                        Input::Pcm(samples, speech) => {
                            recognizer.speech(speech);
                            match recognizer.accept(&samples) {
                                Ok(true) => {
                                    let _ = outgoing.send(Output::Final(recognizer.result()));
                                    last_partial.clear();
                                }
                                Ok(false) => {
                                    partial_samples += samples.len();
                                    if partial_samples >= 2560 {
                                        partial_samples = 0;
                                        let partial = recognizer.partial();
                                        if partial != last_partial {
                                            let _ = outgoing.send(Output::Partial(partial.clone()));
                                            last_partial = partial;
                                        }
                                    }
                                }
                                Err(error) => {
                                    let _ = outgoing.send(Output::Error(error));
                                    break;
                                }
                            }
                        }
                        Input::Finish(reply) => {
                            let _ = reply.send(recognizer.finish_checked());
                            last_partial.clear();
                        }
                        Input::Reset(reply) => {
                            recognizer.reset();
                            partial_samples = 0;
                            last_partial.clear();
                            let _ = reply.send(());
                        }
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender: Some(sender),
            receiver,
            thread: Some(thread),
            ready: VecDeque::new(),
            partial: String::new(),
            error: None,
            speech: false,
        })
    }
    fn collect(&mut self) {
        while let Ok(output) = self.receiver.try_recv() {
            match output {
                Output::Final(text) => {
                    if !text.trim().is_empty() {
                        self.ready.push_back(text);
                    }
                    self.partial.clear();
                }
                Output::Partial(text) => self.partial = text,
                Output::Error(error) => self.error = Some(error),
            }
        }
    }
    fn flush(&mut self) -> Result<String, String> {
        let (reply, received) = mpsc::channel();
        self.sender
            .as_ref()
            .unwrap()
            .send(Input::Finish(reply))
            .map_err(|_| "Движок распознавания остановился")?;
        let tail = received
            .recv()
            .map_err(|_| "Движок не завершил распознавание")??;
        self.collect();
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        let mut chunks = self.ready.drain(..).collect::<Vec<_>>();
        if !tail.trim().is_empty() {
            chunks.push(tail);
        }
        self.partial.clear();
        Ok(chunks.join("\n"))
    }
}
impl Recognizer for Decoder {
    fn speech(&mut self, speech: bool) {
        self.speech = speech;
    }
    fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
        self.collect();
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        self.sender.as_ref().unwrap().try_send(Input::Pcm(samples.to_vec(), self.speech)).map_err(|error| match error {
            TrySendError::Full(_) => "Модель не успевает распознавать речь на этом CPU. Очередь заполнена; принятый звук будет завершён.".to_owned(),
            TrySendError::Disconnected(_) => "Движок распознавания остановился".to_owned(),
        })?;
        Ok(!self.ready.is_empty())
    }
    fn result(&mut self) -> String {
        self.ready.pop_front().unwrap_or_default()
    }
    fn partial(&mut self) -> String {
        self.collect();
        self.partial.clone()
    }
    fn finish(&mut self) -> String {
        self.flush().unwrap_or_default()
    }
    fn finish_checked(&mut self) -> Result<String, String> {
        self.flush()
    }
    fn reset(&mut self) {
        let (reply, received) = mpsc::channel();
        if self
            .sender
            .as_ref()
            .unwrap()
            .send(Input::Reset(reply))
            .is_ok()
        {
            let _ = received.recv();
        }
        self.collect();
        self.ready.clear();
        self.partial.clear();
        self.speech = false;
    }
}
impl Drop for Decoder {
    fn drop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Counter(usize);
    impl Recognizer for Counter {
        fn accept(&mut self, samples: &[i16]) -> Result<bool, String> {
            self.0 += samples.len();
            Ok(samples.contains(&9))
        }
        fn result(&mut self) -> String {
            let text = self.0.to_string();
            self.0 = 0;
            text
        }
        fn partial(&mut self) -> String {
            self.0.to_string()
        }
        fn finish(&mut self) -> String {
            self.result()
        }
        fn reset(&mut self) {
            self.0 = 0;
        }
    }
    #[test]
    fn fifo_preserves_completed_chunks_and_flushes_tail_once() {
        let mut decoder = Decoder::new(Box::new(Counter(0))).unwrap();
        decoder.accept(&[1, 9]).unwrap();
        decoder.accept(&[1, 2, 3]).unwrap();
        let mut finals = Vec::new();
        if !decoder.ready.is_empty() {
            finals.push(decoder.result());
        }
        finals.push(decoder.finish_checked().unwrap());
        assert_eq!(finals.join("\n"), "2\n3");
        assert!(decoder.ready.is_empty());
    }
    #[test]
    fn reset_discards_queued_old_generation_before_next_pcm() {
        let mut decoder = Decoder::new(Box::new(Counter(0))).unwrap();
        decoder.accept(&[1, 9]).unwrap();
        decoder.reset();
        decoder.accept(&[1, 2, 3]).unwrap();
        assert_eq!(decoder.finish_checked().unwrap(), "3");
    }
}

#[cfg(test)]
mod lifetime_tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };
    struct OwnedModel(Arc<AtomicUsize>);
    impl Drop for OwnedModel {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Recognizer for OwnedModel {
        fn accept(&mut self, _: &[i16]) -> Result<bool, String> {
            Ok(false)
        }
        fn result(&mut self) -> String {
            String::new()
        }
        fn partial(&mut self) -> String {
            String::new()
        }
        fn finish(&mut self) -> String {
            String::new()
        }
        fn reset(&mut self) {}
    }
    #[test]
    fn dropping_decoder_joins_worker_and_destroys_owned_model_once() {
        let released = Arc::new(AtomicUsize::new(0));
        for expected in 1..=3 {
            let mut decoder = Decoder::new(Box::new(OwnedModel(released.clone()))).unwrap();
            for _ in 0..100 {
                decoder.accept(&[0; 256]).unwrap();
            }
            drop(decoder);
            assert_eq!(released.load(Ordering::SeqCst), expected);
        }
    }
}
