//! Anthropic Claude cleanup (Haiku). CLEANUP ONLY — never a transcriber.

use async_trait::async_trait;
use serde_json::json;

use super::{
    check_response, model_not_available, net_err, require_key, Cleaner, ProviderError,
    CLEANUP_CLIENT, CLEANUP_PROMPT,
};

const NAME: &str = "Claude";
const MODEL: &str = "claude-3-5-haiku-latest";
const MESSAGES_URL: &str = "https://api.anthropic.com/v1/messages";
const MODELS_URL: &str = "https://api.anthropic.com/v1/models";
const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct Claude {
    key: String,
    /// Pinned model ID; empty = use `MODEL`.
    model: String,
}

impl Claude {
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
impl Cleaner for Claude {
    async fn clean(&self, raw: &str) -> Result<String, ProviderError> {
        let body = json!({
            "model": self.effective_model(),
            "max_tokens": 1024,
            "system": CLEANUP_PROMPT,
            "messages": [{ "role": "user", "content": raw }],
        });

        let resp = CLEANUP_CLIENT
            .post(MESSAGES_URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .json(&body)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;
        let json: serde_json::Value = resp.json().await.map_err(net_err)?;
        Ok(json
            .get("content")
            .and_then(|c| c.get(0))
            .and_then(|b| b.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string())
    }

    async fn test(&self) -> Result<String, ProviderError> {
        let resp = CLEANUP_CLIENT
            .get(MODELS_URL)
            .header("x-api-key", &self.key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;

        // Validate a pinned model. Anthropic's list carries dated snapshot IDs
        // (e.g. `claude-haiku-4-5-20251001`) while curated pins may be dateless
        // (`claude-haiku-4-5`): accept an exact match OR a list entry that adds
        // a date suffix (`<pinned>-…`). Exact match also covers dateless
        // 4.6-gen IDs the list may carry verbatim.
        if !self.model.is_empty() {
            let json: serde_json::Value = resp.json().await.map_err(net_err)?;
            let prefix = format!("{}-", self.model);
            let found = json
                .get("data")
                .and_then(|d| d.as_array())
                .is_some_and(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("id").and_then(|i| i.as_str()))
                        .any(|id| id == self.model || id.starts_with(&prefix))
                });
            if !found {
                return Err(model_not_available(NAME, &self.model));
            }
        }
        Ok(self.effective_model().to_string())
    }
}
