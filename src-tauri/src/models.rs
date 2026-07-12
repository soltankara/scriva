//! Local-model download manager: the shell side of the curated registry in
//! `scriva_core::registry`. Owns the `<app data>/models/` directory, the four
//! model IPC commands (`list_local_models`, `download_model`, `cancel_download`,
//! `delete_model`), and the `model-download-progress` event the settings UI
//! renders. Downloads stream to a `<file>.part` sidecar and are atomically
//! renamed only after a size sanity check, so a present final file is always a
//! complete one. Progress/errors are plain human text — never URLs or tokens.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime, State};
use tokio::io::AsyncWriteExt;

use scriva_core::registry::{self, Layer, ModelInfo};

/// Where downloaded on-device models live: `<app data dir>/models`. Single
/// source of truth — the pipeline and `test_provider` resolve it here too.
pub fn models_dir<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_data_dir()
        .map_err(|_| "Couldn't resolve the app data directory.".to_string())?
        .join("models"))
}

/// Shared per-download state: the cancel flag `cancel_download` sets and the
/// last computed percentage `list_local_models` reports for in-flight rows.
#[derive(Default)]
pub struct DownloadProgress {
    cancel: AtomicBool,
    pct: AtomicU8,
}

/// Managed state: in-flight downloads keyed by model id. Presence in the map
/// is the concurrency guard (one download per model); the entry is removed
/// when the task finishes, fails, or is cancelled.
#[derive(Default)]
pub struct Downloads(Mutex<HashMap<String, Arc<DownloadProgress>>>);

/// `model-download-progress` event payload. `done`/`error` are omitted from
/// the JSON when unset, matching the UI contract `{id, pct, done?, error?}`.
#[derive(Serialize, Clone)]
struct ProgressEvent<'a> {
    id: &'a str,
    pct: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    done: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'a str>,
}

/// Emit a progress event to the settings window (same target as
/// `pipeline-error` in lib.rs). Failures are ignored — the window may be
/// closed, and `list_local_models` re-syncs the UI whenever it reopens.
fn emit_progress<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    pct: u8,
    done: bool,
    error: Option<&str>,
) {
    let _ = app.emit_to(
        "main",
        "model-download-progress",
        ProgressEvent {
            id,
            pct,
            done: done.then_some(true),
            error,
        },
    );
}

/// One row of `list_local_models` — exactly the fields the settings UI reads.
#[derive(Serialize)]
pub struct LocalModel {
    id: &'static str,
    layer: &'static str,
    label: &'static str,
    size_mb: u32,
    sub: &'static str,
    state: &'static str,
    pct: u8,
}

/// Registry snapshot + on-disk/in-flight status for every curated model.
#[tauri::command]
pub fn list_local_models(
    app: AppHandle,
    downloads: State<'_, Downloads>,
) -> Result<Vec<LocalModel>, String> {
    let dir = models_dir(&app)?;
    let in_flight = downloads.0.lock().unwrap();
    Ok(registry::MODELS
        .iter()
        .map(|m| {
            let (state, pct) = if let Some(p) = in_flight.get(m.id) {
                ("downloading", p.pct.load(Ordering::Relaxed))
            } else if dir.join(m.file_name).exists() {
                ("downloaded", 100)
            } else {
                ("not", 0)
            };
            LocalModel {
                id: m.id,
                layer: match m.layer {
                    Layer::Transcription => "transcription",
                    Layer::Cleanup => "cleanup",
                },
                label: m.label,
                size_mb: m.size_mb,
                sub: m.sub,
                state,
                pct,
            }
        })
        .collect())
}

/// Start downloading a model in the background and return immediately.
/// Progress, completion, and failure all arrive as `model-download-progress`
/// events; the command itself only errors on bad preconditions.
#[tauri::command]
pub fn download_model(
    app: AppHandle,
    downloads: State<'_, Downloads>,
    id: String,
) -> Result<(), String> {
    let info = registry::model_by_id(&id).ok_or_else(|| "Unknown model.".to_string())?;
    let dir = models_dir(&app)?;

    let progress = {
        let mut in_flight = downloads.0.lock().unwrap();
        if in_flight.contains_key(&id) {
            return Err("Already downloading.".to_string());
        }
        if dir.join(info.file_name).exists() {
            return Err("Already downloaded.".to_string());
        }
        let progress = Arc::new(DownloadProgress::default());
        in_flight.insert(id.clone(), Arc::clone(&progress));
        progress
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        downloads.0.lock().unwrap().remove(&id);
        eprintln!("[scriva] couldn't create models dir: {e}");
        return Err("Couldn't create the models folder.".to_string());
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let result = fetch_model(&app, info, &dir, &progress).await;
        // Remove from the in-flight map BEFORE emitting, so a UI refresh
        // triggered by the event sees the final state, not "downloading".
        app.state::<Downloads>().0.lock().unwrap().remove(&id);
        match result {
            Ok(()) => {
                eprintln!("[scriva] model download complete: {id}");
                emit_progress(&app, &id, 100, true, None);
            }
            Err(msg) => {
                eprintln!("[scriva] model download ended: {id} — {msg}");
                emit_progress(
                    &app,
                    &id,
                    progress.pct.load(Ordering::Relaxed),
                    false,
                    Some(&msg),
                );
            }
        }
    });
    Ok(())
}

