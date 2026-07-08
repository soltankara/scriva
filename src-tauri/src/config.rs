//! Settings model, persistence, dev-only `.env` key override, and hotkey
//! token → accelerator mapping.

use tauri::{AppHandle, Runtime};
use tauri_plugin_store::StoreExt;

pub use scriva_core::settings::{effective_key, Settings};

/// Store file in the app data dir. M2 upgrade: move API keys into the macOS
/// Keychain; for M1 they live in this plain-JSON store (never logged).
pub const STORE_FILE: &str = "settings.json";
const STORE_KEY: &str = "settings";
/// Sibling store key for the first-run flag. Kept out of core's `Settings` —
/// it's shell UX state, not a pipeline setting, and the UI's debounced
/// whole-struct auto-save must not be able to clobber it.
const ONBOARDED_KEY: &str = "onboarded";

/// Load settings from the store, falling back to defaults if the store or the
/// entry is missing / unreadable.
pub fn load<R: Runtime>(app: &AppHandle<R>) -> Settings {
    match app.store(STORE_FILE) {
        Ok(store) => store
            .get(STORE_KEY)
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// Persist settings to the store (app data dir).
pub fn save<R: Runtime>(app: &AppHandle<R>, settings: &Settings) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    let value = serde_json::to_value(settings).map_err(|e| e.to_string())?;
    store.set(STORE_KEY, value);
    store.save().map_err(|e| e.to_string())
}

/// Whether first-run onboarding has been completed. Missing/unreadable = false.
pub fn load_onboarded<R: Runtime>(app: &AppHandle<R>) -> bool {
    match app.store(STORE_FILE) {
        Ok(store) => store
            .get(ONBOARDED_KEY)
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Persist the onboarding-completed flag.
pub fn save_onboarded<R: Runtime>(app: &AppHandle<R>, done: bool) -> Result<(), String> {
    let store = app.store(STORE_FILE).map_err(|e| e.to_string())?;
    store.set(ONBOARDED_KEY, done);
    store.save().map_err(|e| e.to_string())
}

/// Map UI hotkey tokens to a `tauri-plugin-global-shortcut` accelerator string.
///
/// Modifiers: ⌘→Super, ⌥→Alt, ⌃→Control, ⇧→Shift (emitted first, in order).
/// Keys: "Space" as-is, F1–F12 / Escape / arrows (`ArrowUp`…) as-is, any single
/// character uppercased. Exactly one non-modifier key is required. Joined with
/// "+". Example: `["⌥","Space"]` → `"Alt+Space"`.
pub fn combo_to_accelerator(combo: &[String]) -> Result<String, String> {
    if combo.is_empty() {
        return Err("Hotkey is empty — press a key combination.".to_string());
    }

    let mut modifiers: Vec<String> = Vec::new();
    let mut keys: Vec<String> = Vec::new();

    for token in combo {
        match token.as_str() {
            "⌘" => push_modifier(&mut modifiers, "Super"),
            "⌥" => push_modifier(&mut modifiers, "Alt"),
            "⌃" => push_modifier(&mut modifiers, "Control"),
            "⇧" => push_modifier(&mut modifiers, "Shift"),
            other => keys.push(map_key(other)?),
        }
    }

    match keys.len() {
        0 => Err("Hotkey needs a main key, not just modifiers (e.g. ⌥ then Space).".to_string()),
        1 => {
            let mut parts = modifiers;
            parts.push(keys.into_iter().next().unwrap());
            Ok(parts.join("+"))
        }
        n => Err(format!(
            "Hotkey must have exactly one main key, but got {n}."
        )),
    }
}

fn push_modifier(mods: &mut Vec<String>, name: &str) {
    if !mods.iter().any(|m| m == name) {
        mods.push(name.to_string());
    }
}

fn map_key(key: &str) -> Result<String, String> {
    match key {
        "Space" => Ok("Space".to_string()),
        "Escape" => Ok("Escape".to_string()),
        "ArrowUp" | "ArrowDown" | "ArrowLeft" | "ArrowRight" => Ok(key.to_string()),
        _ if is_function_key(key) => Ok(key.to_string()),
        _ if key.chars().count() == 1 => Ok(key.to_uppercase()),
        _ => Err(format!("Unsupported key in hotkey: \"{key}\".")),
    }
}

fn is_function_key(key: &str) -> bool {
    key.strip_prefix('F')
        .and_then(|n| n.parse::<u8>().ok())
        .map(|n| (1..=12).contains(&n))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn combo(tokens: &[&str]) -> Vec<String> {
        tokens.iter().map(|t| t.to_string()).collect()
    }

    #[test]
    fn maps_default_alt_space() {
        assert_eq!(
            combo_to_accelerator(&combo(&["⌥", "Space"])).unwrap(),
            "Alt+Space"
        );
    }

    #[test]
    fn maps_all_modifiers_first_then_key() {
        assert_eq!(
            combo_to_accelerator(&combo(&["⌘", "⇧", "D"])).unwrap(),
            "Super+Shift+D"
        );
    }

    #[test]
    fn uppercases_single_char_and_keeps_fkeys_escape_arrows() {
        assert_eq!(
            combo_to_accelerator(&combo(&["⌃", "d"])).unwrap(),
            "Control+D"
        );
        assert_eq!(combo_to_accelerator(&combo(&["F5"])).unwrap(), "F5");
        assert_eq!(
            combo_to_accelerator(&combo(&["⌥", "ArrowUp"])).unwrap(),
            "Alt+ArrowUp"
        );
        assert_eq!(
            combo_to_accelerator(&combo(&["⌃", "`"])).unwrap(),
            "Control+`"
        );
    }

    #[test]
    fn rejects_modifier_only_and_multiple_keys() {
        assert!(combo_to_accelerator(&combo(&["⌥"])).is_err());
        assert!(combo_to_accelerator(&combo(&["A", "B"])).is_err());
        assert!(combo_to_accelerator(&[]).is_err());
    }

    #[test]
    fn output_parses_as_a_valid_global_shortcut() {
        // Round-trip through the plugin's parser to confirm the string is valid.
        for tokens in [
            vec!["⌥", "Space"],
            vec!["⌘", "⇧", "D"],
            vec!["F5"],
            vec!["⌥", "ArrowUp"],
            vec!["⌃", "`"],
        ] {
            let accel = combo_to_accelerator(&combo(&tokens)).unwrap();
            assert!(
                accel
                    .parse::<tauri_plugin_global_shortcut::Shortcut>()
                    .is_ok(),
                "accelerator {accel:?} failed to parse"
            );
        }
    }
}
