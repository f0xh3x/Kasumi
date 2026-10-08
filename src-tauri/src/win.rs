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
    KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_SPACE,
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

fn wide_to_string(buf: &[u16]) -> String {
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
    unsafe {
        let w = hwnd(h);
        if !SetForegroundWindow(w).as_bool() || GetForegroundWindow() != w {
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

/// Gives the foreground back (closing the launcher without launching anything).
pub fn restore_foreground(h: isize) {
    if h != 0 {
        unsafe {
            let _ = SetForegroundWindow(hwnd(h));
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
    if msg == WM_STYLECHANGING && wparam.0 as i32 == GWL_EXSTYLE.0 {
        let ss = &mut *(lparam.0 as *mut STYLESTRUCT);
        let noactivate = if activatable(hwnd) { 0 } else { WS_EX_NOACTIVATE.0 };
        ss.styleNew = (ss.styleNew | WS_EX_TOOLWINDOW.0 | noactivate) & !WS_EX_APPWINDOW.0;
        if noactivate == 0 {
            ss.styleNew &= !WS_EX_NOACTIVATE.0;
        }
    }
    DefSubclassProc(hwnd, msg, wparam, lparam)
}

/// Installs the style guard. Must run on the window's thread, before it is shown.
pub fn guard_overlay_style(h: isize) {
    unsafe {
        let _ = SetWindowSubclass(hwnd(h), Some(style_guard), 1, 0);
    }
    apply_overlay_style(h);
}

/// No activation on click, no taskbar / alt-tab entry.
pub fn apply_overlay_style(h: isize) {
    unsafe {
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
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = [0u16; 512];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or_default().to_lowercase()
    }
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

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// Visible application windows (title bar or resizable frame; not minimized,
/// cloaked, tool/overlay windows or the shell), as (class, rect) in physical px.
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
            if ours.contains(&(h.0 as isize)) || !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                continue;
            }
            let style = GetWindowLongW(h, GWL_STYLE) as u32;
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            if style & (WS_CAPTION.0 | WS_THICKFRAME.0) == 0
                || ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0) != 0
                || GetWindowTextLengthW(h) == 0
            {
                continue;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
            if cloaked != 0 {
                continue;
            }
            let class = class_name(h);
            if matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
                continue;
            }
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
    let input = |vk: VIRTUAL_KEY, up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT { wVk: vk, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, ..Default::default() },
        },
    };
    let seq = [input(modifier, false), input(key, false), input(key, true), input(modifier, true)];
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
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
