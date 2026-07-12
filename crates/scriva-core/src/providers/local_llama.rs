//! On-device cleanup via llama.cpp (`llama-cpp-2`), compiled only with the
//! `local-models` feature. No network, no API key — the GGUF model file lives
//! under the shell's models dir and is resolved through `registry`.
//!
//! Runtime requirement: `clean()`/`test()` offload blocking work with
//! `tokio::task::spawn_blocking`, so they must be awaited from inside a tokio
//! runtime. The Tauri shell guarantees this (pipeline and IPC commands both
//! run on tauri's tokio runtime).

use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use async_trait::async_trait;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::TokenToStringError;

use super::{strip_chatter, Cleaner, ProviderError, CLEANUP_PROMPT};
use crate::registry::{self, Layer};

/// llama.cpp's whole-process backend, initialized at most once (`init()` on a
/// second call returns `BackendAlreadyInitialized`) and never dropped — the
/// `Drop` impl would free llama.cpp global state under any live model.
/// `LlamaBackend` is a fieldless token type, so the `OnceLock` is `Sync`.
/// `None` records an init failure so we never retry-panic in the audio path.
static BACKEND: OnceLock<Option<LlamaBackend>> = OnceLock::new();

/// Model cache keyed by path — the multi-second GGUF load survives across
/// dictations. Exactly one model is held; picking a different model evicts
/// the old one. The `Arc` is cloned OUT of the lock before inference so a
/// long-running generation never blocks a concurrent `test()` or reload.
static MODEL: Mutex<Option<(PathBuf, Arc<LlamaModel>)>> = Mutex::new(None);

/// Drop the cached model (frees ~1–3 GB RAM). Called by the shell when the
/// user switches the cleanup layer away from `"local"`.
pub(crate) fn unload() {
    *MODEL.lock().unwrap() = None;
}

/// Load the selected cleanup model into the cache ahead of time (fire-and-
/// forget warm-up from the shell). All errors are swallowed — a missing file
/// simply errors politely at use time.
pub(crate) fn preload(models_dir: &Path, model: &str) {
    let Some(info) = registry::resolve(Layer::Cleanup, model) else {
        return;
    };
    let _ = cached_or_load(&models_dir.join(info.file_name), info.label);
}

/// Get the process-wide llama backend, initializing (and silencing llama.cpp's
/// stderr chatter via `void_logs`) on first use.
fn backend() -> Result<&'static LlamaBackend, ProviderError> {
    BACKEND
        .get_or_init(|| match LlamaBackend::init() {
            Ok(mut b) => {
                // Route llama.cpp/GGML C-side logging to a void callback —
                // keeps model metadata (and anything else) off stderr.
                b.void_logs();
                Some(b)
            }
            Err(_) => None,
        })
        .as_ref()
        .ok_or_else(|| ProviderError::Config("Local cleanup engine failed to start.".to_string()))
}

/// On-device llama.cpp cleaner for one curated registry model.
pub struct LocalLlama {
    /// Full path to the GGUF model file under the shell's models dir.
    path: PathBuf,
    /// Human-readable model name for error messages / test output.
    label: &'static str,
}

impl LocalLlama {
    pub fn new(models_dir: &Path, model: &str) -> Result<Self, ProviderError> {
        let info = registry::resolve(Layer::Cleanup, model).ok_or_else(|| {
            ProviderError::Config(format!(
                "Unknown local model \"{model}\" — pick one in Settings."
            ))
        })?;
        Ok(Self {
            path: models_dir.join(info.file_name),
            label: info.label,
        })
    }
}

/// Friendly error for a selected-but-absent model file.
fn not_downloaded(label: &str) -> ProviderError {
    ProviderError::Config(format!(
        "{label} isn't downloaded yet — open Settings → Cleanup → Local."
    ))
}

/// Friendly error for a present-but-not-a-GGUF-model file.
fn bad_file(label: &str) -> ProviderError {
    ProviderError::Config(format!(
        "{label} file looks corrupted — delete and re-download it."
    ))
}

/// Generic mid-inference failure. The pipeline soft-falls-back to the raw
/// transcript on any cleaner error, so this is never fatal to the dictation.
fn cleanup_failed(label: &str) -> ProviderError {
    ProviderError::Config(format!("{label} cleanup failed — try again."))
}

/// Return the cached model for `path`, or load (and cache) it. Loading holds
/// the lock — concurrent callers wait instead of double-loading. Metal offload
/// is automatic: `LlamaModelParams::default()` leaves `n_gpu_layers = -1`,
/// which llama.cpp reads as "all layers plus the output layer on GPU".
fn cached_or_load(path: &Path, label: &str) -> Result<Arc<LlamaModel>, ProviderError> {
    let backend = backend()?;
    let mut guard = MODEL.lock().unwrap();
    if let Some((cached_path, model)) = guard.as_ref() {
        if cached_path == path {
            return Ok(Arc::clone(model));
        }
    }
    if !path.is_file() {
        return Err(not_downloaded(label));
    }
    let model =
        LlamaModel::load_from_file(backend, path, &LlamaModelParams::default()).map_err(|_| {
            ProviderError::Config(format!("{label} failed to load — try re-downloading it."))
        })?;
    let model = Arc::new(model);
    *guard = Some((path.to_path_buf(), Arc::clone(&model)));
    Ok(model)
}

