//! Tasks: the open app windows (the ones Alt+Tab shows), grouped by app, for the
//! tasks widget. WinEvent hooks mark the list dirty and a timer rescans, so it
//! follows windows opening, closing, activating and moving between monitors
//! without polling. Each app gets one icon (its Start menu tile icon, else the
//! exe's), sent once.

use super::publish;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;
use windows::core::{Interface, HSTRING};
use windows::Win32::Foundation::{HWND, LPARAM, PROPERTYKEY, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetMonitorInfoW, MonitorFromWindow, HBITMAP, HGDIOBJ, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    WM_SYSCOMMAND, SC_MINIMIZE, SC_RESTORE,
    DispatchMessageW, EnumWindows, GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongW,
    GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, PostMessageW,
    SetForegroundWindow, SetTimer, ShowWindow, BringWindowToTop, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GWL_EXSTYLE, GW_OWNER, MSG, OBJID_WINDOW, SW_MINIMIZE, SW_RESTORE,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// Set by the WinEvent hook; the scan timer clears it.
static DIRTY: AtomicBool = AtomicBool::new(true);
/// Windows that asked for attention (FlashWindow: the taskbar button flashes), until
/// they come to the front.
static ATTENTION: std::sync::Mutex<Vec<isize>> = std::sync::Mutex::new(Vec::new());
/// Each window's app (its key), as last scanned: where a minimizing window goes.
static WINDOW_APPS: std::sync::Mutex<Vec<(isize, String)>> = std::sync::Mutex::new(Vec::new());

/// HSHELL_GETMINRECT's lParam: the window, and where its minimize animation should go.
#[repr(C)]
struct ShellHookInfo {
    hwnd: HWND,
    rc: windows::Win32::Foundation::RECT,
}

/// The shell hook's message (RegisterWindowMessage "SHELLHOOK").
static SHELLHOOK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The shell hook window's messages: HSHELL_FLASH marks a window as asking for attention.
unsafe extern "system" fn hook_proc(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> windows::Win32::Foundation::LRESULT {
    const HSHELL_FLASH: usize = 0x8006;
    const HSHELL_GETMINRECT: usize = 5;
    // A window minimizing: towards its app's icon in Kasumi's dock on its own monitor
    // (else Explorer's hidden taskbar answers: its button on the main monitor).
    if msg != 0 && msg == SHELLHOOK.load(Ordering::Relaxed) && wp.0 == HSHELL_GETMINRECT && lp.0 != 0 {
        let info = &mut *(lp.0 as *mut ShellHookInfo);
        let key = WINDOW_APPS.lock().unwrap().iter().find(|(h, _)| *h == info.hwnd.0 as isize).map(|e| e.1.clone());
        if let Some(r) = key.and_then(|k| crate::native::icon_rect(&monitor_of(info.hwnd), &k)) {
            info.rc = windows::Win32::Foundation::RECT { left: r[0], top: r[1], right: r[2], bottom: r[3] };
            return windows::Win32::Foundation::LRESULT(1);
        }
    }
    if msg != 0 && msg == SHELLHOOK.load(Ordering::Relaxed) && wp.0 == HSHELL_FLASH {
        let mut list = ATTENTION.lock().unwrap();
        if !list.contains(&lp.0) {
            list.push(lp.0);
            DIRTY.store(true, Ordering::Relaxed);
        }
        return windows::Win32::Foundation::LRESULT(0);
    }
    windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(h, msg, wp, lp)
}

/// A hidden window that hears the shell hook (apps flashing their taskbar button).
unsafe fn shell_hook() {
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, RegisterClassW, RegisterShellHookWindow, RegisterWindowMessageW, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP};
    let Ok(instance) = windows::Win32::System::LibraryLoader::GetModuleHandleW(None) else { return };
    RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(hook_proc), hInstance: instance.into(), lpszClassName: windows::core::w!("KasumiShellHook"), ..Default::default() });
    let Ok(h) = CreateWindowExW(WS_EX_TOOLWINDOW, windows::core::w!("KasumiShellHook"), windows::core::w!("Kasumi"), WS_POPUP, 0, 0, 0, 0, None, None, Some(instance.into()), None) else {
        eprintln!("[kasumi] tasks: no shell hook window");
        return;
    };
    SHELLHOOK.store(RegisterWindowMessageW(windows::core::w!("SHELLHOOK")), Ordering::Relaxed);
    if !RegisterShellHookWindow(h).as_bool() {
        eprintln!("[kasumi] tasks: couldn't register the shell hook (no attention dots)");
    }
}

