//! Activation hotkey: validation, registration lifecycle, and persistence.
//!
//! Lifecycle rule from the spec: unregister the old shortcut before registering
//! the new one, and persist only after the new registration succeeded. If the
//! new one fails, the old shortcut is restored so Sparkwell stays reachable.

use serde::Serialize;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::error::{AppError, AppResult};
use crate::state::AppState;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HotkeyStatus {
    /// Canonical accelerator, e.g. `Ctrl+Alt+Space`.
    pub accelerator: String,
    /// Whether the OS accepted the registration.
    pub registered: bool,
    /// Why registration failed, if it did.
    pub error: Option<String>,
    /// Temporarily unregistered while the user records a new shortcut.
    #[serde(skip)]
    pub suspended: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyClass {
    /// Letters, digits, punctuation, Space, Enter: need two modifiers.
    Typing,
    /// F1-F12, navigation, numpad: need one modifier.
    Command,
    /// F13-F24: may be used alone.
    Free,
}

/// Maps a key token (KeyboardEvent.code or global-hotkey name) to its
/// canonical name and class. `None` means the key cannot be used.
fn classify_key(token: &str) -> Option<(String, KeyClass)> {
    let t = token.trim();
    let upper = t.to_ascii_uppercase();

    if let Some(letter) = upper.strip_prefix("KEY").filter(|l| l.len() == 1) {
        return classify_key(letter);
    }
    if let Some(digit) = upper.strip_prefix("DIGIT").filter(|d| d.len() == 1) {
        return classify_key(digit);
    }
    if upper.len() == 1 {
        let c = upper.chars().next()?;
        if c.is_ascii_alphanumeric() {
            return Some((upper, KeyClass::Typing));
        }
    }
    if let Some(n) = upper.strip_prefix('F').and_then(|n| n.parse::<u8>().ok()) {
        return match n {
            1..=12 => Some((format!("F{n}"), KeyClass::Command)),
            13..=24 => Some((format!("F{n}"), KeyClass::Free)),
            _ => None,
        };
    }

    let typing = [
        ("SPACE", "Space"),
        ("ENTER", "Enter"),
        ("BACKQUOTE", "Backquote"),
        ("MINUS", "Minus"),
        ("EQUAL", "Equal"),
        ("BRACKETLEFT", "BracketLeft"),
        ("BRACKETRIGHT", "BracketRight"),
        ("BACKSLASH", "Backslash"),
        ("SEMICOLON", "Semicolon"),
        ("QUOTE", "Quote"),
        ("COMMA", "Comma"),
        ("PERIOD", "Period"),
        ("SLASH", "Slash"),
    ];
    if let Some((_, name)) = typing.iter().find(|(k, _)| *k == upper) {
        return Some((name.to_string(), KeyClass::Typing));
    }

    let command = [
        ("INSERT", "Insert"),
        ("HOME", "Home"),
        ("END", "End"),
        ("PAGEUP", "PageUp"),
        ("PAGEDOWN", "PageDown"),
        ("ARROWUP", "ArrowUp"),
        ("ARROWDOWN", "ArrowDown"),
        ("ARROWLEFT", "ArrowLeft"),
        ("ARROWRIGHT", "ArrowRight"),
        ("UP", "ArrowUp"),
        ("DOWN", "ArrowDown"),
        ("LEFT", "ArrowLeft"),
        ("RIGHT", "ArrowRight"),
        ("PAUSE", "Pause"),
        ("NUMPAD0", "Numpad0"),
        ("NUMPAD1", "Numpad1"),
        ("NUMPAD2", "Numpad2"),
        ("NUMPAD3", "Numpad3"),
        ("NUMPAD4", "Numpad4"),
        ("NUMPAD5", "Numpad5"),
        ("NUMPAD6", "Numpad6"),
        ("NUMPAD7", "Numpad7"),
        ("NUMPAD8", "Numpad8"),
        ("NUMPAD9", "Numpad9"),
        ("NUMPADADD", "NumpadAdd"),
        ("NUMPADSUBTRACT", "NumpadSubtract"),
        ("NUMPADMULTIPLY", "NumpadMultiply"),
        ("NUMPADDIVIDE", "NumpadDivide"),
        ("NUMPADDECIMAL", "NumpadDecimal"),
        ("NUMPADENTER", "NumpadEnter"),
    ];
    command
        .iter()
        .find(|(k, _)| *k == upper)
        .map(|(_, name)| (name.to_string(), KeyClass::Command))
}

const UNUSABLE_KEYS: &[&str] = &[
    "ESCAPE",
    "ESC",
    "TAB",
    "CAPSLOCK",
    "NUMLOCK",
    "SCROLLLOCK",
    "PRINTSCREEN",
    "BACKSPACE",
    "DELETE",
];

/// Shortcuts Windows reserves for itself even though they pass the rules.
const RESERVED: &[&str] = &[
    "Alt+F4",
    "Ctrl+Alt+Delete",
    "Shift+Super+S",
    "Ctrl+Super+D",
    "Ctrl+Super+F4",
    "Ctrl+Super+ArrowLeft",
    "Ctrl+Super+ArrowRight",
    "Super+ArrowUp",
    "Super+ArrowDown",
    "Super+ArrowLeft",
    "Super+ArrowRight",
    "Shift+Super+ArrowLeft",
    "Shift+Super+ArrowRight",
];

/// Human-readable form for messages (`Super` is the Windows key).
pub fn display(accelerator: &str) -> String {
    accelerator.replace("Super", "Win")
}

/// Validates an accelerator and returns its canonical form
/// (`Ctrl+Alt+Shift+Super+Key`).
pub fn validate(accelerator: &str) -> AppResult<String> {
    let tokens: Vec<&str> = accelerator.split('+').map(str::trim).collect();
    if accelerator.trim().is_empty() || tokens.iter().any(|t| t.is_empty()) {
        return Err(AppError::HotkeyInvalid("Press a key combination.".into()));
    }

    let (mut ctrl, mut alt, mut shift, mut sup) = (false, false, false, false);
    let mut key: Option<(String, KeyClass)> = None;
    for token in tokens {
        match token.to_ascii_uppercase().as_str() {
            "CTRL" | "CONTROL" | "COMMANDORCONTROL" | "CMDORCTRL" => ctrl = true,
            "ALT" | "OPTION" => alt = true,
            "SHIFT" => shift = true,
            "SUPER" | "WIN" | "META" | "CMD" | "COMMAND" => sup = true,
            other => {
                if key.is_some() {
                    return Err(AppError::HotkeyInvalid(
                        "Use only one key with your modifiers.".into(),
                    ));
                }
                if UNUSABLE_KEYS.contains(&other) {
                    return Err(AppError::HotkeyInvalid(format!(
                        "{token} can't be used in the activation shortcut."
                    )));
                }
                key = Some(classify_key(token).ok_or_else(|| {
                    AppError::HotkeyInvalid(format!(
                        "{token} isn't supported. Try a letter, number, or function key."
                    ))
                })?);
            }
        }
    }

    let (key_name, class) = key.ok_or_else(|| {
        AppError::HotkeyInvalid("Add a key to go with the modifier, like Ctrl+Alt+Space.".into())
    })?;
    let modifiers = [ctrl, alt, shift, sup].iter().filter(|m| **m).count();

    match class {
        KeyClass::Typing if modifiers < 2 => {
            let example = if modifiers == 0 {
                format!("Ctrl+Alt+{key_name}")
            } else {
                "Ctrl+Alt+Space".into()
            };
            return Err(AppError::HotkeyInvalid(format!(
                "Use at least two modifiers (for example {example}) so the shortcut doesn't take over typing in other apps."
            )));
        }
        KeyClass::Command if modifiers < 1 => {
            return Err(AppError::HotkeyInvalid(format!(
                "Add Ctrl, Alt, Shift, or Win to {key_name}."
            )));
        }
        _ => {}
    }

    let mut parts: Vec<&str> = Vec::new();
    if ctrl {
        parts.push("Ctrl");
    }
    if alt {
        parts.push("Alt");
    }
    if shift {
        parts.push("Shift");
    }
    if sup {
        parts.push("Super");
    }
    parts.push(&key_name);
    let canonical = parts.join("+");

    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&canonical)) {
        return Err(AppError::HotkeyInvalid(format!(
            "Windows reserves {} for itself. Choose another combination.",
            display(&canonical)
        )));
    }
    // Final guard: the OS-level parser must accept it too.
    canonical.parse::<Shortcut>().map_err(|_| {
        AppError::HotkeyInvalid(format!(
            "{} isn't a supported shortcut.",
            display(&canonical)
        ))
    })?;
    Ok(canonical)
}

