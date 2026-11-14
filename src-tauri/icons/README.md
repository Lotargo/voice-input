# Voice Input icon

Violet glass microphone matching the compact desktop interface. The master is `microphone-source.png`, generated with the built-in imagegen tool on 2026-11-14. Transparency is preserved.

`icon.png` and the multi-resolution Windows `icon.ico` were generated with Tauri's icon CLI. `tray.png` is its 32 × 32 version; the same small image is copied to `ui/app-icon.png` for the custom title bar.

Regenerate in a temporary output directory with `cargo tauri icon src-tauri/icons/microphone-source.png --output dist/icon-build`, then copy `icon.png`, `icon.ico` and `32x32.png` to the paths above.

`build.rs` tracks these assets so changing them rebuilds Windows resources. Packaging compares the actual app and installer EXE icons with `icon.ico`, preventing stale icons from reaching portable/NSIS output. NSIS installer/uninstaller icons are explicitly configured in `tauri.conf.json`.

Generation prompt:

```text
Use case: logo-brand
Asset type: production Windows desktop app icon for Voice Input, square 1024 by 1024, transparent outside the icon.
Primary request: a polished violet glass microphone app icon matching a compact midnight-purple voice dictation interface.
Subject: one large centered upright microphone pictogram, luminous ivory and pale lilac, rounded capsule with a thick U-shaped cradle and a short stem. Strong simple silhouette readable at 16 and 32 pixels.
Composition: straight-on flat square app icon, rounded-square midnight violet glass tile filling 92% of the canvas, only a narrow transparent margin. Microphone fills about 62% of the tile. Symmetrical and carefully centered.
Style: refined modern desktop icon, subtle glass depth and restrained soft violet edge glow, dark indigo to amethyst gradient, crisp clean edges. A single delicate soft orbital halo behind the microphone, no small details.
Constraints: actual transparent alpha beyond the rounded tile, high contrast, no text, no letters, no watermark, no mockup, no perspective, no external scene, no extra symbols, no headphones, no detailed grille.
```
