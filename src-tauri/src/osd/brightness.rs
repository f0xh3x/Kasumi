//! Brightness: built-in panel changes (laptop Fn keys) arrive as WMI
//! `WmiMonitorBrightnessEvent`s. External monitors are left alone.
//!
//! The keys go to Windows directly (nothing to take over, unlike volume), so its
//! own brightness flyout still shows: a small `XamlExplorerHostIslandWindow` of
//! Explorer's at the bottom of the screen. With `[osd] hide_windows_brightness`,
//! one that shows within a moment of a brightness change is hidden again (it can
//! show just before or just after the WMI event).

use crate::providers::SharedConfig;
use serde::Deserialize;
use std::sync::atomic::{AtomicIsize, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetClassNameW, GetMessageW, GetShellWindow, GetWindowRect, GetWindowTextLengthW, GetWindowThreadProcessId,
    IsWindowVisible, ShowWindowAsync, EVENT_OBJECT_SHOW, MSG, OBJID_WINDOW, SW_HIDE, WINEVENT_OUTOFCONTEXT,
};
use wmi::WMIConnection;

#[derive(Deserialize)]
#[serde(rename = "WmiMonitorBrightnessEvent")]
#[allow(non_snake_case)]
struct BrightnessEvent {
    Brightness: u8,
}

/// How close a brightness change and the flyout showing must be to belong together.
const PAIR_MS: u64 = 1500;

/// Milliseconds since start (0: never) of the last change Kasumi showed with the
/// Windows flyout to hide, and of the last flyout seen showing (with its window).
static CHANGED: AtomicU64 = AtomicU64::new(0);
static FLYOUT_AT: AtomicU64 = AtomicU64::new(0);
static FLYOUT: AtomicIsize = AtomicIsize::new(0);

fn now_ms() -> u64 {
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

fn recent(at: u64) -> bool {
    at != 0 && now_ms().saturating_sub(at) < PAIR_MS
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new().name("kasumi-brightness".into()).spawn(move || run(app)).expect("spawn brightness");
    std::thread::Builder::new().name("kasumi-brightness-flyout".into()).spawn(watch_flyout).expect("spawn brightness flyout");
}

fn listen(app: &AppHandle) -> wmi::WMIResult<()> {
    let con = WMIConnection::with_namespace_path("ROOT\\WMI")?;
    for event in con.notification::<BrightnessEvent>()? {
        let event = event?;
        let (show, hide) = {
            let cfg = app.state::<SharedConfig>();
            let cfg = cfg.read().unwrap();
            (cfg.osd.brightness, cfg.osd.enabled && cfg.osd.brightness && cfg.osd.hide_windows_brightness)
        };
        if show {
            super::show(app, "brightness", event.Brightness as f32 / 100.0, false);
        }
        if hide {
            CHANGED.store(now_ms(), Ordering::Relaxed);
            // The flyout may already be up (it showed before the event arrived).
            if recent(FLYOUT_AT.load(Ordering::Relaxed)) {
                hide_flyout(HWND(FLYOUT.load(Ordering::Relaxed) as _));
            }
        }
    }
    Ok(())
}

fn run(app: AppHandle) {
    loop {
        if let Err(e) = listen(&app) {
            eprintln!("[kasumi] brightness events unavailable: {e}");
        }
        // The subscription ended or failed (e.g. no WMI brightness provider); retry later.
        std::thread::sleep(Duration::from_secs(60));
    }
}

fn hide_flyout(h: HWND) {
    unsafe {
        if !h.is_invalid() && IsWindowVisible(h).as_bool() {
            let _ = ShowWindowAsync(h, SW_HIDE);
        }
    }
}

/// Explorer's small, untitled XAML island window: the level flyout. (Alt+Tab's is
/// the same class, but full-screen and titled.)
fn is_flyout(h: HWND) -> bool {
    unsafe {
        let mut buf = [0u16; 64];
        let n = GetClassNameW(h, &mut buf) as usize;
        if String::from_utf16_lossy(&buf[..n]) != "XamlExplorerHostIslandWindow" || GetWindowTextLengthW(h) != 0 {
            return false;
        }
        let (mut pid, mut shell) = (0u32, 0u32);
        GetWindowThreadProcessId(h, Some(&mut pid));
        GetWindowThreadProcessId(GetShellWindow(), Some(&mut shell));
        let mut r = RECT::default();
        pid == shell && GetWindowRect(h, &mut r).is_ok() && r.right - r.left < 600 && r.bottom - r.top < 240
    }
}

unsafe extern "system" fn on_show(_: HWINEVENTHOOK, _: u32, h: HWND, object: i32, child: i32, _: u32, _: u32) {
    if object != OBJID_WINDOW.0 || child != 0 || !is_flyout(h) {
        return;
    }
    if recent(CHANGED.load(Ordering::Relaxed)) {
        hide_flyout(h);
    } else {
        // The brightness event may still be on its way.
        FLYOUT.store(h.0 as isize, Ordering::Relaxed);
        FLYOUT_AT.store(now_ms(), Ordering::Relaxed);
    }
}

fn watch_flyout() {
    unsafe {
        SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, None, Some(on_show), 0, 0, WINEVENT_OUTOFCONTEXT);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}
