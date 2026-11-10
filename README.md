# Voice Input

Offline Russian speech-to-text for **Windows 10/11**. Lightweight Tauri 2 + Rust application; no server, no VPN, no API keys, and no dedicated GPU. Initial implementation uses **Vosk Small RU** (~45 MB downloadable model) and Windows UI Automation for input placement.

## MVP capabilities

- `Ctrl + Alt + Space` starts/stops a microphone recording; the window may be hidden in the tray.
- `Vosk` produces streaming **partial** text in the application UI. Stable **final** segments are delivered to the focused editable field, without pressing Enter or sending a message.
- If no editable field is focused, text stays in an **in-memory pending queue**. Hover over an editable control for ~700 ms to focus and insert pending text; disable this in the UI if not desired.
- No clipboard changes for automatic insertion: Unicode `SendInput` is used. Use **Copy** on a pending item for manual `Ctrl+V`.
- Deduplication by `(session_id, segment_id)`. Every segment has **at most one automatic SendInput attempt**. An ambiguous partial failure is saved as uncertain, never auto-retried.
- Experimental automatic stop on the Vosk utterance endpoint (off by default); manual stop remains supported.
- Select between installed local Vosk-compatible models by path. No network activity at runtime.

## Local preparation (PowerShell)

Requirements: Windows 10/11 x64, Rust stable (1.90+), Microsoft C++ Build Tools, Tauri 2 prerequisites (WebView2). No Node.js is needed; frontend uses static HTML/CSS/JS.

```powershell
# In the repository root
.\scripts\prepare-assets.ps1
cd src-tauri
cargo install tauri-cli --version '^2' --locked
cargo tauri dev
```

`prepare-assets.ps1` downloads the official **Vosk v0.3.45** Windows DLLs and the **Apache-2.0** `vosk-model-small-ru-0.22` language model into `src-tauri/resources/`. Files are downloaded once, never while dictating. They are excluded from Git.

The default model directory is `src-tauri/resources/models/vosk-model-small-ru-0.22`. Another local compatible model may be selected from the UI. Extracted model root must contain `am/` and `conf/` directories.

## Windows artifacts

`.github/workflows/windows.yml` prepares the native assets and builds:

1. `voice-input-windows-installer` — unsigned NSIS setup executable, bundling the local model and runtime.
2. `voice-input-windows-portable` — ZIP containing the unbundled executable and the same native runtime/model. Extract before use; WebView2 must be available.

The first workflow run is the build verification step; binaries are **not** claimed to be tested on real Windows applications until smoke testing.

## Design and safety

- Active field is checked using Microsoft UI Automation. Only enabled, keyboard-focusable `Edit` and `Document` controls in **other** processes are eligible; password and offscreen controls are excluded.
- Hover mode uses `ElementFromPoint` and `SetFocus` after a 700 ms dwell, then confirms that the same UIA element has focus. Some custom Electron/CAD controls won't expose standard UIA controls; for those use manual Copy/Paste.
- `SendInput` does **not** confirm that an external editor actually inserted characters; the app therefore guarantees **at-most-once attempted delivery**, not exactly-once acceptance.
- Recording stops when the user requests it; with auto-stop enabled, Vosk's end-of-utterance result can end the recording. This is not noise-independent VAD. Real noise suppression, Silero VAD, and a wake-word engine are separate future stages.
- Some elevated processes block injection by lower-integrity processes. This is a Windows limitation, not a retry condition.
- The app never reads, modifies or submits Excel workbooks itself; it provides text to a user-selected input field. LLM/API and spreadsheet agent workflows are intentionally excluded.

## Third-party software

- Vosk API binaries from [alphacep/vosk-api](https://github.com/alphacep/vosk-api/releases/tag/v0.3.45); Vosk model from [alphacephei.com](https://alphacephei.com/vosk/models). Keep their license notices with redistribution.
- Tauri 2, CPAL, libloading, UI Automation bindings; follow licenses of respective Rust crates and Windows WebView2 runtime.
