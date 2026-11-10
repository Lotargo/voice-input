# Voice Input — agent instructions

## Scope
- Windows 10/11, Tauri 2 + Rust, static HTML/CSS/JS frontend; native portable + NSIS packages.
- Local CPU-based Russian STT (Vosk) only. No API, VPN, LLM, external service, database, or telemetry for v0.1.
- Dictation is user-controlled. NEVER automatically press Enter, send a message, or mutate an Excel workbook.
- Partial STT hypotheses are **preview only**. Deliver final segments to the user's focused editable field.
- If no field is eligible, keep final text in an in-memory pending queue. Optional 700 ms hover to focus and deliver.
- At-most-once attempts keyed by session/segment. Dequeue BEFORE attempting SendInput; uncertain attempts must NEVER auto-retry. Preserve system clipboard except when user explicitly clicks Copy.

## Architecture boundaries
- `src-tauri/src/audio.rs`: CPAL audio capture, resampling, STT feed, manual stop, experimental endpoint auto-stop.
- `src-tauri/src/vosk.rs`: native Vosk late binding, DLL loading and object lifetime.
- `src-tauri/src/delivery.rs`: UI Automation validation, queue, Unicode SendInput, delivery status.
- `src-tauri/src/models.rs`: local model / DLL discovery.
- `src-tauri/src/lib.rs`: app state, commands, hotkey, tray lifecycle.
- `ui/`: local assets; no hosted scripts, fonts, CDNs or API calls.
- `scripts/prepare-assets.ps1`: downloads build-time native runtime and local model.
- `scripts/build-windows.ps1`: manual installer + portable packaging.

## Development workflow
- Work directly in `main` when requested by the repository owner; do not require a PR or branch.
- CI workflows have deliberately been removed. Do not add GitHub Actions unless explicitly requested.
- Keep README aligned with code. Do not document planned features as implemented.
- Check locally on Windows: `cargo fmt`, `cargo test`, `cargo tauri build --bundles nsis`; package with `scripts/build-windows.ps1`.
- Manually smoke test: Notepad, Chrome/Edge edit controls, OpenCode, VS Code, Excel formula field; rapid focus switching; identical final chunks; no target; hover target; password control; clipboard preservation; hotkey while window hidden.
- Known limits: UIA is imperfect in custom editors; Windows injection lacks acceptance ACK; elevated apps can block input.

## Roadmap (not yet implemented)
Dedicated VAD, noise suppression, wake word, adjustable hotkey, better resampler, auto-start; API/LLM only if requested later.
