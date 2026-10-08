//! Tasks: the open app windows (the ones Alt+Tab shows), grouped by app, for the
//! tasks widget. WinEvent hooks mark the list dirty and a timer rescans, so it
//! follows windows opening, closing, activating and moving between monitors
//! without polling. Each app gets one icon (its Start menu tile icon, else the
//! exe's), sent once.

use super::publish;
use base64::Engine as _;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;
use windows::core::{Interface, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, PROPERTYKEY, SIZE, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetMonitorInfoW, GetObjectW, MonitorFromWindow, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{
    IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, EnumWindows, GetClassNameW, GetForegroundWindow, GetMessageW, GetWindow, GetWindowLongW,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, PostMessageW,
    SetForegroundWindow, SetTimer, ShowWindow, BringWindowToTop, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GWL_EXSTYLE, GW_OWNER, MSG, OBJID_WINDOW, SW_MINIMIZE, SW_RESTORE,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};

/// Set by the WinEvent hook; the scan timer clears it.
static DIRTY: AtomicBool = AtomicBool::new(true);

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

/// The pinned apps: StatusNotch's own list (`[widgets.tasks] pins`, app keys
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
        Some(keys) => keys
            .into_iter()
            .map(|key| {
                let target = if is_path(&key) { key.to_lowercase() } else { aumid_program(&key) };
                Pin { name: String::new(), lnk: None, target, key }
            })
            .collect(),
        None => windows_pins(),
    }
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
            Some(Pin { key, name, lnk: Some(lnk), target })
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
        let s = text(&buf);
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
    icons: serde_json::Map<String, Value>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(Default::default)
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("statusnotch-tasks".into())
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
        // Rescan when something changed (at most every 120 ms), and every few
        // seconds anyway in case an event was missed.
        SetTimer(None, 0, 120, None);
        let mut ticks = 0u32;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == windows::Win32::UI::WindowsAndMessaging::WM_TIMER {
                ticks = ticks.wrapping_add(1);
                if (DIRTY.swap(false, Ordering::Relaxed) || ticks % 25 == 0) && wanted(&app) {
                    scan(&app);
                }
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
}

/// Only work while a tasks widget is placed somewhere.
fn wanted(app: &AppHandle) -> bool {
    use tauri::Manager;
    let cfg = app.state::<super::SharedConfig>();
    let cfg = cfg.read().unwrap();
    cfg.widget_placed("tasks")
}

