mod audio;
mod commands;
mod config;
mod inject;
mod menu_width;
mod overlay;
pub(crate) use scriva_core::providers;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};

use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, Runtime, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use audio::RecorderHandle;
use config::Settings;

/// Managed application state.
pub struct AppState {
    /// Current settings (source of truth in memory; mirrors the store).
    pub settings: RwLock<Settings>,
    /// `Some` while a dictation capture is in flight.
    pub recorder: Mutex<Option<RecorderHandle>>,
    /// True while the transcribe→clean→inject pipeline is running; used to drop
    /// overlapping triggers.
    pub pipeline_busy: AtomicBool,
    /// The currently-registered global shortcut, tracked so `set_hotkey` can
    /// unregister the previous one before registering the new. (Beyond the
    /// minimal AppState spec, but needed to re-register cleanly.)
    pub hotkey: Mutex<Option<Shortcut>>,
}

impl AppState {
    fn new(settings: Settings) -> Self {
        Self {
            settings: RwLock::new(settings),
            recorder: Mutex::new(None),
            pipeline_busy: AtomicBool::new(false),
            hotkey: Mutex::new(None),
        }
    }
}

/// Register `combo` as the global push-to-talk shortcut, unregistering whatever
/// was registered before. On a registration conflict, the previous shortcut is
/// restored and a human-readable error returned.
pub(crate) fn apply_hotkey<R: Runtime>(app: &AppHandle<R>, combo: &[String]) -> Result<(), String> {
    let accel = config::combo_to_accelerator(combo)?;
    let new_shortcut: Shortcut = accel
        .parse()
        .map_err(|_| format!("\"{accel}\" isn't a valid shortcut."))?;

    let gs = app.global_shortcut();
    let state = app.state::<AppState>();
    let mut guard = state.hotkey.lock().unwrap();
    let previous = *guard;

    // Unregister the old shortcut first, then register the new one.
    if let Some(old) = previous {
        let _ = gs.unregister(old);
    }
    match gs.register(new_shortcut) {
        Ok(()) => {
            *guard = Some(new_shortcut);
            Ok(())
        }
        Err(_) => {
            // Restore the previous shortcut so we're never left with none.
            if let Some(old) = previous {
                let _ = gs.register(old);
            }
            Err(format!(
                "Couldn't register {accel} — it may already be in use by macOS or another app."
            ))
        }
    }
}

/// Swap the menu-bar tray icon to reflect recording state: the bordered "rec"
/// glyph while capturing, the idle glyph otherwise. Both are monochrome template
/// images, and the template flag can reset on an icon change, so re-assert it.
/// Any failure (missing tray, undecodable image) is ignored — the tray simply
/// keeps whatever icon it currently shows.
fn set_tray_recording<R: Runtime>(app: &AppHandle<R>, recording: bool) {
    let bytes: &[u8] = if recording {
        include_bytes!("../icons/tray-rec.png")
    } else {
        include_bytes!("../icons/tray.png")
    };
    let Some(tray) = app.tray_by_id("main") else {
        eprintln!("[scriva] tray 'main' not found — icon not swapped");
        return;
    };
    let image = match tauri::image::Image::from_bytes(bytes) {
        Ok(img) => img,
        Err(_) => {
            eprintln!("[scriva] tray glyph failed to decode");
            return;
        }
    };
    match tray.set_icon(Some(image)) {
        Ok(()) => {
            let _ = tray.set_icon_as_template(true);
        }
        Err(e) => eprintln!("[scriva] tray set_icon failed: {e}"),
    }
}

