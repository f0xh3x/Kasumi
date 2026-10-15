//! The bottom dock: Kasumi's [surfaces.bottom] in dock mode, with the tasks widget
//! in its middle notch. The notch sits on the screen's bottom edge at the rest size
//! ([sizes], `rest`) and grows to the open size (`open`) while the cursor is on it,
//! its curves scaling with it (Kasumi's docks); the icons grow along.
//!
//! Tasks, a port of providers/tasks.rs and widgets/tasks.ts: the apps with windows
//! on this monitor (the windows Alt+Tab shows), one button each, in the order they
//! first appeared. Click: front it (again: minimize; several windows: cycle).
//! Middle click: close the front one. A dot per window (up to 3), a line under the
//! app in front; dimmed when minimized, magnify and marker glide as the config says.
//! Not here yet: pins, the right-click / hover panel, dragging to reorder, labels.
//! WinEvent hooks mark the list dirty: rescanned 120 ms later (and every 3 s anyway).
//! App icons come from a short-lived copy of this program (`--icons`): the shell's
//! imaging stack costs ~13 MB in whichever process loads it, and never unloads.
//!
//! `[shell] hide_taskbar`: the main monitor's Windows taskbar is hidden while the
//! test runs (as Kasumi does) and shown again when it quits.

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use windows::core::{w, Interface, Result, BOOL, HSTRING, PWSTR};
use windows::Win32::Foundation::{CloseHandle, COLORREF, HWND, LPARAM, POINT, PROPERTYKEY, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F, D2D1_PIXEL_FORMAT, D2D_RECT_F, D2D_SIZE_U};
use windows::Win32::Graphics::Direct2D::{
    ID2D1Bitmap, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, D2D1_BITMAP_PROPERTIES, D2D1_LAYER_OPTIONS_NONE,
    D2D1_LAYER_PARAMETERS, D2D1_ROUNDED_RECT,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, MonitorFromRect, MonitorFromWindow, AC_SRC_ALPHA, AC_SRC_OVER, BITMAP,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ, HMONITOR, MONITOR_DEFAULTTONEAREST,
};
use windows::Win32::System::Threading::{
    AttachThreadInput, GetCurrentProcessId, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_MENU};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::Win32::UI::Shell::{IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, FindWindowW, GetClassNameW, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowPlacement,
    GetWindowTextLengthW, GetWindowThreadProcessId, IsIconic, IsWindow, IsWindowVisible, KillTimer, PeekMessageW, PostMessageW,
    SetForegroundWindow, SetTimer, SetWindowPos, ShowWindow, ShowWindowAsync, UpdateLayeredWindow, EVENT_OBJECT_CLOAKED, EVENT_OBJECT_CREATE,
    EVENT_OBJECT_HIDE, EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, EVENT_OBJECT_UNCLOAKED, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, GWL_EXSTYLE, GW_OWNER, HWND_TOP, MSG, OBJID_WINDOW, PM_NOREMOVE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SW_HIDE, SW_MINIMIZE, SW_RESTORE, SW_SHOWNA, ULW_ALPHA, WINDOWPLACEMENT, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_APP, WM_CLOSE, WS_EX_APPWINDOW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
};
use windows_numerics::Matrix3x2;

use crate::geometry::{self, lerp, Bump, Geometry};
use crate::{color, geometry_of, rect, white, Spring, NOTCH_PAD};

/// Timers on the dock window: a rescan after window events, and the regular one.
pub const TASKS: usize = 8;
pub const TASKS_ANYWAY: usize = 9;

/// System.AppUserModel.ID
const PKEY_APP_USER_MODEL_ID: PROPERTYKEY = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0x9F4C2855_9F79_4B39_A8D0_E1D42DE1D5F3), pid: 5 };

/// Icon size asked of the shell (covers 36 px icons on 150% DPI), as Kasumi.
const ICON_PX: i32 = 64;

/// Posted to the dock window when fetched icons are waiting in ICONS_IN.
pub const WM_ICONS: u32 = WM_APP + 1;

/// An icon: width, height, premultiplied BGRA rows.
type Icon = (u32, u32, Vec<u8>);
static ICONS_IN: Mutex<Vec<(String, Option<Icon>)>> = Mutex::new(Vec::new());

// The dock window (for the WinEvent hook to schedule a rescan) and a rescan pending.
static DOCK_HWND: AtomicIsize = AtomicIsize::new(0);
static SCHEDULED: AtomicBool = AtomicBool::new(false);
// The taskbar hidden while the test runs (0: none).
static TASKBAR: AtomicIsize = AtomicIsize::new(0);

/// CSS timing functions.
type Curve = (f32, f32, f32, f32);
/// widgets/tasks.ts EASE.
const EASE: Curve = (0.2, 0.8, 0.2, 1.0);
const CSS_EASE: Curve = (0.25, 0.1, 0.25, 1.0);
const CSS_EASE_OUT: Curve = (0.0, 0.0, 0.58, 1.0);

/// A CSS cubic-bezier() at progress t.
fn bezier(c: Curve, t: f32) -> f32 {
    if t <= 0.0 {
        return 0.0;
    }
    if t >= 1.0 {
        return 1.0;
    }
    let f = |u: f32, a: f32, b: f32| {
        let v = 1.0 - u;
        3.0 * v * v * u * a + 3.0 * v * u * u * b + u * u * u
    };
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if f(mid, c.0, c.2) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    f((lo + hi) / 2.0, c.1, c.3)
}

/// A timed animation of one value (a CSS transition, or a Web Animation).
#[derive(Clone, Copy)]
struct Trans {
    from: f32,
    to: f32,
    start: Instant,
    ms: f32,
    curve: Curve,
}

