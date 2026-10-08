//! Hiding the Windows taskbar while StatusNotch provides its own (`[shell]
//! hide_taskbar`). Explorer's taskbars are set to auto-hide, so maximized windows
//! get the whole screen, and their windows are hidden outright, so they never
//! slide back in at the screen edge. A WinEvent hook hides them again the moment
//! Explorer shows them (also after an Explorer restart).
//!
//! Nothing is removed or replaced: the Win key and Start menu keep working, and
//! the taskbar comes back when the option is switched off, when StatusNotch quits
//! or panics, and — through a marker file holding the original auto-hide state —
//! on the next start after a crash.
//!
//! All calls into Explorer happen on this module's thread and never wait on a
//! window message, so a hung Explorer can't stall the rest of the app.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Shell::{SHAppBarMessage, ABM_GETSTATE, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, FindWindowW, GetClassNameW, GetMessageW, GetWindowThreadProcessId, IsWindowVisible,
    SetTimer, ShowWindowAsync, EVENT_OBJECT_SHOW, MSG, OBJID_WINDOW, SW_HIDE, SW_SHOWNA, WINEVENT_OUTOFCONTEXT, WM_TIMER,
};

/// What the config asks for.
static WANT: AtomicBool = AtomicBool::new(false);
/// Whether we are hiding the taskbar right now.
static ACTIVE: AtomicBool = AtomicBool::new(false);

fn marker() -> PathBuf {
    crate::config::dir().join("taskbar-hidden")
}

/// Called whenever the config (re)loads.
pub fn want(hide: bool) {
    WANT.store(hide, Ordering::Relaxed);
}

pub fn start() {
    std::thread::Builder::new().name("statusnotch-taskbar".into()).spawn(run).expect("spawn taskbar thread");
}

fn is_taskbar_class(h: HWND) -> bool {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    matches!(String::from_utf16_lossy(&buf[..n]).as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

/// Explorer's taskbar windows (not our own hidden tray host, which shares the class).
fn taskbars() -> Vec<HWND> {
    unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
        (*(data.0 as *mut Vec<HWND>)).push(h);
        BOOL(1)
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));
    }
    let own = unsafe { GetCurrentProcessId() };
    all.into_iter()
        .filter(|&h| {
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
            pid != own && is_taskbar_class(h)
        })
        .collect()
}

fn appbar() -> APPBARDATA {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    d.hWnd = unsafe { FindWindowW(windows::core::w!("Shell_TrayWnd"), None) }.unwrap_or_default();
    d.rc = RECT::default();
    d
}

fn autohide() -> bool {
    let mut d = appbar();
    unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) as u32 & ABS_AUTOHIDE != 0 }
}

fn set_autohide(on: bool) {
    let mut d = appbar();
    let state = unsafe { SHAppBarMessage(ABM_GETSTATE, &mut d) } as u32;
    let next = if on { state | ABS_AUTOHIDE } else { state & !ABS_AUTOHIDE };
    if next != state {
        d.lParam = LPARAM(next as isize);
        unsafe { SHAppBarMessage(ABM_SETSTATE, &mut d) };
    }
}

fn hide_all() {
    for h in taskbars() {
        if unsafe { IsWindowVisible(h).as_bool() } {
            let _ = unsafe { ShowWindowAsync(h, SW_HIDE) };
        }
    }
}

fn enable() {
    // Remember the user's own setting once, so a crash can't lose it.
    if !marker().exists() {
        let _ = std::fs::write(marker(), if autohide() { "1" } else { "0" });
    }
    set_autohide(true);
    hide_all();
    ACTIVE.store(true, Ordering::Relaxed);
    eprintln!("[statusnotch] taskbar hidden");
}

/// Shows the taskbar again and puts auto-hide back the way it was.
pub fn restore() {
    let Ok(original) = std::fs::read_to_string(marker()) else {
        ACTIVE.store(false, Ordering::Relaxed);
        return;
    };
    ACTIVE.store(false, Ordering::Relaxed);
    for h in taskbars() {
        let _ = unsafe { ShowWindowAsync(h, SW_SHOWNA) };
    }
    set_autohide(original.trim() == "1");
    let _ = std::fs::remove_file(marker());
    eprintln!("[statusnotch] taskbar restored");
}

unsafe extern "system" fn on_show(_: HWINEVENTHOOK, _: u32, h: HWND, object: i32, child: i32, _: u32, _: u32) {
    if ACTIVE.load(Ordering::Relaxed) && object == OBJID_WINDOW.0 && child == 0 && is_taskbar_class(h) {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid != GetCurrentProcessId() {
            let _ = ShowWindowAsync(h, SW_HIDE);
        }
    }
}

fn run() {
    unsafe {
        SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, None, Some(on_show), 0, 0, WINEVENT_OUTOFCONTEXT);
        // Follow the config, and catch anything the hook missed (new monitors,
        // an Explorer restart), four times a second.
        SetTimer(None, 0, 250, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_TIMER {
                match (WANT.load(Ordering::Relaxed), ACTIVE.load(Ordering::Relaxed)) {
                    (true, false) => enable(),
                    (true, true) => hide_all(),
                    // Off, or left hidden by a crashed run: bring it back.
                    (false, _) if marker().exists() => restore(),
                    _ => {}
                }
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
}
