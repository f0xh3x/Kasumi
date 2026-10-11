//! Hiding the Windows taskbar while Kasumi provides its own (`[shell]
//! hide_taskbar`). Explorer's taskbars are set to auto-hide, so maximized windows
//! get the whole screen, and their windows are hidden outright, so they never
//! slide back in at the screen edge. A WinEvent hook hides them again the moment
//! Explorer shows them (also after an Explorer restart).
//!
//! Nothing is removed or replaced: the Win key and Start menu keep working, and
//! the taskbar comes back when the option is switched off, when Kasumi quits
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
use windows::Win32::UI::Shell::{
    SHAppBarMessage, ABM_GETSTATE, ABM_NEW, ABM_QUERYPOS, ABM_REMOVE, ABM_SETPOS, ABM_SETSTATE, ABS_AUTOHIDE, APPBARDATA,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, FindWindowW, GetClassNameW, GetMessageW, GetWindowThreadProcessId, IsWindowVisible,
    SetTimer, ShowWindowAsync, EVENT_OBJECT_SHOW, MSG, OBJID_WINDOW, SW_HIDE, SW_SHOWNA, WINEVENT_OUTOFCONTEXT, WM_TIMER,
};

/// What the config asks for.
static WANT: AtomicBool = AtomicBool::new(false);

/// A strip of a monitor a surface keeps for itself (physical px), like the taskbar.
#[derive(Debug, Clone, PartialEq)]
pub struct Reservation {
    pub hwnd: isize,
    pub bottom_edge: bool,
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    /// The monitor's rect (left, top, right, bottom).
    pub monitor: (i32, i32, i32, i32),
}

/// Reservations the engine wants, and the ones registered with the shell.
static RESERVE_WANT: std::sync::Mutex<Vec<Reservation>> = std::sync::Mutex::new(Vec::new());
static RESERVED: std::sync::Mutex<Vec<Reservation>> = std::sync::Mutex::new(Vec::new());

/// Sets the strips Kasumi surfaces reserve (applied on this module's thread).
pub fn reserve(list: Vec<Reservation>) {
    *RESERVE_WANT.lock().unwrap() = list;
}

fn appbar_for(r: &Reservation) -> APPBARDATA {
    let mut d = APPBARDATA { cbSize: std::mem::size_of::<APPBARDATA>() as u32, ..Default::default() };
    d.hWnd = HWND(r.hwnd as *mut _);
    d.uCallbackMessage = 0x8000 + 0x51; // WM_APP + n; we don't act on the notifications
    d.uEdge = if r.bottom_edge { 3 } else { 1 }; // ABE_BOTTOM / ABE_TOP
    d.rc = RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom };
    d
}

/// Reservations Explorer refused (an empty AppBar rect): kept by setting the
/// monitor's work area ourselves, and the work areas they replaced.
static FALLBACK: std::sync::Mutex<Vec<Reservation>> = std::sync::Mutex::new(Vec::new());
/// When the AppBar reservations were last (re)registered, to check they took effect.
static REGISTERED_AT: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
static WORK_ORIGINAL: std::sync::Mutex<Vec<(RECT, RECT)>> = std::sync::Mutex::new(Vec::new());

/// Per monitor: what Windows made of our last work-area request (it may adjust
/// it), when something reset it lately, and until when we leave it alone.
struct WorkState {
    monitor: RECT,
    got: RECT,
    resets: Vec<std::time::Instant>,
    paused_until: Option<std::time::Instant>,
    /// How long the next pause is: doubles while it keeps happening (up to 10 min).
    pause: std::time::Duration,
}
static WORK_STATE: std::sync::Mutex<Vec<WorkState>> = std::sync::Mutex::new(Vec::new());

fn work_marker() -> PathBuf {
    crate::config::dir().join("workarea-reserved")
}

fn save_work_marker(list: &[(RECT, RECT)]) {
    if list.is_empty() {
        let _ = std::fs::remove_file(work_marker());
        return;
    }
    let text: Vec<String> = list
        .iter()
        .map(|(m, w)| format!("{} {} {} {} {} {} {} {}", m.left, m.top, m.right, m.bottom, w.left, w.top, w.right, w.bottom))
        .collect();
    let _ = std::fs::write(work_marker(), text.join("\n"));
}

