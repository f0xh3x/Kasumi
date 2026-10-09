//! Thin Win32 helpers. All coordinates here are physical pixels
//! (the process is per-monitor DPI aware v2, set up by tao).

use std::collections::HashMap;
use windows::core::BOOL;
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QueryDisplayConfig,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO,
    MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::Globalization::{GetLocaleInfoW, LOCALE_SENGLISHLANGUAGENAME, LOCALE_SISO639LANGNAME2};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, GetKeyboardLayoutList, SendInput, HKL, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_RWIN, VK_SPACE,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, SetWindowSubclass, SHQueryUserNotificationState, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, EnumWindows, GetWindowTextLengthW, IsIconic, IsWindowVisible, ASFW_ANY,
    WS_EX_TRANSPARENT, WS_THICKFRAME, STYLESTRUCT, WM_STYLECHANGING,
    GetClassNameW, GetCursorPos, GetGUIThreadInfo, GetWindowThreadProcessId, PostMessageW, GUITHREADINFO, WM_INPUTLANGCHANGEREQUEST, GetForegroundWindow, SetForegroundWindow, GetWindowLongPtrW, GetWindowLongW,
    GetWindowRect, IsZoomed, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE,
    GWL_STYLE, HWND_TOPMOST, MONITORINFOF_PRIMARY, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
    SWP_NOMOVE, SWP_NOSIZE, WS_CAPTION, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

#[derive(Debug, Clone, PartialEq)]
pub struct Monitor {
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub device: String,
    /// EDID name, e.g. `ARZOPA`. Empty if unknown.
    pub friendly: String,
    pub primary: bool,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub scale: f64,
}

impl Monitor {
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
    pub fn same_rect(&self, r: &RECT) -> bool {
        self.left == r.left && self.top == r.top && self.right == r.right && self.bottom == r.bottom
    }
}

/// A NUL-terminated UTF-16 buffer as a String.
pub fn wide_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// All active monitors, sorted left to right (then top to bottom).
pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn proc(hmon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<Monitor>);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
            let (mut dx, mut dy) = (96u32, 96u32);
            let _ = GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
            let r = info.monitorInfo.rcMonitor;
            list.push(Monitor {
                device: wide_to_string(&info.szDevice),
                friendly: String::new(),
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
                scale: dx as f64 / 96.0,
            });
        }
        BOOL(1)
    }

    let mut list: Vec<Monitor> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(proc), LPARAM(&mut list as *mut _ as isize));
    }
    let names = friendly_names();
    for m in &mut list {
        if let Some(n) = names.get(&m.device) {
            m.friendly = n.clone();
        }
    }
    list.sort_by_key(|m| (m.left, m.top));
    list
}

/// Maps GDI device names to monitor EDID names via the DisplayConfig API.
fn friendly_names() -> HashMap<String, String> {
    let mut out = HashMap::new();
    unsafe {
        let (mut np, mut nm) = (0u32, 0u32);
        if GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm).is_err() {
            return out;
        }
        let mut paths: Vec<DISPLAYCONFIG_PATH_INFO> = vec![std::mem::zeroed(); np as usize];
        let mut modes: Vec<DISPLAYCONFIG_MODE_INFO> = vec![std::mem::zeroed(); nm as usize];
        if QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        )
        .is_err()
        {
            return out;
        }
        for path in &paths[..np as usize] {
            let mut src: DISPLAYCONFIG_SOURCE_DEVICE_NAME = std::mem::zeroed();
            src.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
            src.header.size = std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
            src.header.adapterId = path.sourceInfo.adapterId;
            src.header.id = path.sourceInfo.id;
            if DisplayConfigGetDeviceInfo(&mut src.header) != 0 {
                continue;
            }
            let mut tgt: DISPLAYCONFIG_TARGET_DEVICE_NAME = std::mem::zeroed();
            tgt.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME;
            tgt.header.size = std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32;
            tgt.header.adapterId = path.targetInfo.adapterId;
            tgt.header.id = path.targetInfo.id;
            if DisplayConfigGetDeviceInfo(&mut tgt.header) != 0 {
                continue;
            }
            out.insert(
                wide_to_string(&src.viewGdiDeviceName),
                wide_to_string(&tgt.monitorFriendlyDeviceName),
            );
        }
    }
    out
}