/// Decode one token to its raw bytes (NOT a lossy per-token string: a single
/// UTF-8 character can span multiple tokens, so bytes are accumulated across
/// the whole generation and converted to text once at the end). Special
/// tokens render as nothing. A first pass with a small buffer is retried at
/// the exact size llama.cpp reports when it doesn't fit.
fn piece_bytes(model: &LlamaModel, token: LlamaToken) -> Vec<u8> {
    match model.token_to_piece_bytes(token, 32, false, None) {
        Ok(bytes) => bytes,
        Err(TokenToStringError::InsufficientBufferSpace(needed)) => model
            .token_to_piece_bytes(token, usize::try_from(-needed).unwrap_or(512), false, None)
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// The whole blocking inference pass: template → tokenize → prompt decode →
/// greedy generation → strip. Runs inside `spawn_blocking`.
fn run_cleanup(path: &Path, label: &'static str, raw: &str) -> Result<String, ProviderError> {
    let model = cached_or_load(path, label)?;

    // Prompt from the GGUF-embedded chat template (all curated models have
    // one). System = the shared injection-hardened CLEANUP_PROMPT; user = the
    // transcript, strictly data. `true` appends the assistant prefix so the
    // model completes the answer instead of continuing the conversation.
    let template = model.chat_template(None).map_err(|_| {
        ProviderError::Config(format!(
            "{label} has no chat template — try re-downloading it."
        ))
    })?;
    let messages = [("system", CLEANUP_PROMPT), ("user", raw)]
        .into_iter()
        .map(|(role, text)| LlamaChatMessage::new(role.to_string(), text.to_string()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| cleanup_failed(label))?;
    let prompt = model
        .apply_chat_template(&template, &messages, true)
        .map_err(|_| cleanup_failed(label))?;

    // `AddBos::Always` defers to the model's own `add_bos_token` metadata
    // (Llama 3 adds one, Qwen doesn't); the templates themselves never emit a
    // BOS, so this cannot double it. Special tokens in the template parse as
    // special.
    let tokens = model
        .str_to_token(&prompt, AddBos::Always)
        .map_err(|_| cleanup_failed(label))?;
    if tokens.is_empty() {
        return Err(cleanup_failed(label));
    }

    // Cleanup output is roughly input-sized; cap generation at about half the
    // raw char count in tokens (~2 chars/token), within [64, 1024]. Size the
    // context to exactly what prompt + output need (plus slack), min 1024,
    // and let one batch hold the whole prompt.
    let max_out = (raw.chars().count() / 2).clamp(64, 1024) as i32;
    let n_ctx = ((tokens.len() as i32 + max_out + 64).max(1024)) as u32;
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8) as i32;
    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(NonZeroU32::new(n_ctx))
        .with_n_batch(n_ctx)
        .with_n_threads(threads)
        .with_n_threads_batch(threads);
    let mut ctx = model.new_context(backend()?, ctx_params).map_err(|_| {
        ProviderError::Config(format!(
            "{label} couldn't start — it may need more memory than is available."
        ))
    })?;

    // Feed the whole prompt in one batch; logits only for the last position.
    let mut batch = LlamaBatch::new(tokens.len(), 1);
    let last = tokens.len() - 1;
    for (i, token) in tokens.iter().enumerate() {
        batch
            .add(*token, i as i32, &[0], i == last)
            .map_err(|_| cleanup_failed(label))?;
    }
    ctx.decode(&mut batch).map_err(|_| cleanup_failed(label))?;

    // Greedy decoding — deterministic output is exactly what cleanup wants.
    let mut sampler = LlamaSampler::greedy();
    let mut out_bytes: Vec<u8> = Vec::new();
    let mut pos = tokens.len() as i32;
    for _ in 0..max_out {
        let token = sampler.sample(&ctx, batch.n_tokens() - 1);
        if model.is_eog_token(token) {
            break;
        }
        out_bytes.extend_from_slice(&piece_bytes(&model, token));
        batch.clear();
        batch
            .add(token, pos, &[0], true)
            .map_err(|_| cleanup_failed(label))?;
        pos += 1;
        ctx.decode(&mut batch).map_err(|_| cleanup_failed(label))?;
    }

    let text = String::from_utf8_lossy(&out_bytes);
    let cleaned = strip_chatter(&text);
    if cleaned.is_empty() {
        // The pipeline falls back to the raw transcript on any cleaner error,
        // so "nothing came out" must be an Err, not an empty Ok.
        return Err(ProviderError::Config(
            "Local cleanup returned nothing.".to_string(),
        ));
    }
    Ok(cleaned)
}

#[async_trait]
impl Cleaner for LocalLlama {
    async fn clean(&self, raw: &str) -> Result<String, ProviderError> {
        let path = self.path.clone();
        let label = self.label;
        let raw = raw.to_string();
        tokio::task::spawn_blocking(move || run_cleanup(&path, label, &raw))
            .await
            .map_err(|_| ProviderError::Config("Local cleanup task failed.".to_string()))?
    }

    async fn test(&self) -> Result<String, ProviderError> {
        let path = self.path.clone();
        let label = self.label;
        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::File::open(&path).map_err(|_| not_downloaded(label))?;
            let mut magic = [0u8; 4];
            std::io::Read::read_exact(&mut file, &mut magic).map_err(|_| bad_file(label))?;
            if &magic != b"GGUF" {
                return Err(bad_file(label));
            }
            Ok(format!("{label} (on-device)"))
        })
        .await
        .map_err(|_| ProviderError::Config("Local model check failed.".to_string()))?
    }
}