/// System.AppUserModel.ID
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3),
    pid: 5,
};

/// Icon size requested from the shell (covers 36 px icons on 150% DPI).
const ICON_PX: i32 = 64;

#[derive(Clone)]
struct App {
    key: String,
    name: String,
    exe: String,
    aumid: Option<String>,
    /// Pinned to the Windows taskbar: its shortcut (launched when not running).
    lnk: Option<String>,
}

/// A pinned app: the app key it matches, its name, and (Windows pins) its shortcut.
#[derive(Clone)]
struct Pin {
    key: String,
    name: String,
    lnk: Option<String>,
    /// How it's written in the pin list (an app key or a file path).
    entry: String,
    /// The program it starts (lower case), so windows that report no
    /// AppUserModelID still join it (e.g. Explorer's folder windows).
    target: String,
}

/// Pins, re-read when the pins folder changes.
static PINS: Mutex<Option<(std::time::SystemTime, Vec<Pin>)>> = Mutex::new(None);

fn pins_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(std::env::var("APPDATA").unwrap_or_default())
        .join(r"Microsoft\Internet Explorer\Quick Launch\User Pinned\TaskBar")
}

/// The pinned apps: Kasumi's own list (`[widgets.tasks] pins`, app keys
/// in order) once the user has pinned or unpinned something, until then the
/// Windows taskbar's pins. None while `pinning = false`.
fn pins(app: &AppHandle) -> Vec<Pin> {
    use tauri::Manager;
    let (enabled, own) = {
        let cfg = app.state::<super::SharedConfig>();
        let cfg = cfg.read().unwrap();
        let t = cfg.widgets.get("tasks").and_then(|v| v.as_table());
        let enabled = t.and_then(|t| t.get("pinning")).and_then(|v| v.as_bool()) != Some(false)
            && t.and_then(|t| t.get("pinned")).and_then(|v| v.as_bool()) != Some(false);
        let own: Option<Vec<String>> = t
            .and_then(|t| t.get("pins"))
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect());
        (enabled, own)
    };
    if !enabled {
        return Vec::new();
    }
    match own {
        Some(entries) => entries.into_iter().filter_map(|entry| own_pin(&entry)).collect(),
        None => windows_pins(),
    }
}

/// One entry of Kasumi's pin list: an app key (AppUserModelID or exe), or a
/// file dropped onto the tasks widget (a shortcut, a program, an internet link).
fn own_pin(entry: &str) -> Option<Pin> {
    let lower = entry.to_lowercase();
    if lower.ends_with(".lnk") {
        // Matched to windows like a Windows pin; started through the shortcut.
        let aumid = shortcut_aumid(entry);
        let mut target = shortcut_target(entry).map(|t| t.to_lowercase()).unwrap_or_default();
        if target.is_empty() {
            target = aumid.as_deref().map(aumid_program).unwrap_or_default();
        }
        let key = aumid.or_else(|| (!target.is_empty()).then(|| target.clone())).unwrap_or_else(|| lower.clone());
        let name = std::path::Path::new(entry).file_stem()?.to_string_lossy().to_string();
        return Some(Pin { key, name, lnk: Some(entry.to_string()), target, entry: entry.to_string() });
    }
    if is_path(entry) && !lower.ends_with(".exe") {
        // Other files (.url, documents...): opened through the shell.
        let name = std::path::Path::new(entry).file_stem()?.to_string_lossy().to_string();
        return Some(Pin { key: lower, name, lnk: Some(entry.to_string()), target: String::new(), entry: entry.to_string() });
    }
    let target = if is_path(entry) { lower.clone() } else { aumid_program(entry) };
    let key = if is_path(entry) { lower } else { entry.to_string() };
    Some(Pin { name: String::new(), lnk: None, target, key, entry: entry.to_string() })
}

/// App keys are AppUserModelIDs or (apps without one) exe paths.
fn is_path(key: &str) -> bool {
    key.contains('\\') || key.contains(':')
}

