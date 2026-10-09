//! System tray hosting.
//!
//! Apps add/update/remove notification-area icons with `Shell_NotifyIcon`, which
//! sends `WM_COPYDATA` to the first top-level window of class `Shell_TrayWnd`.
//! We register our own hidden `Shell_TrayWnd`, keep it above Explorer's in
//! z-order so apps find ours first, record the icons, and forward every message
//! to Explorer unchanged so the regular taskbar tray keeps working. Clicks in
//! our tray widget are forwarded to the owning app's callback message.

use super::{publish, widget_enabled, widget_opts};
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, HGDIOBJ,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, CreateWindowExW, DefWindowProcW, DispatchMessageW, FindWindowExW, FindWindowW, GetIconInfo, GetMessageW, KillTimer, SetTimer,
    GetWindowRect, GetWindowThreadProcessId, IsWindow, PostMessageW, RegisterClassW,
    RegisterWindowMessageW, SendMessageTimeoutW, SendNotifyMessageW, SetWindowPos, TranslateMessage, HICON,
    HWND_BROADCAST, HWND_TOPMOST, ICONINFO, MSG, SMTO_ABORTIFHUNG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, WM_TIMER, WM_CONTEXTMENU,
    WM_COPYDATA, WM_USER, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

const SH_TRAY_DATA: usize = 1;
const NIM_ADD: u32 = 0;
const NIM_MODIFY: u32 = 1;
const NIM_DELETE: u32 = 2;
const NIM_SETVERSION: u32 = 4;
const NIF_MESSAGE: u32 = 0x1;
const NIF_ICON: u32 = 0x2;
const NIF_TIP: u32 = 0x4;
const NIF_STATE: u32 = 0x8;
const NIF_GUID: u32 = 0x20;
const NIS_HIDDEN: u32 = 0x1;
const NIN_SELECT: u32 = 0x400;

/// One notification-area icon, as reported by its app.
#[derive(Clone, Default)]
struct TrayIcon {
    hwnd: u32,
    uid: u32,
    guid: Option<String>,
    callback: u32,
    version: u32,
    tip: String,
    hidden: bool,
    /// (width, height, RGBA) of the current icon.
    image: Option<(u32, u32, Vec<u8>)>,
    exe: String,
    /// Insertion order, so the widget lists icons stably.
    order: u64,
}

#[derive(Default)]
struct Tray {
    icons: BTreeMap<String, TrayIcon>,
    next_order: u64,
}

static TRAY: OnceLock<Mutex<Tray>> = OnceLock::new();
static APP: OnceLock<AppHandle> = OnceLock::new();
static OUR_HWND: OnceLock<isize> = OnceLock::new();

fn tray() -> &'static Mutex<Tray> {
    TRAY.get_or_init(Default::default)
}

// ---------------------------------------------------------------------------
// Parsing: Shell_NotifyIcon sends a SHELLTRAYDATA with a 32-bit-layout
// NOTIFYICONDATAW (handles as u32) regardless of the sender's bitness.

fn u32_at(b: &[u8], off: usize) -> u32 {
    b.get(off..off + 4).map_or(0, |s| u32::from_le_bytes(s.try_into().unwrap()))
}

fn wstr_at(b: &[u8], off: usize, chars: usize) -> String {
    let Some(s) = b.get(off..off + chars * 2) else { return String::new() };
    let units: Vec<u16> = s.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    String::from_utf16_lossy(&units)
}

fn guid_at(b: &[u8], off: usize) -> Option<String> {
    let s = b.get(off..off + 16)?;
    Some(s.iter().map(|x| format!("{x:02x}")).collect())
}

// NOTIFYICONDATAW32 field offsets, relative to the start of the struct.
const N_HWND: usize = 4;
const N_UID: usize = 8;
const N_FLAGS: usize = 12;
const N_CALLBACK: usize = 16;
const N_ICON: usize = 20;
const N_TIP: usize = 24; // WCHAR[128]
const N_STATE: usize = 280;
const N_STATE_MASK: usize = 284;
const N_VERSION: usize = 800;
const N_GUID: usize = 936;

