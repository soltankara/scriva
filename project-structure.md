# Project structure

Annotated map of the repository. **Keep this file in sync**: whenever a file or
folder is added, moved, renamed, or deleted, update this document in the same
change.

The repo is a Cargo workspace with two crates: `crates/scriva-core` (the
platform-independent engine — future iOS/Windows shells reuse it) and
`src-tauri` (the macOS desktop shell). The dividing rule: anything that would
have to be rewritten per OS (mic capture, hotkey, text injection, tray,
settings persistence) lives in the shell; everything identical on every
platform (provider HTTP, audio processing, settings model) lives in core.

```
scriva/
├── CLAUDE.md                        # Instructions + architecture invariants for Claude Code
├── README.md                        # Public-facing readme: what it is, setup, milestones
├── project-desc.md                  # SOURCE OF TRUTH: full product/design description
├── project-structure.md             # This file
├── LICENSE                          # MIT, Soltan Garayev
│
├── Cargo.toml                       # Workspace root: members + shared [workspace.dependencies]
├── Cargo.lock                       # Single workspace lockfile (tracked in git)
├── .cargo/
│   └── config.toml                  # MACHINE-LOCAL, git-ignored: redirects cargo target dir to
│                                    #   /Users/soltan/.cargo/target-scriva (repo is on exFAT;
│                                    #   AppleDouble ._* sidecars break tauri-build globbing).
│                                    #   Must stay at the root; run cargo from the repo root.
├── .gitignore                       # Ignores /target/, /.cargo/, .env, node_modules, ._* junk
├── .env.example                     # Template for dev-only API-key overrides (SCRIVA_*_KEY)
│
├── package.json                     # Only dev dep: @tauri-apps/cli; script: npm run tauri
├── package-lock.json                # npm lockfile
│
├── scriva-icon.svg               # SOURCE icon (waveform on light rounded square); input for
│                                    #   `npx tauri icon scriva-icon.svg`, which regenerates
│                                    #   src-tauri/icons/ (but never the hand-made tray glyphs)
├── .claude/
│   └── agents/
│       └── tauri.md                 # tauri subagent definition: owns src-tauri/ + crates/ work
│
├── landing/
│   └── index.html                   # Static marketing landing page ("Scriva" branding) —
│                                    #   single self-contained file (inline CSS/JS, no build
│                                    #   step, no deps). NOT loaded by the app; deployable
│                                    #   as-is to any static host. Download/GitHub URLs are
│                                    #   placeholders marked with <!-- PLACEHOLDER --> comments.
│
├── crates/
│   └── scriva-core/              # ── THE ENGINE (platform-independent) ──
│       │                            # Invariant #8: may never depend on tauri, tauri-plugin-*,
│       │                            # cpal, tokio, or any OS framework.
│       ├── Cargo.toml               # Minimal deps: reqwest, hound, serde, serde_json, async-trait
│       └── src/
│           ├── lib.rs               # Module declarations + pub use re-exports (crate surface)
│           ├── settings.rs          # Settings struct (NO Debug derive — holds API keys),
│           │                        #   defaults, effective_key() .env override (debug builds)
│           ├── audio.rs             # Audio PROCESSING (not capture): downmix to mono, resample
│           │                        #   to 16 kHz, silence/too-short gates (RMS), WAV encode
│           │                        #   via hound. Unit tests live here.
│           └── providers/           # One adapter file per AI provider — the backbone.
│               │                    # Adding a provider = one new file + one factory line.
│               ├── mod.rs           # Transcriber + Cleaner traits, ProviderError, shared
│               │                    #   reqwest clients, CLEANUP_PROMPT (injection-hardened),
│               │                    #   factories make_transcriber() / make_cleaner()
│               ├── groq.rs          # Groq whisper-large-v3 (default transcriber)
│               ├── openai_transcribe.rs  # OpenAI whisper-1 (transcriber)
│               ├── claude.rs        # Anthropic Claude Haiku (CLEANUP-ONLY — no STT API)
│               ├── openai_clean.rs  # OpenAI gpt-4o-mini (cleaner)
│               └── gemini.rs        # Google Gemini 2.0 Flash (cleaner)
│
├── src/                             # ── UI (vanilla web, NO build step) ──
│   ├── index.html                   # The entire settings window: HTML/CSS/JS in one file.
│   │                                #   Talks to Rust via window.__TAURI__.core.invoke.
│   │                                #   Also hosts the first-run onboarding layer (#onboard,
│   │                                #   5-step machine over the settings view; shown until
│   │                                #   set_onboarded).
│   │                                #   Dev watcher does NOT watch this — restart dev to see edits.
│   └── overlay.html                 # Recording-indicator pill (window label "overlay"): pure
│                                    #   HTML/CSS, no Tauri API, so it needs no capability grant.
│                                    #   Shown/hidden + positioned by src-tauri/src/overlay.rs.
│
└── src-tauri/                       # ── THE MACOS DESKTOP SHELL (Tauri 2) ──
    ├── Cargo.toml                   # Shell crate `scriva` (lib scriva_lib); depends on
    │                                #   scriva-core + tauri stack + cpal + tokio + dotenvy
    ├── tauri.conf.json              # App config: identifier com.scriva.app, hidden window,
    │                                #   withGlobalTauri, frontendDist ../src, bundle settings
    ├── Info.plist                   # NSMicrophoneUsageDescription, LSUIElement (background
    │                                #   agent), CFBundleIdentifier for the dev binary.
    │                                #   Gotcha: edits need a lib.rs touch to re-embed.
    ├── build.rs                     # Standard tauri-build script
    ├── capabilities/
    │   └── default.json             # Tauri 2 permission grants for the webview (IPC surface)
    ├── gen/                         # GENERATED by Tauri CLI (schemas for config validation).
    │   └── schemas/                 #   Do not edit; git-ignored.
    ├── icons/                       # App + tray icons.
    │   ├── icon.icns                # macOS app icon (from `tauri icon`)
    │   ├── 32x32.png, 128x128.png, 128x128@2x.png  # Size variants (generated; in bundle.icon)
    │   ├── tray.png                 # Menu-bar glyph, idle. MUST be monochrome-with-alpha:
    │   │                            #   icon_as_template(true) renders alpha channel only.
    │   └── tray-rec.png             # Menu-bar glyph while recording (bordered variant)
    └── src/
        ├── main.rs                  # Binary entry point; calls scriva_lib::run()
        ├── lib.rs                   # App wiring: AppState, global hotkey registration +
        │                            #   press/release handler, tray creation + recording-icon
        │                            #   swap, run_pipeline (capture→encode→transcribe→clean→
        │                            #   inject), builder/setup, first-run window show (when
        │                            #   not onboarded), Accessory activation policy (set
        │                            #   BEFORE .run(), never in setup()). CloseRequested
        │                            #   handler is scoped to the "main" window.
        ├── overlay.rs               # Recording-indicator overlay window (label "overlay"):
        │                            #   built once hidden in setup; show()/hide() on hotkey
        │                            #   press/release; click-through + non-focusable; positioned
        │                            #   bottom-center of the display under the cursor. objc2
        │                            #   bridge raises it to NSStatusWindowLevel + fullscreen-
        │                            #   auxiliary so it floats over native-fullscreen apps.
        ├── menu_width.rs            # macOS-only: widens the tray NSMenu panel. Tauri/muda
        │                            #   expose no NSMenu handle, so it observes
        │                            #   NSMenuDidBeginTrackingNotification (objc2 + block2) and
        │                            #   calls setMinimumWidth: on our 2-item tray menu.
        ├── commands.rs              # The nine #[tauri::command] IPC handlers (contract in
        │                            #   CLAUDE.md): load/save settings, test_provider, set_hotkey,
        │                            #   check_permissions, request_microphone, request_accessibility,
        │                            #   get_onboarded, set_onboarded
        ├── config.rs                # Settings persistence (tauri-plugin-store, settings.json),
        │                            #   first-run `onboarded` flag (sibling store key, kept out
        │                            #   of core's Settings), hotkey UI-token → accelerator
        │                            #   mapping (⌥→Alt etc.) + its tests. Re-exports
        │                            #   Settings/effective_key from core.
        ├── audio.rs                 # Mic CAPTURE only (cpal): dedicated OS thread owns the
        │                            #   !Send stream, ships samples over mpsc. Also mic TCC
        │                            #   status/request (AVFoundation via objc2). Re-exports
        │                            #   the processing fns from scriva_core::audio.
        └── inject.rs                # macOS text injection: CGEvent Unicode path (chunked)
                                     #   + AXIsProcessTrusted accessibility check/prompt
```

## Not in git / generated

- `node_modules/` — npm deps (the Tauri CLI).
- `/target/` — would be cargo's build dir, but on this machine builds are
  redirected to `/Users/soltan/.cargo/target-scriva` via `.cargo/config.toml`.
- `.env` — real dev API keys (never committed).
- `._*` files — AppleDouble sidecars the exFAT volume creates; junk, ignored.
