# Scriva

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
  → Transcription  (audio → text)      Groq whisper-large-v3 (default) or OpenAI whisper-1
  → Cleanup        (optional polish)   None (default), Claude, OpenAI, or Gemini
  → text is typed into the focused app
```

The cleanup layer removes filler words ("um", "uh"), fixes punctuation and
capitalization, and never changes your meaning. Claude appears only as a
cleanup option — Anthropic has no speech-to-text API.

Each provider also has a model picker in Settings (e.g. Groq's faster
`whisper-large-v3-turbo`, or Claude Sonnet for maximum cleanup quality).
"Default" follows the app's recommended model for that provider.

## Getting started (Milestone 1 — run from source)

Requirements: macOS 12+, [Rust](https://rustup.rs), Node.js, and at least one
API key (a free [Groq](https://console.groq.com) key is the fastest way in).

```sh
git clone <this repo> && cd scriva
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

## Building a release (maintainers)

```sh
cd <repo root>
find src src-tauri -name '._*' -delete   # sweep AppleDouble junk (exFAT)
npm run tauri build
```

This produces `Scriva.app` and a `.dmg` under the cargo target dir
(`release/bundle/{macos,dmg}/`). Notes:

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

## Costs

You pay your chosen provider directly, per use. Rough orders of magnitude
(check current pricing pages):

| Provider | Used for | Ballpark |
|---|---|---|
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
- **M2 (current):** Signed, notarized `.dmg` with first-run onboarding.
- **M3:** Local/offline transcription (whisper.cpp) — zero keys, zero cloud.
- **M4:** Streaming transcription — text appears as you speak.
- **M5:** Windows and Linux.

Adding a new AI provider is designed to be trivial: one adapter file + one
registry line (`crates/scriva-core/src/providers/`). PRs welcome.

## License

MIT © 2026 Soltan Garayev