/// The program behind a known shell app ID whose windows report no ID.
fn aumid_program(aumid: &str) -> String {
    if aumid == "Microsoft.Windows.Explorer" {
        let windir = std::env::var("WINDIR").unwrap_or_else(|_| r"C:\Windows".into());
        return format!(r"{windir}\explorer.exe").to_lowercase();
    }
    String::new()
}

/// The Windows taskbar's pinned shortcuts, in pin order (oldest first). Each
/// pin's key is the shortcut's AppUserModelID, else its target exe, so it
/// matches the app's windows.
fn windows_pins() -> Vec<Pin> {
    let dir = pins_dir();
    let stamp = std::fs::metadata(&dir).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
    if let Some((at, list)) = PINS.lock().unwrap().as_ref() {
        if *at == stamp {
            return list.clone();
        }
    }
    let mut files: Vec<(std::time::SystemTime, std::path::PathBuf)> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("lnk")))
        .map(|p| (std::fs::metadata(&p).and_then(|m| m.created()).unwrap_or(std::time::UNIX_EPOCH), p))
        .collect();
    files.sort();
    let list: Vec<Pin> = files
        .into_iter()
        .filter_map(|(_, p)| {
            let lnk = p.to_string_lossy().to_string();
            let aumid = shortcut_aumid(&lnk);
            let mut target = shortcut_target(&lnk).map(|t| t.to_lowercase()).unwrap_or_default();
            // File Explorer's pin points at a shell location, not a program, and its
            // folder windows report no AppUserModelID: link them by program.
            if target.is_empty() {
                target = aumid.as_deref().map(aumid_program).unwrap_or_default();
            }
            let key = aumid.or_else(|| (!target.is_empty()).then(|| target.clone()))?;
            let name = p.file_stem()?.to_string_lossy().to_string();
            Some(Pin { key: key.clone(), name, lnk: Some(lnk), target, entry: key })
        })
        .collect();
    // The same app pinned twice (e.g. "Microsoft Edge (2).lnk") shows once.
    let mut list = list;
    let mut keys = std::collections::HashSet::new();
    list.retain(|p| keys.insert(p.key.clone()));
    *PINS.lock().unwrap() = Some((stamp, list.clone()));
    list
}

/// A shortcut's System.AppUserModel.ID, if it has one.
fn shortcut_aumid(lnk: &str) -> Option<String> {
    use windows::Win32::UI::Shell::PropertiesSystem::{SHGetPropertyStoreFromParsingName, GPS_DEFAULT};
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreFromParsingName(&HSTRING::from(lnk), None, GPS_DEFAULT).ok()?;
        let v = store.GetValue(&PKEY_APP_USER_MODEL_ID).ok()?;
        let s = v.to_string();
        (!s.is_empty()).then_some(s)
    }
}

/// The program a shortcut starts.
fn shortcut_target(lnk: &str) -> Option<String> {
    use windows::Win32::System::Com::{CoCreateInstance, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ};
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};
    unsafe {
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        link.cast::<IPersistFile>().ok()?.Load(&HSTRING::from(lnk), STGM_READ).ok()?;
        let mut buf = [0u16; 1024];
        link.GetPath(&mut buf, std::ptr::null_mut(), 0).ok()?;
        let s = crate::win::wide_to_string(&buf);
        (!s.is_empty()).then_some(s)
    }
}

#[derive(Default)]
struct State {
    /// App keys in the order they first appeared (taskbar order).
    order: Vec<String>,
    apps: HashMap<String, App>,
    /// Window handles in the order they first appeared.
    seen: Vec<isize>,
    /// The order the user dragged apps into (this session).
    manual: Vec<String>,
    /// Listed windows that got cloaked, and when (see `scan`).
    cloaked_since: HashMap<isize, std::time::Instant>,
    /// When each app last had a window: it keeps its place for a moment after.
    last_window: HashMap<String, std::time::Instant>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(Default::default)
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-tasks".into())
        .spawn(move || run(app))
        .expect("spawn tasks thread");
}

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    if event == EVENT_SYSTEM_FOREGROUND || (object == OBJID_WINDOW.0 && child == 0 && !hwnd.is_invalid()) {
        DIRTY.store(true, Ordering::Relaxed);
    }
}

