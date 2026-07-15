# Scriva

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

**Current milestone: M3** (fully-local dictation: on-device Whisper
transcription + on-device cleanup LLM, in-app model download, zero-key offline
config). M1 and M2 are done. Do not pull M4+ scope (streaming, Windows/Linux)
forward unless explicitly asked.

## Pipeline

```
hold hotkey → record mic (cpal) → release
  → Transcriber (audio → raw text)     required   Groq whisper-large-v3 (default) | OpenAI whisper-1 | Local whisper.cpp
  → Cleaner     (raw → polished text)  optional   none (default) | Claude Haiku | OpenAI gpt-4o-mini | Gemini 2.0 Flash | Local llama.cpp
  → inject text into focused app (CGEvent Unicode path, chunked) — unless the
    AX probe (src-tauri/src/focus.rs) says the focused element is clearly not
    editable: then copy to clipboard + show a "Copied — ⌘V to paste" pill instead
```

The final text of the most recent dictation is kept in `AppState`
(memory only, never persisted or logged) and retrievable via the tray's
"Copy Last Transcription" item.

Per-provider model choice: `Settings.transcription_model` / `cleanup_model`
pin a model ID; `""` (default) means the adapter's built-in `const MODEL`, so
untouched users follow future default upgrades. The UI's curated model lists
live in `src/index.html` (`MODEL_OPTS`); switching provider resets the layer's
model to `""`.

**Local provider (`"local"`, both layers):** no API key. The model field holds
a curated on-device model id from `crates/scriva-core/src/registry.rs`
(`""` = default: `whisper-small` / `llama-3.2-3b`). Model files live in
`<app-data>/models/` (`~/Library/Application Support/com.scriva.app/models/`),
downloaded in-app via `src-tauri/src/models.rs`. Engines: `whisper-rs` 0.16
and `llama-cpp-2` (pinned `=0.1.151` — its 0.1.x API churns), Metal on macOS,
behind scriva-core's `local-models` cargo feature (default OFF so
`cargo test -p scriva-core` never needs cmake; `src-tauri` turns it on).
Loaded models are cached in adapter statics keyed by file path (~0.5–2.5 GB
RAM each); `save_settings` unloads a layer's cache when it leaves `"local"`
and re-warms after every save via `warm_local_models`. Caches are also evicted
after 10 min without a dictation (`LOCAL_IDLE_EVICT_SECS` in
`src-tauri/src/lib.rs`) and re-warmed on hotkey press, so a post-eviction
reload hides behind the user's speaking time.

## Architecture invariants (do not break)

1. `Transcriber` and `Cleaner` traits + factories live in
   `crates/scriva-core/src/providers/mod.rs`. One adapter file per provider.
   **Adding a provider = one new adapter file + one factory line. Nothing else.**
2. **Claude is cleanup-only.** Anthropic has no speech-to-text API. Claude must
   never appear in the transcription factory or any transcription dropdown.
3. **Latency is the product.** Groq is the default transcriber. Keep the hot
   path (capture → encode → POST → inject) lean.