/// Push-to-talk handler. Pressed starts capture; Released stops it and runs the
/// transcribe → (optional clean) → inject pipeline off the event thread.
fn on_shortcut_event(app: &AppHandle, event: ShortcutState) {
    let state = app.state::<AppState>();
    match event {
        ShortcutState::Pressed => {
            // Debounce key-repeat / re-entry: ignore if busy or already recording.
            if state.pipeline_busy.load(Ordering::SeqCst) {
                return;
            }
            if state.recorder.lock().map(|g| g.is_some()).unwrap_or(true) {
                return;
            }
            match audio::start_recording() {
                Ok(handle) => {
                    *state.recorder.lock().unwrap() = Some(handle);
                    let _ = app.emit_to("main", "recording-state", true);
                    eprintln!("[scriva] recording started");
                    set_tray_recording(app, true);
                    overlay::show(app);
                }
                Err(_) => {
                    let msg = "Couldn't access the microphone. Check that a mic is connected \
                               and permission is granted.";
                    let _ = app.emit_to("main", "pipeline-error", msg);
                    eprintln!("[scriva] {msg}");
                }
            }
        }
        ShortcutState::Released => {
            let handle = state.recorder.lock().unwrap().take();
            let Some(handle) = handle else {
                return; // no active recording (e.g. mic failed to open)
            };
            let _ = app.emit_to("main", "recording-state", false);
            eprintln!("[scriva] recording stopped");
            set_tray_recording(app, false);
            overlay::hide(app);
            state.pipeline_busy.store(true, Ordering::SeqCst);

            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let outcome = run_pipeline(&app, handle).await;
                if let Err(msg) = outcome {
                    eprintln!("[scriva] {msg}");
                    let _ = app.emit_to("main", "pipeline-error", msg);
                }
                app.state::<AppState>()
                    .pipeline_busy
                    .store(false, Ordering::SeqCst);
            });
        }
    }
}