impl Trans {
    fn at_rest(v: f32, ms: f32, curve: Curve) -> Self {
        Trans { from: v, to: v, start: Instant::now(), ms, curve }
    }
    fn run(from: f32, to: f32, ms: f32, curve: Curve, now: Instant) -> Self {
        Trans { from, to, start: now, ms, curve }
    }
    fn at(&self, now: Instant) -> f32 {
        let k = now.saturating_duration_since(self.start).as_secs_f32() * 1000.0 / self.ms;
        self.from + (self.to - self.from) * bezier(self.curve, k)
    }
    /// A transition to a new value starts from where it is now.
    fn set(&mut self, to: f32, now: Instant) -> bool {
        if (to - self.to).abs() < 1e-4 {
            return false;
        }
        self.from = self.at(now);
        self.to = to;
        self.start = now;
        true
    }
    fn busy(&self, now: Instant) -> bool {
        self.from != self.to && now.saturating_duration_since(self.start).as_secs_f32() * 1000.0 < self.ms
    }
}

/// What config.toml says about the dock and its tasks widget, in px.
#[derive(Clone)]
pub struct DockLook {
    rest_h: f32,
    open_h: f32,
    notch_h: f32,
    /// Resting as the tiny handle: no content, a narrower notch.
    tiny: bool,
    hot_left: bool,
    hot_right: bool,
    hot_size: f32,
    edge_px: f32,
    dim_minimized: bool,
    mono: bool,
    accent_details: bool,
    magnify: bool,
    glide: bool,
    pub hide_taskbar: bool,
}

impl DockLook {
    /// How tall the dock gets.
    pub fn open_height(&self) -> f32 {
        self.open_h
    }
}

fn at<'a>(cfg: &'a toml::Value, path: &[&str]) -> Option<&'a toml::Value> {
    path.iter().try_fold(cfg, |v, p| v.get(p))
}

pub fn read(cfg: &toml::Value, scale: f32) -> DockLook {
    let num = |p: &[&str], d: f32| at(cfg, p).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).map_or(d, |v| v as f32);
    let flag = |p: &[&str], d: bool| at(cfg, p).and_then(|v| v.as_bool()).unwrap_or(d);
    let text = |p: &[&str], d: &str| at(cfg, p).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    // [sizes] (Kasumi's defaults: 12 / 30 / 45 / 60).
    let size = |name: &str| match name {
        "tiny" => num(&["sizes", "tiny"], 12.0),
        "medium" => num(&["sizes", "medium"], 45.0),
        "large" => num(&["sizes", "large"], 60.0),
        _ => num(&["sizes", "small"], 30.0),
    };
    let rest_name = text(&["surfaces", "bottom", "rest"], "small");
    let rest = size(&rest_name);
    let open = size(&text(&["surfaces", "bottom", "open"], "medium")).max(rest);
    // A widget's own setting, else the global one (as Kasumi's accentFor / animFor).
    let own = |key: &str, global: &[&str]| at(cfg, &["widgets", "tasks", key]).and_then(|v| v.as_bool()).unwrap_or_else(|| flag(global, false));
    DockLook {
        rest_h: rest * scale,
        open_h: open * scale,
        notch_h: num(&["geometry", "notch_height"], 30.0) * scale,
        tiny: rest_name == "tiny",
        hot_left: flag(&["surfaces", "bottom", "hot_left"], false),
        hot_right: flag(&["surfaces", "bottom", "hot_right"], false),
        hot_size: num(&["surfaces", "bottom", "hot_corner_size"], 12.0) * scale,
        edge_px: num(&["timing", "edge_px"], 8.0) * scale,
        dim_minimized: flag(&["widgets", "tasks", "dim_minimized"], false),
        mono: text(&["widgets", "tasks", "icons"], "color") != "color",
        accent_details: own("accent_details", &["theme", "accent_details"]),
        magnify: own("anim_magnify", &["animation", "magnify"]),
        glide: own("anim_glide", &["animation", "marker_glide"]),
        hide_taskbar: flag(&["shell", "hide_taskbar"], false),
    }
}

// ---------------------------------------------------------------------------
// The open windows (providers/tasks.rs).

#[derive(Clone, PartialEq)]
pub struct Win {
    hwnd: isize,
    app: String,
    active: bool,
    minimized: bool,
    /// Front to back: 0 is the most recent.
    z: usize,
    monitor: isize,
}

#[derive(Default)]
struct Tasks {
    /// App keys in the order they first appeared (taskbar order).
    order: Vec<String>,
    /// Each app's AppUserModelID and program, for its icon.
    sources: HashMap<String, (Option<String>, String)>,
    /// Window handles in the order they first appeared.
    seen: Vec<isize>,
    /// Listed windows that got cloaked, and when.
    cloaked_since: HashMap<isize, Instant>,
    /// When each app last had a window: it keeps its place for a moment after.
    last_window: HashMap<String, Instant>,
    windows: Vec<Win>,
}

fn top_windows() -> Vec<HWND> {
    unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
        (*(data.0 as *mut Vec<HWND>)).push(h);
        BOOL(1)
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(collect), LPARAM(&mut all as *mut _ as isize));
    }
    all
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 128];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

fn process_path(h: HWND) -> Option<String> {
    let mut pid = 0u32;
    unsafe {
        GetWindowThreadProcessId(h, Some(&mut pid));
        let p = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len).is_ok();
        let _ = CloseHandle(p);
        ok.then(|| String::from_utf16_lossy(&buf[..len as usize]))
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

/// The monitor a window is on; a minimized one, where it will be restored to.
fn monitor_of(h: HWND) -> isize {
    unsafe {
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
        mon.0 as isize
    }
}

/// Alt+Tab rules: visible, titled, not a tool window, and unowned unless it asks
/// for a taskbar button.
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
    }
    !matches!(class_name(h).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
}

fn is_cloaked(h: HWND) -> bool {
    let mut cloaked = 0u32;
    unsafe {
        let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
    }
    cloaked != 0
}

