# Scriva — Project Description & Build Spec

> This document is the source of truth for the project. It is written to be fed
> to AI build tools (Claude Code, Claude Design, Google Stitch) so they share a
> consistent understanding of scope, stack, and target devices. Implement
> strictly within the milestone you are working on; do not pull future-milestone
> scope forward unless asked.

---

## 1. What this project is

Scriva is an **open-source voice dictation tool**. The user holds a hotkey,
speaks, releases, and their speech is transcribed and typed into whatever
application currently has focus — email, chat, code editor, browser, anywhere a
text cursor lives. It runs quietly in the background as a menu-bar / tray
utility, not as a window you type into.

It is a community alternative to commercial dictation apps (e.g. Wispr Flow).
The deliberate differentiator is **bring-your-own-key and model-agnostic**: the
user supplies their own API keys and chooses which AI providers to use. There is
no subscription, no hosted backend, no vendor lock-in, and no telemetry. This is
a positioning the paid competitors structurally cannot match, and it is the
project's reason to exist — it should be reflected in the README, the UI copy,
and every product decision.

**License:** MIT. Copyright (c) 2026 Soltan Garayev.

### Core principles (apply to every milestone)
- **Privacy by default.** Audio and text go only to the provider the user chose,
  using the user's own key. No analytics, no phone-home, no logging of
  transcripts to disk beyond what the OS/app needs to function.
- **Latency is the product.** A dictation tool lives or dies on how fast text
  appears after the user stops talking. Treat perceived speed as a primary
  feature, not a nice-to-have. Prefer the fastest provider as the default.
- **Model-agnostic.** Adding or swapping an AI provider must be a small,
  isolated change. Never hard-code a single vendor into core logic.
- **Invisible until summoned.** No dock icon, no window stealing focus. The tool
  is a background agent triggered by a hotkey, configured through a small
  settings window opened from the menu bar.

---

## 2. How it works (conceptual pipeline)

```
[hold hotkey] → record microphone → [release hotkey]
   → Transcriber  (audio → raw text)      [required layer]
   → Cleaner      (raw text → polished)   [optional layer]
   → inject text into the focused app
```

Two **independent, pluggable layers**. This separation is the central
architectural decision and must be preserved:

- **Transcription layer (required):** converts recorded audio to raw text.
  Speech-to-text providers only.
- **Cleanup layer (optional):** takes the raw transcript and removes filler
  words ("um", "uh", "like"), fixes punctuation and capitalization, and applies
  natural formatting — without changing meaning. Any capable LLM can do this.
  A "none" option must always be valid (raw transcript passes straight through).

**Critical provider constraint:** Anthropic's Claude has **no speech-to-text
API** (its API accepts text and images, not audio). Therefore Claude can only
ever be used in the **cleanup** layer, never the transcription layer. Any UI or
config that lists providers must enforce this — Claude must not appear as a
transcription option.

---

## 3. Provider matrix

| Layer | Provider | Role | Notes |
|---|---|---|---|
| Transcription | **Groq** (Whisper-large-v3) | default | Fastest; latency is the product, so this is the recommended default. |
| Transcription | **OpenAI** (whisper-1) | alternative | Same Whisper family; for users who already have an OpenAI key. |
| Cleanup | **none** | default | Raw transcript passes through unchanged. |
| Cleanup | **Claude** (Haiku-class) | option | Cleanup ONLY. Fast/cheap model is the correct choice for short-text formatting. |
| Cleanup | **OpenAI** (GPT-mini-class) | option | Via chat completions. |
| Cleanup | **Gemini** (Flash-class) | option | Via generateContent. |

**Extensibility requirement:** the code must define a `Transcriber` interface
and a `Cleaner` interface. Each provider is an adapter implementing one of them.
Adding a new provider = one new adapter file + one line in a factory/registry.
No other code should need to change. Treat "support my favorite model" as the
project's most common future contribution and make it trivial.

Every provider adapter must expose a cheap **`test()`** method that does a tiny
round-trip to validate the API key, so the settings UI can verify a key the
moment it's entered and fail loudly (e.g. "Groq returned 401") rather than
failing silently mid-dictation.

