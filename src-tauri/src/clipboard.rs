//! Write-only `NSPasteboard` bridge.
//!
//! All clipboard writes originate in Rust (the pipeline's not-editable divert
//! and the tray's "Copy Last Transcription"), never in JS, so the
//! `tauri-plugin-clipboard-manager` (with its capability surface and `arboard`
//! dependency) would be dead weight. Instead this is a ~40-line raw `objc2`
//! `msg_send!` bridge in the house style of `menu_width.rs` / `overlay.rs`.
//!
//! Best-effort by design: a failed clipboard write logs to stderr and returns.
//! It **never** logs the text being copied (invariant #5).

/// Copy `text` onto the general pasteboard as UTF-8 plain text. Best-effort;
/// failures are logged (never the text itself) and swallowed. Marshaled to the
/// main thread (`NSPasteboard` is not documented thread-safe); runs inline when
/// already on main, e.g. the tray-menu handler. No-op on non-macOS targets.
pub fn write_text(app: &tauri::AppHandle, text: &str) {
    #[cfg(target_os = "macos")]
    {
        let text = text.to_string();
        let _ = app.run_on_main_thread(move || macos::write_text(&text));
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, text);
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};

    /// `NSUTF8StringEncoding`.
    const NS_UTF8_STRING_ENCODING: usize = 4;

    /// Build an `NSString` from a Rust `&str` via `-initWithBytes:length:
    /// encoding:` — not `+stringWithUTF8String:`, which truncates at an interior
    /// NUL. Returned string is `+1` retained; the caller releases it.
    unsafe fn ns_string(s: &str) -> *mut AnyObject {
        let cls = AnyClass::get(c"NSString").expect("NSString class");
        let alloc: *mut AnyObject = msg_send![cls, alloc];
        msg_send![
            alloc,
            initWithBytes: s.as_ptr() as *const std::ffi::c_void,
            length: s.len(),
            encoding: NS_UTF8_STRING_ENCODING,
        ]
    }

    pub fn write_text(text: &str) {
        unsafe {
            let ns_text = ns_string(text);
            if ns_text.is_null() {
                eprintln!("[scriva] clipboard: NSString build failed");
                return;
            }
            // Uniform Type Identifier for plain UTF-8 text on the pasteboard.
            let ns_type = ns_string("public.utf8-plain-text");

            let pb_cls = AnyClass::get(c"NSPasteboard").expect("NSPasteboard class");
            let pasteboard: *mut AnyObject = msg_send![pb_cls, generalPasteboard];
            if pasteboard.is_null() {
                let _: () = msg_send![ns_text, release];
                if !ns_type.is_null() {
                    let _: () = msg_send![ns_type, release];
                }
                eprintln!("[scriva] clipboard: no general pasteboard");
                return;
            }

            let _: isize = msg_send![pasteboard, clearContents];
            let ok: Bool = msg_send![pasteboard, setString: ns_text, forType: ns_type];
            if !ok.as_bool() {
                eprintln!("[scriva] clipboard: setString:forType: failed");
            }

            let _: () = msg_send![ns_text, release];
            if !ns_type.is_null() {
                let _: () = msg_send![ns_type, release];
            }
        }
    }
}
