//! Settings model, defaults, and the dev-only `.env` key override.

use serde::{Deserialize, Serialize};

/// User settings. Field names stay snake_case because the settings UI sends and
/// receives exactly these JSON keys.
///
/// NOTE: intentionally **no `Debug` derive**. This struct holds plaintext API
/// keys; a `Debug` impl makes it a one-liner to leak them via `{:?}` in a log
/// or panic message. Do not add `Debug` here (invariant: no secret leakage).
#[derive(Clone, Serialize, Deserialize)]
pub struct Settings {
    // `#[serde(default)]` per field so older/partial stores still load.
    #[serde(default = "default_transcription")]
    pub transcription_provider: String,
    /// Pinned transcription model ID; `""` = use the provider's default model.
    #[serde(default)]
    pub transcription_model: String,
    #[serde(default = "default_cleanup")]
    pub cleanup_provider: String,
    /// Pinned cleanup model ID; `""` = use the provider's default model.
    #[serde(default)]
    pub cleanup_model: String,
    #[serde(default = "default_hotkey")]
    pub hotkey: Vec<String>,
    #[serde(default)]
    pub groq_key: String,
    #[serde(default)]
    pub openai_key: String,
    #[serde(default)]
    pub claude_key: String,
    #[serde(default)]
    pub gemini_key: String,
}

fn default_transcription() -> String {
    "groq".to_string()
}
fn default_cleanup() -> String {
    "none".to_string()
}
fn default_hotkey() -> Vec<String> {
    vec!["⌥".to_string(), "Space".to_string()]
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            transcription_provider: default_transcription(),
            transcription_model: String::new(),
            cleanup_provider: default_cleanup(),
            cleanup_model: String::new(),
            hotkey: default_hotkey(),
            groq_key: String::new(),
            openai_key: String::new(),
            claude_key: String::new(),
            gemini_key: String::new(),
        }
    }
}

/// Resolve the API key actually used for a provider.
///
/// In debug builds, a non-empty `OPENWISPR_*` env var (loaded from a dev-only
/// `.env`) overrides the stored key so the pipeline can be iterated without
/// re-typing keys. `OPENWISPR_OPENAI_KEY` serves both OpenAI layers. In release
/// builds env vars are ignored — keys come only from the store.
pub fn effective_key(settings: &Settings, provider: &str) -> String {
    #[cfg(debug_assertions)]
    {
        let env_var = match provider {
            "groq" => Some("OPENWISPR_GROQ_KEY"),
            "openai" => Some("OPENWISPR_OPENAI_KEY"),
            "claude" => Some("OPENWISPR_CLAUDE_KEY"),
            "gemini" => Some("OPENWISPR_GEMINI_KEY"),
            _ => None,
        };
        if let Some(var) = env_var {
            if let Ok(val) = std::env::var(var) {
                if !val.trim().is_empty() {
                    return val;
                }
            }
        }
    }
    match provider {
        "groq" => settings.groq_key.clone(),
        "openai" => settings.openai_key.clone(),
        "claude" => settings.claude_key.clone(),
        "gemini" => settings.gemini_key.clone(),
        _ => String::new(),
    }
}
