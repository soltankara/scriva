//! macOS OS-integration surface: the Accessibility trust check and CGEvent
//! Unicode-string text injection. Single home for the a11y / injection layer.

#[cfg(target_os = "macos")]
mod macos {
    use std::thread;
    use std::time::Duration;

    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::event::{CGEvent, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        // Reports whether this process is trusted for Accessibility. If
        // `options` sets kAXTrustedCheckOptionPrompt=true and we are not yet
        // trusted, macOS shows its one-time "grant access" prompt.
        // Returns a C `Boolean` (unsigned char).
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> u8;
        static kAXTrustedCheckOptionPrompt: CFStringRef;
    }

    pub fn accessibility_trusted(prompt: bool) -> bool {
        unsafe {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let value = CFBoolean::from(prompt);
            let options =
                CFDictionary::from_CFType_pairs(&[(key.as_CFType(), value.as_CFType())]);
            AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef()) != 0
        }
    }

    /// Max UTF-16 code units per synthetic event. Some apps drop a single event
    /// carrying a very long string, so we chunk.
    const CHUNK_UTF16: usize = 20;
    const CHUNK_DELAY: Duration = Duration::from_millis(8);

    pub fn type_text(text: &str) {
        if text.is_empty() {
            return;
        }
        let source = match CGEventSource::new(CGEventSourceStateID::HIDSystemState) {
            Ok(s) => s,
            Err(_) => return,
        };

        // Accumulate whole chars (never split a surrogate pair across chunks).
        let mut buf: Vec<u16> = Vec::with_capacity(CHUNK_UTF16 + 2);
        let mut scratch = [0u16; 2];
        for ch in text.chars() {
            let units = ch.encode_utf16(&mut scratch);
            if buf.len() + units.len() > CHUNK_UTF16 {
                post_chunk(&source, &buf);
                buf.clear();
            }
            buf.extend_from_slice(units);
        }
        post_chunk(&source, &buf);
    }

    /// Post a key-down/up pair carrying `buf` as the event's Unicode string.
    /// Keycode 0 is a placeholder; the Unicode string is what actually types.
    fn post_chunk(source: &CGEventSource, buf: &[u16]) {
        if buf.is_empty() {
            return;
        }
        if let Ok(down) = CGEvent::new_keyboard_event(source.clone(), 0, true) {
            down.set_string_from_utf16_unchecked(buf);
            down.post(CGEventTapLocation::HID);
        }
        if let Ok(up) = CGEvent::new_keyboard_event(source.clone(), 0, false) {
            up.set_string_from_utf16_unchecked(buf);
            up.post(CGEventTapLocation::HID);
        }
        thread::sleep(CHUNK_DELAY);
    }
}

/// Is this process trusted for macOS Accessibility? (Required to inject
/// keystrokes into other apps — the #1 support issue when missing.) When
/// `prompt` is true and access is absent, macOS surfaces its grant prompt.
/// On non-macOS targets this is a no-op returning `true`.
pub fn accessibility_trusted(prompt: bool) -> bool {
    #[cfg(target_os = "macos")]
    {
        macos::accessibility_trusted(prompt)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = prompt;
        true
    }
}

/// Type `text` into the focused app via synthesized Unicode keyboard events,
/// chunked to survive long strings. Requires Accessibility permission (check
/// with `accessibility_trusted` first). No-op on non-macOS targets.
pub fn type_text(text: &str) {
    #[cfg(target_os = "macos")]
    {
        macos::type_text(text);
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = text;
    }
}
