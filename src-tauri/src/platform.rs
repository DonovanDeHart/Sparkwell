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

/// Preferred and minimum panel widths in logical pixels.
pub const PREFERRED_WIDTH: f64 = 436.0;
pub const MIN_WIDTH: f64 = 392.0;
/// Gap between the panel and the work-area edges, like a Windows flyout.
pub const EDGE_MARGIN: f64 = 8.0;
/// The panel fits its content within this range (logical pixels), and never
/// leaves the work area.
pub const MIN_HEIGHT: f64 = 700.0;
pub const MAX_HEIGHT: f64 = 900.0;
/// Used until the UI has measured its content.
pub const DEFAULT_HEIGHT: f64 = 760.0;

/// Computes the panel rectangle for a work area: right edge, top-anchored,
/// as tall as its content (`content_height`, logical px) within
/// [`MIN_HEIGHT`]..[`MAX_HEIGHT`].
pub fn dock_rect(area: &WorkArea, content_height: f64) -> DockRect {
    let scale = area.scale;
    let margin = (EDGE_MARGIN * scale).round() as u32;
    let fit = |extent: u32| extent.saturating_sub(2 * margin).max(1);

    let logical_work_width = area.width as f64 / scale;
    // Stay a companion: never more than ~30% of a small screen, but never
    // narrower than the usable minimum.
    let logical_width = (logical_work_width * 0.30).clamp(MIN_WIDTH, PREFERRED_WIDTH);
    let width = ((logical_width * scale).round() as u32).min(fit(area.width));
    let logical_height = if content_height.is_finite() {
        content_height.clamp(MIN_HEIGHT, MAX_HEIGHT)
    } else {
        DEFAULT_HEIGHT
    };
    let height = ((logical_height * scale).round() as u32).min(fit(area.height));
    DockRect {
        x: area.x + area.width as i32 - margin as i32 - width as i32,
        y: area.y + margin as i32,
        width,
        height,
    }
}

/// The work area of the monitor the window is on (for resizing in place).
pub fn current_work_area<R: Runtime>(window: &WebviewWindow<R>) -> Option<WorkArea> {
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
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

/// Windows build number (0 elsewhere or if unknown).
#[cfg(windows)]
pub fn windows_build() -> u32 {
    use windows_sys::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    registry_string(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
        "CurrentBuildNumber",
    )
    .and_then(|b| b.trim().parse().ok())
    .unwrap_or(0)
}

#[cfg(not(windows))]
pub fn windows_build() -> u32 {
    0
}

/// Frosted glass needs DWM system backdrops (Windows 11 22H2 and later), and
/// is skipped when the user has turned off Windows transparency effects.
#[cfg(windows)]
pub fn glass_supported() -> bool {
    use windows_sys::Win32::System::Registry::HKEY_CURRENT_USER;
    let transparency = registry_dword(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "EnableTransparency",
    )
    .unwrap_or(1);
    windows_build() >= 22621 && transparency != 0
}

#[cfg(not(windows))]
pub fn glass_supported() -> bool {
    false
}

#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
fn registry_dword(
    root: windows_sys::Win32::System::Registry::HKEY,
    key: &str,
    value: &str,
) -> Option<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_DWORD};
    let (key, value) = (wide(key), wide(value));
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    // SAFETY: valid NUL-terminated strings and an out-buffer of `size` bytes.
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            (&mut data as *mut u32).cast(),
            &mut size,
        )
    };
    (status == 0).then_some(data)
}

#[cfg(windows)]
fn registry_string(
    root: windows_sys::Win32::System::Registry::HKEY,
    key: &str,
    value: &str,
) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, RRF_RT_REG_SZ};
    let (key, value) = (wide(key), wide(value));
    let mut buf = [0u16; 64];
    let mut size = std::mem::size_of_val(&buf) as u32;
    // SAFETY: valid NUL-terminated strings and an out-buffer of `size` bytes.
    let status = unsafe {
        RegGetValueW(
            root,
            key.as_ptr(),
            value.as_ptr(),
            RRF_RT_REG_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]))
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
            rcMonitor: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
            rcWork: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
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

    fn area(x: i32, y: i32, width: u32, height: u32, scale: f64) -> WorkArea {
        WorkArea {
            x,
            y,
            width,
            height,
            scale,
        }
    }

    #[test]
    fn docks_top_right_with_a_small_gap_at_100_percent() {
        let r = dock_rect(&area(0, 0, 1920, 1040, 1.0), 780.0);
        assert_eq!(
            r,
            DockRect {
                x: 1920 - 8 - 436,
                y: 8,
                width: 436,
                height: 780
            }
        );
    }

    #[test]
    fn fits_content_within_the_compact_range() {
        let a = area(0, 0, 3440, 1392, 1.0);
        assert_eq!(dock_rect(&a, 520.0).height, 700, "short content");
        assert_eq!(dock_rect(&a, 812.4).height, 812, "fits content");
        assert_eq!(dock_rect(&a, 1600.0).height, 900, "long content scrolls");
        assert_eq!(dock_rect(&a, f64::NAN).height, DEFAULT_HEIGHT as u32);
        // Top-anchored: the top never moves as the height changes.
        assert_eq!(dock_rect(&a, 520.0).y, dock_rect(&a, 1600.0).y);
    }

    #[test]
    fn never_leaves_a_short_work_area() {
        // 1366x768 laptop with the taskbar: 720 px of work area.
        let r = dock_rect(&area(0, 0, 1366, 720, 1.0), 900.0);
        assert_eq!(r.y, 8);
        assert_eq!(r.height, 704);
        assert!(r.y + r.height as i32 <= 720);
    }

    #[test]
    fn scales_with_dpi() {
        for scale in [1.25, 1.5, 2.0] {
            let width = (2560.0 * scale) as u32;
            let r = dock_rect(&area(0, 0, width, (1400.0 * scale) as u32, scale), 800.0);
            assert_eq!(r.width, (436.0 * scale).round() as u32, "scale {scale}");
            assert_eq!(r.height, (800.0 * scale).round() as u32, "scale {scale}");
            let margin = (8.0 * scale).round() as i32;
            assert_eq!(r.x + r.width as i32, width as i32 - margin);
            assert_eq!(r.y, margin);
        }
    }

    #[test]
    fn small_logical_screens_use_minimum_width() {
        // 1920x1080 at 200% = 960 logical px wide.
        let r = dock_rect(&area(0, 0, 1920, 1032, 2.0), 800.0);
        assert_eq!(r.width, (MIN_WIDTH * 2.0) as u32);
        assert_eq!(r.height, 1032 - 32, "clamped to the work area");
    }

    #[test]
    fn secondary_monitor_offsets_and_taskbar_are_respected() {
        // Secondary monitor to the left of primary, taskbar at the top (y=40).
        let r = dock_rect(&area(-2560, 40, 2560, 1400, 1.0), 760.0);
        assert_eq!(r.x, -8 - 436);
        assert_eq!(r.y, 48);
        assert_eq!(r.height, 760);
    }

    #[test]
    fn never_wider_than_the_work_area() {
        let r = dock_rect(&area(0, 0, 300, 600, 1.0), 760.0);
        assert_eq!(r.width, 284);
        assert!(r.x >= 0 && r.x + r.width as i32 <= 300);
        assert!(r.y + r.height as i32 <= 600);
    }
}
