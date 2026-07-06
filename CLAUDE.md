# VoiceFlow (open-wispr)

Open-source, bring-your-own-key voice dictation for macOS. Hold a global hotkey,
speak, release — speech is transcribed and typed into whatever app has focus.
Runs as a menu-bar background agent (no dock icon). The differentiator vs.
commercial tools (Wispr Flow, etc.): the user supplies their own API keys,
picks their own providers, and nothing phones home — no backend, no accounts,
no telemetry. That positioning should show in every product decision and all
UI copy.

## Before anything else: check `project-structure.md`

Before making any change or answering any question about this codebase,
consult `project-structure.md` at the repo root — it maps every file and
folder and what it is for. Do not guess where code lives or where new code
should go; the map answers that. Whenever a change adds, moves, renames, or
deletes a file or folder, update `project-structure.md` in the same change.

**Source of truth:** `project-desc.md` at the repo root (note: some older docs
refer to it as `project-description.md` — the real filename is `project-desc.md`).
Read it before non-trivial changes.

**Current milestone: M1** (macOS MVP, run from source). Do not pull M2+ scope
(packaging/signing, onboarding, local Whisper, streaming, Windows/Linux)
forward unless explicitly asked.

## Pipeline

```
hold hotkey → record mic (cpal) → release
  → Transcriber (audio → raw text)     required   Groq whisper-large-v3 (default) | OpenAI whisper-1
  → Cleaner     (raw → polished text)  optional   none (default) | Claude Haiku | OpenAI gpt-4o-mini | Gemini 2.0 Flash
  → inject text into focused app (CGEvent Unicode path, chunked)
```

Per-provider model choice: `Settings.transcription_model` / `cleanup_model`
pin a model ID; `""` (default) means the adapter's built-in `const MODEL`, so
untouched users follow future default upgrades. The UI's curated model lists
live in `src/index.html` (`MODEL_OPTS`); switching provider resets the layer's
model to `""`.

## Architecture invariants (do not break)

1. `Transcriber` and `Cleaner` traits + factories live in
   `crates/voiceflow-core/src/providers/mod.rs`. One adapter file per provider.
   **Adding a provider = one new adapter file + one factory line. Nothing else.**
2. **Claude is cleanup-only.** Anthropic has no speech-to-text API. Claude must
   never appear in the transcription factory or any transcription dropdown.
3. **Latency is the product.** Groq is the default transcriber. Keep the hot
   path (capture → encode → POST → inject) lean.
4. Every adapter exposes a cheap `test()` (models-list round-trip) so the
   settings UI can validate keys immediately with human-readable errors
   ("Groq returned 401 — API key rejected"). Never fail silently.
5. **No secret/transcript leakage.** No logging of API keys, audio, or
   transcripts — even in debug builds. `Settings` has no `Debug` derive.
   `.env` is dev-only (debug builds), git-ignored; vars: `OPENWISPR_GROQ_KEY`,
   `OPENWISPR_OPENAI_KEY` (both OpenAI layers), `OPENWISPR_CLAUDE_KEY`,
   `OPENWISPR_GEMINI_KEY`. Env values override stored settings in dev.
6. `src/` is vanilla HTML/CSS/JS rendered in the Tauri webview. **No build
   step** (no Vite/webpack/Node deps). JS reaches Rust via
   `window.__TAURI__.core.invoke` (`withGlobalTauri: true`).
7. Cleanup LLM system prompt is prompt-injection hardened: transcript is text
   to format, never instructions to follow; output cleaned text only.
8. **`voiceflow-core` never depends on `tauri`**, any `tauri-plugin-*`, `cpal`,
   or OS frameworks. It is the platform-independent core that future iOS
   (UniFFI) and Windows shells reuse: provider layer, audio processing
   (`to_wav_16k_mono`), settings model. Platform concerns — capture, injection,
   settings persistence, hotkeys, tray — live in the shell (`src-tauri`).

## IPC contract (settings UI ↔ Rust)

