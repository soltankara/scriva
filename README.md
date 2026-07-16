# Scriva

[![CI](https://github.com/soltankara/scriva/actions/workflows/ci.yml/badge.svg)](https://github.com/soltankara/scriva/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

**Open-source voice dictation for macOS. Your keys, your models, your data.**

Hold a hotkey, speak, release — your words are transcribed and typed into
whatever app has focus: email, chat, your code editor, anywhere a text cursor
lives. Scriva runs quietly in the menu bar; there's no window to type into
and no dock icon.

It's a community alternative to commercial dictation apps like Wispr Flow,
with one deliberate difference the paid tools can't match: **you bring your
own API key and choose your own AI providers.** No subscription, no hosted
backend, no account, no telemetry. Audio and text go only to the provider you
picked, directly from your machine.

## How it works

```
hold hotkey → record → release
  → Transcription  (audio → text)      Groq whisper-large-v3 (default), OpenAI whisper-1,
                                       or Local (on-device whisper.cpp)
  → Cleanup        (optional polish)   None (default), Claude, OpenAI, Gemini,
                                       or Local (on-device llama.cpp)
  → text is typed into the focused app
```

The cleanup layer removes filler words ("um", "uh"), fixes punctuation and
capitalization, applies your spoken self-corrections ("at five — actually,
make it ten"), and never changes your meaning. Claude appears only as a
cleanup option — Anthropic has no speech-to-text API.

Each provider also has a model picker in Settings (e.g. Groq's faster
`whisper-large-v3-turbo`, or Claude Sonnet for maximum cleanup quality).
"Default" follows the app's recommended model for that provider.

## Fully offline — no keys, no cloud

Pick **Local (on-device)** for either layer (or both) in Settings and download
a model right in the app. With local transcription + local (or no) cleanup,
dictation needs **zero API keys and zero network** — audio and text never
leave your Mac. Models run on Apple Silicon's GPU via Metal.

| Model | Layer | Size | Notes |
|---|---|---|---|
| Whisper Tiny / Base / Small | transcription | 74 / 141 / 465 MB | Small is the recommended balance |
| Whisper Large v3 Turbo (q5) | transcription | 547 MB | best accuracy, a bit slower |
| Llama 3.2 1B (Q4_K_M) | cleanup | 0.8 GB | fastest, may miss spoken corrections |
| Llama 3.2 3B (Q4_K_M) | cleanup | 1.9 GB | recommended default |
| Qwen3 4B Instruct (Q4_K_M) | cleanup | 2.3 GB | best quality |

Downloaded models are stored in
`~/Library/Application Support/com.scriva.app/models/`. A selected local model
stays loaded in RAM between dictations (that's what makes it fast); switching
back to a cloud provider frees it, and after ~10 minutes without a dictation
it is released automatically (and re-warmed the moment you press the hotkey).

## Install (macOS)

1. Download the dmg from [the landing page](https://soltankara.github.io/scriva/)
   or the [releases page](https://github.com/soltankara/scriva/releases/latest)
   (Apple Silicon).
2. Open the dmg and drag **Scriva** into **Applications**.
3. Launch it — the app is signed and notarized, so it opens with no warnings.
4. First-run onboarding walks you through Microphone + Accessibility
   permissions and your first API key (a free Groq key works).

## Getting started (Milestone 1 — run from source)

Requirements: macOS 12+, [Rust](https://rustup.rs), Node.js, and cmake
(`brew install cmake` — the on-device engines build from C++). Either an API
key (a free [Groq](https://console.groq.com) key is the fastest way in) or a
downloaded local model gets you dictating.

```sh
git clone https://github.com/soltankara/scriva.git && cd scriva
npm install
npm run tauri dev
```

Then:

1. Click the Scriva icon in the menu bar → **Settings…**
2. Paste your Groq (or OpenAI) key and hit **Test** — you'll get an immediate
   ✓ or a plain-English error.
3. Grant **Microphone** access when macOS asks (first recording).
4. Grant **Accessibility** access (Settings window → Permissions → Open System
   Settings). ⚠️ This is the #1 setup issue: without it, your speech is
   transcribed but **no text appears** in the target app. In development,
   macOS may ask you to re-grant after rebuilds because the binary changes.
5. Hold **⌥ Space** (configurable), speak, release. Text appears where your
   cursor is.

Optional: copy `.env.example` to `.env` for dev-only key overrides (never
committed, debug builds only).

## Releasing (maintainers)

**Releases are automated.** Every merge to `main` triggers the Release
workflow (`.github/workflows/release.yml`) on a macOS runner: it builds,
signs, notarizes (the `.app` *and* the `.dmg`), and publishes a GitHub
release with all five assets — the versioned dmg, the stable-named
`Scriva-macOS.dmg` the landing page links to, the updater payload
(`.app.tar.gz` + `.sig`), and the `latest.json` manifest the in-app updater
fetches. Nothing to do by hand.

- **Versioning is calendar-based**: tag `v<yyyymmdd>` (UTC), internal semver
  `<yyyymmdd>.0.0`; a second release the same day becomes `v<yyyymmdd>.1` /
  `<yyyymmdd>.1.0`. The version is computed in the workflow — the checked-in
  `0.1.0` strings are placeholders and never bumped.
- **Doc-only change?** Put `[skip release]` in the PR title (it lands in the
  squash-merge commit message). Don't use `[skip ci]` — that skips tests too.
- **Re-run / test**: Actions → Release → Run workflow; the `dry_run` input
  builds and gates everything but uploads the assets as a workflow artifact
  instead of publishing.
- The workflow needs seven repo secrets: `APPLE_CERTIFICATE` (base64 .p12),
  `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
  `APPLE_PASSWORD` (app-specific), `APPLE_TEAM_ID`,
  `TAURI_SIGNING_PRIVATE_KEY` (the minisign key; its password is hardcoded
  empty in the workflow).

### Manual fallback

The by-hand procedure below is disaster recovery (and documents what the
workflow automates).

```sh
cd <repo root>
find src src-tauri -name '._*' -delete   # sweep AppleDouble junk (exFAT)
touch src-tauri/src/lib.rs               # force re-embed of src/ web assets

# Sign the updater artifacts. Empty password; key lives OUTSIDE the repo.
export TAURI_SIGNING_PRIVATE_KEY="$(cat /Users/soltan/.tauri/scriva-updater.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=""

npm run tauri build
```

This produces `Scriva.app` and a `.dmg` under the cargo target dir
(`release/bundle/{macos,dmg}/`). Because `bundle.createUpdaterArtifacts` is on,
it *also* emits the in-app updater payload next to the `.app`
(`release/bundle/macos/`): a `Scriva.app.tar.gz` and its detached signature
`Scriva.app.tar.gz.sig`. Notes:

- **Release builds ignore `.env`** — the dev key override is compiled out.
  Enter API keys in the Settings window.
- The bundled app is a different code identity from the dev binary, so macOS
  will ask for Microphone and Accessibility again on first run.
- To sign + notarize (required for a warning-free download experience), set
  the Apple env vars before building — the bundler then signs, notarizes, and
  staples automatically:

```sh
export APPLE_SIGNING_IDENTITY="Developer ID Application: <name> (<TEAMID>)"
export APPLE_ID="<apple id email>"
export APPLE_PASSWORD="<app-specific password>"
export APPLE_TEAM_ID="<TEAMID>"
npm run tauri build
```

(The sweep matters beyond the embedded `src/` assets: tauri-build also chokes
on `._*` sidecars in `src-tauri/capabilities/`.)

The bundler notarizes and staples the **.app only** — the `.dmg` wrapper still
needs its own ticket or downloads get flagged:

```sh
xcrun notarytool submit <dmg> --keychain-profile scriva-notary --wait
xcrun stapler staple <dmg>
spctl -a -vv -t install <dmg>   # expect: accepted, Notarized Developer ID
```

### In-app updater (manual "Check for Updates…")

The tray's **Check for Updates…** item contacts GitHub Releases only when the
user clicks it (nothing runs automatically). For it to find an update, each
release must publish the updater payload *and* a `latest.json` manifest that
the app fetches from `releases/latest/download/latest.json`.

The signing keypair lives outside the repo (never commit it):

- Private key: `/Users/soltan/.tauri/scriva-updater.key` (empty password) —
  **back this up.** Losing it means future builds can't be signed and users
  must re-download manually.
- Public key: already baked into `src-tauri/tauri.conf.json`
  (`plugins.updater.pubkey`). Only rotate it in lockstep with the private key.

Per release, the Release workflow does all of this automatically. If ever
releasing by hand (assuming `TAURI_SIGNING_PRIVATE_KEY*` were set at build
time, so the `.sig` exists):

1. The workflow computes the calver version and patches
   `src-tauri/tauri.conf.json` in its own workspace; by hand, set it yourself
   before building.
2. Upload these to the GitHub release, alongside the notarized `.dmg`:
   - `Scriva.app.tar.gz` — rename to a versioned asset, e.g.
     `Scriva_<version>_aarch64.app.tar.gz`.
   - `Scriva.app.tar.gz.sig` — the detached signature.
   - `latest.json` — the manifest below.
3. Hand-write (or generate) `latest.json`. `signature` is the **entire
   contents** of the `.sig` file; `url` is the versioned `.app.tar.gz` release
   asset URL:

```json
{
  "version": "20260716.0.0",
  "pub_date": "2026-07-16T00:00:00Z",
  "platforms": {
    "darwin-aarch64": {
      "signature": "<paste the full contents of Scriva.app.tar.gz.sig>",
      "url": "https://github.com/soltankara/scriva/releases/download/v20260716/Scriva_20260716.0.0_aarch64.app.tar.gz"
    }
  }
}
```

Today's builds are Apple-Silicon only (`darwin-aarch64`). If a universal /
Intel build ever ships, add a sibling `darwin-x86_64` entry with its own
signature + url. The updater downloads the `.app.tar.gz`, verifies the
signature against the baked-in pubkey, installs, and relaunches — an unsigned
or mismatched artifact is rejected, and a plain dev build (no signature) simply
surfaces the calm "Couldn't check for updates" dialog rather than updating.

## Costs

You pay your chosen provider directly, per use. Rough orders of magnitude
(check current pricing pages):

| Provider | Used for | Ballpark |
|---|---|---|
| Local (whisper.cpp / llama.cpp) | both | free — runs on your Mac |
| Groq (whisper-large-v3) | transcription | ~$0.04–0.11 per hour of audio |
| OpenAI (whisper-1) | transcription | ~$0.36 per hour of audio |
| Claude (Haiku) | cleanup | fractions of a cent per dictation |
| OpenAI (gpt-4o-mini) | cleanup | fractions of a cent per dictation |
| Gemini (Flash) | cleanup | fractions of a cent per dictation |

Typical daily dictation use costs pennies per month.

## Privacy

- Audio is recorded only while you hold the hotkey.
- Audio/text is sent only to the provider you configured, with your key.
- No analytics, no phone-home, no transcript logging.
- API keys are stored locally and never leave your machine (except to
  authenticate with your chosen provider).

## Roadmap

- **M1 (done):** macOS MVP — the full pipeline, run from source.
- **M2 (done):** Signed, notarized `.dmg` with first-run onboarding.
- **M3 (done):** Fully local dictation — on-device Whisper transcription *and*
  on-device cleanup LLM, in-app model downloads, zero keys, zero cloud.
- **M4:** Streaming transcription — text appears as you speak.
- **M5:** Windows and Linux.

Adding a new AI provider is designed to be trivial: one adapter file + one
registry line (`crates/scriva-core/src/providers/`). PRs welcome — see
[CONTRIBUTING.md](CONTRIBUTING.md), and report security issues privately per
[SECURITY.md](SECURITY.md).

## License

MIT © 2026 Soltan Garayev