pub fn cursor_pos() -> Option<(i32, i32)> {
    let mut p = POINT::default();
    unsafe { GetCursorPos(&mut p).ok()? };
    Some((p.x, p.y))
}

fn hwnd(h: isize) -> HWND {
    HWND(h as *mut _)
}

/// Overlays allowed to take keyboard focus right now (the launcher while it's open).
static ACTIVATABLE: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());

fn activatable(h: HWND) -> bool {
    ACTIVATABLE.lock().unwrap().contains(&(h.0 as isize))
}

/// Lets an overlay take keyboard focus (`on`), or makes it non-activating again.
pub fn set_activatable(h: isize, on: bool) {
    {
        let mut list = ACTIVATABLE.lock().unwrap();
        list.retain(|&x| x != h);
        if on {
            list.push(h);
        }
    }
    apply_overlay_style(h);
}

/// Brings an overlay to the foreground (we just got the hotkey or click, so we may).
pub fn focus(h: isize) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowThreadProcessId, PeekMessageW, MSG, PM_NOREMOVE};
    unsafe {
        let w = hwnd(h);
        // Windows only lets the app in front hand the foreground over: borrow its
        // input queue for the switch (this thread needs a message queue for that).
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let me = GetCurrentThreadId();
        let front = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached = front != 0 && front != me && AttachThreadInput(me, front, true).as_bool();
        let _ = SetForegroundWindow(w);
        if attached {
            let _ = AttachThreadInput(me, front, false);
        }
        if GetForegroundWindow() != w {
            use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
            keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            let _ = SetForegroundWindow(w);
        }
    }
}

/// The window in front, to give the foreground back to.
pub fn foreground() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

/// Gives the foreground back to `h` (the app that had it before the launcher), or,
/// when that's gone or no longer an app window someone works in (hidden Search,
/// minimized...), to the front-most app window. Never to one of `ours`.
pub fn restore_foreground(h: isize, ours: &[isize]) {
    let target = Some(hwnd(h)).filter(|&w| is_app_window(w, ours)).or_else(|| top_app_window(ours));
    if let Some(w) = target {
        unsafe {
            let _ = SetForegroundWindow(w);
        }
    }
}

