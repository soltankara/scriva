//! Pre-injection editability probe via the macOS Accessibility API.
//!
//! Before typing dictated text into the frontmost app, we ask AX for the
//! system-wide focused UI element and inspect its role. If it is *clearly* not
//! a text control (e.g. a native button, a paused video's player), injecting
//! would misfire the app's keyboard shortcuts instead of entering text — so the
//! caller diverts to the clipboard. Anything ambiguous or any AX error **fails
//! open**: we inject exactly as before.
//!
//! Raw `extern "C"` bindings (same pattern as `inject.rs`) rather than the
//! `accessibility`/`accessibility-sys` crates: those pin `core-foundation-sys`
//! 0.8 while this repo is on `core-foundation` 0.10 — incompatible CF types at
//! the boundary. Attribute names are plain CFStrings, so nothing extra to link.

/// Outcome of the focused-element editability probe.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FocusCheck {
    /// The focused element is a known text control — inject.
    Editable,
    /// The focused element is a known non-text control with a non-settable
    /// value — divert to the clipboard. The **only** outcome that diverts.
    NotEditable,
    /// Ambiguous, no permission, no focused element, or an AX error — fail
    /// open and inject.
    Unknown,
}

/// Native text-entry roles. A focused element with one of these accepts text.
/// (Search fields are `AXTextField` with subrole `AXSearchField`; the role
/// alone is enough, no subrole reading needed.)
#[cfg(target_os = "macos")]
const EDITABLE_ROLES: &[&str] = &["AXTextField", "AXTextArea", "AXComboBox"];

/// Explicit native non-text controls. These divert **only** when `AXValue` was
/// affirmatively probed as not settable — otherwise they stay `Unknown`.
///
/// Deliberately EXCLUDES `AXWebArea`, `AXGroup`, `AXScrollArea`, `AXWindow`,
/// and `AXUnknown`: Chromium (Chrome, Electron) builds its AX tree lazily, so a
/// focused web text field reports as one of those until an AX client touches
/// it. Diverting on them would break e.g. Gmail-compose-in-Chrome on the first
/// dictation after launch. Fail open instead.
#[cfg(target_os = "macos")]
const NOT_EDITABLE_ROLES: &[&str] = &[
    "AXButton",
    "AXPopUpButton",
    "AXMenuButton",
    "AXMenuItem",
    "AXMenuBarItem",
    "AXCheckBox",
    "AXRadioButton",
    "AXDisclosureTriangle",
    "AXStaticText",
    "AXImage",
    "AXLink",
    "AXSlider",
    "AXIncrementor",
    "AXScrollBar",
    "AXValueIndicator",
    "AXTable",
    "AXOutline",
    "AXList",
    "AXRow",
    "AXCell",
    "AXColumn",
    "AXToolbar",
    "AXTabGroup",
    "AXDockItem",
];

/// Classify a focused element from its role and (when known) whether `AXValue`
/// is settable. Pure logic, unit-tested below — kept free of any AX server
/// interaction so the decision tree is exercised without a UI session.
///
/// - Editable role → `Editable`.
/// - Explicit non-text role → `NotEditable` only when `AXValue` was probed as
///   not settable (`Some(false)`); otherwise `Unknown`.
/// - Anything else → `Unknown`, promoted to `Editable` when `AXValue` is
///   affirmatively settable (`Some(true)`). Both `Editable` and `Unknown`
///   inject; the distinction is only for the stderr log.
#[cfg(target_os = "macos")]
fn classify(role: &str, value_settable: Option<bool>) -> FocusCheck {
    if EDITABLE_ROLES.contains(&role) {
        return FocusCheck::Editable;
    }
    if NOT_EDITABLE_ROLES.contains(&role) {
        return match value_settable {
            Some(false) => FocusCheck::NotEditable,
            _ => FocusCheck::Unknown,
        };
    }
    match value_settable {
        Some(true) => FocusCheck::Editable,
        _ => FocusCheck::Unknown,
    }
}

#[cfg(target_os = "macos")]
mod macos {
    use super::{classify, FocusCheck};

    use core_foundation::base::{CFGetTypeID, CFType, CFTypeRef, TCFType};
    use core_foundation::string::{CFString, CFStringGetTypeID};

    /// Opaque `AXUIElementRef`. We only pass it back to AX functions, so an
    /// opaque pointer type suffices.
    #[repr(C)]
    struct AXUIElement {
        _private: [u8; 0],
    }
    type AXUIElementRef = *const AXUIElement;