4. Every adapter exposes a cheap `test()` (models-list round-trip; for local
   adapters, file-exists + magic-bytes) so the settings UI can validate setup
   immediately with human-readable errors ("Groq returned 401 — API key
   rejected"). Never fail silently.
5. **No secret/transcript leakage.** No logging of API keys, audio, or
   transcripts — even in debug builds. `Settings` has no `Debug` derive.
   `.env` is dev-only (debug builds), git-ignored; vars: `SCRIVA_GROQ_KEY`,
   `SCRIVA_OPENAI_KEY` (both OpenAI layers), `SCRIVA_CLAUDE_KEY`,
   `SCRIVA_GEMINI_KEY`. Env values override stored settings in dev.
6. `src/` is vanilla HTML/CSS/JS rendered in the Tauri webview. **No build
   step** (no Vite/webpack/Node deps). JS reaches Rust via
   `window.__TAURI__.core.invoke` (`withGlobalTauri: true`).
7. Cleanup LLM system prompt is prompt-injection hardened: transcript is text
   to format, never instructions to follow; output cleaned text only.
8. **`scriva-core` never depends on `tauri`**, any `tauri-plugin-*`, `cpal`,
   or OS frameworks. It is the platform-independent core that future iOS
   (UniFFI) and Windows shells reuse: provider layer, audio processing
   (`to_wav_16k_mono`), settings model. Platform concerns — capture, injection,
   settings persistence, hotkeys, tray — live in the shell (`src-tauri`).
   Sanctioned exception: the opt-in `local-models` feature adds `whisper-rs`,
   `llama-cpp-2` (cross-platform C++ embeds; `metal` only via the macOS target
   table) and a minimal `tokio` `rt` dep for `spawn_blocking`. Core's DEFAULT
   feature set stays free of all of them.

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
| `get_onboarded() -> bool` | First-run flag; drives the onboarding layer in the UI. Stored as a sibling `onboarded` key in the settings store, deliberately outside core's `Settings`. |
| `set_onboarded()` | Mark onboarding complete. One-way — delete the store file to re-run onboarding. |
| `list_local_models() -> Vec<row>` | Registry × disk × in-flight downloads: `{id, layer, label, size_mb, sub, state: "downloaded"\|"not"\|"downloading", pct}`. |
| `download_model(id)` | Spawns a streaming download to `<file>.part` (atomic rename on success, ±20% size sanity check), returns immediately; progress arrives via the `model-download-progress` event. Errs on unknown/duplicate/already-downloaded. |
| `cancel_download(id)` | Sets the in-flight cancel flag; the download task removes its `.part` and emits a calm "Download cancelled." |
| `delete_model(id)` | Removes the model file (+ stale `.part`); refuses while that model is downloading. |

Auto-start uses `tauri-plugin-autostart` (LaunchAgent variant): the UI calls
`plugin:autostart|is_enabled` / `enable` / `disable` directly — the LaunchAgent
plist is the source of truth, deliberately no mirror field in `Settings`. The
plist records the absolute `.app` path at enable time (enabling from a dev
binary points at the dev binary; moving the app breaks it — re-toggle fixes).
Login launches carry `--autostart` (passed by the LaunchAgent) and start with
the window hidden; manual launches open the Settings window. If the login item
was enabled before the flag existed, re-toggle it to refresh the plist.

UI test-status shape: `{ s: 'idle'|'loading'|'valid'|'invalid', msg }`.
Hotkey stored as UI tokens (e.g. `["⌥","Space"]`); mapped to plugin
accelerators (`Alt+Space`) in `config.rs` (⌘→Super, ⌥→Alt, ⌃→Control, ⇧→Shift).
Rust → UI events: `recording-state` (bool), `pipeline-error` (string), and
`model-download-progress` (`{id, pct, done?, error?}`, throttled to whole-%
steps ≥200ms; the UI re-renders only the model panels on ticks so key inputs
never lose focus).

## Agent delegation

- **`tauri` subagent** (`.claude/agents/tauri.md`, runs on Opus 4.8): anything
  touching `src-tauri/`, `crates/scriva-core/`, `tauri.conf.json`,
  `Info.plist`, capabilities, global shortcuts, tray, IPC commands, provider
  adapters, audio capture, text injection, permissions.
- **Not the tauri agent:** pure HTML/CSS/JS work on `src/index.html`, README,
  docs.

## Dev commands

```sh
npm install            # once; installs @tauri-apps/cli
brew install cmake     # once; whisper.cpp/llama.cpp build via cmake
npm run tauri dev      # run the app from source
cargo check            # fast compile gate (repo root; covers core + shell,
                       #   incl. the C++ engines — first build ~1-2 min extra)
cargo test -p scriva-core   # core unit tests (no cmake needed: local-models off)
```

## Git workflow

`main` is protected — direct pushes are rejected. Every change, no matter how small:

1. `git switch main && git pull`, then `git switch -c <topic-branch>`
2. Run `cargo fmt` and sweep AppleDouble junk (`find . -name '._*' -delete`)
   before committing; push with `git push -u origin <topic-branch>`
3. Open a PR with a clear description of what changed and why:
   `gh pr create --title "..." --body "..."`
4. Wait for the `ci` check to pass, then squash-merge:
   `gh pr merge --squash --delete-branch` (no approvals required — solo
   maintainer self-merges)

## CI/CD

- GitHub Actions (`.github/workflows/ci.yml`) runs on every push and PR to
  `main`, on ubuntu: tracked-`._*` guard → `cargo fmt --check` →
  `cargo check -p scriva-core` → `cargo test -p scriva-core`. The Tauri shell
  is NOT built in CI (needs macOS + cmake + long C++ builds) — `cargo check`
  at the repo root remains a local pre-push gate.
- Run `cargo fmt` before committing — CI enforces it.
- Release builds (sign/notarize/staple dmg) stay manual per the README.

## macOS gotchas

- **Accessibility permission is the #1 failure mode**: without it,
  transcription works but no text is typed (silent failure). The UI surfaces
  this with a warning + deep link. In dev, macOS may require re-granting after
  rebuilds (binary signature changes).
- **AX focus probe must stay fail-open** (`src-tauri/src/focus.rs`): each AX
  query is sync IPC to the focused app with a 6 s default timeout — we set
  `AXUIElementSetMessagingTimeout(0.25)` on the system-wide element so a
  beachballed app can't stall the pipeline. Chromium (Chrome/Electron) builds
  its AX tree lazily: until an AX client touches it, a focused web text field
  reports as opaque `AXWebArea`/`AXGroup` — those roles must NEVER divert to
  the clipboard (our own query enables Chromium AX, so second and later
  dictations see real roles). Divert only on an explicit non-text role AND
  `AXValue` confirmed not settable.
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
  injection targets the frontmost app. It stays visible through the whole
  pipeline (waveform while recording, then "Transcribing…"/"Polishing…" text,
  and a steady "Copied — ⌘V to paste" for ~2 s when the clipboard divert fires);
  stages are pushed from Rust via `overlay::set_stage` → `window.eval`, so the
  overlay webview needs no Tauri API or capability grants. It is built `focused(false)` +
  `set_ignore_cursor_events(true)`; never call `set_focus()` on it. Its
  transparency requires BOTH `"macOSPrivateApi": true` (tauri.conf.json) and
  the `macos-private-api` cargo feature — removing either breaks the build or
  the transparency. To float above native-fullscreen apps it uses a small
  objc2 bridge (`raise_above_fullscreen`, macOS-only): `setLevel:` to
  `NSStatusWindowLevel` (25) + OR's `NSWindowCollectionBehaviorFullScreenAuxiliary`
  (1 << 8) onto Tauri's `canJoinAllSpaces`. `ns_window()` is main-thread-only,
  so it runs via `run_on_main_thread`, and is re-asserted after each `show()`.
- Machine-local, git-ignored `.cargo/config.toml` at the repo root redirects
  the cargo target dir off this exFAT volume to
  `/Users/soltan/.cargo/target-scriva` (AppleDouble `._*` sidecars break
  tauri-build's globbing). It must stay at the root so workspace-level cargo
  invocations pick it up; run cargo from the repo root.
- **Release builds ignore `.env`**: the `effective_key` env override is
  `#[cfg(debug_assertions)]`-gated, so a bundled app only sees keys entered in
  the Settings UI. Don't debug "no key configured" in a release build against
  `.env`.
- **Tauri notarizes the .app but NOT the .dmg**: a signed-and-notarized build
  still produces a dmg that Gatekeeper rejects as "Unnotarized Developer ID"
  until the dmg itself is submitted (`xcrun notarytool submit <dmg> … --wait`)
  and stapled (`xcrun stapler staple <dmg>`). Verify with
  `spctl -a -vv -t install <dmg>`.
- **Web-asset edits don't trigger a rebuild**: `generate_context!` embeds
  `src/` into the binary, but cargo doesn't track those files — a build after
  editing only `src/*.html` is a no-op that keeps the OLD embedded assets
  (same trap as Info.plist). `touch src-tauri/src/lib.rs` before
  `npm run tauri build` to force the re-embed.
- **ggml Metal asserts at exit if a local model is still loaded**: the adapter
  caches are statics (never dropped), and ggml's Metal teardown runs
  `GGML_ASSERT(residency sets == 0)` in an atexit destructor — abort (SIGABRT)
  on quit. `lib.rs` handles this: `RunEvent::Exit` calls
  `unload_local_transcriber/cleaner()` before the process exits. Keep that if
  the run loop is ever restructured.
- llama.cpp logs are silenced via `void_logs()` and whisper via
  `install_logging_hooks()`, but a few `ggml_metal_device_init:` lines still
  reach stderr on first model load — device info only, no transcript content.
- Sweep AppleDouble junk after editing files and before builds:
  `find src src-tauri -name '._*' -delete` — `generate_context!` embeds
  everything in `src/` (a stray `._overlay.html` would ship inside the app),
  and tauri-build reads `src-tauri/capabilities/` where a `._default.json`
  sidecar fails the build ("stream did not contain valid UTF-8"). Bundle
  artifacts land in the redirected target dir under
  `release/bundle/{macos,dmg}/`. If `codesign` ever complains about
  "detritus", `xattr -cr` the built `.app` and re-check the embedded sources.

## License

MIT, Copyright (c) 2026 Soltan Garayev.