/// Forces the overlay's extended style whenever anyone (tao included) changes it:
/// tool window (no taskbar button, not in Alt+Tab), never activated (unless it's
/// the open launcher), never an app window.
unsafe extern "system" fn style_guard(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_STYLECHANGING && wparam.0 as i32 == GWL_STYLE.0 {
        // No title bar, system menu or minimize box (even invisible ones): "show
        // desktop" / "minimize all" (and mods like Win+D per monitor) would
        // minimize a window that has them.
        let ss = &mut *(lparam.0 as *mut STYLESTRUCT);
        // A popup: without a caption an "overlapped" window gets a title bar anyway.
        ss.styleNew = (ss.styleNew & !FRAME_BITS) | WS_POPUP.0;
    }
    if msg == WM_STYLECHANGING && wparam.0 as i32 == GWL_EXSTYLE.0 {
        let ss = &mut *(lparam.0 as *mut STYLESTRUCT);
        let noactivate = if activatable(hwnd) { 0 } else { WS_EX_NOACTIVATE.0 };
        ss.styleNew = (ss.styleNew | WS_EX_TOOLWINDOW.0 | noactivate) & !WS_EX_APPWINDOW.0;
        if noactivate == 0 {
            ss.styleNew &= !WS_EX_NOACTIVATE.0;
        }
    }
    if msg == windows::Win32::UI::WindowsAndMessaging::WM_NCACTIVATE {
        // Activated (the launcher): don't paint a title bar (lParam -1 = no frame repaint).
        return DefSubclassProc(hwnd, msg, wparam, LPARAM(-1));
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Installs the style guard. Must run on the window's thread, before it is shown.
pub fn guard_overlay_style(h: isize) {
    unsafe {
        let _ = SetWindowSubclass(hwnd(h), Some(style_guard), 1, 0);
        // No frame from the compositor either: themes and mods (accent-colored
        // title bars, translucency) would draw one over the bar when it's active.
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMNCRP_DISABLED, DWMWA_NCRENDERING_POLICY};
        let policy = DWMNCRP_DISABLED;
        let _ = DwmSetWindowAttribute(
            hwnd(h),
            DWMWA_NCRENDERING_POLICY,
            &policy as *const _ as *const _,
            std::mem::size_of_val(&policy) as u32,
        );
    }
    apply_overlay_style(h);
}

/// Window-frame style bits an overlay must not have (see `style_guard`).
use windows::Win32::UI::WindowsAndMessaging::{SWP_FRAMECHANGED, SWP_NOZORDER, WS_POPUP};

const FRAME_BITS: u32 = WS_CAPTION.0
    | windows::Win32::UI::WindowsAndMessaging::WS_SYSMENU.0
    | windows::Win32::UI::WindowsAndMessaging::WS_MINIMIZEBOX.0
    | windows::Win32::UI::WindowsAndMessaging::WS_MAXIMIZEBOX.0
    | WS_THICKFRAME.0;

/// Brings back an overlay something minimized anyway.
pub fn unminimize(h: isize) -> bool {
    unsafe {
        if IsIconic(hwnd(h)).as_bool() {
            let _ = windows::Win32::UI::WindowsAndMessaging::ShowWindowAsync(
                hwnd(h),
                windows::Win32::UI::WindowsAndMessaging::SW_SHOWNOACTIVATE,
            );
            return true;
        }
    }
    false
}

/// WebView2 flags for every StatusNotch window (they share one browser process, so
/// they must match). wry's defaults, plus: all pages are one site, so they share one
/// renderer process instead of one each (the bars are tiny; a renderer is ~70 MB).
pub const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --process-per-site";

/// No activation on click, no taskbar / alt-tab entry, no frame (unminimizable).
pub fn apply_overlay_style(h: isize) {
    unsafe {
        let style = GetWindowLongPtrW(hwnd(h), GWL_STYLE);
        let bare = (style & !(FRAME_BITS as isize)) | WS_POPUP.0 as isize;
        if bare != style {
            SetWindowLongPtrW(hwnd(h), GWL_STYLE, bare);
            // Have Windows recompute the (now absent) frame.
            let _ = SetWindowPos(
                hwnd(h),
                None,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            );
        }
        let ex = GetWindowLongPtrW(hwnd(h), GWL_EXSTYLE);
        let mut want = (ex | (WS_EX_NOACTIVATE.0 | WS_EX_TOOLWINDOW.0) as isize) & !(WS_EX_APPWINDOW.0 as isize);
        if activatable(hwnd(h)) {
            want &= !(WS_EX_NOACTIVATE.0 as isize);
        }
        if want != ex {
            SetWindowLongPtrW(hwnd(h), GWL_EXSTYLE, want);
        }
    }
}

/// Places the window at a physical rect and makes it topmost. Call on the window's
/// thread: moving onto a monitor with another DPI makes tao apply the system's
/// suggested size, so the second SetWindowPos restores the size we want.
pub fn place(h: isize, x: i32, y: i32, w: i32, ht: i32) {
    unsafe {
        for _ in 0..2 {
            let _ = SetWindowPos(hwnd(h), Some(HWND_TOPMOST), x, y, w, ht, SWP_NOACTIVATE);
        }
    }
}

pub fn allow_foreground() {
    unsafe {
        let _ = AllowSetForegroundWindow(ASFW_ANY);
    }
}

pub fn raise_topmost(h: isize) {
    unsafe {
        let _ = SetWindowPos(
            hwnd(h),
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_ASYNCWINDOWPOS,
        );
    }
}

/// Processes that host Windows shell UI rather than apps.
const SHELL_PROCESSES: &[&str] = &[
    "explorer.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchapp.exe",
    "textinputhost.exe",
    "lockapp.exe",
    "shellhost.exe",
];

/// Lower-case executable name of the process owning `hwnd` ("" if unknown).
pub fn window_process(hwnd: HWND) -> String {
    process_path(window_pid(hwnd))
        .and_then(|p| p.rsplit(['\\', '/']).next().map(str::to_lowercase))
        .unwrap_or_default()
}

/// If the foreground window is fullscreen (or a D3D exclusive / presentation app
/// is running), returns the rect of the monitor it occupies.
pub fn fullscreen_monitor(ours: &[isize]) -> Option<(RECT, String)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() || ours.contains(&(fg.0 as isize)) {
            return None;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(fg, &mut class) as usize;
        let class = String::from_utf16_lossy(&class[..n]);
        if matches!(
            class.as_str(),
            "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
        ) {
            return None;
        }
        // Windows' own shell surfaces (Alt+Tab switcher, Start, Search, flyouts,
        // notification center) cover the screen without being fullscreen apps.
        if SHELL_PROCESSES.contains(&window_process(fg).as_str()) {
            return None;
        }

        let mon = MonitorFromWindow(fg, MONITOR_DEFAULTTONEAREST);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if !GetMonitorInfoW(mon, &mut mi).as_bool() {
            return None;
        }
        let mr = mi.rcMonitor;

        // This asks Explorer; poll it gently (the rect check below is local).
        static LAST: std::sync::Mutex<Option<(std::time::Instant, bool)>> = std::sync::Mutex::new(None);
        let exclusive = {
            let mut last = LAST.lock().unwrap();
            match *last {
                Some((at, v)) if at.elapsed() < std::time::Duration::from_secs(2) => v,
                _ => {
                    let v = SHQueryUserNotificationState()
                        .is_ok_and(|s| s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE);
                    *last = Some((std::time::Instant::now(), v));
                    v
                }
            }
        };
        if exclusive {
            return Some((mr, format!("exclusive fullscreen / presentation ({class})")));
        }

        // A maximized normal window can cover the monitor when the taskbar auto-hides;
        // that is not fullscreen.
        if IsZoomed(fg).as_bool() {
            return None;
        }
        let style = GetWindowLongW(fg, GWL_STYLE) as u32;
        if style & WS_CAPTION.0 == WS_CAPTION.0 {
            return None;
        }
        let mut wr = RECT::default();
        GetWindowRect(fg, &mut wr).ok()?;
        let covers = wr.left <= mr.left && wr.top <= mr.top && wr.right >= mr.right && wr.bottom >= mr.bottom;
        covers.then(|| (mr, format!("{class} covers monitor ({},{},{},{})", wr.left, wr.top, wr.right, wr.bottom)))
    }
}