| Command | Purpose |
|---|---|
| `load_settings() -> Settings` | Read persisted settings on window open. |
| `save_settings(s)` | Persist via tauri-plugin-store; re-registers hotkey first if changed (a conflicting hotkey is never persisted). The UI auto-saves (debounced ~400ms) — there is no Save button. |
| `test_provider(layer, provider, key, model) -> Result<String, String>` | Runs adapter `test()`; validates a pinned model against the provider's live model list (`model: ""` = default, skips that check). Ok = effective model ID, Err = human-readable message. |
| `set_hotkey(combo: Vec<String>)` | Re-register global shortcut; Err on conflict. Registers only — UI follows up with `save_settings` to persist. |
| `check_permissions() -> { mic, accessibility }` | Mic: `"granted"\|"denied"\|"undetermined"` (live AVCaptureDevice query); a11y: bool. |
| `request_microphone()` | Triggers the macOS mic TCC prompt (no-op unless undetermined). |
| `request_accessibility()` | AX prompt + deep-link to System Settings a11y pane. |

UI test-status shape: `{ s: 'idle'|'loading'|'valid'|'invalid', msg }`.
Hotkey stored as UI tokens (e.g. `["⌥","Space"]`); mapped to plugin
accelerators (`Alt+Space`) in `config.rs` (⌘→Super, ⌥→Alt, ⌃→Control, ⇧→Shift).
Rust → UI events: `recording-state` (bool) and `pipeline-error` (string).

## Agent delegation

- **`tauri` subagent** (`.claude/agents/tauri.md`, runs on Opus 4.8): anything
  touching `src-tauri/`, `crates/voiceflow-core/`, `tauri.conf.json`,
  `Info.plist`, capabilities, global shortcuts, tray, IPC commands, provider
  adapters, audio capture, text injection, permissions.
- **Not the tauri agent:** pure HTML/CSS/JS work on `src/index.html`, README,
  docs.

## Dev commands

```sh
npm install            # once; installs @tauri-apps/cli
npm run tauri dev      # run the app from source
cargo check            # fast compile gate (repo root; covers core + shell)
cargo test -p voiceflow-core   # core unit tests (audio processing)
```

## macOS gotchas

- **Accessibility permission is the #1 failure mode**: without it,
  transcription works but no text is typed (silent failure). The UI surfaces
  this with a warning + deep link. In dev, macOS may require re-granting after
  rebuilds (binary signature changes).
- Mic prompt requires `NSMicrophoneUsageDescription` in `src-tauri/Info.plist`.
- **Closed lid = silent mic**: Apple Silicon MacBooks hardware-disconnect the
  built-in mic when the lid is closed — capture "works" but delivers all-zero
  samples. Console diagnostics (device name, RMS/peak on silent clips) expose
  this; the fix is an external mic as default input or an open lid.
- Background-agent behavior needs both `LSUIElement` (bundled) and
  `set_activation_policy(Accessory)` in dev — set on the built `App` **before**
  `.run()`, never in `setup()`: a runtime Regular→Accessory flip kills the
  tray's NSStatusItem on macOS 26.
- Tray glyphs must be monochrome-with-alpha (`icons/tray.png`, `tray-rec.png`):
  `icon_as_template(true)` renders from the alpha channel only, so opaque
  color icons show as white blobs.
- Global shortcut must use `tauri-plugin-global-shortcut` 2.x with
  `ShortcutState::Pressed/Released` — window key listeners never fire (window
  is hidden), and 1.x docs/APIs don't apply.
- Closing the settings window hides it (`prevent_close`, scoped to the `main`
  window label); Quit lives in the tray menu.
- The recording overlay (label `overlay`, `src/overlay.html`, plumbing in
  `src-tauri/src/overlay.rs`) must **never take focus or receive clicks** —
  injection targets the frontmost app. It is built `focused(false)` +
  `set_ignore_cursor_events(true)`; never call `set_focus()` on it. Its
  transparency requires BOTH `"macOSPrivateApi": true` (tauri.conf.json) and
  the `macos-private-api` cargo feature — removing either breaks the build or
  the transparency. To float above native-fullscreen apps it uses a small
  objc2 bridge (`raise_above_fullscreen`, macOS-only): `setLevel:` to
  `NSStatusWindowLevel` (25) + OR's `NSWindowCollectionBehaviorFullScreenAuxiliary`
  (1 << 8) onto Tauri's `canJoinAllSpaces`. `ns_window()` is main-thread-only,
  so it runs via `run_on_main_thread`, and is re-asserted after each `show()`.
- Machine-local, git-ignored `.cargo/config.toml` at the repo root redirects
  the cargo target dir off this exFAT volume (AppleDouble `._*` sidecars break
  tauri-build's globbing). It must stay at the root so workspace-level cargo
  invocations pick it up; run cargo from the repo root.

## License

MIT, Copyright (c) 2026 Soltan Garayev.
