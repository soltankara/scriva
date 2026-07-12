//! Curated local-model registry — pure static data, deliberately NOT gated
//! behind the `local-models` feature: the shell's download manager needs it
//! even in builds without the inference engines, and its tests always run.
//!
//! Every entry is a file the app can download to `<appdata>/models/` and feed
//! to whisper.cpp (transcription) or llama.cpp (cleanup). URLs point at
//! ungated Hugging Face repos (bartowski quants — the official meta-llama
//! repos are gated and must never be linked). Verified live 2026-07-09.

/// Which pipeline layer a model serves.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    Transcription,
    Cleanup,
}

/// One downloadable model: identity, download source, and UI copy.
pub struct ModelInfo {
    /// Stable id stored in `Settings.transcription_model` / `cleanup_model`.
    pub id: &'static str,
    pub layer: Layer,
    /// Human-readable name for the settings UI.
    pub label: &'static str,
    /// On-disk file name under the models dir (never a path).
    pub file_name: &'static str,
    /// Direct download URL (`https://huggingface.co/<repo>/resolve/main/<file>`).
    pub url: &'static str,
    /// Approximate download size, for the UI and post-download sanity check.
    pub size_mb: u32,
    /// One-line tradeoff hint shown under the label.
    pub sub: &'static str,
}

/// The full curated list. Order is the UI display order (small → large).
pub const MODELS: &[ModelInfo] = &[
    ModelInfo {
        id: "whisper-tiny",
        layer: Layer::Transcription,
        label: "Whisper Tiny",
        file_name: "ggml-tiny.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
        size_mb: 74,
        sub: "Fastest · lowest accuracy",
    },
    ModelInfo {
        id: "whisper-base",
        layer: Layer::Transcription,
        label: "Whisper Base",
        file_name: "ggml-base.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
        size_mb: 141,
        sub: "Fast · decent accuracy",
    },
    ModelInfo {
        id: "whisper-small",
        layer: Layer::Transcription,
        label: "Whisper Small",
        file_name: "ggml-small.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
        size_mb: 465,
        sub: "Best balance · recommended",
    },
    ModelInfo {
        id: "whisper-large-v3-turbo",
        layer: Layer::Transcription,
        label: "Whisper Large v3 Turbo",
        file_name: "ggml-large-v3-turbo-q5_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        size_mb: 547,
        sub: "Best accuracy · slower",
    },
    ModelInfo {
        id: "llama-3.2-1b",
        layer: Layer::Cleanup,
        label: "Llama 3.2 1B",
        file_name: "Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/Llama-3.2-1B-Instruct-GGUF/resolve/main/Llama-3.2-1B-Instruct-Q4_K_M.gguf",
        size_mb: 770,
        sub: "Fastest · may miss spoken corrections",
    },
    ModelInfo {
        id: "llama-3.2-3b",
        layer: Layer::Cleanup,
        label: "Llama 3.2 3B",
        file_name: "Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/main/Llama-3.2-3B-Instruct-Q4_K_M.gguf",
        size_mb: 1926,
        sub: "Best balance · recommended",
    },
    ModelInfo {
        id: "qwen3-4b-2507",
        layer: Layer::Cleanup,
        label: "Qwen3 4B Instruct",
        file_name: "Qwen_Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/bartowski/Qwen_Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen_Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        size_mb: 2382,
        sub: "Best quality · larger",
    },
];

/// Look up a model by its stable id.
pub fn model_by_id(id: &str) -> Option<&'static ModelInfo> {
    MODELS.iter().find(|m| m.id == id)
}

/// The model an empty selection (`""`) means for a layer: the "recommended"
/// entry — whisper-small for transcription, llama-3.2-3b for cleanup.
pub fn default_model(layer: Layer) -> &'static ModelInfo {
    let id = match layer {
        Layer::Transcription => "whisper-small",
        Layer::Cleanup => "llama-3.2-3b",
    };
    model_by_id(id).expect("default model must exist in MODELS")
}

/// Resolve a stored model id for a layer. `""` (or whitespace) means the
/// layer's default; anything else must name a registered model of that layer,
/// otherwise `None` (unknown id, or a cleanup model selected for transcription).
pub fn resolve(layer: Layer, id: &str) -> Option<&'static ModelInfo> {
    if id.trim().is_empty() {
        return Some(default_model(layer));
    }
    model_by_id(id).filter(|m| m.layer == layer)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique() {
        for (i, a) in MODELS.iter().enumerate() {
            for b in &MODELS[i + 1..] {
                assert_ne!(a.id, b.id, "duplicate model id {}", a.id);
            }
        }
    }

    #[test]
    fn urls_are_huggingface_resolve_links() {
        for m in MODELS {
            assert!(
                m.url.starts_with("https://huggingface.co/"),
                "{}: url must be a Hugging Face link",
                m.id
            );
            assert!(
                m.url.contains("/resolve/main/"),
                "{}: url must be a /resolve/main/ direct-download link",
                m.id
            );
        }
    }

    #[test]
    fn file_names_are_plain_names() {
        // The shell joins file_name onto the models dir; a separator or `..`
        // would allow escaping it.
        for m in MODELS {
            assert!(
                !m.file_name.contains('/')
                    && !m.file_name.contains('\\')
                    && !m.file_name.contains(".."),
                "{}: file_name must not contain path separators or ..",
                m.id
            );
        }
    }

    #[test]
    fn both_layers_are_populated() {
        assert!(MODELS.iter().any(|m| m.layer == Layer::Transcription));
        assert!(MODELS.iter().any(|m| m.layer == Layer::Cleanup));
    }

    #[test]
    fn defaults_resolve() {
        assert_eq!(default_model(Layer::Transcription).id, "whisper-small");
        assert_eq!(default_model(Layer::Cleanup).id, "llama-3.2-3b");
        // `""` and whitespace both mean the default.
        assert_eq!(
            resolve(Layer::Transcription, "").unwrap().id,
            "whisper-small"
        );
        assert_eq!(resolve(Layer::Cleanup, "  ").unwrap().id, "llama-3.2-3b");
    }

    #[test]
    fn resolve_rejects_cross_layer_ids() {
        // A cleanup model id passed for the transcription layer must not resolve.
        assert!(resolve(Layer::Transcription, "llama-3.2-3b").is_none());
        // Sanity: it resolves fine on its own layer, and unknown ids never do.
        assert!(resolve(Layer::Cleanup, "llama-3.2-3b").is_some());
        assert!(resolve(Layer::Cleanup, "no-such-model").is_none());
    }
}
