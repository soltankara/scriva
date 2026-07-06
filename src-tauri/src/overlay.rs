//! Recording-indicator overlay window.
//!
//! A small always-on-top pill shown at the bottom-center of the active display
//! while the push-to-talk hotkey is held. It loads `overlay.html` (pure CSS,
//! no Tauri API usage) and is built once during setup, hidden; hotkey press/
//! release only toggle `show()`/`hide()` — never create/destroy — so it adds
//! no work to the capture hot path.
//!
//! Focus discipline is load-bearing: text injection targets the OS-frontmost
//! focused app, so the overlay must never activate or take key focus. It is
//! built `focused(false)`, we never call `set_focus()`, and it is made
//! click-through with `set_ignore_cursor_events(true)`.

use tauri::{
    AppHandle, LogicalSize, Manager, PhysicalPosition, Runtime, WebviewUrl, WebviewWindowBuilder,
};

/// Window label for the overlay. Kept distinct from `"main"` so the settings
/// window's prevent-close handler never targets it.
pub const LABEL: &str = "overlay";

/// Logical size of the overlay window. The pill inside is ~34px tall; the extra
/// height leaves room for the pulse glow to bleed past the pill edge.
const WIDTH: f64 = 160.0;
const HEIGHT: f64 = 56.0;

/// Logical gap between the bottom edge of the display and the overlay.
const BOTTOM_MARGIN: f64 = 64.0;

/// `NSStatusWindowLevel` (25): floats the pill above native-fullscreen app
/// content and above the `NSFloatingWindowLevel` (3) that Tauri's
/// `always_on_top` sets, while staying below the screensaver.
#[cfg(target_os = "macos")]
const NS_STATUS_WINDOW_LEVEL: isize = 25;

/// `NSWindowCollectionBehaviorFullScreenAuxiliary` (1 << 8): lets the window
/// join a native-fullscreen app's Space, OR'd on top of the `canJoinAllSpaces`
/// bit Tauri already sets via `visible_on_all_workspaces(true)`.
#[cfg(target_os = "macos")]
const NS_FULLSCREEN_AUXILIARY: usize = 1 << 8;

/// Create the overlay window once, hidden. Called from `setup`. Any failure is
/// logged and swallowed — a missing overlay must never break dictation.
pub fn create<R: Runtime>(app: &AppHandle<R>) {
    if app.get_webview_window(LABEL).is_some() {
        return;
    }
    let result = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("")
        .inner_size(WIDTH, HEIGHT)
        .visible(false)
        .focused(false)
        .transparent(true)
        .decorations(false)
        .shadow(false)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .accept_first_mouse(false)
        .build();

    match result {
        Ok(window) => {
            // Click-through: the pill must never intercept the pointer.
            let _ = window.set_ignore_cursor_events(true);
            // Float above native-fullscreen apps (once, at creation).
            raise_above_fullscreen(&window);
        }
        Err(e) => eprintln!("[voiceflow] overlay window build failed: {e}"),
    }
}

/// Raise the overlay to `NSStatusWindowLevel` and add the fullscreen-auxiliary
/// collection behavior so it floats above native-fullscreen apps. macOS-only;
/// a no-op elsewhere. `ns_window()` is only valid on the main thread, so the
/// native calls are marshaled there via `run_on_main_thread`. Re-asserted after
/// each `show()` in case the platform resets level/behavior.
#[cfg(target_os = "macos")]
fn raise_above_fullscreen<R: Runtime>(window: &tauri::WebviewWindow<R>) {
    use objc2::msg_send;
    use objc2::runtime::AnyObject;

    let window = window.clone();
    let _ = window.clone().run_on_main_thread(move || {
        let ns_window = match window.ns_window() {
            Ok(ptr) if !ptr.is_null() => ptr as *mut AnyObject,
            _ => return,
        };
        unsafe {
            let _: () = msg_send![ns_window, setLevel: NS_STATUS_WINDOW_LEVEL];
            let behavior: usize = msg_send![ns_window, collectionBehavior];
            let _: () = msg_send![ns_window, setCollectionBehavior: behavior | NS_FULLSCREEN_AUXILIARY];
        }
    });
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
fn raise_above_fullscreen<R: Runtime>(_window: &tauri::WebviewWindow<R>) {}

/// Position the overlay bottom-center of the display under the cursor, then show
/// it. Repositioned on every show so it follows the user across displays.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };

    reposition(app, &window);

    // Never take focus; just order the window in.
    let _ = window.show();
    // Re-assert click-through and the fullscreen window level after show in case
    // the platform reset them.
    let _ = window.set_ignore_cursor_events(true);
    raise_above_fullscreen(&window);
}

/// Hide the overlay. Safe to call when already hidden or absent.
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let _ = window.hide();
    }
}

/// Place the overlay at the bottom-center of the monitor containing the cursor,
/// falling back to the primary monitor. Monitor geometry is in physical pixels;
/// the window size is logical, so scale by the monitor's factor before centering.
fn reposition<R: Runtime>(app: &AppHandle<R>, window: &tauri::WebviewWindow<R>) {
    // Prefer the monitor under the cursor so the pill appears on the display the
    // user is actively dictating into; fall back to primary on any error.
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());

    let Some(monitor) = monitor else {
        return; // no monitor info — leave the window where it is
    };

    let scale = monitor.scale_factor();
    let pos = monitor.position(); // physical, i32
    let size = monitor.size(); // physical, u32

    // Overlay dimensions in physical pixels for this monitor's scale.
    let win = LogicalSize::new(WIDTH, HEIGHT).to_physical::<f64>(scale);

    let x = pos.x as f64 + (size.width as f64 - win.width) / 2.0;
    let y = pos.y as f64 + size.height as f64 - win.height - BOTTOM_MARGIN * scale;

    let _ = window.set_position(PhysicalPosition::new(x, y));
}
