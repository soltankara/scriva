//! OpenAI cleanup (gpt-4o-mini) via chat completions.

use async_trait::async_trait;
use serde_json::json;

use super::{
    check_response, model_not_available, net_err, require_key, Cleaner, ProviderError,
    CLEANUP_CLIENT, CLEANUP_PROMPT,
};

const NAME: &str = "OpenAI";
const MODEL: &str = "gpt-4o-mini";
const CHAT_URL: &str = "https://api.openai.com/v1/chat/completions";
const MODELS_URL: &str = "https://api.openai.com/v1/models";

pub struct OpenAiClean {
    key: String,
    /// Pinned model ID; empty = use `MODEL`.
    model: String,
}

impl OpenAiClean {
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
impl Cleaner for OpenAiClean {
    async fn clean(&self, raw: &str) -> Result<String, ProviderError> {
        let body = json!({
            "model": self.effective_model(),
            "messages": [
                { "role": "system", "content": CLEANUP_PROMPT },
                { "role": "user", "content": raw },
            ],
        });

        let resp = CLEANUP_CLIENT
            .post(CHAT_URL)
            .bearer_auth(&self.key)
            .json(&body)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;
        let json: serde_json::Value = resp.json().await.map_err(net_err)?;
        Ok(json
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string())
    }

    async fn test(&self) -> Result<String, ProviderError> {
        let resp = CLEANUP_CLIENT
            .get(MODELS_URL)
            .bearer_auth(&self.key)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;

        // Validate a pinned model against the list (plain IDs, exact match).
        // The list is shared across OpenAI layers — membership only, no
        // capability filtering.
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