/// A window's class name.
pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// An application window someone could be working in: visible, not minimized or
/// cloaked, with a title bar or resizable frame and a title; not a tool / overlay
/// window, the shell (desktop, taskbar) or one of `ours`.
fn is_app_window(h: HWND, ours: &[isize]) -> bool {
    unsafe {
        if h.is_invalid() || ours.contains(&(h.0 as isize)) || !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return false;
        }
        let style = GetWindowLongW(h, GWL_STYLE) as u32;
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        if style & (WS_CAPTION.0 | WS_THICKFRAME.0) == 0
            || ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0) != 0
            || GetWindowTextLengthW(h) == 0
        {
            return false;
        }
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        cloaked == 0 && !matches!(class_name(h).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    }
}

/// The front-most application window (z-order), if any.
fn top_app_window(ours: &[isize]) -> Option<HWND> {
    unsafe extern "system" fn proc(h: HWND, data: LPARAM) -> BOOL {
        let (ours, found) = &mut *(data.0 as *mut (&[isize], Option<HWND>));
        if is_app_window(h, ours) {
            *found = Some(h);
            return BOOL(0); // stop: EnumWindows goes front to back
        }
        BOOL(1)
    }
    let mut state: (&[isize], Option<HWND>) = (ours, None);
    unsafe {
        let _ = EnumWindows(Some(proc), LPARAM(&mut state as *mut _ as isize));
    }
    state.1
}

/// Visible application windows (see `is_app_window`), as (class, rect) in physical px.
pub fn app_windows(ours: &[isize]) -> Vec<(String, RECT)> {
    unsafe extern "system" fn proc(h: HWND, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<HWND>);
        list.push(h);
        BOOL(1)
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(proc), LPARAM(&mut all as *mut _ as isize));
    }
    let mut out = Vec::new();
    for h in all {
        unsafe {
            if !is_app_window(h, ours) {
                continue;
            }
            let class = class_name(h);
            // Visible bounds: GetWindowRect includes invisible resize borders, which on a
            // maximized window reach ~8px into the neighbouring monitor.
            let mut r = RECT::default();
            let visible = DwmGetWindowAttribute(
                h,
                DWMWA_EXTENDED_FRAME_BOUNDS,
                &mut r as *mut _ as *mut _,
                std::mem::size_of::<RECT>() as u32,
            )
            .is_ok();
            if (visible || GetWindowRect(h, &mut r).is_ok()) && r.right > r.left && r.bottom > r.top {
                out.push((class, r));
            }
        }
    }
    out
}