fn register<R: Runtime>(app: &AppHandle<R>, accelerator: &str) -> Result<(), String> {
    let shortcut: Shortcut = accelerator.parse().map_err(|e| format!("{e}"))?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| e.to_string())
}

fn unregister<R: Runtime>(app: &AppHandle<R>, accelerator: &str) {
    if let Ok(shortcut) = accelerator.parse::<Shortcut>() {
        if let Err(e) = app.global_shortcut().unregister(shortcut) {
            log::warn!("failed to unregister {accelerator}: {e}");
        }
    }
}

fn conflict_message(accelerator: &str) -> String {
    format!(
        "{} is already in use by another app or by Windows. Try a different combination.",
        display(accelerator)
    )
}

/// Registers the configured shortcut at startup. Failure is recorded, not fatal.
pub fn register_initial<R: Runtime>(app: &AppHandle<R>, configured: &str) -> HotkeyStatus {
    let accelerator =
        validate(configured).unwrap_or_else(|_| crate::settings::DEFAULT_HOTKEY.to_string());
    match register(app, &accelerator) {
        Ok(()) => HotkeyStatus {
            accelerator,
            registered: true,
            error: None,
            suspended: false,
        },
        Err(e) => {
            log::warn!("could not register {accelerator}: {e}");
            HotkeyStatus {
                error: Some(conflict_message(&accelerator)),
                accelerator,
                registered: false,
                suspended: false,
            }
        }
    }
}

