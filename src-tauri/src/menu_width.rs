//! Widen the tray `NSMenu` panel on macOS 26.
//!
//! The tray dropdown renders too tight — labels crowd the panel's rounded edge.
//! The correct fix is `-[NSMenu setMinimumWidth:]`, but Tauri 2.11 wraps muda's
//! `Menu` and exposes no public `NSMenu` handle (`inner()` is `pub(crate)`, only
//! `muda::MenuId` is re-exported), and title padding (ASCII or U+00A0) is trimmed
//! by macOS menu sizing so it has no visible effect.
//!
//! So we reach the `NSMenu` at runtime: register an `NSNotificationCenter`
//! observer for `NSMenuDidBeginTrackingNotification`. The notification's `object`
//! is the menu that began tracking; when it's our tray menu (Enabled ·
//! separator · Settings · Quit — separators count toward `numberOfItems`) we
//! set its minimum width. Registration happens once during setup on the main
//! thread; the observer and its block are intentionally leaked for the app's
//! lifetime. **Keep the guard below in sync with the tray menu in `lib.rs`.**

/// Comfortable minimum panel width, in points (CGFloat).
#[cfg(target_os = "macos")]
const MIN_WIDTH: f64 = 210.0;

/// Register the tray-menu widener. macOS-only; no-op elsewhere. Call once on the
/// main thread during setup.
#[cfg(target_os = "macos")]
pub fn install() {
    use block2::RcBlock;
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use std::ffi::c_char;

    // Build an NSString via +[NSString stringWithUTF8String:].
    unsafe fn ns_string(s: &std::ffi::CStr) -> *mut AnyObject {
        let cls = AnyClass::get(c"NSString").expect("NSString class");
        unsafe { msg_send![cls, stringWithUTF8String: s.as_ptr() as *const c_char] }
    }

    unsafe {
        let name = ns_string(c"NSMenuDidBeginTrackingNotification");

        // Block runs synchronously on the posting (main) thread because we pass a
        // nil queue. `notification` is an NSNotification; its `object` is the menu.
        let block = RcBlock::new(move |notification: *mut AnyObject| {
            if notification.is_null() {
                return;
            }
            let menu: *mut AnyObject = msg_send![notification, object];
            if menu.is_null() {
                return;
            }
            // Guard: only our tray menu — exactly four items (Enabled, separator,
            // Settings, Quit) with the first titled "Enabled". Prevents touching
            // any other NSMenu in the process.
            let count: isize = msg_send![menu, numberOfItems];
            if count != 4 {
                return;
            }
            let item0: *mut AnyObject = msg_send![menu, itemAtIndex: 0isize];
            if item0.is_null() {
                return;
            }
            let title: *mut AnyObject = msg_send![item0, title];
            if title.is_null() {
                return;
            }
            let prefix = ns_string(c"Enabled");
            let has_prefix: Bool = msg_send![title, hasPrefix: prefix];
            if !has_prefix.as_bool() {
                return;
            }
            let _: () = msg_send![menu, setMinimumWidth: MIN_WIDTH];
        });

        let center_cls =
            AnyClass::get(c"NSNotificationCenter").expect("NSNotificationCenter class");
        let center: *mut AnyObject = msg_send![center_cls, defaultCenter];
        let nil: *mut AnyObject = std::ptr::null_mut();
        let observer: *mut AnyObject = msg_send![
            center,
            addObserverForName: name,
            object: nil,
            queue: nil,
            usingBlock: &*block,
        ];

        // One-time setup: keep the observer + block alive for the app's lifetime.
        // The notification center copies/retains the block, but we forget our
        // RcBlock (and drop the observer token without unregistering) so nothing
        // tears the observation down.
        std::mem::forget(block);
        let _ = observer;
    }
}

/// No-op on non-macOS platforms.
#[cfg(not(target_os = "macos"))]
pub fn install() {}