/// The foreground window and the language of its keyboard layout (a LANGID).
pub fn foreground_layout() -> Option<(isize, u16)> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return None;
        }
        let tid = GetWindowThreadProcessId(fg, None);
        let hkl = GetKeyboardLayout(tid);
        Some((fg.0 as isize, (hkl.0 as usize & 0xffff) as u16))
    }
}

/// Short code and display name for a LANGID, e.g. ("ENG", "English") or ("LIT", "Lithuanian").
pub fn language_names(langid: u16) -> (String, String) {
    let get = |kind: u32| -> String {
        let mut buf = [0u16; 128];
        let n = unsafe { GetLocaleInfoW(langid as u32, kind, Some(&mut buf)) } as usize;
        String::from_utf16_lossy(&buf[..n.saturating_sub(1)])
    };
    let code = get(LOCALE_SISO639LANGNAME2).to_uppercase();
    let name = get(LOCALE_SENGLISHLANGUAGENAME);
    (if code.is_empty() { format!("{langid:04X}") } else { code }, name)
}

/// Presses the user's layout-switch hotkey (Settings > Typing > Advanced keyboard
/// settings > Input language hot keys): Alt+Shift by default, Ctrl+Shift, or
/// Win+Space when the classic hotkey is disabled. Works whatever has focus.
fn press_layout_hotkey() {
    let mut buf = [0u16; 8];
    let mut len = (buf.len() * 2) as u32;
    let setting = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!(r"Keyboard Layout\Toggle"),
            w!("Language Hotkey"),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    let value = if setting.is_ok() { String::from_utf16_lossy(&buf[..(len as usize / 2).saturating_sub(1)]) } else { "1".into() };
    let (modifier, key) = match value.as_str() {
        "2" => (VK_LCONTROL, VK_LSHIFT),
        "3" | "4" => (VK_LWIN, VK_SPACE),
        _ => (VK_LMENU, VK_LSHIFT),
    };
    send_keys(&[KeyEvent::key(modifier, false), KeyEvent::key(key, false), KeyEvent::key(key, true), KeyEvent::key(modifier, true)], 0);
}

/// One key event to send. `scan` 0 lets Windows derive it from `vk`.
#[derive(Clone, Copy)]
pub struct KeyEvent {
    pub vk: u16,
    pub scan: u16,
    pub up: bool,
    pub extended: bool,
}

impl KeyEvent {
    /// A press (`up` false) or release of `vk`; the Windows keys are extended keys.
    pub fn key(vk: VIRTUAL_KEY, up: bool) -> Self {
        Self { vk: vk.0, scan: 0, up, extended: vk == VK_LWIN || vk == VK_RWIN }
    }
}