fn run(app: AppHandle) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
        for (lo, hi) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND),
            (EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE),
            (EVENT_OBJECT_CLOAKED, EVENT_OBJECT_UNCLOAKED),
        ] {
            SetWinEventHook(lo, hi, None, Some(on_event), 0, 0, flags);
        }
        shell_hook();
        // Rescan when something changed (at most every 120 ms), and every few
        // seconds anyway in case an event was missed.
        SetTimer(None, 0, 120, None);
        let mut ticks = 0u32;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == windows::Win32::UI::WindowsAndMessaging::WM_TIMER {
                ticks = ticks.wrapping_add(1);
                if (DIRTY.swap(false, Ordering::Relaxed) || ticks.is_multiple_of(25)) && super::placed(&app, "tasks") {
                    scan(&app);
                }
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
}

/// The window's explicit AppUserModelID (Store apps, Explorer, PWAs, ...).
fn aumid_of(h: HWND) -> Option<String> {
    unsafe {
        let store: IPropertyStore = SHGetPropertyStoreForWindow(h).ok()?;
        let v = store.GetValue(&PKEY_APP_USER_MODEL_ID).ok()?;
        let s = v.to_string();
        (!s.is_empty()).then_some(s)
    }
}

fn monitor_of(h: HWND) -> String {
    use windows::Win32::Graphics::Gdi::MonitorFromRect;
    use windows::Win32::UI::WindowsAndMessaging::{GetWindowPlacement, WINDOWPLACEMENT};
    unsafe {
        // Minimized windows are parked far off-screen (-32000, -32000): use the
        // place they'll be restored to, or they'd jump to another monitor's list.
        let mon = if IsIconic(h).as_bool() {
            let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
            if GetWindowPlacement(h, &mut wp).is_ok() {
                MonitorFromRect(&wp.rcNormalPosition, MONITOR_DEFAULTTONEAREST)
            } else {
                MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST)
            }
        } else {
            MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST)
        };
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(mon, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
            crate::win::wide_to_string(&info.szDevice)
        } else {
            String::new()
        }
    }
}

/// Alt+Tab rules: visible, titled, not a tool window, and unowned unless it asks
/// for a taskbar button. (Cloaking is checked by the caller: see `scan`.)
fn is_task(h: HWND, own_pid: u32) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() || GetWindowTextLengthW(h) == 0 {
            return false;
        }
        let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
        let app_window = ex & WS_EX_APPWINDOW.0 != 0;
        if ex & WS_EX_TOOLWINDOW.0 != 0 || (ex & WS_EX_NOACTIVATE.0 != 0 && !app_window) {
            return false;
        }
        if GetWindow(h, GW_OWNER).is_ok_and(|o| !o.is_invalid()) && !app_window {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == own_pid {
            return false;
        }
        !matches!(crate::win::class_name(h).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    }
}

/// Hidden by the compositor: on another virtual desktop, a suspended Store app,
/// or (briefly) a window in its minimize / restore animation.
fn is_cloaked(h: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
    }
    cloaked != 0
}