/// Replaces the activation shortcut (unregister old -> register new -> persist).
pub fn change<R: Runtime>(app: &AppHandle<R>, requested: &str) -> AppResult<HotkeyStatus> {
    let state = app.state::<AppState>();
    let canonical = validate(requested)?;
    let mut hk = state
        .hotkey
        .lock()
        .map_err(|_| AppError::Internal("hotkey state poisoned".into()))?;

    let old = hk.accelerator.clone();
    let old_was_active = hk.registered && !hk.suspended;
    if canonical == old && hk.registered {
        if hk.suspended {
            register(app, &canonical)
                .map_err(|_| AppError::HotkeyConflict(conflict_message(&canonical)))?;
            hk.suspended = false;
        }
        return Ok(hk.clone());
    }

    if old_was_active {
        unregister(app, &old);
    }
    if let Err(e) = register(app, &canonical) {
        log::info!("hotkey {canonical} rejected by OS: {e}");
        // Restore the previous shortcut so Sparkwell stays reachable.
        if hk.registered {
            if let Err(e) = register(app, &old) {
                log::warn!("could not restore previous hotkey {old}: {e}");
                hk.registered = false;
            }
        }
        hk.suspended = false;
        return Err(AppError::HotkeyConflict(conflict_message(&canonical)));
    }

    // Registered: persist. If persisting fails, roll back to keep disk and OS in sync.
    let save_result = state.update_config(|cfg| cfg.hotkey = canonical.clone());
    if let Err(err) = save_result {
        unregister(app, &canonical);
        if hk.registered {
            let _ = register(app, &old);
        }
        return Err(err);
    }

    *hk = HotkeyStatus {
        accelerator: canonical,
        registered: true,
        error: None,
        suspended: false,
    };
    Ok(hk.clone())
}

