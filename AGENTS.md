# Voice Input — agent instructions

## Scope
- Windows 10/11, Tauri 2 + Rust, static HTML/CSS/JS frontend; native portable + NSIS packages.
- Local CPU-based Russian STT: bundled Vosk and optional GigaAM v3 E2E / Nemotron 3.5 ASR. No cloud STT, VPN, LLM, database, or telemetry. Runtime networking is limited to explicit update checks/downloads and user-started model downloads from the curated pinned catalog. Never poll model hubs at startup.
- Dictation is user-controlled. NEVER automatically press Enter, send a message, or mutate an Excel workbook.
- Partial STT hypotheses are **preview only**. Deliver final segments to the user's focused editable field.
- If no field is eligible, keep final text in an in-memory pending queue. Optional 700 ms hover to focus and deliver.
- At-most-once attempts keyed by session/segment. Dequeue BEFORE attempting SendInput; uncertain attempts must NEVER auto-retry. Preserve system clipboard except when user explicitly clicks Copy.

## Architecture boundaries
- `src-tauri/src/audio.rs`: CPAL capture, one FIFO for PCM and record controls, manual stop and device-loss handling. Keep heavy DSP out of the callback. Vosk endpoints never stop capture.
- `src-tauri/src/audio/settings.rs`: persist stable input device IDs and the noise-suppression choice atomically. A missing selected device must never silently switch to another microphone. Lock changes during capture or update installation.
- `src-tauri/src/audio/processing.rs`: worker-side resampling with an anti-aliasing filter; optional 48 kHz 100 Hz high-pass + nnnoiseless/RNNoise, then 16 kHz for VAD/Vosk. Preserve raw levels before filtering so clipping stays visible. Noise suppression is off by default.
- `src-tauri/src/audio/pipeline.rs`: same-stream local VAD, decoder flush/reset, continuous manual/automatic record boundaries. Preserve audio before/after controls; clear intentionally discards the current unfinished hypothesis.
- `src-tauri/src/audio/silence.rs`: optional 1–60 second silence clock, default 3/off. Start at last speech end, never at capture start; open the next record only when speech resumes.
- `src-tauri/src/records.rs`: logical records independent of capture sessions and STT segments; keep completed history and edited text in RAM. Clear only the current generation/pending IDs; retain uncertain attempts and deduplication.
- `src-tauri/src/audio/visualizer.rs`: read-only RMS/AC peak and FFT-band analysis of the processed 16 kHz PCM. The processor supplies raw RMS, digital peak and near-rail sample ratio before filtering; no second capture stream or VAD. Signal overload is digital headroom, not acoustic SPL.
- `src-tauri/src/vosk.rs`: native Vosk late binding, DLL loading and object lifetime.
- `src-tauri/src/delivery.rs`: UI Automation validation, queue, Unicode SendInput, delivery status.
- `src-tauri/src/models.rs`: local model and runtime-file discovery. Resolve the Vosk DLL for Vosk and CPU profile for native models independently in installed/portable layouts; a native recognizer must never receive an empty runtime path.
- `src-tauri/src/catalog.rs`: pinned model metadata, native persistent primary selection, resumable per-file downloads and managed deletion. New model weights never enter installer/portable resources. Preserve partial files on pause, disconnect and app exit; resume via validated HTTP ranges. Download all files before an explicit cancellable verification phase with byte progress; verify every SHA-256 before atomic installation and model availability. Interrupted verification must reuse saved files without downloading them again. Cancel deletes only the unfinished managed download. Never delete a custom model directory or silently replace a missing primary model.
- `src-tauri/src/recognizers.rs`: local CPU GigaAM/Nemotron adapters. Use the existing same-stream VAD for phrase buffering; keep partials preview-only.
- `src-tauri/src/audio/decoder.rs`: bounded PCM/control FIFO for heavy native inference, preserving flush/reset order without stalling DSP and audio-level feedback.
- `src-tauri/installer-hooks.nsh`: ask to retain downloaded models/settings on interactive uninstall; preserve data during upgrades and unattended uninstall.
- `src-tauri/src/lib.rs`: app state, commands, hotkey, tray lifecycle.
- `src-tauri/src/hotkey.rs`: persistent global shortcut; register the candidate before releasing the active shortcut. Conflicts/storage failures preserve the previous binding. Capture mode must not start dictation and must end when focus is lost.
- `src-tauri/src/updates.rs`: user-initiated signed updates, separate portable/NSIS manifests, no startup polling or embedded GitHub token. Block installation while recording and new capture during updates. Signatures must bind the announced version.
- `src-tauri/src/updates/portable.rs`: validate signature/paths, stage before exiting, wait for the actual parent, back up and roll back replaced files, restart in the same folder. Preserve unrelated files. Never convert portable into an installation.
- `ui/`: local assets; no hosted scripts, fonts, CDNs or API calls.
- `ui/audio.js`: microphone selection, optional noise suppression and a bounded 20-second level check. The check uses the native capture pipeline without Vosk, transcript, queue delivery or saved audio. Stop it when leaving the audio panel, hiding or unfocusing the window. Do not affect ordinary dictation when focus changes.
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
Wake word, auto-start and other STT providers remain deferred. Do not add other audio features beyond the approved catalog/device selection/loss handling/level check/optional noise suppression without a new request. API/LLM only if requested later.
