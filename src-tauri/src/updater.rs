//! Manual-only in-app updater.
//!
//! Scriva's positioning is "nothing phones home", so this is the ONLY place
//! the app touches the network without a user action driving it, and even here
//! the single trigger is the tray's "Check for Updates…" item — there is no
//! automatic or startup check of any kind.
//!
//! Flow (all in Rust — the webview never touches the updater or dialog plugins):
//!   click → `app.updater().check().await`
//!     · newer version → native ask dialog → on confirm download + install +
//!       relaunch
//!     · already current → info dialog
//!     · any error (offline, no `latest.json` published, unsigned dev build) →
//!       calm info dialog, never a crash
//!
//! Never logs keys, audio, or transcripts (there are none here); logs only the
//! check outcome to stderr with the `[scriva]` prefix.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::AppHandle;
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons};
use tauri_plugin_updater::UpdaterExt;

/// Guards against concurrent checks: a second menu click while a check (or an
/// in-progress download) is still in flight is a silent no-op.
static CHECK_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

/// The current app version, for user-facing copy.
fn current_version(app: &AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Entry point for the tray's "Check for Updates…" item. Returns immediately;
/// the whole check runs off the menu-event thread on the async runtime.
pub fn check_for_updates(app: &AppHandle) {
    // Reject overlapping checks. If we don't win the flag, another check is
    // already running — do nothing.
    if CHECK_IN_FLIGHT
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        eprintln!("[scriva] update check already in flight — ignoring");
        return;
    }

    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        run_check(&app).await;
        CHECK_IN_FLIGHT.store(false, Ordering::SeqCst);
    });
}

/// The check itself. All exits (update, up-to-date, error) surface a native
/// dialog; nothing here can panic the caller.
async fn run_check(app: &AppHandle) {
    let current = current_version(app);

    // Build the updater. In an unsigned dev build (or if the plugin is
    // misconfigured) this can already fail — treat it like any other check
    // error and show the calm dialog.
    let updater = match app.updater() {
        Ok(u) => u,
        Err(e) => {
            eprintln!("[scriva] updater unavailable: {e}");
            couldnt_check(app, &e.to_string());
            return;
        }
    };

    match updater.check().await {
        // A newer version is available.
        Ok(Some(update)) => {
            let new_version = update.version.clone();
            eprintln!("[scriva] update available: {current} -> {new_version}");

            let install = app
                .dialog()
                .message(format!(
                    "Scriva {new_version} is available — you have {current}.\n\n\
                     Install and relaunch now?"
                ))
                .title("Update Available")
                .buttons(MessageDialogButtons::OkCancelCustom(
                    "Install and Relaunch".to_string(),
                    "Later".to_string(),
                ))
                .blocking_show();

            if !install {
                eprintln!("[scriva] update deferred by user");
                return;
            }

            eprintln!("[scriva] downloading update {new_version}");
            match update.download_and_install(|_, _| {}, || {}).await {
                Ok(()) => {
                    eprintln!("[scriva] update installed — relaunching");
                    app.restart();
                }
                Err(e) => {
                    eprintln!("[scriva] update install failed: {e}");
                    app.dialog()
                        .message(format!(
                            "Couldn't install the update: {e}\n\n\
                             You can download the latest version from the releases page."
                        ))
                        .title("Update Failed")
                        .blocking_show();
                }
            }
        }

        // Already on the latest version.
        Ok(None) => {
            eprintln!("[scriva] up to date ({current})");
            app.dialog()
                .message(format!(
                    "You're up to date — Scriva {current} is the latest version."
                ))
                .title("Up to Date")
                .blocking_show();
        }

        // Couldn't check (offline, no latest.json yet, unsigned dev build, …).
        Err(e) => {
            eprintln!("[scriva] update check failed: {e}");
            couldnt_check(app, &e.to_string());
        }
    }
}

/// Calm, human-readable failure dialog shared by every error path.
fn couldnt_check(app: &AppHandle, detail: &str) {
    app.dialog()
        .message(format!("Couldn't check for updates: {detail}"))
        .title("Check for Updates")
        .blocking_show();
}