fn scan(app: &AppHandle) {
    unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> windows::core::BOOL {
        (*(data.0 as *mut Vec<HWND>)).push(h);
        windows::core::BOOL(1)
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));
    }
    let own = unsafe { GetCurrentProcessId() };
    let fg = unsafe { GetForegroundWindow() };

    let pinned = pins(app);
    let mut st = state().lock().unwrap();
    let mut windows = Vec::new();
    // Icons found this scan; the pages are sent just these (`publish_entries`).
    let mut fresh = serde_json::Map::new();
    // Pinned apps are listed (first, in pin order) even without windows.
    for p in &pinned {
        if !st.apps.contains_key(&p.key) {
            // Name and icon: the pin's shortcut, else its Start entry / exe.
            let (aumid, exe) = if is_path(&p.key) { (None, p.key.clone()) } else { (Some(p.key.clone()), String::new()) };
            let (name, icon) = match &p.lnk {
                Some(lnk) => (p.name.clone(), shell_item(lnk).and_then(|i| item_icon(&i))),
                None => describe(aumid.as_deref(), &exe),
            };
            let (aumid, exe) = if p.lnk.is_some() && !p.target.is_empty() { (aumid, p.target.clone()) } else { (aumid, exe) };
            if let Some((w, hgt, rgba)) = icon {
                fresh.insert(p.key.clone(), super::rgba_json(w, hgt, &rgba));
            }
            st.apps.insert(p.key.clone(), App { key: p.key.clone(), name, exe, aumid, lnk: None });
        }
        if let Some(entry) = st.apps.get_mut(&p.key) {
            entry.lnk = p.lnk.clone();
        }
    }
    // EnumWindows lists top-level windows front to back: z = 0 is the most recent.
    // Windows are cloaked for a moment while they minimize or restore: one already
    // listed stays listed through that (a window on another desktop goes after it).
    let now = std::time::Instant::now();
    let mut cloak_wait = false;
    let listed: Vec<HWND> = all
        .iter()
        .copied()
        .filter(|&h| is_task(h, own))
        .filter(|&h| {
            let id = h.0 as isize;
            if !is_cloaked(h) {
                st.cloaked_since.remove(&id);
                return true;
            }
            if !st.seen.contains(&id) {
                return false;
            }
            let since = *st.cloaked_since.entry(id).or_insert(now);
            let keep = since.elapsed() < std::time::Duration::from_millis(1500);
            cloak_wait |= keep;
            keep
        })
        .collect();
    st.cloaked_since.retain(|id, _| listed.iter().any(|h| h.0 as isize == *id) || all.iter().any(|h| h.0 as isize == *id));
    if cloak_wait {
        DIRTY.store(true, Ordering::Relaxed);
    }
    for (z, &h) in listed.iter().enumerate() {
        let exe = crate::win::process_path(crate::win::window_pid(h)).unwrap_or_default();
        let aumid = aumid_of(h);
        let mut key = aumid.clone().unwrap_or_else(|| exe.to_lowercase());
        if key.is_empty() {
            continue;
        }
        // Windows of a pinned program join its pin.
        if aumid.is_none() {
            if let Some(p) = pinned.iter().find(|p| !p.target.is_empty() && p.target == exe.to_lowercase()) {
                key = p.key.clone();
            }
        }
        if !st.apps.contains_key(&key) {
            let (name, icon) = describe(aumid.as_deref(), &exe);
            if let Some((w, hgt, rgba)) = icon {
                fresh.insert(key.clone(), super::rgba_json(w, hgt, &rgba));
            }
            st.apps.insert(key.clone(), App { key: key.clone(), name, exe: exe.clone(), aumid: aumid.clone(), lnk: None });
        }
        if !st.order.contains(&key) {
            st.order.push(key.clone());
        }
        let id = h.0 as isize;
        if !st.seen.contains(&id) {
            st.seen.push(id);
        }
        windows.push((id, crate::win::window_title(h), key, h == fg, unsafe { IsIconic(h).as_bool() }, monitor_of(h), z));
    }
    // Forget closed windows; an app without windows keeps its place for a few
    // seconds (a window being replaced or re-created), then goes (it comes back
    // at the end when reopened).
    st.seen.retain(|id| windows.iter().any(|w| w.0 == *id));
    let now = std::time::Instant::now();
    for w in &windows {
        st.last_window.insert(w.2.clone(), now);
    }
    let recent = st.last_window.clone();
    st.order.retain(|k| recent.get(k).is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(3)));
    st.last_window.retain(|_, t| t.elapsed() < std::time::Duration::from_secs(60));
    let seen = st.seen.clone();
    windows.sort_by_key(|w| seen.iter().position(|id| *id == w.0));

    let mut order: Vec<String> = pinned
        .iter()
        .map(|p| p.key.clone())
        .chain(st.order.iter().filter(|k| !pinned.iter().any(|p| &p.key == *k)).cloned())
        .collect();
    order.dedup();
    // The order the user dragged apps into wins; anything new keeps its place after.
    let manual = st.manual.clone();
    order.sort_by_key(|k| manual.iter().position(|m| m == k).unwrap_or(usize::MAX));
    let apps: Vec<Value> = order
        .iter()
        .filter_map(|k| st.apps.get(k))
        .map(|a| {
            let pin = pinned.iter().find(|p| p.key == a.key);
            json!({
                "key": a.key, "name": a.name, "exe": a.exe, "aumid": a.aumid,
                "pinned": pin.is_some(), "pin": pin.map(|p| p.entry.clone()),
            })
        })
        .collect();
    *WINDOW_APPS.lock().unwrap() = windows.iter().map(|w| (w.0 as isize, w.2.clone())).collect();
    // Asking for attention until it comes to the front (or goes away).
    let attention = {
        let mut a = ATTENTION.lock().unwrap();
        a.retain(|h| windows.iter().any(|w| w.0 as isize == *h && !w.3));
        a.clone()
    };
    let list: Vec<Value> = windows
        .iter()
        .map(|(id, title, key, active, minimized, monitor, z)| {
            let attention = attention.contains(&(*id as isize));
            json!({ "hwnd": id, "title": title, "app": key, "active": active, "minimized": minimized, "monitor": monitor, "z": z, "attention": attention })
        })
        .collect();
    drop(st);
    if !fresh.is_empty() {
        super::publish_entries(app, "taskIcons", fresh);
    }
    publish(app, "tasks", json!({ "apps": apps, "windows": list }));
}

