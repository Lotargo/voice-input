# Voice Input — agent instructions

## Scope
- Windows 10/11, Tauri 2 + Rust, static HTML/CSS/JS frontend; native portable + NSIS packages.
- Local CPU-based Russian STT (Vosk) only. No cloud STT, VPN, LLM, database, or telemetry. The only runtime network feature is an explicitly requested update check/download from public GitHub Releases.
- Dictation is user-controlled. NEVER automatically press Enter, send a message, or mutate an Excel workbook.
- Partial STT hypotheses are **preview only**. Deliver final segments to the user's focused editable field.
- If no field is eligible, keep final text in an in-memory pending queue. Optional 700 ms hover to focus and deliver.
- At-most-once attempts keyed by session/segment. Dequeue BEFORE attempting SendInput; uncertain attempts must NEVER auto-retry. Preserve system clipboard except when user explicitly clicks Copy.

## Architecture boundaries
- `src-tauri/src/audio.rs`: CPAL capture/resampling, one FIFO for PCM and record controls, manual stop. Vosk endpoints never stop capture.
- `src-tauri/src/audio/pipeline.rs`: same-stream local VAD, decoder flush/reset, continuous manual/automatic record boundaries. Preserve audio before/after controls; clear intentionally discards the current unfinished hypothesis.
- `src-tauri/src/audio/silence.rs`: optional 1–60 second silence clock, default 3/off. Start at last speech end, never at capture start; open the next record only when speech resumes.
- `src-tauri/src/records.rs`: logical records independent of capture sessions and STT segments; keep completed history and edited text in RAM. Clear only the current generation/pending IDs; retain uncertain attempts and deduplication.
- `src-tauri/src/audio/visualizer.rs`: read-only RMS/AC peak, raw digital peak, near-rail sample ratio and FFT-band analysis of the existing 16 kHz PCM; no second capture stream or VAD. Signal overload is digital headroom, not acoustic SPL.
- `src-tauri/src/vosk.rs`: native Vosk late binding, DLL loading and object lifetime.
- `src-tauri/src/delivery.rs`: UI Automation validation, queue, Unicode SendInput, delivery status.
- `src-tauri/src/models.rs`: local model / DLL discovery.
- `src-tauri/src/lib.rs`: app state, commands, hotkey, tray lifecycle.
- `src-tauri/src/hotkey.rs`: persistent global shortcut; register the candidate before releasing the active shortcut. Conflicts/storage failures preserve the previous binding. Capture mode must not start dictation and must end when focus is lost.
- `src-tauri/src/updates.rs`: user-initiated signed updates, separate portable/NSIS manifests, no startup polling or embedded GitHub token. Block installation while recording and new capture during updates. Signatures must bind the announced version.
- `src-tauri/src/updates/portable.rs`: validate signature/paths, stage before exiting, wait for the actual parent, back up and roll back replaced files, restart in the same folder. Preserve unrelated files. Never convert portable into an installation.
- `ui/`: local assets; no hosted scripts, fonts, CDNs or API calls.
- `ui/wave.js`: gentle idle motion at about 10 fps and stronger audio response at about 30 fps; stop both when hidden and respect reduced motion. Amber/high and coral/overload feedback needs a text equivalent, hysteresis and stale-data reset. Compact dictation is the default; opening the optional editor must not take pending delivery ownership.
- `ui/records.js`: preserve draft edits and reject stale snapshots/old record generations. Record clearing only affects Voice Input and pending delivery; externally inserted text is deleted by the user.
- `scripts/prepare-assets.ps1`: downloads build-time native runtime and local model.
- `scripts/build-windows.ps1`: manual installer + portable packaging.
- `src-tauri/Cargo.toml`: single version source (SemVer); Tauri inherits it. Tags are `vX.Y.Z`. Signed packages go in `dist/portable/X.Y.Z`, `dist/installers/X.Y.Z`, metadata in `dist/releases/X.Y.Z`.
- `scripts/publish-release.ps1`: local manual release to public `Lotargo/voice-input`; verify clean source, matching build commit and checksums, upload a draft before publishing. Private signing material stays outside Git, protected for the current Windows account.

## Development workflow
- Work directly in `main` when requested by the repository owner; do not require a PR or branch.
- CI workflows have deliberately been removed. Do not add GitHub Actions unless explicitly requested.
- Keep README aligned with code. Do not document planned features as implemented.
- Check locally on Windows: `cargo fmt`, `cargo test`, `cargo tauri build --bundles nsis`; package with `scripts/build-windows.ps1`.
- Manually smoke test: Notepad, Chrome/Edge edit controls, OpenCode, VS Code, Excel formula field; rapid focus switching; identical final chunks; no target; hover target; password control; clipboard preservation; hotkey while window hidden.
- Known limits: UIA is imperfect in custom editors; Windows injection lacks acceptance ACK; elevated apps can block input.

## Roadmap (not yet implemented)
Noise suppression, wake word, better resampler, auto-start; API/LLM only if requested later.
