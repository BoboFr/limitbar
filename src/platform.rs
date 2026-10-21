//! Thin Win32 helpers: popup placement, visibility, DWM styling, autostart.

use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HWND, POINT, RECT};
use windows_sys::Win32::Graphics::Dwm::{
    DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND,
};
use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetWindowRect, IsWindowVisible, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_NOSIZE, SW_HIDE, SW_SHOW,
};

/// Screen rectangle of the tray icon, in physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Anchor {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

fn hwnd(raw: isize) -> HWND {
    raw as HWND
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Returns false when another instance already holds the mutex.
pub fn single_instance() -> bool {
    let name = wide("Local\\limitbar-single-instance");
    unsafe {
        CreateMutexW(std::ptr::null(), 1, name.as_ptr());
        GetLastError() != ERROR_ALREADY_EXISTS
    }
}

/// Rounded Windows 11 corners, light frame and a hairline border.
pub fn style_window(raw: isize) {
    let h = hwnd(raw);
    let corner = DWMWCP_ROUND;
    let dark: i32 = 0;
    let border: u32 = 0x00DD_DDDD; // colors.hairline; COLORREF is 0x00BBGGRR
    unsafe {
        DwmSetWindowAttribute(h, DWMWA_WINDOW_CORNER_PREFERENCE as u32, &corner as *const _ as _, 4);
        DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, &dark as *const _ as _, 4);
        DwmSetWindowAttribute(h, DWMWA_BORDER_COLOR as u32, &border as *const _ as _, 4);
    }
}

pub fn is_visible(raw: isize) -> bool {
    unsafe { IsWindowVisible(hwnd(raw)) != 0 }
}

pub fn show(raw: isize) {
    unsafe {
        ShowWindow(hwnd(raw), SW_SHOW);
        SetForegroundWindow(hwnd(raw));
    }
}

pub fn hide(raw: isize) {
    unsafe {
        ShowWindow(hwnd(raw), SW_HIDE);
    }
}

/// Places the popup next to the tray icon, inside the monitor's work area
/// (i.e. above a bottom taskbar, below a top one, etc.).
pub fn position(raw: isize, anchor: Option<Anchor>) {
    let h = hwnd(raw);
    unsafe {
        let mut wr: RECT = std::mem::zeroed();
        GetWindowRect(h, &mut wr);
        let (w, ht) = (wr.right - wr.left, wr.bottom - wr.top);

        let pt = match anchor {
            Some(a) => POINT { x: a.x + a.w / 2, y: a.y + a.h / 2 },
            None => {
                let mut p: POINT = std::mem::zeroed();
                GetCursorPos(&mut p);
                p
            }
        };
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(mon, &mut mi);
        let (work, full) = (mi.rcWork, mi.rcMonitor);

        let dpi = GetDpiForWindow(h).max(96) as i32;
        let m = 12 * dpi / 96;

        let mut x = match anchor {
            Some(_) => (pt.x - w / 2).clamp(work.left + m, (work.right - w - m).max(work.left + m)),
            None => work.right - w - m, // no icon position: bottom-right corner, like Windows flyouts
        };
        let mut y = work.bottom - ht - m;
        if work.top > full.top {
            y = work.top + m; // taskbar at the top
        }
        if work.left > full.left {
            x = work.left + m; // taskbar on the left
            y = (pt.y - ht / 2).clamp(work.top + m, work.bottom - ht - m);
        } else if work.right < full.right {
            x = work.right - w - m; // taskbar on the right
            y = (pt.y - ht / 2).clamp(work.top + m, work.bottom - ht - m);
        }
        SetWindowPos(h, HWND_TOPMOST, x, y, 0, 0, SWP_NOSIZE | SWP_NOACTIVATE);
    }
}

// ---------------------------------------------------------------------------
// Launch at login (HKCU\...\Run)

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "limitbar";

pub fn autostart_enabled() -> bool {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    hkcu.open_subkey(RUN_KEY)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

pub fn set_autostart(enable: bool) {
    let hkcu = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER);
    let Ok((key, _)) = hkcu.create_subkey(RUN_KEY) else { return };
    if enable {
        if let Ok(exe) = std::env::current_exe() {
            let _ = key.set_value(RUN_VALUE, &format!("\"{}\"", exe.display()));
        }
    } else {
        let _ = key.delete_value(RUN_VALUE);
    }
}
