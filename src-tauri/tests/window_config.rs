//! The main window's configuration carries acceptance fixes that a config
//! edit could silently undo.

use serde_json::Value;

fn main_window() -> Value {
    let conf: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
    conf["app"]["windows"][0].clone()
}

#[test]
fn a_hidden_start_never_takes_focus() {
    // A window created focused is focused by WebView2 even while hidden;
    // showing it from the hotkey or tray focuses it explicitly.
    assert_eq!(main_window()["focus"], Value::Bool(false));
}

#[test]
fn browser_autofill_is_off() {
    // WebView2 otherwise offers previously typed values in Sparkwell's fields.
    assert_eq!(main_window()["generalAutofillEnabled"], Value::Bool(false));
}