/// Stream `info.url` to `<models dir>/<file>.part`, then sanity-check the size
/// and rename to the final file name. Checks the cancel flag every chunk and
/// keeps the shared pct atomic current for `list_local_models`. All errors are
/// human-readable text (never the URL).
async fn fetch_model(
    app: &AppHandle,
    info: &'static ModelInfo,
    dir: &std::path::Path,
    progress: &DownloadProgress,
) -> Result<(), String> {
    let part = dir.join(format!("{}.part", info.file_name));

    let mut resp = reqwest::get(info.url)
        .await
        .map_err(|_| "Download failed — check your internet connection.".to_string())?
        .error_for_status()
        .map_err(|e| match e.status() {
            Some(status) => format!("Download failed (HTTP {}).", status.as_u16()),
            None => "Download failed.".to_string(),
        })?;

    // Basis for the percentage: the server's Content-Length when present,
    // otherwise the registry's approximate size.
    let expected = u64::from(info.size_mb) * 1024 * 1024;
    let total = resp.content_length().filter(|&n| n > 0).unwrap_or(expected);

    let mut file = tokio::fs::File::create(&part)
        .await
        .map_err(|_| "Couldn't create the download file.".to_string())?;

    let mut received: u64 = 0;
    let mut last_pct: u8 = 0;
    let mut last_emit = Instant::now();
    loop {
        if progress.cancel.load(Ordering::SeqCst) {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err("Download cancelled.".to_string());
        }
        let chunk = match resp.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break, // body complete
            Err(_) => {
                drop(file);
                let _ = tokio::fs::remove_file(&part).await;
                return Err(
                    "Download interrupted — check your connection and try again.".to_string(),
                );
            }
        };
        if file.write_all(&chunk).await.is_err() {
            drop(file);
            let _ = tokio::fs::remove_file(&part).await;
            return Err("Couldn't write the download to disk — is it full?".to_string());
        }
        received += chunk.len() as u64;

        // Cap at 99 while streaming: 100 is reserved for the done event, and
        // an estimated `total` (unknown Content-Length) may undershoot.
        let pct = ((received.saturating_mul(100) / total).min(99)) as u8;
        progress.pct.store(pct, Ordering::Relaxed);
        // Throttle events to whole-percent steps at most every ~200 ms — the
        // shared atomic above stays current for list_local_models regardless.
        if pct != last_pct && last_emit.elapsed() >= Duration::from_millis(200) {
            last_pct = pct;
            last_emit = Instant::now();
            emit_progress(app, info.id, pct, false, None);
        }
    }

    // Ensure everything is on disk before judging or renaming the file.
    if file.flush().await.is_err() || file.sync_all().await.is_err() {
        drop(file);
        let _ = tokio::fs::remove_file(&part).await;
        return Err("Couldn't write the download to disk — is it full?".to_string());
    }
    drop(file);

    // Sanity check: within ±20% of the registry's expected size. Catches
    // truncated bodies and HTML error pages saved as the model file.
    if received * 5 < expected * 4 || received * 5 > expected * 6 {
        let _ = tokio::fs::remove_file(&part).await;
        return Err("Downloaded file looks wrong (size mismatch) — try again.".to_string());
    }

    tokio::fs::rename(&part, dir.join(info.file_name))
        .await
        .map_err(|_| "Couldn't finish the download (rename failed) — try again.".to_string())
}

/// Ask an in-flight download to stop. Ok even if nothing is downloading (the
/// task may have just finished); the cancel outcome arrives as an event.
#[tauri::command]
pub fn cancel_download(downloads: State<'_, Downloads>, id: String) -> Result<(), String> {
    if let Some(progress) = downloads.0.lock().unwrap().get(&id) {
        progress.cancel.store(true, Ordering::SeqCst);
    }
    Ok(())
}

/// Delete a downloaded model file (and any stale `.part` sidecar). Refuses
/// while a download is in flight; Ok if nothing existed.
#[tauri::command]
pub fn delete_model(
    app: AppHandle,
    downloads: State<'_, Downloads>,
    id: String,
) -> Result<(), String> {
    let info = registry::model_by_id(&id).ok_or_else(|| "Unknown model.".to_string())?;
    if downloads.0.lock().unwrap().contains_key(&id) {
        return Err("Cancel the download first.".to_string());
    }
    let dir = models_dir(&app)?;
    let _ = std::fs::remove_file(dir.join(info.file_name));
    let _ = std::fs::remove_file(dir.join(format!("{}.part", info.file_name)));
    Ok(())
}

/// Best-effort startup sweep of stale `*.part` files (a download killed by
/// quit/crash). Called once from setup(); all errors ignored.
pub fn sweep_stale_parts<R: Runtime>(app: &AppHandle<R>) {
    let Ok(dir) = models_dir(app) else { return };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("part") {
            let _ = std::fs::remove_file(&path);
        }
    }
}
