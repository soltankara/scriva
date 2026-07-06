//! Google Gemini cleanup (2.0 Flash) via generateContent.
//!
//! The key goes in the `x-goog-api-key` header, never the `?key=` query param —
//! API keys must not appear in URLs (logs, proxies).

use async_trait::async_trait;
use serde_json::json;

use super::{
    check_response, model_not_available, net_err, require_key, Cleaner, ProviderError,
    CLEANUP_CLIENT, CLEANUP_PROMPT,
};

const NAME: &str = "Gemini";
const MODEL: &str = "gemini-2.0-flash";
const MODELS_URL: &str = "https://generativelanguage.googleapis.com/v1beta/models";
const API_KEY_HEADER: &str = "x-goog-api-key";

pub struct Gemini {
    key: String,
    /// Pinned model ID; empty = use `MODEL`.
    model: String,
}

impl Gemini {
    pub fn new(key: &str, model: &str) -> Result<Self, ProviderError> {
        Ok(Self {
            key: require_key(NAME, key)?,
            model: model.trim().to_string(),
        })
    }

    /// The model actually used: the pinned one, else the default.
    fn effective_model(&self) -> &str {
        if self.model.is_empty() {
            MODEL
        } else {
            &self.model
        }
    }

    /// `…/v1beta/models/<model>:generateContent` for the effective model.
    fn generate_url(&self) -> String {
        format!("{MODELS_URL}/{}:generateContent", self.effective_model())
    }
}

#[async_trait]
impl Cleaner for Gemini {
    async fn clean(&self, raw: &str) -> Result<String, ProviderError> {
        let body = json!({
            "systemInstruction": { "parts": [{ "text": CLEANUP_PROMPT }] },
            "contents": [{ "role": "user", "parts": [{ "text": raw }] }],
        });

        let resp = CLEANUP_CLIENT
            .post(self.generate_url())
            .header(API_KEY_HEADER, &self.key)
            .json(&body)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;
        let json: serde_json::Value = resp.json().await.map_err(net_err)?;
        Ok(json
            .get("candidates")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("content"))
            .and_then(|c| c.get("parts"))
            .and_then(|p| p.get(0))
            .and_then(|p| p.get("text"))
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string())
    }

    async fn test(&self) -> Result<String, ProviderError> {
        let resp = CLEANUP_CLIENT
            .get(MODELS_URL)
            .header(API_KEY_HEADER, &self.key)
            .send()
            .await
            .map_err(net_err)?;
        let resp = check_response(NAME, resp).await?;

        // Validate a pinned model. The list entries are prefixed `models/`
        // (e.g. `models/gemini-2.0-flash`) — strip it before comparing (accept
        // either form to be safe).
        if !self.model.is_empty() {
            let json: serde_json::Value = resp.json().await.map_err(net_err)?;
            let found = json
                .get("models")
                .and_then(|d| d.as_array())
                .is_some_and(|arr| {
                    arr.iter()
                        .filter_map(|m| m.get("name").and_then(|n| n.as_str()))
                        .any(|name| {
                            let id = name.strip_prefix("models/").unwrap_or(name);
                            id == self.model || name == self.model
                        })
                });
            if !found {
                return Err(model_not_available(NAME, &self.model));
            }
        }
        Ok(self.effective_model().to_string())
    }
}