fn text(buf: &[u16]) -> String {
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn title_of(h: HWND) -> String {
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

/// Full path of the window's process image.
fn exe_of(h: HWND) -> (u32, String) {
    unsafe {
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return (pid, String::new()) };
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(p);
        (pid, if ok { String::from_utf16_lossy(&buf[..len as usize]) } else { String::new() })
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
    unsafe {
        let mon = MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(mon, &mut info as *mut _ as *mut MONITORINFO).as_bool() {
            text(&info.szDevice)
        } else {
            String::new()
        }
    }
}

/// Alt+Tab rules: visible, not cloaked (other desktops, suspended Store apps),
/// titled, not a tool window, and unowned unless it asks for a taskbar button.
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
        let mut cloaked = 0u32;
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
        if cloaked != 0 {
            return false;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == own_pid {
            return false;
        }
        !matches!(class_of(h).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    }
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
    let mut icons_changed = false;
    // Pinned apps are listed (first, in pin order) even without windows.
    for p in &pinned {
        if !st.apps.contains_key(&p.key) {
            // Name and icon: the pin's shortcut, else its Start entry / exe.
            let (aumid, exe) = if is_path(&p.key) { (None, p.key.clone()) } else { (Some(p.key.clone()), String::new()) };
            let (name, icon) = match &p.lnk {
                Some(lnk) => (p.name.clone(), shell_item(lnk).and_then(|i| item_icon(&i))),
                None => describe(aumid.as_deref(), &exe),
            };
            if let Some((w, hgt, rgba)) = icon {
                st.icons.insert(
                    p.key.clone(),
                    json!({ "w": w, "h": hgt, "rgba": base64::engine::general_purpose::STANDARD.encode(rgba) }),
                );
                icons_changed = true;
            }
            st.apps.insert(p.key.clone(), App { key: p.key.clone(), name, exe, aumid, lnk: None });
        }
        if let Some(entry) = st.apps.get_mut(&p.key) {
            entry.lnk = p.lnk.clone();
        }
    }
    // EnumWindows lists top-level windows front to back: z = 0 is the most recent.
    for (z, &h) in all.iter().filter(|&&h| is_task(h, own)).enumerate() {
        let (_, exe) = exe_of(h);
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
                st.icons.insert(
                    key.clone(),
                    json!({ "w": w, "h": hgt, "rgba": base64::engine::general_purpose::STANDARD.encode(rgba) }),
                );
                icons_changed = true;
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
        windows.push((id, title_of(h), key, h == fg, unsafe { IsIconic(h).as_bool() }, monitor_of(h), z));
    }
    // Forget closed windows and apps without windows (they go to the end when reopened).
    st.seen.retain(|id| windows.iter().any(|w| w.0 == *id));
    st.order.retain(|k| windows.iter().any(|w| &w.2 == k));
    let seen = st.seen.clone();
    windows.sort_by_key(|w| seen.iter().position(|id| *id == w.0));

    let order: Vec<String> = pinned
        .iter()
        .map(|p| p.key.clone())
        .chain(st.order.iter().filter(|k| !pinned.iter().any(|p| &p.key == *k)).cloned())
        .collect();
    let apps: Vec<Value> = order
        .iter()
        .filter_map(|k| st.apps.get(k))
        .map(|a| {
            let pinned = pinned.iter().any(|p| p.key == a.key);
            json!({ "key": a.key, "name": a.name, "exe": a.exe, "aumid": a.aumid, "pinned": pinned })
        })
        .collect();
    let list: Vec<Value> = windows
        .iter()
        .map(|(id, title, key, active, minimized, monitor, z)| {
            json!({ "hwnd": id, "title": title, "app": key, "active": active, "minimized": minimized, "monitor": monitor, "z": z })
        })
        .collect();
    if icons_changed {
        publish(app, "taskIcons", Value::Object(st.icons.clone()));
    }
    drop(st);
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
    unsafe {
        let mut bm = BITMAP::default();
        GetObjectW(HGDIOBJ(bmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _));
        let (w, h) = (bm.bmWidth.max(0) as u32, bm.bmHeight.unsigned_abs());
        if w == 0 || h == 0 || w > 512 || h > 512 {
            return None;
        }
        let dc = CreateCompatibleDC(None);
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut px = vec![0u8; (w * h * 4) as usize];
        let lines = GetDIBits(dc, bmp, 0, h, Some(px.as_mut_ptr().cast()), &mut bi, DIB_RGB_COLORS);
        let _ = DeleteDC(dc);
        if lines == 0 {
            return None;
        }
        let has_alpha = px.chunks_exact(4).any(|p| p[3] != 0);
        for p in px.chunks_exact_mut(4) {
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
}

// ---------------------------------------------------------------------------
// Actions from the widget.

fn hwnd(id: isize) -> HWND {
    HWND(id as *mut _)
}

/// Brings a window to the front. The click on our overlay is the last input, so
/// we may set the foreground; the Alt tap covers the cases where Windows refuses.
fn activate(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        if !SetForegroundWindow(h).as_bool() || GetForegroundWindow() != h {
            keybd_event(VK_MENU.0 as u8, 0, Default::default(), 0);
            keybd_event(VK_MENU.0 as u8, 0, KEYEVENTF_KEYUP, 0);
            let _ = SetForegroundWindow(h);
        }
        let _ = BringWindowToTop(h);
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
            let _ = ShowWindow(h, SW_MINIMIZE);
        } else {
            activate(h);
            if GetForegroundWindow() != h {
                eprintln!("[statusnotch] tasks: couldn't bring {:#x} ('{}') to the front", hwnd_id, title_of(h));
            }
        }
    }
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