fn work_area(monitor: &RECT) -> Option<RECT> {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromRect, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    unsafe {
        let h = MonitorFromRect(monitor, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        GetMonitorInfoW(h, &mut info).as_bool().then_some(info.rcWork)
    }
}

/// Sets a monitor's work area without broadcasting the change: the broadcast
/// makes Explorer re-show its (hidden) taskbar and reset the work area. Windows
/// maximized afterwards respect it; ones already maximized adjust when re-maximized.
fn set_work_area(r: &RECT) {
    use windows::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_SETWORKAREA, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS};
    let mut r = *r;
    unsafe {
        let _ = SystemParametersInfoW(SPI_SETWORKAREA, 0, Some(&mut r as *mut _ as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
    }
}

/// Tells maximized windows on a monitor that its work area changed, so they
/// re-fit (Explorer itself isn't told: that would re-show its taskbar).
fn refit_maximized(monitor: &RECT) {
    use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows::Win32::UI::WindowsAndMessaging::{IsZoomed, PostMessageW, SPI_SETWORKAREA, WM_SETTINGCHANGE};
    unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
        (*(data.0 as *mut Vec<HWND>)).push(h);
        BOOL(1)
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));
        for h in all {
            if !IsWindowVisible(h).as_bool() || !IsZoomed(h).as_bool() || is_taskbar_class(h) {
                continue;
            }
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
            if GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut info).as_bool() && info.rcMonitor == *monitor {
                let _ = PostMessageW(Some(h), WM_SETTINGCHANGE, windows::Win32::Foundation::WPARAM(SPI_SETWORKAREA.0 as usize), LPARAM(0));
            }
        }
    }
}

/// Keeps the work area of monitors with refused reservations shrunk by our strips
/// (re-applied if something resets it), and gives the original back otherwise.
fn apply_work_areas() {
    let fallback = FALLBACK.lock().unwrap().clone();
    let mut original = WORK_ORIGINAL.lock().unwrap();
    let mut monitors: Vec<(i32, i32, i32, i32)> = fallback.iter().map(|r| r.monitor).collect();
    monitors.dedup();
    for &(l, t, r, b) in &monitors {
        let mon = RECT { left: l, top: t, right: r, bottom: b };
        let orig = match original.iter().find(|(m, _)| *m == mon) {
            Some((_, w)) => *w,
            None => {
                let Some(w) = work_area(&mon) else { continue };
                original.push((mon, w));
                save_work_marker(&original);
                w
            }
        };
        let strip = |bottom: bool| {
            fallback.iter().filter(|x| x.monitor == (l, t, r, b) && x.bottom_edge == bottom).map(|x| x.bottom - x.top).max().unwrap_or(0)
        };
        let want = RECT { left: orig.left, right: orig.right, top: orig.top.max(t + strip(false)), bottom: orig.bottom.min(b - strip(true)) };
        let now = work_area(&mon);
        let mut states = WORK_STATE.lock().unwrap();
        let st = match states.iter().position(|s| s.monitor == mon) {
            Some(i) => &mut states[i],
            None => {
                states.push(WorkState {
                    monitor: mon,
                    got: RECT::default(),
                    resets: Vec::new(),
                    paused_until: None,
                    pause: std::time::Duration::from_secs(60),
                });
                states.last_mut().unwrap()
            }
        };
        // Done: as asked, or as Windows adjusted our request.
        if now == Some(want) || (now == Some(st.got) && st.got != RECT::default()) {
            continue;
        }
        let at = std::time::Instant::now();
        if st.paused_until.is_some_and(|until| at < until) {
            continue;
        }
        // Something keeps resetting it (Explorer, a taskbar mod): every change makes
        // Explorer re-arrange windows, so don't fight it in a loop.
        if st.got != RECT::default() {
            st.resets.retain(|x| at.duration_since(*x) < std::time::Duration::from_secs(60));
            st.resets.push(at);
            if st.resets.len() > 5 {
                eprintln!(
                    "[kasumi] work area of ({l},{t})-({r},{b}) keeps being reset (Explorer / a taskbar mod); leaving it for {} s",
                    st.pause.as_secs()
                );
                st.paused_until = Some(at + st.pause);
                st.pause = (st.pause * 2).min(std::time::Duration::from_secs(600));
                st.resets.clear();
                continue;
            }
        }
        set_work_area(&want);
        st.got = work_area(&mon).unwrap_or(want);
        refit_maximized(&mon);
        if st.got == want {
            eprintln!("[kasumi] work area of ({l},{t})-({r},{b}) set to keep reserved strips");
        } else {
            let g = st.got;
            eprintln!(
                "[kasumi] work area of ({l},{t})-({r},{b}) set to keep reserved strips (Windows made it ({},{})-({},{}))",
                g.left, g.top, g.right, g.bottom
            );
        }
    }
    WORK_STATE.lock().unwrap().retain(|s| monitors.contains(&(s.monitor.left, s.monitor.top, s.monitor.right, s.monitor.bottom)));
    let before = original.len();
    original.retain(|(m, w)| {
        let keep = monitors.contains(&(m.left, m.top, m.right, m.bottom));
        if !keep {
            set_work_area(w);
            refit_maximized(m);
        }
        keep
    });
    if original.len() != before {
        save_work_marker(&original);
    }
}

