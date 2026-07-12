# Security Policy

## Reporting a vulnerability

Please do **not** open a public issue for security problems.

Instead, use GitHub's private vulnerability reporting: go to the repository's
**Security** tab → **Report a vulnerability**. Reports are typically
acknowledged within a few days.

## Scope notes

Scriva has no backend, no accounts, and no telemetry — the attack surface is
the local app and its direct connections to the AI provider the user
configured. Reports of particular interest:

- API key leakage (logging, files, crash reports)
- Audio or transcript data going anywhere other than the configured provider
- Prompt-injection paths that make the cleanup LLM emit instructions instead
  of cleaned text
- Insecure handling of downloaded local model files

## Supported versions

Only the latest release is supported with security fixes.