/// Applies one Shell_NotifyIcon message to our icon table. Returns true if it changed.
fn apply(data: &[u8]) -> bool {
    // SHELLTRAYDATA: dwSignature, dwMessage, then NOTIFYICONDATAW32.
    if data.len() < 8 + N_TIP || u32_at(data, 0) != 0x3475_3423 {
        return false;
    }
    let message = u32_at(data, 4);
    let n = &data[8..];
    let flags = u32_at(n, N_FLAGS);
    let hwnd = u32_at(n, N_HWND);
    let uid = u32_at(n, N_UID);
    let guid = if flags & NIF_GUID != 0 { guid_at(n, N_GUID) } else { None };
    let key = guid.clone().unwrap_or_else(|| format!("{hwnd:x}:{uid}"));

    let mut t = tray().lock().unwrap();
    match message {
        NIM_DELETE => t.icons.remove(&key).is_some(),
        NIM_ADD | NIM_MODIFY | NIM_SETVERSION => {
            if !t.icons.contains_key(&key) {
                if message != NIM_ADD {
                    return false;
                }
                let order = t.next_order;
                t.next_order += 1;
                let exe = process_name(hwnd);
                t.icons.insert(key.clone(), TrayIcon { hwnd, uid, guid, exe, order, ..Default::default() });
            }
            let icon = t.icons.get_mut(&key).unwrap();
            icon.hwnd = hwnd;
            if message == NIM_SETVERSION {
                icon.version = u32_at(n, N_VERSION);
                return false;
            }
            if flags & NIF_MESSAGE != 0 {
                icon.callback = u32_at(n, N_CALLBACK);
            }
            if flags & NIF_TIP != 0 {
                icon.tip = wstr_at(n, N_TIP, 128);
            }
            if flags & NIF_STATE != 0 {
                let mask = u32_at(n, N_STATE_MASK);
                if mask & NIS_HIDDEN != 0 {
                    icon.hidden = u32_at(n, N_STATE) & NIS_HIDDEN != 0;
                }
            }
            if flags & NIF_ICON != 0 {
                icon.image = icon_rgba(HICON(u32_at(n, N_ICON) as usize as *mut _));
            }
            true
        }
        _ => false,
    }
}

fn process_name(hwnd: u32) -> String {
    crate::win::window_process(HWND(hwnd as usize as *mut _))
}

/// Reads an icon's pixels as straight RGBA (falls back to the AND mask for icons without alpha).
fn icon_rgba(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let mut info = ICONINFO::default();
        GetIconInfo(icon, &mut info).ok()?;
        let color = (!info.hbmColor.is_invalid()).then(|| crate::win::bitmap_bgra(info.hbmColor)).flatten();
        let mask = crate::win::bitmap_bgra(info.hbmMask);
        let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        let (w, h, mut px) = color?;
        if w > 256 || h > 256 {
            return None;
        }
        // The mask only counts when it's the same size (color icons' masks are).
        let mask = mask.filter(|(mw, mh, _)| (*mw, *mh) == (w, h)).map(|(_, _, m)| m);
        let has_alpha = px.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
        for (i, p) in px.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            p.swap(0, 2); // BGRA -> RGBA
            if !has_alpha {
                let transparent = mask.as_ref().is_some_and(|m| m[i * 4] != 0);
                p[3] = if transparent { 0 } else { 255 };
            }
        }
        Some((w, h, px))
    }
}

// ---------------------------------------------------------------------------
// Publishing to the frontend.

fn snapshot(app: &AppHandle) -> Value {
    let opts = widget_opts(app, "tray");
    let hide: Vec<String> = opts
        .get("hide")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.to_lowercase()).collect())
        .unwrap_or_else(|| vec!["explorer.exe".into()]);
    let t = tray().lock().unwrap();
    let mut icons: Vec<&TrayIcon> = t
        .icons
        .values()
        .filter(|i| !i.hidden && i.image.is_some())
        .filter(|i| !hide.iter().any(|h| i.exe == *h || i.exe.trim_end_matches(".exe") == h.trim_end_matches(".exe")))
        .collect();
    icons.sort_by_key(|i| i.order);
    let b64 = base64::engine::general_purpose::STANDARD;
    Value::Array(
        icons
            .into_iter()
            .map(|i| {
                let (w, h, px) = i.image.as_ref().unwrap();
                json!({
                    "id": i.guid.clone().unwrap_or_else(|| format!("{:x}:{}", i.hwnd, i.uid)),
                    "tip": i.tip,
                    "exe": i.exe,
                    "w": w, "h": h,
                    "rgba": b64.encode(px),
                })
            })
            .collect(),
    )
}

fn republish() {
    if let Some(app) = APP.get() {
        publish(app, "tray", snapshot(app));
    }
}

