//! Groq transcription (Whisper-large-v3) — the default, fastest provider.

use async_trait::async_trait;
use reqwest::multipart::{Form, Part};

use super::{
    check_response, model_not_available, net_err, require_key, ProviderError, Transcriber,
    AUDIO_CLIENT,
};

const NAME: &str = "Groq";
const MODEL: &str = "whisper-large-v3";
const TRANSCRIBE_URL: &str = "https://api.groq.com/openai/v1/audio/transcriptions";
const MODELS_URL: &str = "https://api.groq.com/openai/v1/models";

pub struct Groq {
    key: String,
    /// Pinned model ID; empty = use `MODEL`.
    model: String,
}

impl Groq {
    pub fn new(key: &str, model: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            key: require_key(NAME, key)?,
            model: model.trim().to_string(),
        })
    }

    /// The model actually sent to the API: the pinned one, else the default.
    fn effective_model(&self) -> &str {
        if self.model.is_empty() {
            MODEL
        } else {
            &self.model
        }
    }
}

#[async_trait]
impl Transcriber for Groq {
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, ProviderError> {
        let part = Part::bytes(wav)
            .file_name("audio.wav")
            .mime_str("audio/wav")
            .map_err(net_err)?;
        let form = Form::new()
            .part("file", part)
            .text("model", self.effective_model().to_string())
            .text("response_format", "json");

        let resp = AUDIO_CLIENT
            .post(TRANSCRIBE_URL)
            .bearer_auth(&self.key)
            .multipart(form)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;
        let json: serde_json::Value = resp.json().await.map_err(net_err)?;
        Ok(json
            .get("text")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string())
    }

    async fn test(&self) -> Result<String, ProviderError> {
        let resp = AUDIO_CLIENT
            .get(MODELS_URL)
            .bearer_auth(&self.key)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;

        // Validate a pinned model against the list (plain IDs, exact match).
        if !self.model.is_empty() {
            let json: serde_json::Value = resp.json().await.map_err(net_err)?;
            let found = json
                .get("data")
                .and_then(|d| d.as_array())
                .is_some_and(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                        .any(|id| id == self.model)
                });
            if !found {
                return Err(model_not_available(NAME, &self.model));
            }
        }
        Ok(self.effective_model().to_string())
    }
}
