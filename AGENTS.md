# Voice Input — agent instructions

## Goals
- Windows 10/11 portable + NSIS installer. Tauri 2 shell, Rust backend, static HTML/CSS/JS frontend.
- Offline, CPU-only speech recognition. No API providers, VPN, database, or background telemetry.
- Dictation goes into the user's focused editable control, never submits or presses Enter automatically.
- Partial hypotheses are presentation-only. Only Vosk FINAL chunks may be sent to a foreign editor.
- If no edit control is focused, hold data in process memory; optionally send once after 700 ms hover over a valid UI Automation edit field.
- At-most-once attempts keyed by `(session_id, segment_id)`: dequeue BEFORE calling SendInput, never automatically retry uncertain sends. Never touch system clipboard unless the user explicitly clicks Copy.

## Boundaries
- `src-tauri/src/audio.rs`: `cpal` microphone capture, 16k mono conversion, STT feed, manual stop and experimental endpoint auto-stop.
- `src-tauri/src/vosk.rs`: native Vosk dynamic loading and object lifetime.
- `src-tauri/src/delivery.rs`: UI Automation checks, delivery queue, and Unicode SendInput.
- `src-tauri/src/models.rs`: local model and DLL discovery.
- `src-tauri/src/lib.rs`: Tauri commands, lifecycle, hotkey, tray.
- `ui/`: static frontend; no hosted scripts or remote fonts.

## Never do
- Do not automatically submit text, click arbitrary screen coordinates, or send keys to password fields.
- Do not apply LLM actions or edit Excel documents directly; this phase is dictation only.
- Do not claim exactly-once insertion into external applications; Windows SendInput only acknowledges event injection, not editor acceptance.
- Do not download weights or runtime libraries during transcription.

## Tests
- Windows CI: `cargo fmt --check`, `cargo test`, `cargo tauri build --bundles nsis`.
- Manual smoke tests: Notepad, Chrome/Edge textarea, OpenCode, VS Code, Excel formula edit field; fast focus changes and two identical final chunks; no field selected; hover; password field; clipboard preservation; hotkey while window hidden.
- Hardening later: dedicated VAD / noise suppression, wake word, refined resampler, configurable shortcut, auto-start.