const TIMER_SYNC: usize = 1;
const TIMER_PUBLISH: usize = 2;

/// Publishes shortly after a burst of changes instead of on every message
/// (animated tray icons can update many times a second).
fn schedule_publish() {
    if let Some(&h) = OUR_HWND.get() {
        unsafe {
            SetTimer(Some(HWND(h as *mut _)), TIMER_PUBLISH, 150, None);
        }
    }
}

// ---------------------------------------------------------------------------
// The hidden Shell_TrayWnd.

/// Explorer's taskbar window (the first `Shell_TrayWnd` that isn't ours).
fn explorer_tray() -> Option<HWND> {
    let ours = *OUR_HWND.get()?;
    let mut after: Option<HWND> = None;
    loop {
        let h = unsafe { FindWindowExW(None, after, w!("Shell_TrayWnd"), PCWSTR::null()) }.ok()?;
        if h.0 as isize != ours {
            return Some(h);
        }
        after = Some(h);
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_COPYDATA {
        let cds = &*(lparam.0 as *const COPYDATASTRUCT);
        if cds.dwData == SH_TRAY_DATA && !cds.lpData.is_null() {
            let data = std::slice::from_raw_parts(cds.lpData as *const u8, cds.cbData as usize);
            if apply(data) {
                schedule_publish();
            }
        }
    }
    // Messages apps send the taskbar (copy-data, app-defined and registered
    // messages) go on to Explorer unchanged, so its tray and taskbar behave as
    // before. Our own window's lifecycle messages must never be forwarded.
    let from_apps = msg == WM_COPYDATA || msg >= WM_USER;
    if let Some(explorer) = explorer_tray().filter(|_| from_apps) {
        let mut result = 0usize;
        if SendMessageTimeoutW(explorer, msg, wparam, lparam, SMTO_ABORTIFHUNG, 2000, Some(&mut result)).0 != 0 {
            return LRESULT(result as isize);
        }
    }
    if msg == WM_COPYDATA {
        return LRESULT(1);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

/// Raise/resize bookkeeping for the hidden window (only touched on the tray thread).
#[derive(Default)]
struct SyncState {
    rect: (i32, i32, i32, i32),
    raises: Vec<Instant>,
    backed_off_until: Option<Instant>,
}

/// Keeps our window the one apps find (first `Shell_TrayWnd`) and the same size
/// as the real taskbar (some apps position popups from its rect). Does nothing
/// when nothing changed: z-order churn makes Explorer and taskbar mods busy.
fn sync_with_explorer(ours: HWND, st: &mut SyncState) {
    unsafe {
        let mut rect = Default::default();
        let r = if explorer_tray().is_some_and(|e| GetWindowRect(e, &mut rect).is_ok()) {
            (rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top)
        } else {
            (0, 0, 0, 0)
        };
        if r != st.rect {
            st.rect = r;
            let _ = SetWindowPos(ours, None, r.0, r.1, r.2, r.3, SWP_NOACTIVATE | SWP_NOZORDER);
        }

        let first = FindWindowW(w!("Shell_TrayWnd"), PCWSTR::null()).ok();
        if first.is_some_and(|h| h != ours) {
            let now = Instant::now();
            if st.backed_off_until.is_some_and(|t| now < t) {
                return;
            }
            // Something (e.g. a taskbar z-order mod) keeps putting the taskbar on top:
            // don't fight it in a loop.
            st.raises.retain(|t| now.duration_since(*t) < Duration::from_secs(60));
            st.raises.push(now);
            if st.raises.len() > 10 {
                eprintln!("[statusnotch] tray: taskbar keeps moving above us; backing off for 60s");
                st.backed_off_until = Some(now + Duration::from_secs(60));
                st.raises.clear();
                return;
            }
            let _ = SetWindowPos(ours, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
        }
    }
}

/// Drops icons whose owner window no longer exists (app exited without NIM_DELETE).
fn prune() -> bool {
    let mut t = tray().lock().unwrap();
    let before = t.icons.len();
    t.icons.retain(|_, i| unsafe { IsWindow(Some(HWND(i.hwnd as usize as *mut _))).as_bool() });
    before != t.icons.len()
}

fn run() {
    unsafe {
        let class = w!("Shell_TrayWnd");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), lpszClassName: class, ..Default::default() };
        if RegisterClassW(&wc) == 0 {
            eprintln!("[statusnotch] tray: could not register Shell_TrayWnd class");
            return;
        }
        // Hidden (never shown); WS_POPUP so it has no frame and can mirror the taskbar rect.
        let hwnd = match CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
            class,
            PCWSTR::null(),
            WS_POPUP,
            0,
            0,
            0,
            0,
            None,
            None,
            None,
            None,
        ) {
            Ok(h) => h,
            Err(e) => {
                eprintln!("[statusnotch] tray: could not create window: {e}");
                return;
            }
        };
        let _ = OUR_HWND.set(hwnd.0 as isize);

        // Ask running apps to re-add their icons (they now reach us first).
        let created = RegisterWindowMessageW(w!("TaskbarCreated"));
        let _ = SendNotifyMessageW(HWND_BROADCAST, created, WPARAM(0), LPARAM(0));
        eprintln!("[statusnotch] tray: hosting started");

        // Block on the queue so apps' (synchronous) tray calls are answered at once;
        // housekeeping runs on a timer.
        let mut sync = SyncState::default();
        sync_with_explorer(hwnd, &mut sync);
        SetTimer(Some(hwnd), TIMER_SYNC, 1000, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_TIMER && msg.hwnd == hwnd {
                match msg.wParam.0 {
                    TIMER_SYNC => {
                        sync_with_explorer(hwnd, &mut sync);
                        if prune() {
                            schedule_publish();
                        }
                    }
                    TIMER_PUBLISH => {
                        let _ = KillTimer(Some(hwnd), TIMER_PUBLISH);
                        republish();
                    }
                    _ => {}
                }
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    // Hosting intercepts system-wide tray messages; only do it when the tray widget is used.
    // (Changing this takes effect on the next start.)
    if !widget_enabled(app, "tray") || widget_opts(app, "tray").get("host").and_then(|v| v.as_bool()) == Some(false) {
        eprintln!("[statusnotch] tray: hosting disabled in config");
        publish(app, "tray", Value::Array(vec![]));
        return;
    }
    publish(app, "tray", Value::Array(vec![]));
    std::thread::Builder::new().name("statusnotch-tray".into()).spawn(run).expect("spawn tray thread");
}

// ---------------------------------------------------------------------------
// Clicks from the tray widget.

fn make_long(lo: u32, hi: u32) -> usize {
    ((hi & 0xffff) << 16 | (lo & 0xffff)) as usize
}

/// `button`: "left" | "right" | "double". `x`, `y`: icon center in logical px of the overlay.
#[tauri::command]
pub async fn tray_click(
    window: tauri::WebviewWindow,
    id: String,
    button: String,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let icon = {
        let t = tray().lock().unwrap();
        t.icons
            .values()
            .find(|i| i.guid.as_deref() == Some(id.as_str()) || format!("{:x}:{}", i.hwnd, i.uid) == id)
            .cloned()
            .ok_or("icon is gone")?
    };
    let (sx, sy) = crate::engine::to_screen(window.app_handle(), window.label(), x, y).unwrap_or((0, 0));
    eprintln!(
        "[statusnotch] tray click {button} {} v{} cb={:#x} hwnd={:#x} uid={} at {sx},{sy}",
        icon.exe, icon.version, icon.callback, icon.hwnd, icon.uid
    );
    unsafe {
        let target = HWND(icon.hwnd as usize as *mut _);
        let mut pid = 0u32;
        GetWindowThreadProcessId(target, Some(&mut pid));
        // Let the app take the foreground for its window or context menu.
        let _ = AllowSetForegroundWindow(pid);
        let post = |m: u32| {
            let (wp, lp) = if icon.version >= 4 {
                (make_long(sx as u32, sy as u32), make_long(m, icon.uid))
            } else {
                (icon.uid as usize, m as usize)
            };
            let _ = PostMessageW(Some(target), icon.callback, WPARAM(wp), LPARAM(lp as isize));
        };
        match button.as_str() {
            "left" => {
                post(WM_LBUTTONDOWN);
                post(WM_LBUTTONUP);
                if icon.version >= 4 {
                    post(NIN_SELECT);
                }
            }
            "double" => post(WM_LBUTTONDBLCLK),
            "right" => {
                post(WM_RBUTTONDOWN);
                post(WM_RBUTTONUP);
                if icon.version >= 4 {
                    post(WM_CONTEXTMENU);
                }
            }
            other => return Err(format!("unknown button '{other}'")),
        }
    }
    Ok(())
}