---

## 4. Technology stack

### Application shell — **Tauri 2 (Rust core + web UI)**
Chosen over Electron (smaller binaries, lower memory; uses the OS webview) and
over Python (Python is fast to prototype but painful to package into a clean,
signed desktop app, and we do not want to rewrite later). Tauri gives us a
native settings window, tray/menu-bar support, global shortcuts, and a signed
installer pipeline as first-class features — exactly the roadmap below.

- **Core logic:** Rust. Handles audio capture, the provider pipeline, hotkey
  events, and OS text injection.
- **Settings UI:** web technologies (HTML/CSS/JS or a light framework) rendered
  in the Tauri webview. This is the surface Claude Design / Stitch will design.
- **Key Tauri pieces:** `tauri-plugin-global-shortcut` (system-wide push-to-talk
  — a window key listener will NOT fire for a hidden menu-bar app, so the native
  plugin is mandatory), `TrayIconBuilder` (menu-bar icon), `tauri-plugin-store`
  (settings persistence).

### Supporting Rust crates
- **Audio capture:** `cpal` (cross-platform; keeps the door open for Windows/
  Linux later without rewriting this layer).
- **Audio encoding:** `hound` (encode captured samples to 16-bit PCM WAV — the
  format all transcription APIs accept).
- **HTTP:** `reqwest` (multipart upload for audio; JSON for cleanup calls).
- **Async:** `tokio`.
- **macOS text injection:** `core-graphics` (synthesize keyboard events via
  CGEvent using the Unicode-string path, so we don't have to map keycodes per
  layout — handles punctuation and any language).

### AI / external services (all user-supplied keys)
- Groq API, OpenAI API, Anthropic API, Google Gemini API. No backend of our own;
  the app calls these providers directly from the user's machine.

### Design tools (your workflow)
- **Google Stitch / Claude Design** → settings-window UI and visual identity.
- **Claude Code** → implementation against this spec.

---

## 5. Milestones & target devices

Each milestone is shippable on its own. **Prove one platform end-to-end before
widening.** Target devices are called out per milestone because they drive the
hardest platform-specific code (hotkey capture, text injection, permissions).

### Milestone 1 — macOS MVP (the core loop)
**Target device:** macOS (Apple Silicon + Intel, macOS 11+).
**Goal:** the full pipeline works for one platform, run from source.

Scope:
- Push-to-talk: hold hotkey → record → release → transcribe → (optional clean) →
  type into focused app. **Batch mode** (record fully, then send) — not
  streaming.
- Transcription: Groq (default) + OpenAI. Cleanup: none / Claude / OpenAI /
  Gemini.
- Menu-bar tray icon with Settings and Quit.
- Settings window: provider pickers, API-key fields (masked), per-key "Test"
  button, configurable hotkey.
- Settings persisted to disk; `.env` override supported for development.
- macOS permission handling: Microphone + **Accessibility** (the latter is
  required for typing into other apps; without it, transcription works but no
  text appears — this is the #1 failure mode and must be surfaced clearly in the
  UI with a link to System Settings).
- Runs as a background agent (no dock icon; `LSUIElement`).

Out of scope for M1: packaged installer, signing, streaming, local models,
other OSes.

**Definition of done:** a developer clones the repo, adds an icon, runs
`npm run tauri dev`, grants permissions, pastes a Groq key, and can dictate into
any macOS app.

### Milestone 2 — Distributable macOS app
**Target device:** macOS, same as M1.
**Goal:** a normal person can install and run it without a terminal or toolchain.

Scope:
- Package as `.dmg` / `.app` via Tauri's bundler.
- **Code signing + notarization** (requires an Apple Developer account, ~$99/yr)
  so it opens without Gatekeeper warnings.
- First-run onboarding: walk the user through granting Microphone + Accessibility
  and entering their first API key (with the live key-test from M1).
- Auto-start on login (optional toggle).
- Polished menu-bar UX: enable/disable toggle, quick access to settings.
  (The recording indicator — tray swap + on-screen overlay pill — already
  shipped during M1.)

