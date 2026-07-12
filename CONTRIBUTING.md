# Contributing to Scriva

Thanks for helping build open-source dictation! Before writing code, skim:

- `CLAUDE.md` — architecture invariants (the short list of rules that must not break)
- `project-structure.md` — a map of every file and folder
- `project-desc.md` — the full product spec and milestones

## Development setup

Requirements: macOS 12+, [Rust](https://rustup.rs), Node.js, and cmake
(`brew install cmake`).

```sh
git clone https://github.com/soltankara/scriva.git && cd scriva
npm install
npm run tauri dev
```

Useful commands:

```sh
cargo fmt                   # format (CI enforces cargo fmt --check)
cargo test -p scriva-core   # core unit tests (no cmake needed)
cargo check                 # full compile gate incl. the Tauri shell (macOS)
```

Optional: copy `.env.example` to `.env` for dev-only API key overrides
(git-ignored, debug builds only).

## Adding a provider

The most-wanted kind of contribution, and it's designed to be trivial:

1. Add one adapter file in `crates/scriva-core/src/providers/` implementing
   `Transcriber` and/or `Cleaner` (copy an existing adapter, e.g. `gemini.rs`).
2. Add one factory line in `crates/scriva-core/src/providers/mod.rs`.
3. Add the provider's curated models to `MODEL_OPTS` in `src/index.html`.

Rules that apply (see `CLAUDE.md` for the full list):

- Every adapter needs a cheap `test()` method with human-readable errors.
- Claude is cleanup-only — never add it to transcription.
- Never log API keys, audio, or transcripts — even in debug builds.

## Pull request checklist

- [ ] `cargo fmt --check` passes
- [ ] `cargo test -p scriva-core` is green
- [ ] `cargo check` compiles (macOS)
- [ ] `project-structure.md` updated if files were added/moved/renamed
- [ ] No secrets anywhere: never paste real API keys into code, commits, or
      PR descriptions (see the warnings in `.env.example`)

## Scope

Current milestone and roadmap live in the README. Please don't pull future
milestone scope (streaming, Windows/Linux) forward without discussing in an
issue first.

## License

By contributing, you agree that your contributions will be licensed under the
[MIT License](LICENSE).