    /// AX error code type. We only need the `Success == 0` distinction.
    type AXError = i32;
    const K_AX_ERROR_SUCCESS: AXError = 0;

    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXUIElementCreateSystemWide() -> AXUIElementRef;
        fn AXUIElementCopyAttributeValue(
            element: AXUIElementRef,
            attribute: core_foundation::string::CFStringRef,
            value: *mut CFTypeRef,
        ) -> AXError;
        fn AXUIElementIsAttributeSettable(
            element: AXUIElementRef,
            attribute: core_foundation::string::CFStringRef,
            settable: *mut u8,
        ) -> AXError;
        fn AXUIElementSetMessagingTimeout(element: AXUIElementRef, timeout: f32) -> AXError;
    }

    /// Copy an attribute value as a Create-rule `CFType`. Returns `None` on any
    /// AX error (no focused element, no permission, beachballed app, …).
    unsafe fn copy_attr(element: AXUIElementRef, name: &CFString) -> Option<CFType> {
        let mut value: CFTypeRef = std::ptr::null();
        let err = AXUIElementCopyAttributeValue(element, name.as_concrete_TypeRef(), &mut value);
        if err != K_AX_ERROR_SUCCESS || value.is_null() {
            return None;
        }
        // Create-rule return: wrap so Drop calls CFRelease.
        Some(CFType::wrap_under_create_rule(value))
    }

    pub fn focused_element_accepts_text() -> FocusCheck {
        unsafe {
            let system_wide = AXUIElementCreateSystemWide();
            if system_wide.is_null() {
                return FocusCheck::Unknown;
            }
            // RAII-release the system-wide element via a CFType wrapper. It is a
            // Create-rule return like any other AX element.
            let _system_wide_guard = CFType::wrap_under_create_rule(system_wide as CFTypeRef);

            // Cap AX IPC at 250 ms. The default is 6 s to a beachballed app's
            // main thread — unacceptable on the release path. Set process-wide
            // via the system-wide element right after creating it; a stalled app
            // then returns kAXErrorCannotComplete fast → Unknown → fail open.
            let _ = AXUIElementSetMessagingTimeout(system_wide, 0.25);

            let focused_name = CFString::from_static_string("AXFocusedUIElement");
            let Some(focused) = copy_attr(system_wide, &focused_name) else {
                eprintln!("[scriva] focus check: no focused element -> Unknown");
                return FocusCheck::Unknown;
            };
            let focused_element = focused.as_CFTypeRef() as AXUIElementRef;

            // Read the role (an AXString). Type-check with CFGetTypeID before
            // treating the CFType as a CFString.
            let role_name = CFString::from_static_string("AXRole");
            let Some(role_value) = copy_attr(focused_element, &role_name) else {
                eprintln!("[scriva] focus check: role unavailable -> Unknown");
                return FocusCheck::Unknown;
            };
            if CFGetTypeID(role_value.as_CFTypeRef()) != CFStringGetTypeID() {
                eprintln!("[scriva] focus check: role not a string -> Unknown");
                return FocusCheck::Unknown;
            }
            let role = CFString::wrap_under_get_rule(
                role_value.as_CFTypeRef() as core_foundation::string::CFStringRef
            )
            .to_string();

            // Probe whether AXValue is settable — only meaningful for the
            // explicit non-text roles, so skip the IPC otherwise.
            let value_settable = if super::NOT_EDITABLE_ROLES.contains(&role.as_str()) {
                let value_name = CFString::from_static_string("AXValue");
                let mut settable: u8 = 0;
                let err = AXUIElementIsAttributeSettable(
                    focused_element,
                    value_name.as_concrete_TypeRef(),
                    &mut settable,
                );
                if err == K_AX_ERROR_SUCCESS {
                    Some(settable != 0)
                } else {
                    None
                }
            } else {
                None
            };

            let decision = classify(&role, value_settable);
            eprintln!("[scriva] focus check: role={role} -> {decision:?}");
            decision
        }
    }
}

/// Probe the system-wide focused UI element and decide whether it accepts typed
/// text. ~1–3 synchronous AX IPC round-trips, capped at 250 ms total by the
/// messaging timeout. **Call off the main thread** (it can block for up to that
/// cap). Non-macOS targets return `Unknown` (fail open).
pub fn focused_element_accepts_text() -> FocusCheck {
    #[cfg(target_os = "macos")]
    {
        macos::focused_element_accepts_text()
    }
    #[cfg(not(target_os = "macos"))]
    {
        FocusCheck::Unknown
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{classify, FocusCheck};

    #[test]
    fn editable_roles_are_editable() {
        for role in ["AXTextField", "AXTextArea", "AXComboBox"] {
            assert_eq!(classify(role, None), FocusCheck::Editable);
            // Settability is irrelevant once the role is a known text control.
            assert_eq!(classify(role, Some(false)), FocusCheck::Editable);
            assert_eq!(classify(role, Some(true)), FocusCheck::Editable);
        }
    }

    #[test]
    fn non_text_role_diverts_only_when_value_not_settable() {
        assert_eq!(classify("AXButton", Some(false)), FocusCheck::NotEditable);
        assert_eq!(classify("AXButton", None), FocusCheck::Unknown);
        assert_eq!(classify("AXButton", Some(true)), FocusCheck::Unknown);
        // A representative sample of the rest of the list.
        assert_eq!(classify("AXCheckBox", Some(false)), FocusCheck::NotEditable);
        assert_eq!(classify("AXMenuItem", Some(false)), FocusCheck::NotEditable);
        assert_eq!(classify("AXSlider", Some(false)), FocusCheck::NotEditable);
    }

    #[test]
    fn chromium_lazy_ax_roles_never_divert() {
        // Chromium reports these while a real text field is focused — must fail
        // open, never NotEditable, for every settability value.
        for role in [
            "AXWebArea",
            "AXGroup",
            "AXScrollArea",
            "AXWindow",
            "AXUnknown",
        ] {
            assert_eq!(classify(role, None), FocusCheck::Unknown);
            assert_eq!(classify(role, Some(false)), FocusCheck::Unknown);
            // Affirmatively settable promotes an ambiguous role to Editable.
            assert_eq!(classify(role, Some(true)), FocusCheck::Editable);
        }
    }

    #[test]
    fn unknown_role_with_settable_value_is_editable() {
        assert_eq!(classify("AXSomethingNew", Some(true)), FocusCheck::Editable);
        assert_eq!(classify("AXSomethingNew", None), FocusCheck::Unknown);
        assert_eq!(classify("AXSomethingNew", Some(false)), FocusCheck::Unknown);
    }
}