**Definition of done:** download a `.dmg`, drag to Applications, open with no
scary warnings, complete onboarding, dictate.

### Milestone 3 — Local / offline transcription
**Target device:** macOS (Apple Silicon benefits most).
**Goal:** remove the hard dependency on a cloud key; enable fully private,
offline, zero-cost transcription.

Scope:
- Add a **local Whisper** transcriber (e.g. `whisper.cpp` / a Rust binding) as a
  new `Transcriber` adapter — proving the M1 abstraction pays off, since this
  should slot in without touching the pipeline.
- Model download/management UX (pick model size: tiny/base/small/medium for the
  speed-vs-accuracy tradeoff).
- "Local" becomes a transcription option alongside Groq/OpenAI; cleanup can
  still be cloud or skipped. A fully offline configuration (local transcribe +
  cleanup "none") must work with **zero API keys**.

**Definition of done:** with no API keys configured, the user can dictate using
an on-device model, and no audio leaves the machine.

### Milestone 4 — Streaming transcription
**Target device:** macOS.
**Goal:** text appears *as the user speaks*, not after they finish — the biggest
perceived-latency upgrade.

Scope:
- Stream audio chunks to the transcriber and inject text incrementally.
- Handle interim vs. finalized results gracefully (don't leave half-corrected
  text behind).
- This is a meaningful re-architecture of the record→send→inject flow; isolate
  it behind the existing interfaces so batch mode remains available as a
  fallback / for providers that don't stream.

**Definition of done:** speaking produces a visibly live, flowing transcription.

### Milestone 5 — Cross-platform: Windows & Linux
**Target devices:** Windows 10/11; Linux (X11 first, then Wayland).
**Goal:** widen beyond macOS now that the model is proven.

Scope:
- **Windows:** text injection via SendInput; no special permission prompts;
  handle the SmartScreen warning for unsigned `.exe` (signing optional/later).
  Package as `.msi` / `.exe`.
- **Linux:** the most variable target. X11 vs Wayland changes how global hotkeys
  and text injection work (Wayland is stricter about synthetic input). Tackle
  X11 first. Package as AppImage / `.deb`.
- Abstract the OS-specific layer (hotkey + injection + permissions) behind a
  trait so each platform is an implementation, mirroring the provider pattern.

**Definition of done:** the core loop works on at least Windows and X11 Linux,
distributed as native packages.

---

## 6. Settings UI — design brief (for Claude Design / Stitch)

A single small, fixed-size window (≈480×620), opened from the menu bar. Not a
full app window. It should feel like a native macOS utility preference pane:
calm, dense-but-clean, no marketing fluff.

