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
        return;
    };
    let rect = dock_rect(&area);
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
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    let _ = app.emit(EVENT_SHOWN, !was_visible);
    crate::ai::on_panel_shown(app);
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