/// Capture → encode → transcribe → optional cleanup → inject. Returns `Err`
/// with a human-readable message for hard failures (emitted as `pipeline-error`
/// by the caller). Soft issues (cleanup failure) emit their own soft warning
/// and continue with the raw transcript. Never logs keys, audio, or transcripts.
async fn run_pipeline(app: &AppHandle, handle: RecorderHandle) -> Result<(), String> {
    // 1. Stop the recorder and collect audio (blocking recv off-runtime).
    let audio = tauri::async_runtime::spawn_blocking(move || handle.stop_and_collect())
        .await
        .map_err(|_| "Recording failed unexpectedly.".to_string())??;
    let frames = if audio.channels > 0 {
        audio.samples.len() / audio.channels as usize
    } else {
        0
    };
    let duration = if audio.sample_rate > 0 {
        frames as f32 / audio.sample_rate as f32
    } else {
        0.0
    };
    eprintln!(
        "[scriva] captured {} samples ({:.1}s at {} Hz)",
        audio.samples.len(),
        duration,
        audio.sample_rate
    );

    // 2. Encode to 16 kHz mono WAV; skip silently if empty/silent.
    let wav = match audio::to_wav_16k_mono(&audio) {
        Some(w) => w,
        None => {
            eprintln!(
                "[scriva] audio empty or silent — nothing sent to transcriber \
                 (stale mic permission after a rebuild delivers silence; re-grant \
                 Microphone for the app)"
            );
            return Ok(());
        }
    };

    // 3. Snapshot provider settings + resolved keys (drop the guard before await).
    let (trans_provider, trans_key, trans_model, clean_provider, clean_key, clean_model) = {
        let s = app.state::<AppState>();
        let s = s.settings.read().unwrap();
        (
            s.transcription_provider.clone(),
            config::effective_key(&s, &s.transcription_provider),
            s.transcription_model.clone(),
            s.cleanup_provider.clone(),
            config::effective_key(&s, &s.cleanup_provider),
            s.cleanup_model.clone(),
        )
    };

    // 4. Transcribe (required).
    let transcriber = providers::make_transcriber(&trans_provider, &trans_key, &trans_model)
        .map_err(|e| e.to_string())?;
    let raw = transcriber
        .transcribe(wav)
        .await
        .map_err(|e| e.to_string())?;
    let raw = raw.trim().to_string();
    if raw.is_empty() {
        eprintln!("[scriva] transcriber returned empty text — nothing to type");
        return Ok(()); // nothing recognized — don't type anything
    }
    eprintln!("[scriva] transcribed {} chars", raw.chars().count());

    // 5. Optional cleanup. Never lose the user's words: on any cleanup failure,
    //    fall back to the raw transcript and warn softly.
    let mut text = raw.clone();
    if clean_provider != "none" {
        match providers::make_cleaner(&clean_provider, &clean_key, &clean_model) {
            Ok(Some(cleaner)) => match cleaner.clean(&raw).await {
                Ok(cleaned) => {
                    let cleaned = cleaned.trim().to_string();
                    if !cleaned.is_empty() {
                        text = cleaned;
                    }
                    eprintln!("[scriva] cleanup ok — {} chars", text.chars().count());
                }
                Err(_) => {
                    let msg = "Cleanup failed — typed the raw transcript instead.";
                    let _ = app.emit_to("main", "pipeline-error", msg);
                    eprintln!("[scriva] {msg}");
                }
            },
            Ok(None) => {}
            Err(_) => {
                let msg = "Cleanup provider isn't configured — typed the raw transcript instead.";
                let _ = app.emit_to("main", "pipeline-error", msg);
                eprintln!("[scriva] {msg}");
            }
        }
    }

    // 6. Accessibility gate — without it CGEvent injection silently no-ops
    //    (transcription works but no text appears). Warn on stderr for
    //    terminal-only users, then still attempt injection.
    if !inject::accessibility_trusted(false) {
        eprintln!("[scriva] accessibility not granted — text will not be typed");
    }

    // 7. Inject (blocking CGEvent posting off-runtime).
    let n = text.chars().count();
    match tauri::async_runtime::spawn_blocking(move || inject::type_text(&text)).await {
        Ok(()) => eprintln!("[scriva] injected {n} chars"),
        Err(_) => eprintln!("[scriva] injection failed unexpectedly"),
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Dev-only: load `.env` so SCRIVA_* key overrides are available.
    #[cfg(debug_assertions)]
    {
        let _ = dotenvy::dotenv();
    }

    let mut app = tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| on_shortcut_event(app, event.state))
                .build(),
        )
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            commands::load_settings,
            commands::save_settings,
            commands::test_provider,
            commands::set_hotkey,
            commands::check_permissions,
            commands::request_accessibility,
            commands::request_microphone,
        ])
        .setup(|app| {
            // Load settings and manage state before registering the hotkey.
            let settings = config::load(app.handle());
            let stored_combo = settings.hotkey.clone();
            app.manage(AppState::new(settings));

            // Register the push-to-talk shortcut; fall back to ⌥Space if the
            // stored combo is invalid or already taken.
            let handle = app.handle().clone();
            if let Err(e) = apply_hotkey(&handle, &stored_combo) {
                eprintln!("[scriva] stored hotkey unavailable ({e}); falling back to Alt+Space");
                let fallback = vec!["⌥".to_string(), "Space".to_string()];
                if let Err(e2) = apply_hotkey(&handle, &fallback) {
                    eprintln!("[scriva] fallback hotkey registration failed: {e2}");
                } else {
                    eprintln!("[scriva] hotkey registered: Alt+Space");
                }
            } else {
                eprintln!("[scriva] hotkey registered from settings");
            }

            let settings_item = MenuItemBuilder::with_id("settings", "Settings").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
            let menu = MenuBuilder::new(app).item(&settings_item).item(&quit).build()?;

            // Widen the native tray NSMenu panel so labels don't crowd its
            // rounded edge on macOS 26. Title padding (ASCII or NBSP) is trimmed
            // by macOS menu sizing and has no effect; the working fix is
            // `NSMenu setMinimumWidth:`, but Tauri/muda expose no NSMenu handle.
            // menu_width::install grabs it at runtime via an NSMenu-tracking
            // notification observer. macOS-only; no-op elsewhere.
            menu_width::install();

            // Dedicated monochrome glyph: template icons render from the alpha
            // channel only, so the colorful app icon would show as a solid blob.
            let tray_icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray.png"))
                .expect("bundled tray icon");
            TrayIconBuilder::with_id("main")
                .icon(tray_icon)
                .icon_as_template(true)
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "settings" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            eprintln!("[scriva] tray created (setup)");

            // Build the recording-indicator overlay once, hidden. Press/release
            // only toggle its visibility — never create/destroy on the hot path.
            overlay::create(app.handle());
            eprintln!("[scriva] overlay window created (setup)");

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the settings window hides it; Quit lives in the tray menu.
            // Scope to the "main" window only — the overlay must never be hidden
            // or intercepted by this handler.
            if window.label() == "main" {
                if let WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building Scriva");

    // Background agent: launch as Accessory (no dock icon). Must be set before
    // the event loop runs — a runtime Regular→Accessory flip removes the tray's
    // NSStatusItem on macOS 26. Bundled builds get this via LSUIElement.
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    app.run(|_app_handle, _event| {});
}
