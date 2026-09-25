//! Monitor/work-area discovery for right-edge docking.
//!
//! On Windows the panel opens on the monitor containing the foreground window
//! (where the user is working when they press the hotkey), falling back to the
//! monitor under the cursor. The *work area* excludes the taskbar.

use tauri::{Runtime, WebviewWindow};

/// A monitor work area in physical pixels plus its scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// Target geometry for the docked sidebar, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DockRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Preferred and minimum sidebar widths in logical pixels (the panel itself is
/// 16px narrower; the rest is the transparent shadow gutter).
pub const PREFERRED_WIDTH: f64 = 436.0;
pub const MIN_WIDTH: f64 = 392.0;

/// Computes the right-edge dock rectangle for a work area.
pub fn dock_rect(area: &WorkArea) -> DockRect {
    let logical_work_width = area.width as f64 / area.scale;
    // Stay a companion: never more than ~30% of a small screen, but never
    // narrower than the usable minimum.
    let logical = (logical_work_width * 0.30).clamp(MIN_WIDTH, PREFERRED_WIDTH);
    let width = ((logical * area.scale).round() as u32).min(area.width);
    DockRect {
        x: area.x + area.width as i32 - width as i32,
        y: area.y,
        width,
        height: area.height,
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
fn scale_for_point<R: Runtime>(window: &WebviewWindow<R>, x: i32, y: i32) -> f64 {
    window
        .available_monitors()
        .ok()
        .and_then(|monitors| {
            monitors.into_iter().find(|m| {
                let p = m.position();
                let s = m.size();
                x >= p.x && y >= p.y && x < p.x + s.width as i32 && y < p.y + s.height as i32
            })
        })
        .map(|m| m.scale_factor())
        .unwrap_or_else(|| window.scale_factor().unwrap_or(1.0))
}

#[cfg(windows)]
pub fn target_work_area<R: Runtime>(window: &WebviewWindow<R>) -> Option<WorkArea> {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetCursorPos, GetForegroundWindow};

    let own = window.hwnd().ok().map(|h| h.0 as isize).unwrap_or(0);
    // SAFETY: plain Win32 queries with valid out-pointers; handles are only
    // passed back to the same API family and never dereferenced.
    unsafe {
        let foreground = GetForegroundWindow();
        let monitor = if !foreground.is_null() && foreground as isize != own {
            MonitorFromWindow(foreground, MONITOR_DEFAULTTONEAREST)
        } else {
            let mut pt = POINT { x: 0, y: 0 };
            if GetCursorPos(&mut pt) == 0 {
                return fallback_work_area(window);
            }
            MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST)
        };
        if monitor.is_null() {
            return fallback_work_area(window);
        }
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            rcMonitor: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            rcWork: RECT { left: 0, top: 0, right: 0, bottom: 0 },
            dwFlags: 0,
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return fallback_work_area(window);
        }
        let w = info.rcWork;
        let cx = (w.left + w.right) / 2;
        let cy = (w.top + w.bottom) / 2;
        Some(WorkArea {
            x: w.left,
            y: w.top,
            width: (w.right - w.left).max(1) as u32,
            height: (w.bottom - w.top).max(1) as u32,
            scale: scale_for_point(window, cx, cy),
        })
    }
}

#[cfg(not(windows))]
pub fn target_work_area<R: Runtime>(window: &WebviewWindow<R>) -> Option<WorkArea> {
    fallback_work_area(window)
}

/// Cross-platform: the monitor under the cursor, else the current/primary one.
fn fallback_work_area<R: Runtime>(window: &WebviewWindow<R>) -> Option<WorkArea> {
    let monitor = window
        .cursor_position()
        .ok()
        .and_then(|p| window.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| window.current_monitor().ok().flatten())
        .or_else(|| window.primary_monitor().ok().flatten())?;
    let area = monitor.work_area();
    Some(WorkArea {
        x: area.position.x,
        y: area.position.y,
        width: area.size.width.max(1),
        height: area.size.height.max(1),
        scale: monitor.scale_factor(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docks_to_right_edge_at_100_percent() {
        let r = dock_rect(&WorkArea { x: 0, y: 0, width: 1920, height: 1040, scale: 1.0 });
        assert_eq!(r, DockRect { x: 1920 - 436, y: 0, width: 436, height: 1040 });
    }

    #[test]
    fn scales_with_dpi() {
        for scale in [1.25, 1.5, 2.0] {
            let width = (2560.0 * scale) as u32;
            let r = dock_rect(&WorkArea { x: 0, y: 0, width, height: 1400, scale });
            assert_eq!(r.width, (436.0 * scale).round() as u32, "scale {scale}");
            assert_eq!(r.x + r.width as i32, width as i32);
        }
    }

    #[test]
    fn small_logical_screens_use_minimum_width() {
        // 1920x1080 at 200% = 960 logical px wide.
        let r = dock_rect(&WorkArea { x: 0, y: 0, width: 1920, height: 1032, scale: 2.0 });
        assert_eq!(r.width, (MIN_WIDTH * 2.0) as u32);
    }

    #[test]
    fn secondary_monitor_offsets_and_taskbar_are_respected() {
        // Secondary monitor to the left of primary, taskbar at the top (y=40).
        let r = dock_rect(&WorkArea { x: -2560, y: 40, width: 2560, height: 1400, scale: 1.0 });
        assert_eq!(r.x, -436);
        assert_eq!(r.y, 40);
        assert_eq!(r.height, 1400);
    }

    #[test]
    fn never_wider_than_the_work_area() {
        let r = dock_rect(&WorkArea { x: 0, y: 0, width: 300, height: 600, scale: 1.0 });
        assert_eq!(r.width, 300);
        assert_eq!(r.x, 0);
    }
}
