//! Sidebar window behaviour: dock, show/hide/toggle, pin, safe auto-hide.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

use crate::platform::{dock_rect, target_work_area};
use crate::state::AppState;

pub const MAIN: &str = "main";
pub const EVENT_SHOWN: &str = "sparkwell://shown";
pub const EVENT_HIDDEN: &str = "sparkwell://hidden";

/// Ignore focus loss right after showing (Windows can bounce focus while the
/// window activates).
const SHOW_GRACE: Duration = Duration::from_millis(400);
/// Wait before hiding on blur, then re-check focus; avoids hiding on
/// transient focus changes.
const BLUR_DELAY: Duration = Duration::from_millis(140);
/// A tray click that arrives right after a click-away auto-hide should not
/// re-open the panel the user just dismissed.
const AUTOHIDE_DEBOUNCE: Duration = Duration::from_millis(350);

pub fn main_window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(MAIN)
}

/// Snaps the window to the right edge of the target monitor's work area.
pub fn dock<R: Runtime>(window: &WebviewWindow<R>) {
    let Some(area) = target_work_area(window) else {
        log::warn!("no monitor work area available; showing without docking");
        return;
    };
    let rect = dock_rect(&area);
    log::info!("docking to {area:?} -> {rect:?}");
    let size = PhysicalSize::new(rect.width, rect.height);
    let pos = PhysicalPosition::new(rect.x, rect.y);
    // Size, move, then size again: moving between monitors with different DPI
    // makes Windows rescale the window, so re-apply the physical size.
    let _ = window.set_size(size);
    let _ = window.set_position(pos);
    let _ = window.set_size(size);
    let _ = window.set_position(pos);
}

pub fn show<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        return;
    };
    let state = app.state::<AppState>();
    let was_visible = window.is_visible().unwrap_or(false);
    if !was_visible {
        dock(&window);
    }
    if let Ok(mut t) = state.window.last_shown.lock() {
        *t = Instant::now();
    }
    let _ = window.set_always_on_top(state.is_pinned());
    if let Err(e) = window.show() {
        log::warn!("failed to show the sidebar: {e}");
    }
    let _ = window.unminimize();
    if let Err(e) = window.set_focus() {
        log::warn!("failed to focus the sidebar: {e}");
    }
    let _ = app.emit(EVENT_SHOWN, !was_visible);
    crate::ai::on_panel_shown(app);
}

/// Shows the panel without activating it. The window is created unfocused
/// (`focus: false`), so showing it uses SW_SHOWNOACTIVATE; only [`show`]
/// then takes focus explicitly.
pub fn show_passive<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        return;
    }
    dock(&window);
    let _ = window.set_always_on_top(app.state::<AppState>().is_pinned());
    if let Err(e) = window.show() {
        log::warn!("failed to show the sidebar: {e}");
    }
    let _ = app.emit(EVENT_SHOWN, true);
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        return;
    };
    let _ = window.hide();
    // Never leave the activation shortcut unregistered (e.g. hidden mid-recording).
    crate::hotkey::resume_if_idle(app);
    let _ = app.emit(EVENT_HIDDEN, ());
}

/// Hotkey/tray behaviour: hidden -> show; visible but unfocused -> focus;
/// visible and focused -> hide.
pub fn toggle<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = main_window(app) else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if visible && focused {
        hide(app);
    } else {
        show(app);
    }
}

/// Tray click: like toggle, but ignores the click that caused a click-away hide.
pub fn toggle_from_tray<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    let recently_autohidden = state
        .window
        .last_autohide
        .lock()
        .ok()
        .and_then(|t| *t)
        .map(|t| t.elapsed() < AUTOHIDE_DEBOUNCE)
        .unwrap_or(false);
    if recently_autohidden {
        return;
    }
    let Some(window) = main_window(app) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide(app);
    } else {
        show(app);
    }
}

/// Tray tooltip text for the activation shortcut's state.
pub fn tray_tooltip(status: &crate::hotkey::HotkeyStatus) -> String {
    if status.registered {
        format!(
            "Sparkwell · {}",
            crate::hotkey::display(&status.accelerator)
        )
    } else if status.accelerator.is_empty() {
        "Sparkwell · click to open".into()
    } else {
        format!(
            "Sparkwell · {} unavailable, click to open",
            crate::hotkey::display(&status.accelerator)
        )
    }
}

/// Keeps the tray tooltip in step with the activation shortcut, so a
/// shortcut that doesn't work is visible outside the panel too.
pub fn refresh_tray<R: Runtime>(app: &AppHandle<R>) {
    let Some(tray) = app.tray_by_id("sparkwell") else {
        return;
    };
    let status = app
        .state::<AppState>()
        .hotkey
        .lock()
        .map(|h| h.clone())
        .ok();
    if let Some(status) = status {
        let _ = tray.set_tooltip(Some(tray_tooltip(&status)));
    }
}

pub fn set_pinned<R: Runtime>(app: &AppHandle<R>, pinned: bool) -> crate::error::AppResult<bool> {
    let state = app.state::<AppState>();
    state.update_config(|c| c.pinned = pinned)?;
    if let Some(window) = main_window(app) {
        let _ = window.set_always_on_top(pinned);
    }
    Ok(pinned)
}

/// Blur handler: unpinned panels collapse when the user clicks away.
pub fn on_focus_lost<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<AppState>();
    if state.is_pinned() || state.window.suppress_autohide.load(Ordering::SeqCst) > 0 {
        return;
    }
    let shown_recently = state
        .window
        .last_shown
        .lock()
        .map(|t| t.elapsed() < SHOW_GRACE)
        .unwrap_or(true);
    if shown_recently {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(BLUR_DELAY).await;
        let state = app.state::<AppState>();
        let Some(window) = main_window(&app) else {
            return;
        };
        let still_blurred = !window.is_focused().unwrap_or(true);
        let visible = window.is_visible().unwrap_or(false);
        if still_blurred
            && visible
            && !state.is_pinned()
            && state.window.suppress_autohide.load(Ordering::SeqCst) == 0
        {
            if let Ok(mut t) = state.window.last_autohide.lock() {
                *t = Some(Instant::now());
            }
            hide(&app);
        }
    });
}

/// RAII guard that keeps the panel open while a native dialog has focus.
pub struct AutohideSuppressed<'a>(&'a AppState);

impl<'a> AutohideSuppressed<'a> {
    pub fn new(state: &'a AppState) -> Self {
        state
            .window
            .suppress_autohide
            .fetch_add(1, Ordering::SeqCst);
        Self(state)
    }
}

impl Drop for AutohideSuppressed<'_> {
    fn drop(&mut self) {
        self.0
            .window
            .suppress_autohide
            .fetch_sub(1, Ordering::SeqCst);
        if let Ok(mut t) = self.0.window.last_shown.lock() {
            // Treat returning from the dialog like a fresh show.
            *t = Instant::now();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey::HotkeyStatus;

    fn status(accelerator: &str, registered: bool) -> HotkeyStatus {
        HotkeyStatus {
            accelerator: accelerator.into(),
            registered,
            error: None,
            suspended: false,
        }
    }

    #[test]
    fn tray_tooltip_reflects_the_shortcut() {
        assert_eq!(
            tray_tooltip(&status("Ctrl+Super+K", true)),
            "Sparkwell · Ctrl+Win+K"
        );
        assert_eq!(
            tray_tooltip(&status("", false)),
            "Sparkwell · click to open"
        );
        assert_eq!(
            tray_tooltip(&status("Ctrl+Alt+Space", false)),
            "Sparkwell · Ctrl+Alt+Space unavailable, click to open"
        );
    }
}