/// An app's display name and icon: its Start menu entry when it has an
/// AppUserModelID registered there, else the exe file.
fn describe(aumid: Option<&str>, exe: &str) -> (String, Option<(u32, u32, Vec<u8>)>) {
    let stem = exe.rsplit(['\\', '/']).next().unwrap_or(exe).trim_end_matches(".exe").trim_end_matches(".EXE").to_string();
    if let Some(a) = aumid {
        if let Some(item) = shell_item(&format!("shell:AppsFolder\\{a}")) {
            let name = unsafe { item.GetDisplayName(SIGDN_NORMALDISPLAY).ok().map(|p| { let s = p.to_string().unwrap_or_default(); windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _)); s }) };
            let icon = item_icon(&item).or_else(|| shell_item(exe).and_then(|i| item_icon(&i)));
            return (name.filter(|n| !n.is_empty()).unwrap_or(stem), icon);
        }
    }
    let icon = shell_item(exe).and_then(|i| item_icon(&i));
    (stem, icon)
}

fn shell_item(path: &str) -> Option<IShellItem> {
    if path.is_empty() {
        return None;
    }
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None).ok() }
}

/// A program's icon (RGBA), as Tasks shows it.
pub(crate) fn exe_icon(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    shell_item(path).and_then(|i| item_icon(&i))
}

pub(crate) fn item_icon(item: &IShellItem) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let bmp = factory.GetImage(SIZE { cx: ICON_PX, cy: ICON_PX }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let px = bitmap_rgba(bmp);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        px
    }
}

