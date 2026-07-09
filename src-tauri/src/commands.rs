//! `#[tauri::command]` handlers — the IPC contract with the settings UI.
//! Shapes here must match what `src/index.html` sends and reads.

use serde::Serialize;
use tauri::{AppHandle, State};

use crate::config::{self, Settings};
use crate::{apply_hotkey, audio, inject, providers, AppState};

/// Read persisted settings (mirrored in AppState, loaded from the store at
/// startup). Falls back to defaults if nothing was stored.
#[tauri::command]
pub fn load_settings(state: State<'_, AppState>) -> Settings {
    state.settings.read().unwrap().clone()
}

/// Persist settings, update AppState, and re-register the hotkey if it changed
/// (same path as `set_hotkey`).
#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    s: Settings,
) -> Result<(), String> {
    let hotkey_changed = state.settings.read().unwrap().hotkey != s.hotkey;

    // Register the new hotkey BEFORE persisting: a combo that fails registration
    // must never be written to disk.
    if hotkey_changed {
        apply_hotkey(&app, &s.hotkey)?;
    }

    config::save(&app, &s)?;

    // Leaving the on-device transcriber? Drop its cached model — that's
    // ~0.5–2 GB of RAM the user gets back when moving to a cloud provider.
    if s.transcription_provider != "local" {
        providers::unload_local_transcriber();
    }

    *state.settings.write().unwrap() = s;
    Ok(())
}

/// Validate a provider's API key by building a throwaway adapter from the
/// *passed* key (not stored state) and running its cheap `test()` round-trip.
/// Ok = model name (UI shows "Connected · {model}"); Err = human-readable text.
#[tauri::command]
pub async fn test_provider(
    app: AppHandle,
    layer: String,
    provider: String,
    key: String,
    model: String,
) -> Result<String, String> {
    // Where downloaded on-device models live; only "local" providers read it.
    let models_dir = crate::models::models_dir(&app)?;

    match layer.as_str() {
        "transcription" => {
            if provider == "claude" {
                return Err(
                    "Claude has no speech-to-text API — it can only be used for cleanup."
                        .to_string(),
                );
            }
            let transcriber = providers::make_transcriber(&provider, &key, &model, &models_dir)
                .map_err(|e| e.to_string())?;
            transcriber.test().await.map_err(|e| e.to_string())
        }
        "cleanup" => {
            if provider == "none" {
                return Err(
                    "\"None\" has nothing to test — the raw transcript passes through unchanged."
                        .to_string(),
                );
            }
            match providers::make_cleaner(&provider, &key, &model, &models_dir)
                .map_err(|e| e.to_string())?
            {
                Some(cleaner) => cleaner.test().await.map_err(|e| e.to_string()),
                None => Err("\"None\" has nothing to test.".to_string()),
            }
        }
        other => Err(format!("Unknown layer \"{other}\".")),
    }
}

/// Re-register the global push-to-talk shortcut. Errors (with a human-readable
/// message) on an invalid combo or a registration conflict, leaving the old
/// shortcut in place.
#[tauri::command]
pub fn set_hotkey(
    app: AppHandle,
    state: State<'_, AppState>,
    combo: Vec<String>,
) -> Result<(), String> {
    apply_hotkey(&app, &combo)?;
    // Reflect the new (not-yet-persisted) hotkey in memory so a later
    // save_settings with the same combo doesn't needlessly re-register.
    state.settings.write().unwrap().hotkey = combo;
    Ok(())
}

/// Whether first-run onboarding has been completed (drives the UI's onboarding
/// layer on startup).
#[tauri::command]
pub fn get_onboarded(app: AppHandle) -> bool {
    config::load_onboarded(&app)
}

/// Mark onboarding as completed. Write-only and one-way: there is no IPC path
/// back to "not onboarded" (delete the store file to re-run onboarding).
#[tauri::command]
pub fn set_onboarded(app: AppHandle) -> Result<(), String> {
    config::save_onboarded(&app, true)
}

/// Serializes to `{ "mic": "granted"|"denied"|"undetermined", "accessibility":
/// bool }` — exact field names the UI reads (`p.mic`, `p.accessibility`).
#[derive(Serialize)]
pub struct Permissions {
    pub mic: String,
    pub accessibility: bool,
}

/// Report Microphone + Accessibility status. Both are checked live, without
/// prompting: mic via AVCaptureDevice authorization, accessibility via AX.
#[tauri::command]
pub fn check_permissions() -> Permissions {
    Permissions {
        mic: audio::mic_status().to_string(),
        accessibility: inject::accessibility_trusted(false),
    }
}

/// Trigger the one-time macOS microphone prompt (no-op once decided). The UI
/// polls `check_permissions` to pick up the resulting status.
#[tauri::command]
pub fn request_microphone() {
    audio::request_mic_access();
}

/// Trigger the macOS Accessibility prompt and deep-link to the exact System
/// Settings pane so the user can grant access.
#[tauri::command]
pub fn request_accessibility() {
    let _ = inject::accessibility_trusted(true);
    let _ = tauri_plugin_opener::open_url(
        "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility",
        None::<&str>,
    );
}
