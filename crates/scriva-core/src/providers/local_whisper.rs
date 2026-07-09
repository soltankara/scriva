//! On-device transcription via whisper.cpp (`whisper-rs`), compiled only with
//! the `local-models` feature. No network, no API key — the model file lives
//! under the shell's models dir and is resolved through `registry`.
//!
//! Runtime requirement: `transcribe()`/`test()` offload blocking work with
//! `tokio::task::spawn_blocking`, so they must be awaited from inside a tokio
//! runtime. The Tauri shell guarantees this (pipeline and IPC commands both
//! run on tauri's tokio runtime).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::{ProviderError, Transcriber};
use crate::audio;
use crate::registry::{self, Layer};

/// whisper.cpp ggml model files start with the u32 magic `0x67676d6c`
/// ("ggml"), written little-endian (on-disk bytes `6C 67 67 67`).
const GGML_MAGIC: u32 = 0x6767_6d6c;

/// Model cache keyed by path — the multi-second ggml load survives across
/// dictations. Exactly one model is held; picking a different model evicts
/// the old one. The `Arc` is cloned OUT of the lock before inference so a
/// long-running `full()` never blocks a concurrent `test()` or reload.
static CTX: Mutex<Option<(PathBuf, Arc<WhisperContext>)>> = Mutex::new(None);

/// Drop the cached context (frees ~0.5–2 GB RAM). Called by the shell when
/// the user switches the transcription layer away from `"local"`.
pub(crate) fn unload() {
    *CTX.lock().unwrap() = None;
}

/// On-device whisper.cpp transcriber for one curated registry model.
pub struct LocalWhisper {
    /// Full path to the ggml model file under the shell's models dir.
    path: PathBuf,
    /// Human-readable model name for error messages / test output.
    label: &'static str,
}

impl LocalWhisper {
    pub fn new(models_dir: &Path, model: &str) -> Result<Self, ProviderError> {
        let info = registry::resolve(Layer::Transcription, model).ok_or_else(|| {
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
        "{label} isn't downloaded yet — open Settings → Transcription → Local."
    ))
}

/// Return the cached context for `path`, or load (and cache) it. Loading
/// holds the lock — concurrent callers wait instead of double-loading.
fn cached_or_load(path: &Path, label: &str) -> Result<Arc<WhisperContext>, ProviderError> {
    let mut guard = CTX.lock().unwrap();
    if let Some((cached_path, ctx)) = guard.as_ref() {
        if cached_path == path {
            return Ok(Arc::clone(ctx));
        }
    }
    if !path.is_file() {
        return Err(not_downloaded(label));
    }
    let ctx = WhisperContext::new_with_params(path, WhisperContextParameters::default())
        .map_err(|_| {
            ProviderError::Config(format!("{label} failed to load — try re-downloading it."))
        })?;
    let ctx = Arc::new(ctx);
    *guard = Some((path.to_path_buf(), Arc::clone(&ctx)));
    Ok(ctx)
}

#[async_trait]
impl Transcriber for LocalWhisper {
    async fn transcribe(&self, wav: Vec<u8>) -> Result<String, ProviderError> {
        let path = self.path.clone();
        let label = self.label;
        tokio::task::spawn_blocking(move || {
            // Keep whisper.cpp/GGML C-side chatter off stderr: with neither
            // log backend feature enabled the installed hooks are no-ops, so
            // engine logs are simply dropped. Idempotent (first call wins).
            whisper_rs::install_logging_hooks();

            let samples = audio::wav_to_f32_16k_mono(&wav)
                .ok_or_else(|| ProviderError::Config("Audio decode failed.".to_string()))?;

            let ctx = cached_or_load(&path, label)?;
            let mut state = ctx.create_state().map_err(|_| {
                ProviderError::Config(format!(
                    "{label} failed to load — try re-downloading it."
                ))
            })?;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(Some("auto"));
            params.set_translate(false);
            params.set_print_progress(false);
            params.set_print_special(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            params.set_suppress_blank(true);
            params.set_no_context(true);
            let threads = std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .min(8);
            params.set_n_threads(threads as std::os::raw::c_int);

            state.full(params, &samples).map_err(|_| {
                ProviderError::Config(format!("{label} transcription failed — try again."))
            })?;

            let mut out = String::new();
            for segment in state.as_iter() {
                if let Ok(text) = segment.to_str_lossy() {
                    out.push_str(&text);
                }
            }
            // Empty is fine: the pipeline treats empty text as nothing-to-type.
            Ok(out.trim().to_string())
        })
        .await
        .map_err(|_| ProviderError::Config("Local transcription task failed.".to_string()))?
    }

    async fn test(&self) -> Result<String, ProviderError> {
        let path = self.path.clone();
        let label = self.label;
        tokio::task::spawn_blocking(move || {
            let mut file = std::fs::File::open(&path).map_err(|_| not_downloaded(label))?;
            let mut magic = [0u8; 4];
            std::io::Read::read_exact(&mut file, &mut magic).map_err(|_| bad_file(label))?;
            if u32::from_le_bytes(magic) != GGML_MAGIC {
                return Err(bad_file(label));
            }
            Ok(format!("{label} (on-device)"))
        })
        .await
        .map_err(|_| ProviderError::Config("Local model check failed.".to_string()))?
    }
}

/// Friendly error for a present-but-not-a-ggml-model file.
fn bad_file(label: &str) -> ProviderError {
    ProviderError::Config(format!(
        "{label} file looks corrupted — delete and re-download it."
    ))
}