impl Tasks {
    /// Lists the windows again. True when one is cloaked for a moment (a minimize or
    /// restore animation): look again soon.
    fn scan(&mut self) -> bool {
        let all = top_windows();
        let own = unsafe { GetCurrentProcessId() };
        let fg = unsafe { GetForegroundWindow() };
        let now = Instant::now();
        // Windows are cloaked for a moment while they minimize or restore: one already
        // listed stays listed through that (a window on another desktop goes after it).
        let mut cloak_wait = false;
        let listed: Vec<HWND> = all
            .iter()
            .copied()
            .filter(|&h| is_task(h, own))
            .filter(|&h| {
                let id = h.0 as isize;
                if !is_cloaked(h) {
                    self.cloaked_since.remove(&id);
                    return true;
                }
                if !self.seen.contains(&id) {
                    return false;
                }
                let since = *self.cloaked_since.entry(id).or_insert(now);
                let keep = since.elapsed() < Duration::from_millis(1500);
                cloak_wait |= keep;
                keep
            })
            .collect();
        self.cloaked_since.retain(|id, _| all.iter().any(|h| h.0 as isize == *id));
        let mut windows = Vec::new();
        for (z, &h) in listed.iter().enumerate() {
            let exe = process_path(h).unwrap_or_default();
            let aumid = aumid_of(h);
            let key = aumid.clone().unwrap_or_else(|| exe.to_lowercase());
            if key.is_empty() {
                continue;
            }
            self.sources.entry(key.clone()).or_insert((aumid, exe));
            if !self.order.contains(&key) {
                self.order.push(key.clone());
            }
            let id = h.0 as isize;
            if !self.seen.contains(&id) {
                self.seen.push(id);
            }
            windows.push(Win { hwnd: id, app: key, active: h == fg, minimized: unsafe { IsIconic(h).as_bool() }, z, monitor: monitor_of(h) });
        }
        // Forget closed windows; an app without windows keeps its place for a few
        // seconds (a window being replaced), then goes (back at the end when reopened).
        self.seen.retain(|id| windows.iter().any(|w| w.hwnd == *id));
        for w in &windows {
            self.last_window.insert(w.app.clone(), now);
        }
        let recent = self.last_window.clone();
        self.order.retain(|k| recent.get(k).is_some_and(|t| t.elapsed() < Duration::from_secs(3)));
        self.last_window.retain(|_, t| t.elapsed() < Duration::from_secs(60));
        let seen = &self.seen;
        windows.sort_by_key(|w| seen.iter().position(|id| *id == w.hwnd));
        self.windows = windows;
        cloak_wait
    }
}

fn shell_item(path: &str) -> Option<IShellItem> {
    if path.is_empty() {
        return None;
    }
    unsafe { SHCreateItemFromParsingName(&HSTRING::from(path), None).ok() }
}

/// A shell item's icon as premultiplied BGRA (Direct2D's own format).
fn item_icon(item: &IShellItem) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let bmp = factory.GetImage(SIZE { cx: ICON_PX, cy: ICON_PX }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let px = bitmap_bgra(bmp);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let (w, h, mut px) = px?;
        // Shell images are premultiplied; ones without any alpha are opaque.
        if !px.chunks_exact(4).any(|p| p[3] != 0) {
            px.chunks_exact_mut(4).for_each(|p| p[3] = 255);
        }
        Some((w, h, px))
    }
}