/// A 32-bit (premultiplied) bitmap's pixels as straight RGBA.
fn bitmap_rgba(bmp: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    let (w, h, mut px) = crate::win::bitmap_bgra(bmp)?;
    let has_alpha = px.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
    for p in px.as_chunks_mut::<4>().0 {
        p.swap(0, 2); // BGRA -> RGBA
        if !has_alpha {
            p[3] = 255;
        } else if p[3] > 0 && p[3] < 255 {
            // Shell images are premultiplied.
            let a = p[3] as u32;
            for c in &mut p[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
    Some((w, h, px))
}

// ---------------------------------------------------------------------------
// Actions from the widget.

fn hwnd(id: isize) -> HWND {
    HWND(id as *mut _)
}

/// Brings a window to the front. The click on our overlay is the last input, so
/// we may set the foreground; the Alt tap covers the cases where Windows refuses.
fn activate(h: HWND) {
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        PeekMessageW, SetWindowPos, HWND_TOP, PM_NOREMOVE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    };
    unsafe {
        if IsIconic(h).as_bool() {
            // As the window's own restore does (its animation from where the shell says);
            // else directly (a busy or elevated window).
            let mut result = 0usize;
            let sent = windows::Win32::UI::WindowsAndMessaging::SendMessageTimeoutW(
                h,
                WM_SYSCOMMAND,
                WPARAM(SC_RESTORE as usize),
                LPARAM(0),
                windows::Win32::UI::WindowsAndMessaging::SMTO_ABORTIFHUNG,
                300,
                Some(&mut result),
            );
            if sent.0 == 0 || IsIconic(h).as_bool() {
                let _ = ShowWindow(h, SW_RESTORE);
            }
        }
        // Windows only lets the app in front hand the foreground over: borrow its
        // input queue for the switch (this thread needs a message queue for that).
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
        let me = GetCurrentThreadId();
        let front = GetWindowThreadProcessId(GetForegroundWindow(), None);
        let attached = front != 0 && front != me && AttachThreadInput(me, front, true).as_bool();
        let _ = BringWindowToTop(h);
        let _ = SetForegroundWindow(h);
        if attached {
            let _ = AttachThreadInput(me, front, false);
        }
        if GetForegroundWindow() != h {
            // Still refused: a tapped Alt counts as fresh input and lifts the lock.
            keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            let _ = SetForegroundWindow(h);
        }
        // In front of the other windows even if focus was refused.
        let _ = SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW);
        if GetForegroundWindow() != h {
            eprintln!("[kasumi] tasks: Windows refused to bring a window to the front");
        }
    }
}

/// Click on a task: front it, or minimize it if it already is in front (`toggle`).
#[tauri::command]
pub async fn task_activate(hwnd_id: isize, toggle: bool) -> Result<(), String> {
    let h = hwnd(hwnd_id);
    unsafe {
        if !IsWindow(Some(h)).as_bool() {
            return Err("window is gone".into());
        }
        if toggle && GetForegroundWindow() == h && !IsIconic(h).as_bool() {
            // As its own minimize button does (its animation goes where the shell says;
            // ShowWindow from here flew to the top of the screen); else directly.
            if PostMessageW(Some(h), WM_SYSCOMMAND, WPARAM(SC_MINIMIZE as usize), LPARAM(0)).is_err() {
                let _ = ShowWindow(h, SW_MINIMIZE);
            }
        } else {
            activate(h);
            if GetForegroundWindow() != h {
                eprintln!("[kasumi] tasks: couldn't bring {:#x} ('{}') to the front", hwnd_id, crate::win::window_title(h));
            }
        }
    }
    DIRTY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Scrolling over an app: its volume up or down (its new level, if it plays anything).
pub fn task_app_volume(app_key: &str, delta: f32) -> Option<f32> {
    let exe = state().lock().unwrap().apps.get(app_key).map(|a| a.exe.clone())?;
    super::audio::app_volume_step(&exe, delta)
}

/// Files dropped on an app: opened with it, through its exe (or its taskbar shortcut)
/// with the paths as arguments; apps already running mostly hand them to their open
/// window. Store apps have no exe to give them to: they're only brought up.
pub fn task_open_with(app_key: &str, paths: &[String]) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let a = state().lock().unwrap().apps.get(app_key).cloned().ok_or("unknown app")?;
    let packaged = a.exe.to_lowercase().contains("\\windowsapps\\");
    let target = if !a.exe.is_empty() && !packaged {
        a.exe.clone()
    } else if let Some(lnk) = &a.lnk {
        lnk.clone()
    } else {
        return Err("no program to open files with".into());
    };
    let args = paths.iter().map(|p| format!("\"{p}\"")).collect::<Vec<_>>().join(" ");
    crate::win::allow_foreground();
    let (file, params) = (HSTRING::from(target.as_str()), HSTRING::from(args.as_str()));
    let r = unsafe { ShellExecuteW(None, &HSTRING::from("open"), &file, &params, PCWSTR::null(), SW_SHOWNORMAL) };
    if r.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecute failed ({})", r.0 as isize))
    }
}

/// The user dragged an app to a new place: `keys` is the new order.
#[tauri::command]
pub async fn task_reorder(keys: Vec<String>) -> Result<(), String> {
    state().lock().unwrap().manual = keys;
    DIRTY.store(true, Ordering::Relaxed);
    Ok(())
}

/// Asks a window to close (like its close button).
#[tauri::command]
pub async fn task_close(hwnd_id: isize) -> Result<(), String> {
    unsafe { PostMessageW(Some(hwnd(hwnd_id)), WM_CLOSE, WPARAM(0), LPARAM(0)).map_err(|e| e.to_string()) }
}

/// Starts another instance / window of an app.
#[tauri::command]
pub async fn task_launch(app_key: String) -> Result<(), String> {
    let a = state().lock().unwrap().apps.get(&app_key).cloned().ok_or("unknown app")?;
    crate::win::allow_foreground();
    // A pinned app starts like its pin; others through their Start entry or exe.
    let target = match (&a.lnk, &a.aumid) {
        (Some(lnk), _) => lnk.clone(),
        (None, Some(id)) => format!("shell:AppsFolder\\{id}"),
        (None, None) => a.exe.clone(),
    };
    std::process::Command::new("explorer.exe").arg(target).spawn().map(|_| ()).map_err(|e| e.to_string())
}