/// Sends key events, tagged with `mark` (a keyboard hook can recognise our own
/// keys by it; keep its low bits clear, PowerToys tags its own keys with those).
pub fn send_keys(keys: &[KeyEvent], mark: usize) {
    let inputs: Vec<INPUT> = keys
        .iter()
        .map(|k| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(k.vk),
                    wScan: k.scan,
                    dwFlags: (if k.up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) })
                        | (if k.extended { KEYEVENTF_EXTENDEDKEY } else { KEYBD_EVENT_FLAGS(0) }),
                    dwExtraInfo: mark,
                    ..Default::default()
                },
            },
        })
        .collect();
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Switches the foreground window to the next installed keyboard layout (like Alt+Shift).
pub fn next_keyboard_layout() -> Result<(), String> {
    unsafe {
        let fg = GetForegroundWindow();
        if fg.is_invalid() {
            return Err("no foreground window".into());
        }
        let tid = GetWindowThreadProcessId(fg, None);
        let current = GetKeyboardLayout(tid);
        let count = GetKeyboardLayoutList(None).max(0) as usize;
        let mut list = vec![HKL::default(); count];
        let n = GetKeyboardLayoutList(Some(&mut list)).max(0) as usize;
        list.truncate(n);
        if list.len() < 2 {
            return Err("only one keyboard layout installed".into());
        }
        let i = list.iter().position(|h| *h == current).unwrap_or(0);
        let next = list[(i + 1) % list.len()];

        // Ask the focused control (falling back to the window) to switch...
        let mut gui = GUITHREADINFO { cbSize: std::mem::size_of::<GUITHREADINFO>() as u32, ..Default::default() };
        let target = if GetGUIThreadInfo(tid, &mut gui).is_ok() && !gui.hwndFocus.is_invalid() { gui.hwndFocus } else { fg };
        let _ = PostMessageW(Some(target), WM_INPUTLANGCHANGEREQUEST, WPARAM(0), LPARAM(next.0 as isize));

        // ...and if nothing there handled it (no text field, some app types), use the hotkey.
        std::thread::sleep(std::time::Duration::from_millis(150));
        if GetKeyboardLayout(tid) == current {
            press_layout_hotkey();
        }
        Ok(())
    }
}

/// Windows 11 rounded corners for a frameless window.
pub fn round_corners(h: isize) {
    use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND};
    unsafe {
        let pref = DWMWCP_ROUND;
        let _ = DwmSetWindowAttribute(
            hwnd(h),
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const _,
            std::mem::size_of_val(&pref) as u32,
        );
    }
}

/// The user's short time format (Settings > Time & language > Region) converted
/// to the clock widget's strftime subset: "HH:mm" -> "%H:%M", "h:mm tt" -> "%I:%M %p".
pub fn short_time_pattern() -> String {
    use windows::Win32::Globalization::{GetLocaleInfoEx, LOCALE_SSHORTTIME};
    let mut buf = [0u16; 80];
    let n = unsafe { GetLocaleInfoEx(windows::core::PCWSTR::null(), LOCALE_SSHORTTIME, Some(&mut buf)) } as usize;
    let pattern = String::from_utf16_lossy(&buf[..n.saturating_sub(1)]);
    if pattern.is_empty() {
        return "%H:%M".into();
    }
    let mut out = String::new();
    let chars: Vec<char> = pattern.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let mut run = 1;
        while i + run < chars.len() && chars[i + run] == c {
            run += 1;
        }
        match c {
            'H' => out.push_str("%H"),
            'h' => out.push_str("%I"),
            'm' => out.push_str("%M"),
            's' => out.push_str("%S"),
            't' => out.push_str("%p"),
            '\'' => {}
            _ => (0..run).for_each(|_| out.push(c)),
        }
        i += run;
    }
    out
}

/// A process's program name (its exe file name without ".exe").
pub fn process_name(pid: u32) -> Option<String> {
    let path = process_path(pid)?;
    std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned())
}

/// The process that owns a window.
pub fn window_pid(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

/// A window's title (any length).
pub fn window_title(h: HWND) -> String {
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowTextLengthW, GetWindowTextW};
    unsafe {
        let len = GetWindowTextLengthW(h);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; len as usize + 1];
        let n = GetWindowTextW(h, &mut buf) as usize;
        String::from_utf16_lossy(&buf[..n])
    }
}

/// A NUL-terminated UTF-16 copy of `s`, for Win32 calls.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// A process's program file (full path).
pub fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(p);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// A bitmap's pixels as 32-bit BGRA rows, top-down (as GetDIBits gives them);
/// None for empty or implausibly large ones. Callers fix up alpha their own way
/// (shell images are premultiplied, old icons have a separate mask).
pub fn bitmap_bgra(bmp: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, DeleteDC, GetDIBits, GetObjectW, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };
    unsafe {
        let mut bm = BITMAP::default();
        GetObjectW(HGDIOBJ(bmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.unsigned_abs());
        if w == 0 || h == 0 || w > 512 || h > 512 {
            return None;
        }
        let mut bi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let dc = CreateCompatibleDC(None);
        let lines = GetDIBits(dc, bmp, 0, h, Some(px.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        (lines != 0).then_some((w, h, px))
    }
}