/// After a crash: put back work areas a previous run changed.
fn restore_work_marker() {
    let Ok(text) = std::fs::read_to_string(work_marker()) else { return };
    for line in text.lines() {
        let v: Vec<i32> = line.split_whitespace().filter_map(|x| x.parse().ok()).collect();
        if v.len() == 8 {
            set_work_area(&RECT { left: v[4], top: v[5], right: v[6], bottom: v[7] });
        }
    }
    let _ = std::fs::remove_file(work_marker());
}

/// Registers / moves / drops AppBar reservations to match what's wanted.
fn sync_reservations() {
    sync_appbars();
    check_reservations();
    apply_work_areas();
}

fn sync_appbars() {
    let want = RESERVE_WANT.lock().unwrap().clone();
    let mut have = RESERVED.lock().unwrap();
    if *have == want {
        return;
    }
    let mut refused = Vec::new();
    for r in have.iter().filter(|r| !want.iter().any(|w| w.hwnd == r.hwnd)) {
        let mut d = appbar_for(r);
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut d) };
    }
    for r in &want {
        let mut d = appbar_for(r);
        unsafe {
            if !have.iter().any(|h| h.hwnd == r.hwnd) {
                SHAppBarMessage(ABM_NEW, &mut d);
            }
            SHAppBarMessage(ABM_QUERYPOS, &mut d);
            if d.rc.right <= d.rc.left || d.rc.bottom <= d.rc.top {
                // Explorer won't place it (seen with heavily modded taskbars).
                refused.push(r.clone());
                continue;
            }
            // Keep our own height whatever the query moved.
            if r.bottom_edge {
                d.rc.top = d.rc.bottom - (r.bottom - r.top);
            } else {
                d.rc.bottom = d.rc.top + (r.bottom - r.top);
            }
            SHAppBarMessage(ABM_SETPOS, &mut d);
        }
    }
    eprintln!("[kasumi] reserved screen space: {} strip(s), {} through the work area", want.len(), refused.len());
    *FALLBACK.lock().unwrap() = refused;
    *REGISTERED_AT.lock().unwrap() = Some(std::time::Instant::now());
    *have = want;
}

/// Explorer can accept a reservation without honouring it (seen with modded
/// taskbars): a second after registering, any strip the work area still covers
/// is kept through the work area instead.
fn check_reservations() {
    let due = REGISTERED_AT.lock().unwrap().is_some_and(|t| t.elapsed() >= std::time::Duration::from_secs(1));
    if !due {
        return;
    }
    *REGISTERED_AT.lock().unwrap() = None;
    let have = RESERVED.lock().unwrap().clone();
    let mut fallback = FALLBACK.lock().unwrap();
    for r in have {
        if fallback.iter().any(|f| f.hwnd == r.hwnd) {
            continue;
        }
        let (l, t, rt, b) = r.monitor;
        // Our own work-area change for this monitor doesn't count as Explorer honouring it.
        if WORK_ORIGINAL.lock().unwrap().iter().any(|(m, _)| (m.left, m.top, m.right, m.bottom) == r.monitor) {
            continue;
        }
        let Some(work) = work_area(&RECT { left: l, top: t, right: rt, bottom: b }) else { continue };
        let kept = if r.bottom_edge { work.bottom <= r.top } else { work.top >= r.bottom };
        if !kept {
            eprintln!("[kasumi] reservation on ({l},{t})-({rt},{b}) not honoured by Explorer; using the work area");
            fallback.push(r);
        }
    }
}

/// Gives all reserved space back (quit / panic).
fn release_reservations() {
    let mut have = RESERVED.lock().unwrap();
    for r in have.iter() {
        let mut d = appbar_for(r);
        unsafe { SHAppBarMessage(ABM_REMOVE, &mut d) };
    }
    have.clear();
    FALLBACK.lock().unwrap().clear();
    let mut original = WORK_ORIGINAL.lock().unwrap();
    for (m, w) in original.iter() {
        set_work_area(w);
        refit_maximized(m);
    }
    original.clear();
    let _ = std::fs::remove_file(work_marker());
}
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
    std::thread::Builder::new().name("kasumi-taskbar".into()).spawn(run).expect("spawn taskbar thread");
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
    eprintln!("[kasumi] taskbar hidden");
}

/// Shows the taskbar again and puts auto-hide back the way it was; frees reserved space.
pub fn restore() {
    release_reservations();
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
    eprintln!("[kasumi] taskbar restored");
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
    restore_work_marker();
    unsafe {
        SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, None, Some(on_show), 0, 0, WINEVENT_OUTOFCONTEXT);
        // Follow the config, and catch anything the hook missed (new monitors,
        // an Explorer restart), four times a second.
        SetTimer(None, 0, 250, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_TIMER {
                sync_reservations();
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