/// A bitmap's pixels as 32-bit BGRA rows, top-down.
fn bitmap_bgra(bmp: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
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
                biHeight: -(h as i32),
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

/// An app's icon: its Start menu entry's when it has an AppUserModelID, else its program's.
fn app_icon(aumid: Option<&str>, exe: &str) -> Option<(u32, u32, Vec<u8>)> {
    if let Some(item) = aumid.and_then(|a| shell_item(&format!("shell:AppsFolder\\{a}"))) {
        return item_icon(&item).or_else(|| shell_item(exe).and_then(|i| item_icon(&i)));
    }
    shell_item(exe).and_then(|i| item_icon(&i))
}

/// `--icons AUMID EXE ...` (AUMID "-" for none): the icon helper. Each app's icon
/// goes to stdout as width and height (u32, little-endian; 0 0 for none) and its
/// premultiplied BGRA rows.
pub fn icons_main(args: &[String]) {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    let mut out = std::io::stdout().lock();
    for pair in args.chunks(2) {
        let aumid = (pair[0] != "-").then_some(pair[0].as_str());
        let (w, h, px) = app_icon(aumid, pair.get(1).map_or("", |e| e.as_str())).unwrap_or((0, 0, Vec::new()));
        let _ = out.write_all(&w.to_le_bytes()).and_then(|_| out.write_all(&h.to_le_bytes())).and_then(|_| out.write_all(&px));
    }
}

/// Runs the icon helper for some apps (key, AUMID, program) and reads its answers.
fn fetch_icons(wanted: &[(String, Option<String>, String)]) -> Vec<(String, Option<Icon>)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = match std::env::current_exe() {
        Ok(exe) => std::process::Command::new(exe),
        Err(_) => return wanted.iter().map(|(k, _, _)| (k.clone(), None)).collect(),
    };
    cmd.arg("--icons").creation_flags(CREATE_NO_WINDOW).stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    for (_, aumid, exe) in wanted {
        cmd.arg(aumid.as_deref().unwrap_or("-")).arg(exe);
    }
    let data = cmd.output().map(|o| o.stdout).unwrap_or_default();
    let mut at = 0usize;
    let mut next = |n: usize| {
        let part = data.get(at..at + n);
        at += n;
        part
    };
    wanted
        .iter()
        .map(|(k, _, _)| {
            let size = next(8).map(|b| (u32::from_le_bytes([b[0], b[1], b[2], b[3]]), u32::from_le_bytes([b[4], b[5], b[6], b[7]])));
            let icon = size
                .filter(|&(w, h)| w > 0 && h > 0 && w <= 512 && h <= 512)
                .and_then(|(w, h)| next((w * h * 4) as usize).map(|px| (w, h, px.to_vec())));
            (k.clone(), icon)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Actions (tasks.rs commands).

pub enum Action {
    /// Front a window, or minimize it if it already is in front (`toggle`).
    Activate(isize, bool),
    Close(isize),
}

/// Brings a window to the front. The click on our window is the last input, so we
/// may set the foreground; the Alt tap covers the cases where Windows refuses.
fn activate(h: HWND) {
    unsafe {
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        // Windows only lets the app in front hand the foreground over: borrow its
        // input queue for the switch.
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
    }
}

pub fn run(action: Action) {
    unsafe {
        match action {
            Action::Activate(id, toggle) => {
                let h = HWND(id as *mut _);
                if !IsWindow(Some(h)).as_bool() {
                    return;
                }
                if toggle && GetForegroundWindow() == h && !IsIconic(h).as_bool() {
                    let _ = ShowWindow(h, SW_MINIMIZE);
                } else {
                    activate(h);
                }
            }
            Action::Close(id) => {
                let _ = PostMessageW(Some(HWND(id as *mut _)), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
        }
    }
    schedule();
}

/// The window a click on an app fronts: cycle when one of its windows is in front.
fn next_window(wins: &[Win]) -> Option<(isize, bool)> {
    match wins.len() {
        0 => None,
        1 => Some((wins[0].hwnd, true)),
        n => match wins.iter().position(|w| w.active) {
            Some(i) => Some((wins[(i + 1) % n].hwnd, false)),
            None => wins.iter().min_by_key(|w| w.z).map(|w| (w.hwnd, false)),
        },
    }
}

/// A rescan soon (window events come in bursts: one per 120 ms).
fn schedule() {
    let h = DOCK_HWND.load(Ordering::Relaxed);
    if h != 0 && !SCHEDULED.swap(true, Ordering::Relaxed) {
        unsafe { SetTimer(Some(HWND(h as *mut _)), TASKS, 120, None) };
    }
}

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, hwnd: HWND, object: i32, child: i32, _: u32, _: u32) {
    // Explorer shows its taskbar again now and then: hidden again at once.
    let tb = TASKBAR.load(Ordering::Relaxed);
    if event == EVENT_OBJECT_SHOW && tb != 0 && hwnd.0 as isize == tb {
        let _ = ShowWindowAsync(hwnd, SW_HIDE);
    }
    if event == EVENT_SYSTEM_FOREGROUND || (object == OBJID_WINDOW.0 && child == 0 && !hwnd.is_invalid()) {
        schedule();
    }
}

/// The taskbar back, if the test hid it.
pub fn restore_taskbar() {
    let tb = TASKBAR.swap(0, Ordering::Relaxed);
    if tb != 0 {
        unsafe {
            let _ = ShowWindowAsync(HWND(tb as *mut _), SW_SHOWNA);
        }
    }
}

// ---------------------------------------------------------------------------
// The dock and its widget (widgets/tasks.ts, main.ts's dock parts).

/// One app's button.
struct Button {
    key: String,
    /// Its windows on this monitor.
    wins: Vec<Win>,
    /// Gliding from where it was to its new place (px), growing in (0 to 1).
    slide: Trans,
    grow: Trans,
    /// Magnify scale and the hover highlight (0 to 1).
    mag: Trans,
    hover: Trans,
}

/// The line under the app in front, flowing to the next one ([animation] marker_glide).
struct Glide {
    to: String,
    /// Where it starts: the old app's line centre, in the widget.
    from_x: f32,
    start: Instant,
    ms: f32,
}

pub struct Dock {
    pub hwnd: HWND,
    pub memdc: HDC,
    pub bits: *mut u8,
    pub size: SIZE,
    pub pos: POINT,
    monitor: RECT,
    hmon: isize,
    s: f32,
    g: Geometry,
    look: DockLook,
    accent: D2D1_COLOR_F,
    pub open: Spring,
    /// The notch's left edge and width at rest and open (they glide when apps come and go).
    xs: Spring,
    ws: Spring,
    xo: Spring,
    wo: Spring,
    tasks: Tasks,
    buttons: Vec<Button>,
    /// Each app's icon at full size, half, a quarter (see `mips`).
    icons: HashMap<String, Option<Vec<ID2D1Bitmap>>>,
    /// Apps whose icons the helper is fetching.
    pending: HashSet<String>,
    /// Wait for icons (snapshots) instead of fetching them in the background.
    wait_icons: bool,
    active: Option<String>,
    glide: Option<Glide>,
    away_since: Option<Instant>,
    /// The cursor's last spot, and whether it was over the widget (magnify follows moves).
    last_pt: POINT,
    over: bool,
    pub dirty: bool,
}

/// Where a dock goes and what it draws with (Dock::new's inputs).
pub struct Setup {
    pub hwnd: HWND,
    pub memdc: HDC,
    pub bits: *mut u8,
    pub size: SIZE,
    pub pos: POINT,
    pub monitor: RECT,
    pub hmon: HMONITOR,
    pub s: f32,
    pub g: Geometry,
    pub look: DockLook,
    pub accent: D2D1_COLOR_F,
    pub expand: Spring,
    pub slide: Spring,
    /// Hide the main monitor's Windows taskbar while the test runs.
    pub hide_taskbar: bool,
    pub wait_icons: bool,
}

impl Dock {
    pub fn new(u: Setup, rt: &ID2D1DCRenderTarget) -> Self {
        DOCK_HWND.store(u.hwnd.0 as isize, Ordering::Relaxed);
        unsafe {
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
            SetTimer(Some(u.hwnd), TASKS_ANYWAY, 3000, None);
            if u.hide_taskbar {
                if let Ok(tb) = FindWindowW(w!("Shell_TrayWnd"), None) {
                    if IsWindowVisible(tb).as_bool() {
                        TASKBAR.store(tb.0 as isize, Ordering::Relaxed);
                        let _ = ShowWindowAsync(tb, SW_HIDE);
                    }
                }
            }
        }
        let mut d = Dock {
            hwnd: u.hwnd,
            memdc: u.memdc,
            bits: u.bits,
            size: u.size,
            pos: u.pos,
            monitor: u.monitor,
            hmon: u.hmon.0 as isize,
            s: u.s,
            g: u.g,
            look: u.look,
            accent: u.accent,
            open: u.expand,
            xs: u.slide.clone(),
            ws: u.slide.clone(),
            xo: u.slide.clone(),
            wo: u.slide,
            tasks: Tasks::default(),
            buttons: Vec::new(),
            icons: HashMap::new(),
            pending: HashSet::new(),
            wait_icons: u.wait_icons,
            active: None,
            glide: None,
            away_since: None,
            last_pt: POINT { x: i32::MIN, y: i32::MIN },
            over: false,
            dirty: true,
        };
        d.refresh(rt);
        d.place(true);
        d
    }

    fn width(&self) -> f32 {
        self.size.cx as f32
    }

    /// The dock's openness, 0 to 1 (docks don't overshoot).
    fn e(&self) -> f32 {
        self.open.value.clamp(0.0, 1.0)
    }

    /// The surface height now (Kasumi's --h): what the content sizes itself by.
    fn h(&self) -> f32 {
        lerp(self.look.rest_h, self.look.open_h, self.e())
    }

    /// The tasks widget's width at a surface height: buttons 0.76 h wide, 0.12 h apart.
    fn content(&self, h: f32) -> f32 {
        let n = self.buttons.len() as f32;
        if n == 0.0 {
            0.0
        } else {
            n * 0.76 * h + (n - 1.0) * 0.12 * h
        }
    }

    /// A notch's width around content: both (scaled) round-offs and the padding.
    fn notch_width(&self, h: f32) -> f32 {
        self.content(h) + 2.0 * self.g.notch_corner_radius * (h / self.look.notch_h) + 2.0 * NOTCH_PAD * self.s
    }

    /// Where the middle notch goes at rest and open, for the current buttons.
    fn place(&mut self, snap: bool) {
        let w = self.width();
        let open_w = self.notch_width(self.look.open_h);
        let rest_w = if self.look.tiny { (open_w * 0.6).max(48.0 * self.s) } else { self.notch_width(self.look.rest_h) };
        for (sp, v) in [(&mut self.xs, (w - rest_w) / 2.0), (&mut self.ws, rest_w), (&mut self.xo, (w - open_w) / 2.0), (&mut self.wo, open_w)] {
            sp.target = v;
            if snap {
                sp.value = v;
                sp.velocity = 0.0;
            }
        }
    }

    /// The notch now: left edge, width, height.
    fn notch(&self) -> (f32, f32, f32) {
        let e = self.e();
        (lerp(self.xs.value, self.xo.value, e), lerp(self.ws.value, self.wo.value, e), self.h())
    }

    /// The widget's box (window px): left, top, and the buttons' height.
    fn root(&self) -> (f32, f32, f32) {
        let (x, w, h) = self.notch();
        let row = h.round();
        let bh = 0.82 * h;
        (x + (w - self.content(h)) / 2.0, (self.size.cy as f32 - row + (row - bh) / 2.0).round(), bh)
    }

    /// Each button's box (window px), where it's drawn now.
    fn boxes(&self, now: Instant) -> Vec<D2D_RECT_F> {
        let h = self.h();
        let (left, top, bh) = self.root();
        self.buttons
            .iter()
            .enumerate()
            .map(|(i, b)| rect(left + i as f32 * 0.88 * h + b.slide.at(now), top, 0.76 * h, bh))
            .collect()
    }

    /// Rescans the windows; true if what the dock shows changed (port of draw()).
    pub fn refresh(&mut self, rt: &ID2D1DCRenderTarget) -> bool {
        SCHEDULED.store(false, Ordering::Relaxed);
        let _ = unsafe { KillTimer(Some(self.hwnd), TASKS) };
        if self.tasks.scan() {
            schedule();
        }
        let mine: Vec<&Win> = self.tasks.windows.iter().filter(|w| w.monitor == self.hmon).collect();
        let apps: Vec<(String, Vec<Win>)> = self
            .tasks
            .order
            .iter()
            .map(|k| (k.clone(), mine.iter().filter(|w| &w.app == k).map(|w| (*w).clone()).collect::<Vec<_>>()))
            .filter(|(_, wins)| !wins.is_empty())
            .collect();
        let same = apps.len() == self.buttons.len() && apps.iter().zip(&self.buttons).all(|((k, wins), b)| *k == b.key && *wins == b.wins);
        if same {
            return false;
        }
        // Icons for apps seen for the first time (until they come, a placeholder).
        let wanted: Vec<(String, Option<String>, String)> = apps
            .iter()
            .filter(|(k, _)| !self.icons.contains_key(k) && !self.pending.contains(k))
            .map(|(k, _)| {
                let (aumid, exe) = self.tasks.sources.get(k).cloned().unwrap_or_default();
                (k.clone(), aumid, exe)
            })
            .collect();
        if !wanted.is_empty() {
            self.pending.extend(wanted.iter().map(|(k, _, _)| k.clone()));
            if self.wait_icons {
                ICONS_IN.lock().unwrap().extend(fetch_icons(&wanted));
                self.receive_icons(rt);
            } else {
                let hwnd = self.hwnd.0 as isize;
                std::thread::spawn(move || {
                    let got = fetch_icons(&wanted);
                    ICONS_IN.lock().unwrap().extend(got);
                    unsafe {
                        let _ = PostMessageW(Some(HWND(hwnd as *mut _)), WM_ICONS, WPARAM(0), LPARAM(0));
                    }
                });
            }
        }
        let now = Instant::now();
        let h = self.h();
        // Where each button is drawn now (so moves glide instead of jump), in the dock.
        let (root_was, _, _) = self.root();
        let was: HashMap<String, f32> = self.buttons.iter().enumerate().map(|(i, b)| (b.key.clone(), root_was + i as f32 * 0.88 * h + b.slide.at(now))).collect();
        let mut old: HashMap<String, Button> = self.buttons.drain(..).map(|b| (b.key.clone(), b)).collect();
        for (key, wins) in apps {
            let b = match old.remove(&key) {
                Some(mut b) => {
                    b.wins = wins;
                    b
                }
                None => Button {
                    key,
                    wins,
                    slide: Trans::at_rest(0.0, 200.0, EASE),
                    grow: Trans::at_rest(1.0, 180.0, EASE),
                    mag: Trans::at_rest(1.0, 90.0, CSS_EASE_OUT),
                    hover: Trans::at_rest(0.0, 120.0, CSS_EASE),
                },
            };
            self.buttons.push(b);
        }
        // Moved buttons glide to their new place; new ones grow in (not the first ones).
        let (root_now, _, _) = self.root();
        for (i, b) in self.buttons.iter_mut().enumerate() {
            if let Some(before) = was.get(&b.key) {
                let dx = before - (root_now + i as f32 * 0.88 * h);
                if dx.abs() > 0.5 {
                    b.slide = Trans::run(dx, 0.0, 200.0, EASE, now);
                }
            } else if !was.is_empty() {
                b.grow = Trans::run(0.0, 1.0, 180.0, EASE, now);
            }
        }
        // The line under the app in front flows over to the new one.
        let now_active = self.buttons.iter().find(|b| b.wins.iter().any(|w| w.active)).map(|b| b.key.clone());
        if self.look.glide {
            if let (Some(last), Some(next)) = (&self.active, &now_active) {
                let from = self.buttons.iter().position(|b| &b.key == last);
                let to = self.buttons.iter().position(|b| &b.key == next);
                if let (Some(f), Some(t)) = (from, to) {
                    if f != t {
                        let dist = (t as f32 - f as f32).abs() * 0.88 * h / self.s;
                        self.glide = Some(Glide { to: next.clone(), from_x: f as f32 * 0.88 * h + 0.38 * h, start: now, ms: (170.0 + dist * 0.35).min(320.0) });
                    }
                }
            }
        }
        self.active = now_active;
        self.place(false);
        self.dirty = true;
        true
    }

    /// Icons the helper sent: into bitmaps. True if any came.
    pub fn receive_icons(&mut self, rt: &ID2D1DCRenderTarget) -> bool {
        let got: Vec<(String, Option<Icon>)> = ICONS_IN.lock().unwrap().drain(..).collect();
        for (key, icon) in &got {
            let bitmap = icon.as_ref().and_then(|(w, h, px)| self.mips(rt, *w, *h, px.clone()).ok());
            self.pending.remove(key);
            self.icons.insert(key.clone(), bitmap);
        }
        if !got.is_empty() {
            self.dirty = true;
        }
        !got.is_empty()
    }

    /// An icon as bitmaps of halving sizes (64, 32, 16 px): drawn from the smallest
    /// one at least as big as it shows, with linear filtering, as WebView2 scales
    /// pictures down. (Direct2D's own high-quality scaling loads a shader compiler.)
    fn mips(&self, rt: &ID2D1DCRenderTarget, mut w: u32, mut h: u32, mut px: Vec<u8>) -> Result<Vec<ID2D1Bitmap>> {
        if self.look.mono {
            // CSS grayscale(1) (premultiplied values stay premultiplied).
            for p in px.chunks_exact_mut(4) {
                let y = (0.0722 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.2126 * p[2] as f32).round().min(255.0) as u8;
                p[0] = y;
                p[1] = y;
                p[2] = y;
            }
        }
        let props = D2D1_BITMAP_PROPERTIES {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
        };
        let mut levels = Vec::new();
        loop {
            levels.push(unsafe { rt.CreateBitmap(D2D_SIZE_U { width: w, height: h }, Some(px.as_ptr() as *const _), w * 4, &props)? });
            if w < 32 || h < 32 || levels.len() == 3 {
                return Ok(levels);
            }
            // Half size: each pixel the average of four (premultiplied, so plain means).
            let (hw, hh) = (w / 2, h / 2);
            let mut half = vec![0u8; (hw * hh * 4) as usize];
            for y in 0..hh as usize {
                for x in 0..hw as usize {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| px[((2 * y + dy) * w as usize + 2 * x + dx) * 4 + c] as u32;
                        half[(y * hw as usize + x) * 4 + c] = ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                    }
                }
            }
            (w, h, px) = (hw, hh, half);
        }
    }

    /// Springs one step on; true once all of them have settled.
    pub fn step(&mut self, dt: f32) -> bool {
        let mut done = true;
        for sp in [&mut self.open, &mut self.xs, &mut self.ws, &mut self.xo, &mut self.wo] {
            sp.step(dt);
            done &= sp.snap_if_settled();
        }
        done
    }

    pub fn springs_moving(&self) -> bool {
        [&self.open, &self.xs, &self.ws, &self.xo, &self.wo].iter().any(|sp| sp.value != sp.target || sp.velocity != 0.0)
    }

    /// Timed animations still running (glides, grows, magnify, hover fades, the marker).
    pub fn timed(&self, now: Instant) -> bool {
        self.glide.as_ref().is_some_and(|g| now.saturating_duration_since(g.start).as_secs_f32() * 1000.0 < g.ms)
            || self.buttons.iter().any(|b| b.slide.busy(now) || b.grow.busy(now) || b.mag.busy(now) || b.hover.busy(now))
    }

    /// Near the bottom edge, or open: the cursor is polled fast.
    pub fn near(&self, pt: POINT) -> bool {
        let m = self.monitor;
        let on = pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom;
        (on && ((m.bottom - 1 - pt.y) as f32) < self.look.open_h * 3.0) || self.open.target > 0.5
    }

    /// Like Kasumi's hover for a dock: its notch's trigger (80% of its visible width,
    /// up to its rest height, or the edge below it) or a hot corner opens it; leaving
    /// the open notch closes it after `grace` ms. Open: the hovered app and magnify.
    /// True if something changed.
    pub fn hover(&mut self, pt: POINT, grace: u128) -> bool {
        let m = self.monitor;
        let w = self.width();
        let on = pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom;
        let lx = (pt.x - m.left) as f32;
        let ly = (m.bottom - 1 - pt.y) as f32;
        let l = &self.look;
        let at_edge = on && ly < l.edge_px.max(1.0);
        let hot = on && ly >= 0.0 && ly < l.hot_size && ((l.hot_left && lx < l.hot_size) || (l.hot_right && lx >= w - l.hot_size));
        let visible = self.ws.target + 2.0 * self.g.concave_radius * (l.rest_h / l.notch_h);
        let tw = 0.8 * visible;
        let tx = self.xs.target + (self.ws.target - tw) / 2.0;
        let trigger = on && (hot || (lx >= tx && lx < tx + tw && (at_edge || (ly >= 0.0 && ly < l.rest_h))));
        let r = self.g.concave_radius * (l.open_h / l.notch_h);
        let in_bar = on && self.open.target > 0.5 && lx >= self.xo.target - r && lx < self.xo.target + self.wo.target + r && ly >= 0.0 && ly < l.open_h;
        let was = self.open.target;
        if trigger || in_bar {
            self.away_since = None;
            self.open.target = 1.0;
        } else if self.open.target > 0.5 {
            let since = *self.away_since.get_or_insert_with(Instant::now);
            if since.elapsed().as_millis() >= grace {
                self.open.target = 0.0;
            }
        }
        let mut changed = self.open.target != was;

        // Open, the dock takes the mouse: the app under it and magnify.
        let now = Instant::now();
        let (wx, wy) = ((pt.x - self.pos.x) as f32, (pt.y - self.pos.y) as f32);
        let interactive = self.open.target > 0.5 && on;
        let boxes = self.boxes(now);
        let hit = |r: &D2D_RECT_F| wx >= r.left && wx < r.right && wy >= r.top && wy < r.bottom;
        for (b, r) in self.buttons.iter_mut().zip(&boxes) {
            changed |= b.hover.set(if interactive && hit(r) { 1.0 } else { 0.0 }, now);
        }
        if self.look.magnify {
            let (left, top, bh) = self.root();
            let over = interactive && wx >= left && wx < left + self.content(self.h()) && wy >= top && wy < top + bh;
            let moved = pt.x != self.last_pt.x || pt.y != self.last_pt.y;
            if over && moved {
                // Icons grow as the cursor passes over them (mousemove).
                for (b, r) in self.buttons.iter_mut().zip(&boxes) {
                    let d = (wx - (r.left + r.right) / 2.0).abs();
                    changed |= b.mag.set(1.0 + 0.35 * (1.0 - d / (70.0 * self.s)).max(0.0), now);
                }
            } else if !over && self.over {
                for b in &mut self.buttons {
                    changed |= b.mag.set(1.0, now);
                }
            }
            self.over = over;
        }
        self.last_pt = pt;
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// A click (window px): left fronts / minimizes / cycles the app, middle closes
    /// its front window.
    pub fn click(&self, x: f32, y: f32, middle: bool) -> Option<Action> {
        let i = self.boxes(Instant::now()).iter().position(|r| x >= r.left && x < r.right && y >= r.top && y < r.bottom)?;
        let wins = &self.buttons[i].wins;
        if middle {
            return wins.iter().min_by_key(|w| w.z).map(|w| Action::Close(w.hwnd));
        }
        next_window(wins).map(|(h, toggle)| Action::Activate(h, toggle))
    }

    /// For snapshots: the first app hovered and magnified, as if the cursor were on it.
    pub fn pose_hover(&mut self) {
        let boxes = self.boxes(Instant::now());
        let Some(first) = boxes.first() else { return };
        let x = (first.left + first.right) / 2.0;
        for (i, (b, r)) in self.buttons.iter_mut().zip(&boxes).enumerate() {
            let d = (x - (r.left + r.right) / 2.0).abs();
            let mag = if self.look.magnify { 1.0 + 0.35 * (1.0 - d / (70.0 * self.s)).max(0.0) } else { 1.0 };
            b.hover = Trans::at_rest(if i == 0 { 1.0 } else { 0.0 }, 120.0, CSS_EASE);
            b.mag = Trans::at_rest(mag, 90.0, CSS_EASE_OUT);
        }
    }

    /// One frame: the notch (the shape is drawn from the bottom edge up, flipped;
    /// nothing with content is), and the widget clipped to it.
    pub fn render(&mut self, d2d: &ID2D1Factory, rt: &ID2D1DCRenderTarget, ink: &ID2D1SolidColorBrush, paint: &ID2D1SolidColorBrush) -> Result<()> {
        let now = Instant::now();
        if self.glide.as_ref().is_some_and(|g| now.saturating_duration_since(g.start).as_secs_f32() * 1000.0 >= g.ms) {
            self.glide = None;
        }
        let w = self.width();
        let hw = self.size.cy as f32;
        let (x, nw, h) = self.notch();
        let bump = Bump { x, width: nw, height: h, panel_width: 0.0, panel_height: 0.0, p: 0.0, in_hull: true, scale: h / self.look.notch_h };
        let frame = geometry::compute(&self.g, w, 0.0, 0.0, &[bump]);
        let flip = Matrix3x2 { M11: 1.0, M12: 0.0, M21: 0.0, M22: -1.0, M31: 0.0, M32: hw };
        unsafe {
            rt.BindDC(self.memdc, &RECT { left: 0, top: 0, right: self.size.cx, bottom: self.size.cy })?;
            rt.BeginDraw();
            rt.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            let shape = geometry_of(d2d, &frame.path)?;
            rt.SetTransform(&flip);
            rt.FillGeometry(&shape, ink, None::<&ID2D1Brush>);
            if !frame.corners.is_empty() {
                rt.FillGeometry(&geometry_of(d2d, &frame.corners)?, ink, None::<&ID2D1Brush>);
            }
            rt.SetTransform(&Matrix3x2::identity());
            // The content fades out below a 16 px row (the tiny handle shows none).
            let fits = ((h / self.s - 16.0) / 10.0).clamp(0.0, 1.0);
            if fits > 0.0 && !self.buttons.is_empty() {
                let mut layer = D2D1_LAYER_PARAMETERS {
                    contentBounds: D2D_RECT_F { left: -1e6, top: -1e6, right: 1e6, bottom: 1e6 },
                    geometricMask: std::mem::ManuallyDrop::new(Some(shape.cast()?)),
                    maskAntialiasMode: D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
                    maskTransform: flip,
                    opacity: fits,
                    opacityBrush: std::mem::ManuallyDrop::new(None),
                    layerOptions: D2D1_LAYER_OPTIONS_NONE,
                };
                rt.PushLayer(&layer, None);
                self.draw_tasks(rt, paint, now, h);
                rt.PopLayer();
                std::mem::ManuallyDrop::drop(&mut layer.geometricMask);
            }
            rt.EndDraw(None, None)?;
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            UpdateLayeredWindow(self.hwnd, None, Some(&self.pos), Some(&self.size), Some(self.memdc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
        }
        self.dirty = false;
        Ok(())
    }

    /// The buttons: hover highlight, icon, window dots / the line under the app in front.
    fn draw_tasks(&self, rt: &ID2D1DCRenderTarget, paint: &ID2D1SolidColorBrush, now: Instant, h: f32) {
        let s = self.s;
        let (root_left, root_top, bh) = self.root();
        let line_color = if self.look.accent_details { self.accent } else { white(1.0) };
        let hover_color = if self.look.accent_details { D2D1_COLOR_F { a: 0.3, ..self.accent } } else { white(0.1) };
        let fill = |r: D2D_RECT_F, radius: f32, c: D2D1_COLOR_F| unsafe {
            paint.SetColor(&c);
            rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: r, radiusX: radius, radiusY: radius }, paint);
        };
        let gliding = self.glide.as_ref().map(|g| g.to.as_str());
        for (r, b) in self.boxes(now).iter().zip(&self.buttons) {
            let (cx, cy) = ((r.left + r.right) / 2.0, (r.top + r.bottom) / 2.0);
            // Growing in: the whole button scales up from 0.6 and fades in.
            let g = b.grow.at(now);
            let k = 0.6 + 0.4 * g;
            unsafe { rt.SetTransform(&Matrix3x2 { M11: k, M12: 0.0, M21: 0.0, M22: k, M31: cx * (1.0 - k), M32: cy * (1.0 - k) }) };
            let hv = b.hover.at(now);
            if hv > 0.0 {
                fill(*r, 0.2 * h, D2D1_COLOR_F { a: hover_color.a * hv * g, ..hover_color });
            }
            // The icon, 0.6 h, magnified from its bottom edge.
            let size = 0.6 * h;
            let mag = if self.look.magnify { b.mag.at(now) } else { 1.0 };
            let (ix, iy) = ((r.left + 0.08 * h).round(), (r.top + (bh - size) / 2.0).round());
            let big = size * mag;
            let dest = rect(ix + size / 2.0 - big / 2.0, iy + size - big, big, big);
            let minimized = b.wins.iter().all(|w| w.minimized);
            let alpha = g * if self.look.dim_minimized && minimized { 0.55 } else { 1.0 };
            match self.icons.get(&b.key).and_then(|i| i.as_ref()) {
                Some(levels) => unsafe {
                    let shown = big * k;
                    let bmp = levels.iter().rev().find(|l| l.GetPixelSize().width as f32 >= shown).unwrap_or(&levels[0]);
                    rt.DrawBitmap(bmp, Some(&dest), alpha, D2D1_BITMAP_INTERPOLATION_MODE_LINEAR, None);
                },
                None => fill(dest, 0.25 * big, white(0.15 * g)),
            }
            // A dot per window (up to 3); the app in front: one 10 px line.
            let bottom = (r.bottom - 1.0 * s).round();
            if b.wins.iter().any(|w| w.active) {
                if gliding != Some(b.key.as_str()) {
                    fill(rect(cx - 5.0 * s, bottom - 3.0 * s, 10.0 * s, 3.0 * s), 1.5 * s, D2D1_COLOR_F { a: line_color.a * g, ..line_color });
                }
            } else {
                let n = b.wins.len().min(3);
                let total = (5 * n) as f32 * s - 2.0 * s;
                for i in 0..n {
                    fill(rect(cx - total / 2.0 + i as f32 * 5.0 * s, bottom - 3.0 * s, 3.0 * s, 3.0 * s), 1.5 * s, white(0.45 * g));
                }
            }
        }
        unsafe { rt.SetTransform(&Matrix3x2::identity()) };
        // The marker on its way: the front edge runs ahead (the line stretches and
        // thins), the back edge follows and catches up at the new app.
        if let Some(gl) = &self.glide {
            let Some(to) = self.buttons.iter().position(|b| b.key == gl.to) else { return };
            let t = (now.saturating_duration_since(gl.start).as_secs_f32() * 1000.0 / gl.ms).min(1.0);
            let ease = |k: f32| if k < 0.5 { 4.0 * k * k * k } else { 1.0 - (-2.0 * k + 2.0).powi(3) / 2.0 };
            let part = |from: f32| ease(((t - from) / 0.68).clamp(0.0, 1.0));
            let (line, thick) = (10.0 * s, 3.0 * s);
            let a = gl.from_x;
            // The target is read every frame: the line lands exactly where the app's own is.
            let z = to as f32 * 0.88 * h + 0.38 * h;
            let half = if z > a { line / 2.0 } else { -line / 2.0 };
            let f = a + half + (z - a) * part(0.0);
            let bk = a - half + (z - a) * part(0.32);
            let width = (f - bk).abs().max(thick);
            // Stretched, it thins (down to 2 px), staying on the same baseline.
            let th = (thick * (line / width).sqrt()).clamp(2.0 * s, thick);
            let y = (root_top + bh - 1.0 * s).round() - th;
            fill(rect(root_left + f.min(bk), y, width, th), 1.5 * s, line_color);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::bezier;

    #[test]
    fn bezier_ends_and_linear() {
        assert_eq!(bezier((0.2, 0.8, 0.2, 1.0), 0.0), 0.0);
        assert_eq!(bezier((0.2, 0.8, 0.2, 1.0), 1.0), 1.0);
        assert!((bezier((0.0, 0.0, 1.0, 1.0), 0.3) - 0.3).abs() < 1e-3);
    }
}