Sections, in order:
1. **Header:** product name + one-line tagline ("Hold your hotkey, speak,
   release. Bring your own API key."), plus a live status pill
   (Ready / Listening / Error).
2. **Transcription (required):** provider dropdown (Groq / OpenAI), a model
   dropdown (curated per-provider list; "Default" follows the app's built-in
   choice), masked API-key input with a "Get a key →" link, inline "Test"
   button with a clear ✓/✗ status line (Test also validates a pinned model).
3. **Cleanup (optional):** an on/off toggle ("Polish transcript") with a short
   explainer; when on, a provider dropdown (Claude / OpenAI / Gemini — Claude
   is cleanup-only), a model dropdown (same "Default" semantics), masked
   API-key input + "Test". Off persists as `cleanup_provider: "none"`.
4. **Hotkey:** the push-to-talk shortcut as a click-to-record shortcut
   recorder (click the keycaps, press the new combo).
5. **Permissions status:** live indicators for Microphone (granted / denied /
   not yet requested, with an "Allow" button that triggers the TCC prompt) and
   Accessibility (deep-link to System Settings when missing, with a loud
   warning — it's the #1 silent-failure).
6. **Try it:** a scratch textarea to verify the whole pipeline end-to-end
   (focus it, hold the hotkey, speak — text is injected right there).
7. **Recording indicator:** menu-bar tray icon swap + the header status pill
   while the mic is live, plus an on-screen overlay — a small click-through,
   never-focusable pill (pulsing red dot + animated bars, `src/overlay.html`)
   at the bottom-center of the screen the cursor is on, shown only while the
   hotkey is held.

Settings auto-save (debounced) with a transient "✓ Saved" chip — there is no
Save button; closing the window can never lose changes. Appearance follows the
system (no manual theme toggle).

Tone of copy: plain, trustworthy, privacy-forward. Emphasize "your key, your
data, nothing leaves except to the provider you chose."

States to design for: empty/first-run, key-being-tested (loading), key-valid,
key-invalid (with the actual error text), saved, and "permission not granted."

---

## 7. Things easy to overlook (call-outs)

- **Accessibility permission is the #1 support issue.** Plan the UX around it.
  Silent failure (transcription works, nothing types) confuses everyone.
- **Prompt-injection safety in cleanup.** The cleanup LLM receives transcribed
  speech as input. Its system prompt must instruct it to treat all input as text
  to be formatted, never as instructions to follow — otherwise saying "ignore
  previous instructions and…" could hijack it. Return only cleaned text.
- **Don't truncate long dictation.** Injecting very long strings in a single
  event can be dropped by some apps; chunk the output.
- **Provider errors must be human-readable.** Surface "Groq returned 401" /
  "rate limited" / "no network" plainly. Never fail silently.
- **No secret leakage.** API keys live in the OS-appropriate secure/app-config
  location, never committed, never logged. `.env` is dev-only and git-ignored.
- **Empty/garbage audio.** Handle the case where the user taps the hotkey with no
  speech — don't fire a pointless API call or inject noise.
- **Hotkey conflicts.** The chosen shortcut may collide with an OS or app
  shortcut; let the user reconfigure and warn on obvious conflicts.
- **Cost transparency.** Since users pay per their own key, the README should
  note rough per-provider costs so there are no surprises.
- **Accessibility (the human kind).** A real and underrated user base for
  dictation tools is people with RSI, dyslexia, or limited mobility. Keep the
  trigger ergonomics and onboarding friendly to them.

---

## 8. Repository layout (current scaffold)

```
scriva/
├── README.md
├── PROJECT.md                 ← this file
├── LICENSE                    ← MIT, Soltan Garayev
├── package.json               ← Tauri CLI + dev scripts
├── .env.example               ← dev-only config overrides
├── Cargo.toml                 ← workspace root (scriva-core + src-tauri)
├── crates/
│   └── scriva-core/        ← platform-independent core (no tauri/cpal/OS deps);
│       ├── Cargo.toml            future iOS (UniFFI) and Windows shells reuse it
│       └── src/
│           ├── lib.rs
│           ├── settings.rs    ← Settings model + .env key override
│           ├── audio.rs       ← downmix/resample/WAV encode + silence gate
│           └── providers/
│               ├── mod.rs     ← Transcriber/Cleaner traits + factories
│               ├── groq.rs
│               ├── openai_transcribe.rs
│               ├── claude.rs
│               ├── openai_clean.rs
│               └── gemini.rs
├── src/
│   └── index.html             ← settings UI (to be redesigned via Stitch/Design)
└── src-tauri/                 ← macOS desktop shell
    ├── Cargo.toml
    ├── tauri.conf.json        ← window (hidden), tray, bundle config
    ├── Info.plist             ← mic usage string, LSUIElement
    ├── build.rs
    ├── capabilities/
    │   └── default.json       ← Tauri permission grants
    └── src/
        ├── main.rs
        ├── lib.rs             ← app wiring: hotkey, pipeline, tray, commands
        ├── audio.rs           ← cpal mic capture (processing lives in core)
        ├── inject.rs          ← macOS CGEvent text injection (+ a11y check)
        ├── config.rs          ← settings persistence + hotkey→accelerator mapping
        └── commands.rs        ← #[tauri::command] IPC handlers
```

The provider trait files and the two interfaces in
`crates/scriva-core/src/providers/mod.rs` are the architectural backbone —
preserve that shape as the project grows.