/// Temporarily releases the shortcut while the user records a new one, so
/// pressing the current combination is captured instead of hiding the panel.
pub fn suspend<R: Runtime>(app: &AppHandle<R>) -> AppResult<()> {
    let state = app.state::<AppState>();
    let mut hk = state
        .hotkey
        .lock()
        .map_err(|_| AppError::Internal("hotkey state poisoned".into()))?;
    if hk.registered && !hk.suspended {
        unregister(app, &hk.accelerator);
        hk.suspended = true;
    }
    Ok(())
}

/// Like [`resume`], but never blocks: used from the window-hide path, which
/// can run on the main thread. If a hotkey change currently holds the lock it
/// is waiting on the main thread to register, so blocking here would deadlock;
/// that change leaves the registration consistent on its own.
pub fn resume_if_idle<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let Ok(mut hk) = state.hotkey.try_lock() else {
        return;
    };
    if hk.suspended {
        hk.suspended = false;
        if let Err(e) = register(app, &hk.accelerator) {
            log::warn!("could not re-register {}: {e}", hk.accelerator);
            hk.registered = false;
            hk.error = Some(conflict_message(&hk.accelerator));
        }
    }
}

/// Re-registers a suspended shortcut. Safe to call at any time.
pub fn resume<R: Runtime>(app: &AppHandle<R>) -> AppResult<HotkeyStatus> {
    let state = app.state::<AppState>();
    let mut hk = state
        .hotkey
        .lock()
        .map_err(|_| AppError::Internal("hotkey state poisoned".into()))?;
    if hk.suspended {
        hk.suspended = false;
        if let Err(e) = register(app, &hk.accelerator) {
            log::warn!("could not re-register {}: {e}", hk.accelerator);
            hk.registered = false;
            hk.error = Some(conflict_message(&hk.accelerator));
        }
    }
    Ok(hk.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(a: &str) -> String {
        validate(a).unwrap_or_else(|e| panic!("{a} should be valid: {e}"))
    }

    fn invalid(a: &str) {
        assert!(
            matches!(validate(a), Err(AppError::HotkeyInvalid(_))),
            "{a} should be invalid"
        );
    }

    #[test]
    fn canonicalises_order_and_names() {
        assert_eq!(ok("Alt+Ctrl+Space"), "Ctrl+Alt+Space");
        assert_eq!(ok("control+shift+KeyK"), "Ctrl+Shift+K");
        assert_eq!(ok("Win+Alt+Digit5"), "Alt+Super+5");
        assert_eq!(ok("Ctrl+F9"), "Ctrl+F9");
        assert_eq!(ok("F13"), "F13");
        assert_eq!(ok("Shift+Alt+Up"), "Alt+Shift+ArrowUp");
        assert_eq!(ok("Ctrl+Alt+Period"), "Ctrl+Alt+Period");
        assert_eq!(ok("CommandOrControl+Alt+Space"), "Ctrl+Alt+Space");
    }

    #[test]
    fn rejects_combinations_that_would_hijack_typing() {
        invalid("K");
        invalid("Ctrl+K");
        invalid("Shift+A");
        invalid("Alt+Space");
        invalid("Super+L");
        invalid("F5");
    }

    #[test]
    fn rejects_malformed_and_unusable() {
        invalid("");
        invalid("Ctrl+");
        invalid("Ctrl+Alt");
        invalid("Ctrl+Alt+A+B");
        invalid("Ctrl+Alt+Escape");
        invalid("Ctrl+Alt+Tab");
        invalid("Ctrl+Alt+Delete");
        invalid("Ctrl+Alt+CapsLock");
        invalid("Ctrl+Alt+Banana");
        invalid("Ctrl+F25");
    }

    #[test]
    fn rejects_windows_reserved() {
        invalid("Alt+F4");
        invalid("Win+Shift+S");
        invalid("Win+Up");
        invalid("Ctrl+Win+D");
    }

    #[test]
    fn default_hotkey_is_valid() {
        assert_eq!(
            ok(crate::settings::DEFAULT_HOTKEY),
            crate::settings::DEFAULT_HOTKEY
        );
    }

    #[test]
    fn display_uses_windows_key_name() {
        assert_eq!(display("Ctrl+Super+K"), "Ctrl+Win+K");
    }
}
