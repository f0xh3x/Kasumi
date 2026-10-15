//! Kasumi native test: one top bar on the main monitor, drawn with Direct2D and
//! DirectWrite into a layered window (per-pixel alpha, so clicks pass through
//! everything but the shapes). No WebView2. The shapes are Kasumi's own
//! (geometry.rs is a port of src/geometry.ts) and the look and motion come from
//! your config.toml. It redraws only when something changes; while something
//! moves, once per display refresh.
//!
//! Corner notches move out of the way like Kasumi ([notch.left] / [notch.right]
//! avoid_windows, offset): while a window top edge is under one (a maximized
//! window caption buttons), it slides to its offset; back in the corner when the
//! bar opens or the window goes.
//! The bar: the menu widget in the left corner (its dropdown: your [widgets.menu]
//! items), the clock in the middle (click: the calendar), a "native" label right.
//! Hover a notch or the top-left hot corner: the bar opens. Right-click a notch: quit.
//! The bottom dock (dock.rs): the tasks widget in a notch on the bottom edge.
//! `--offscreen`        drawn off screen (measuring without covering anything)
//! `--cycle`            opens and closes every 2 s by itself (measuring the animation)
//! `--seconds N`        quits after N seconds
//! `--snapshot DIR`     closed, open, menu, calendar and dock frames as BMPs, then quits
//! `--gpu`              Direct2D on the graphics card (default: software, far less memory)
//! `--icons ...`        the dock's icon helper (dock.rs runs it)
//! Starting it on screen replaces a running one (measuring runs leave it alone).

#![windows_subsystem = "windows"]

mod dock;
mod geometry;

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::Instant;

use geometry::{Bump, Cmd, Frame, Geometry, Pt};
use windows::core::{w, Result, BOOL, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_WINDING,
    D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1PathGeometry, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_ROUNDED_RECT, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, IDWriteTextLayout, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL,
    DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dwm::{DwmFlush, DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, GetMonitorInfoW, MonitorFromPoint, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLENDFUNCTION, DIB_RGB_COLORS, HDC, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, GetCurrentProcess, SetEvent, WaitForSingleObject};
use windows::Win32::Foundation::{HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, EnumWindows, GetClassNameW, GetWindowLongW, GetWindowTextLengthW, IsIconic, IsWindowVisible,
    GWL_EXSTYLE, GWL_STYLE, WM_MBUTTONUP, WS_CAPTION, WS_EX_TRANSPARENT, WS_THICKFRAME, GetCursorPos, GetMessageW, KillTimer, LoadCursorW, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassW,
    SetTimer, ShowWindow, TranslateMessage, UpdateLayeredWindow, IDC_ARROW, MSG, PM_REMOVE, SW_SHOWNOACTIVATE, SW_SHOWNORMAL, ULW_ALPHA, WM_LBUTTONUP,
    WM_QUIT, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_numerics::Vector2;

const HOVER: usize = 2; // the cursor, near the top, a notch or an open panel
const CLOCK: usize = 3; // the next minute
const MEMORY: usize = 4; // the memory readout (only while open)
const CYCLE: usize = 5; // --cycle
const QUIT: usize = 6; // --seconds
const DOCK: usize = 7; // windows under the corner notches (every 250 ms, as Kasumi)
const NOTCH_PAD: f32 = 10.0; // as main.ts
const MENU_GLYPH: &str = "\u{E80F}"; // Kasumi's menu widget (Glyph.menu)

/// Kasumi's spring (spring.ts): critically damped at least unless [animation] bounce,
/// with the open / close speed factors.
#[derive(Clone)]
struct Spring {
    value: f32,
    velocity: f32,
    target: f32,
    stiffness: f32,
    damping: f32,
    mass: f32,
    open: f32,
    close: f32,
    bounce: bool,
}

impl Spring {
    fn step(&mut self, dt: f32) {
        let f = if self.target >= 0.5 { self.open } else { self.close };
        let k = self.stiffness * f * f;
        let c = if self.bounce { self.damping * f } else { (self.damping * f).max(2.0 * (k * self.mass).sqrt()) };
        let mut left = dt;
        while left > 0.0 {
            let s = left.min(1.0 / 240.0);
            let a = (-k * (self.value - self.target) - c * self.velocity) / self.mass;
            self.velocity += a * s;
            self.value += self.velocity * s;
            left -= s;
        }
    }
    fn settled(&self) -> bool {
        (self.value - self.target).abs() < 5e-4 && self.velocity.abs() < 5e-3
    }
    fn snap_if_settled(&mut self) -> bool {
        if self.settled() {
            self.value = self.target;
            self.velocity = 0.0;
            true
        } else {
            false
        }
    }
}

struct MenuItem {
    label: String,
    icon: String,
    action: String,
    target: String,
    separator: bool,
}

/// What config.toml says, in px for this monitor.
struct Look {
    g: Geometry,
    notch_height: f32,
    padding: f32,
    font: String,
    font_size: f32,
    bar_content: String,
    grace_ms: u128,
    poll_ms: u32,
    dim: f32,
    accent: D2D1_COLOR_F,
    menu: Vec<MenuItem>,
    /// [notch.left] / [notch.right]: move aside for windows, the offset, sit in the corner.
    avoid: [bool; 2],
    offset: [f32; 2],
    hug: [bool; 2],
    dock: dock::DockLook,
}

fn hex_color(s: &str) -> Option<D2D1_COLOR_F> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(color(((n >> 16) & 0xff) as f32 / 255.0, ((n >> 8) & 0xff) as f32 / 255.0, (n & 0xff) as f32 / 255.0, 1.0))
}

/// Windows' accent colour, as Kasumi's accent_sync reads it.
fn system_accent() -> Option<D2D1_COLOR_F> {
    let mut v: u32 = 0;
    let mut size = 4u32;
    let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, w!(r"Software\Microsoft\Windows\DWM"), w!("AccentColor"), RRF_RT_REG_DWORD, None, Some(&mut v as *mut u32 as *mut _), Some(&mut size)) };
    r.is_ok().then(|| color((v & 0xff) as f32 / 255.0, ((v >> 8) & 0xff) as f32 / 255.0, ((v >> 16) & 0xff) as f32 / 255.0, 1.0))
}

fn read_config(scale: f32) -> (Look, Spring, Spring, Spring) {
    let path = std::env::var("APPDATA").map(|a| std::path::PathBuf::from(a).join("Kasumi").join("config.toml")).unwrap_or_default();
    let cfg: toml::Value = std::fs::read_to_string(path).ok().and_then(|t| t.parse().ok()).unwrap_or(toml::Value::Table(Default::default()));
    let get = |table: &str, key: &str| cfg.get(table).and_then(|t| t.get(key)).cloned();
    let num = |table: &str, key: &str, d: f32| get(table, key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).map_or(d, |v| v as f32);
    let flag = |table: &str, key: &str, d: bool| get(table, key).and_then(|v| v.as_bool()).unwrap_or(d);
    let text = |table: &str, key: &str, d: &str| get(table, key).and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| d.to_string());
    let rounded = flag("general", "rounded_corners", false);
    let font = text("theme", "font", "");
    let side = |s: &str, key: &str| cfg.get("notch").and_then(|n| n.get(s)).and_then(|t| t.get(key)).cloned();
    let side_flag = |s: &str, key: &str| side(s, key).and_then(|v| v.as_bool()).unwrap_or(true);
    let side_px = |s: &str| side(s, "offset").and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).map_or(150.0, |v| v as f32) * scale;
    let accent = if flag("theme", "accent_sync", false) { system_accent() } else { None }.or_else(|| hex_color(&text("theme", "accent", "#ffffff"))).unwrap_or(color(1.0, 1.0, 1.0, 1.0));
    let menu = cfg
        .get("widgets")
        .and_then(|w| w.get("menu"))
        .and_then(|m| m.get("items"))
        .and_then(|i| i.as_array())
        .map(|items| {
            items
                .iter()
                .map(|it| {
                    let s = |k: &str| it.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    MenuItem {
                        label: s("label"),
                        icon: s("icon"),
                        action: s("action"),
                        target: s("target"),
                        separator: it.get("separator").and_then(|v| v.as_bool()).unwrap_or(false),
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let look = Look {
        g: Geometry {
            expanded_height: num("geometry", "expanded_height", 30.0) * scale,
            concave_radius: num("geometry", "concave_radius", 10.0) * scale,
            notch_corner_radius: num("geometry", "notch_corner_radius", 12.0) * scale,
            panel_concave_radius: num("geometry", "panel_concave_radius", 10.0) * scale,
            panel_corner_radius: num("geometry", "panel_corner_radius", 12.0) * scale,
            corner_radius: num("geometry", "corner_radius", 14.0) * scale,
            screen_corner: if rounded { num("general", "corner_radius", 10.0) * scale } else { 0.0 },
            combine: get("notch", "combine_overlapping").and_then(|v| v.as_bool()).unwrap_or(true),
        },
        notch_height: num("geometry", "notch_height", 30.0) * scale,
        padding: num("bar", "padding", 18.0) * scale,
        font: if font.is_empty() { "Inter".into() } else { font },
        font_size: num("theme", "font_size", 13.0),
        bar_content: text("animation", "bar_content", "slide"),
        grace_ms: num("timing", "grace_ms", 300.0) as u128,
        poll_ms: num("timing", "poll_ms", 16.0).clamp(4.0, 100.0) as u32,
        dim: num("theme", "dim_opacity", 0.45),
        accent,
        menu,
        avoid: [side_flag("left", "avoid_windows"), side_flag("right", "avoid_windows")],
        offset: [side_px("left"), side_px("right")],
        hug: [side_flag("left", "hug"), side_flag("right", "hug")],
        dock: dock::read(&cfg, scale),
    };
    let spring = |stiffness: f32, damping: f32| Spring {
        value: 0.0,
        velocity: 0.0,
        target: 0.0,
        stiffness,
        damping,
        mass: num("animation", "mass", 1.0).max(0.01),
        open: num("animation", "open_speed", 1.0).clamp(0.25, 4.0),
        close: num("animation", "close_speed", 1.0).clamp(0.25, 4.0),
        bounce: flag("animation", "bounce", false),
    };
    let expand = spring(num("animation", "expand_stiffness", 420.0), num("animation", "expand_damping", 32.0));
    let panel = spring(num("animation", "panel_stiffness", 380.0), num("animation", "panel_damping", 30.0));
    let spread = spring(num("animation", "spread_stiffness", 260.0), num("animation", "spread_damping", 28.0));
    (look, expand, panel, spread)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Menu,
    Calendar,
}

/// A notch: its shape and its one piece of content.
struct Notch {
    bump: Bump,
    text: String,
    text_x: f32,
    text_w: f32,
    size: f32,
    icon: bool,
    alpha: f32,
    /// What a click on it opens.
    opens: Option<Kind>,
    /// Its place in the corner and moved aside (x, width, text x); 0 to 1 between them.
    home: (f32, f32, f32),
    aside: Option<(f32, f32, f32)>,
    moved: Spring,
}

/// A dropdown: its spring, size, where it ends up, and its content's offset.
struct Panel {
    kind: Kind,
    spring: Spring,
    width: f32,
    height: f32,
    /// The widget it hangs from (its centre).
    anchor: f32,
    /// Content shifted right (a menu pulled flush to the screen edge keeps its icons
    /// under the widget's).
    shift: f32,
}

/// One menu row: its item and its top, inside the panel.
struct Row {
    item: usize,
    top: f32,
    height: f32,
}

struct App {
    hwnd: HWND,
    pos: POINT,
    size: SIZE,
    monitor: RECT,
    s: f32,
    look: Look,
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    ink: ID2D1SolidColorBrush,
    paint: ID2D1SolidColorBrush,
    formats: RefCell<HashMap<(u32, bool, bool), IDWriteTextFormat>>,
    memdc: HDC,
    open: Spring,
    panels: Vec<Panel>,
    rows: Vec<Row>,
    hovered: Option<usize>,
    last: Instant,
    animating: bool,
    away_since: Option<Instant>,
    notches: Vec<Notch>,
    memory: String,
    /// The spread spring (notches sliding aside), as a template.
    slide: Spring,
    /// The bottom dock with the tasks widget.
    dock: Option<dock::Dock>,
}

/// Set when a newer test asks this one to quit (see take_over).
static QUIT_EVENT: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);

/// One test on screen at a time: a new one asks the running one to quit and takes
/// over once it's gone (it holds the mutex while it runs).
fn take_over() -> Option<HANDLE> {
    unsafe {
        let quit = CreateEventW(None, false, false, w!(r"Local\KasumiNativeTest.Quit")).ok()?;
        QUIT_EVENT.store(quit.0 as isize, std::sync::atomic::Ordering::Relaxed);
        let running = CreateMutexW(None, false, w!(r"Local\KasumiNativeTest.Running")).ok()?;
        if WaitForSingleObject(running, 0) == WAIT_TIMEOUT {
            let _ = SetEvent(quit);
            let _ = WaitForSingleObject(running, 5000);
        }
        Some(running)
    }
}

fn quit_requested() -> bool {
    let h = QUIT_EVENT.load(std::sync::atomic::Ordering::Relaxed);
    h != 0 && unsafe { WaitForSingleObject(HANDLE(h as *mut _), 0) } == WAIT_OBJECT_0
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn white(a: f32) -> D2D1_COLOR_F {
    color(1.0, 1.0, 1.0, a)
}

fn v(p: Pt) -> Vector2 {
    Vector2 { X: p.0, Y: p.1 }
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> D2D_RECT_F {
    D2D_RECT_F { left: x, top: y, right: x + w, bottom: y + h }
}

/// Days since 1970-01-01 (proleptic Gregorian).
fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// A day number back to (year, month, day).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Monday = 0.
fn weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

/// ISO-8601 week number of a day.
fn iso_week(days: i64) -> i64 {
    let thursday = days - weekday(days) + 3;
    let (y, _, _) = civil_from_days(thursday);
    (thursday - days_from_civil(y, 1, 1)) / 7 + 1
}

fn clock_text() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:02}:{:02}", t.wHour, t.wMinute)
}

/// Milliseconds to the next minute.
fn to_next_minute() -> u32 {
    let t = unsafe { GetLocalTime() };
    (60 - t.wSecond as u32) * 1000 - t.wMilliseconds as u32 + 20
}

fn private_mb() -> f64 {
    let mut pmc = PROCESS_MEMORY_COUNTERS_EX::default();
    unsafe {
        let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32);
    }
    pmc.PrivateUsage as f64 / (1024.0 * 1024.0)
}

/// Application windows on a monitor, by their visible bounds (as Kasumi
/// app_windows): visible, not minimized or cloaked, with a frame and a title; not
/// tool windows, the desktop or the taskbar.
fn app_windows(m: RECT) -> Vec<RECT> {
    unsafe extern "system" fn proc(h: HWND, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<HWND>);
        list.push(h);
        BOOL(1)
    }
    let mut all: Vec<HWND> = Vec::new();
    let mut out = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(proc), LPARAM(&mut all as *mut _ as isize));
        for h in all {
            if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
                continue;
            }
            let style = GetWindowLongW(h, GWL_STYLE) as u32;
            let ex = GetWindowLongW(h, GWL_EXSTYLE) as u32;
            if style & (WS_CAPTION.0 | WS_THICKFRAME.0) == 0 || ex & (WS_EX_TOOLWINDOW.0 | WS_EX_NOACTIVATE.0 | WS_EX_TRANSPARENT.0) != 0 || GetWindowTextLengthW(h) == 0 {
                continue;
            }
            let mut cloaked = 0u32;
            let _ = DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4);
            let mut buf = [0u16; 64];
            let n = GetClassNameW(h, &mut buf) as usize;
            let class = String::from_utf16_lossy(&buf[..n]);
            if cloaked != 0 || matches!(class.as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd") {
                continue;
            }
            let mut r = RECT::default();
            if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_ok()
                && r.right > r.left
                && r.bottom > r.top
                && r.left < m.right
                && r.right > m.left
                && r.top < m.bottom
                && r.bottom > m.top
            {
                out.push(r);
            }
        }
    }
    out
}

/// Path commands into a Direct2D geometry (filled like SVG: nonzero).
fn geometry_of(d2d: &ID2D1Factory, cmds: &[Cmd]) -> Result<ID2D1PathGeometry> {
    unsafe {
        let geom = d2d.CreatePathGeometry()?;
        let sink = geom.Open()?;
        sink.SetFillMode(D2D1_FILL_MODE_WINDING);
        let mut open = false;
        for c in cmds {
            match c {
                Cmd::M(p) => {
                    if open {
                        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    }
                    sink.BeginFigure(v(*p), D2D1_FIGURE_BEGIN_FILLED);
                    open = true;
                }
                Cmd::L(p) => sink.AddLine(v(*p)),
                Cmd::C(a, b, p) => sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v(*a), point2: v(*b), point3: v(*p) }),
                Cmd::Z => {
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    open = false;
                }
            }
        }
        if open {
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        sink.Close()?;
        Ok(geom)
    }
}

/// A menu item's action. In the test only "open" runs: settings and the power
/// buttons (sleep, restart, shut down, lock) do nothing.
fn run(item: &MenuItem) {
    if item.action != "open" || item.target.is_empty() {
        return;
    }
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let target = if let Some(rest) = item.target.strip_prefix('~') { format!("{home}{rest}") } else { item.target.clone() };
    unsafe {
        ShellExecuteW(None, w!("open"), &HSTRING::from(target.as_str()), None, None, SW_SHOWNORMAL);
    }
}

impl App {
    /// A text format: Kasumi's font (semibold) or the icon font, at a CSS px size.
    fn format(&self, size: f32, icon: bool, regular: bool) -> Result<IDWriteTextFormat> {
        let key = ((size * 10.0) as u32, icon, regular);
        if let Some(f) = self.formats.borrow().get(&key) {
            return Ok(f.clone());
        }
        let family = if icon { HSTRING::from("Segoe Fluent Icons") } else { HSTRING::from(self.look.font.as_str()) };
        let weight = if icon || regular { DWRITE_FONT_WEIGHT_NORMAL } else { DWRITE_FONT_WEIGHT_SEMI_BOLD };
        let f = unsafe { self.dw.CreateTextFormat(&family, None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * self.s, w!("en-us"))? };
        self.formats.borrow_mut().insert(key, f.clone());
        Ok(f)
    }

    fn layout_of(&self, text: &str, size: f32, icon: bool, w: f32, h: f32) -> Result<IDWriteTextLayout> {
        let wide: Vec<u16> = text.encode_utf16().collect();
        unsafe { self.dw.CreateTextLayout(&wide, &self.format(size, icon, false)?, w, h) }
    }

    fn measure(&self, text: &str, size: f32, icon: bool) -> Result<f32> {
        let layout = self.layout_of(text, size, icon, 4000.0, 400.0)?;
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        Ok(m.widthIncludingTrailingWhitespace)
    }

    /// Text in a box: vertically centred; left-aligned or centred.
    #[allow(clippy::too_many_arguments)]
    fn text_in(&self, text: &str, size: f32, icon: bool, x: f32, y: f32, w: f32, h: f32, centred: bool, c: D2D1_COLOR_F) -> Result<()> {
        unsafe {
            let layout = self.layout_of(text, size, icon, w.max(1.0), h.max(1.0))?;
            layout.SetTextAlignment(if centred { DWRITE_TEXT_ALIGNMENT_CENTER } else { DWRITE_TEXT_ALIGNMENT_LEADING })?;
            layout.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            self.paint.SetColor(&c);
            self.rt.DrawTextLayout(Vector2 { X: x.round(), Y: y.round() }, &layout, &self.paint, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    /// The notches, sized like Kasumi's: content + both round-offs + padding (corner
    /// notches add the bar's edge padding), the corner ones hugging their corners.
    fn layout(&mut self) -> Result<()> {
        let s = self.s;
        let w = self.size.cx as f32;
        let nc = self.look.g.notch_corner_radius;
        let extra = (self.look.padding - NOTCH_PAD * s).max(0.0);
        let pad = NOTCH_PAD * s;
        let size = self.look.font_size;
        let items: [(String, f32, bool, f32, Option<Kind>); 3] = [
            (MENU_GLYPH.into(), 14.0, true, 1.0, Some(Kind::Menu)),
            (clock_text(), size + 1.0, false, 1.0, Some(Kind::Calendar)),
            ("native".into(), size, false, self.look.dim, None),
        ];
        let mut notches = Vec::new();
        for (i, (text, size, icon, alpha, opens)) in items.into_iter().enumerate() {
            let cw = self.measure(&text, size, icon)?;
            let corner = i != 1;
            let width = cw + 2.0 * nc + 2.0 * pad + if corner { extra } else { 0.0 };
            let home = match i {
                0 => (-nc, width, extra + pad),
                1 => (w / 2.0 - width / 2.0, width, w / 2.0 - cw / 2.0),
                _ => (w + nc - width, width, w - extra - pad - cw),
            };
            // Moved aside: off the corner (flares both sides, no edge padding), at the offset.
            let side = if i == 0 { 0 } else { 1 };
            let free = cw + 2.0 * nc + 2.0 * pad;
            let off = self.look.offset[side];
            let away = if i == 0 { (off, free, off + (free - cw) / 2.0) } else { (w - off - free, free, w - off - free + (free - cw) / 2.0) };
            let (home, aside) = match corner {
                false => (home, None),
                true if !self.look.hug[side] => (away, None),
                true if self.look.avoid[side] => (home, Some(away)),
                true => (home, None),
            };
            // Keep a notch slide going across a relayout (the clock minute).
            let moved = self.notches.get(i).map(|n| n.moved.clone()).unwrap_or_else(|| self.slide.clone());
            notches.push(Notch {
                bump: Bump { x: home.0, width: home.1, height: self.look.notch_height, panel_width: 0.0, panel_height: 0.0, p: 0.0, in_hull: true, scale: 1.0 },
                text,
                text_x: home.2,
                text_w: cw,
                size,
                icon,
                alpha,
                opens,
                home,
                aside,
                moved,
            });
        }
        self.notches = notches;
        self.place();
        self.layout_panels()
    }

    /// Each notch where its slide has it, between its corner and its offset.
    fn place(&mut self) {
        for n in &mut self.notches {
            let k = n.moved.value.clamp(0.0, 1.0);
            let (x, w, t) = n.aside.map_or(n.home, |a| (n.home.0 + (a.0 - n.home.0) * k, n.home.1 + (a.1 - n.home.1) * k, n.home.2 + (a.2 - n.home.2) * k));
            n.bump.x = x;
            n.bump.width = w;
            n.text_x = t;
        }
    }

    /// Like Kasumi update_docking: a corner notch moves aside while an app window
    /// top edge is under it (its own spot, flares included); the open bar docks all.
    fn dock_check(&mut self) {
        let wins = if self.open.target > 0.5 { Vec::new() } else { app_windows(self.monitor) };
        let g = &self.look.g;
        let m = self.monitor;
        let reach = self.look.notch_height;
        let mut changed = false;
        for n in &mut self.notches {
            if n.aside.is_none() {
                continue;
            }
            let (x0, x1) = ((n.home.0 - g.concave_radius).max(0.0), (n.home.0 + n.home.1 + g.concave_radius).min((m.right - m.left) as f32));
            let under = wins.iter().any(|r| {
                let (l, t, rr, b) = ((r.left - m.left) as f32, (r.top - m.top) as f32, (r.right - m.left) as f32, (r.bottom - m.top) as f32);
                l < x1 && rr > x0 && b > 0.0 && t < reach
            });
            let target = if under { 1.0 } else { 0.0 };
            if n.moved.target != target {
                n.moved.target = target;
                changed = true;
            }
        }
        if changed {
            self.kick();
        }
    }

    /// The menu's rows and size, and both panels' places (Kasumi's paddings).
    fn layout_panels(&mut self) -> Result<()> {
        let s = self.s;
        let (pad_x, near, far) = (6.0 * s, 6.0 * s, 10.0 * s);
        let row_h = 32.0 * s;
        let sep_h = 11.0 * s;
        let mut rows = Vec::new();
        let mut y = near;
        let mut widest: f32 = 0.0;
        for (i, item) in self.look.menu.iter().enumerate() {
            if item.separator {
                rows.push(Row { item: i, top: y, height: sep_h });
                y += sep_h;
                continue;
            }
            let label = self.measure(&item.label, self.look.font_size, false)?;
            widest = widest.max(12.0 * s + 16.0 * s + 12.0 * s + label + 12.0 * s);
            rows.push(Row { item: i, top: y, height: row_h });
            y += row_h;
        }
        // The menu's icons under the widget's own; just short of the screen edge it
        // goes flush instead, its content shifted to keep them lined up.
        let icon_x = self.notches[0].text_x;
        let icon_w = self.notches[0].text_w;
        let mut left = icon_x + icon_w / 2.0 - (pad_x + 12.0 * s + 8.0 * s);
        let mut shift = 0.0;
        if left > 0.0 && left < 24.0 * s {
            shift = left;
            left = 0.0;
        }
        let menu_w = widest + 2.0 * pad_x + shift;
        let menu_h = y + far;
        let cal_w = (228.0 + 32.0) * s;
        let cal_h = (10.0 + 24.0 + 16.0 + 6.0 * 26.0 + 14.0) * s;
        let clock = &self.notches[1];
        let keep = |k: Kind, panels: &[Panel]| panels.iter().find(|p| p.kind == k).map(|p| p.spring.clone());
        let base = self.panels.first().map(|p| p.spring.clone());
        let fresh = |k: Kind| keep(k, &self.panels).or_else(|| base.clone());
        let menu_spring = fresh(Kind::Menu);
        let cal_spring = fresh(Kind::Calendar);
        if let (Some(ms), Some(cs)) = (menu_spring, cal_spring) {
            self.panels = vec![
                Panel { kind: Kind::Menu, spring: ms, width: menu_w, height: menu_h, anchor: left + menu_w / 2.0, shift },
                Panel { kind: Kind::Calendar, spring: cs, width: cal_w, height: cal_h, anchor: clock.text_x + clock.text_w / 2.0, shift: 0.0 },
            ];
        }
        self.rows = rows;
        Ok(())
    }

    /// Notches and dropdowns, left to right, as computeFrame takes them.
    fn bumps(&self) -> Vec<Bump> {
        let mut out: Vec<Bump> = self
            .notches
            .iter()
            .map(|n| Bump { x: n.bump.x, width: n.bump.width, height: n.bump.height, panel_width: 0.0, panel_height: 0.0, p: 0.0, in_hull: true, scale: 1.0 })
            .collect();
        for p in &self.panels {
            out.push(Bump { x: p.anchor, width: 0.0, height: 0.0, panel_width: p.width, panel_height: p.height, p: p.spring.value, in_hull: false, scale: 1.0 });
        }
        out.sort_by(|a, b| (a.x + a.width / 2.0).total_cmp(&(b.x + b.width / 2.0)));
        out
    }

    fn panel_frame<'a>(&self, frame: &'a Frame, p: &Panel) -> Option<&'a geometry::BumpFrame> {
        let bumps = self.bumps();
        let i = bumps.iter().position(|b| !b.in_hull && (b.x - p.anchor).abs() < 0.01 && (b.panel_width - p.width).abs() < 0.01)?;
        frame.bumps.get(i)
    }

    fn draw_menu(&self, x: f32, y: f32, p: &Panel) -> Result<()> {
        let s = self.s;
        let pad_x = 6.0 * s;
        for (ri, row) in self.rows.iter().enumerate() {
            let item = &self.look.menu[row.item];
            let top = y + row.top;
            let left = x + p.shift + pad_x;
            let width = p.width - p.shift - 2.0 * pad_x;
            if item.separator {
                unsafe {
                    self.paint.SetColor(&white(0.12));
                    self.rt.FillRectangle(&rect(left + 12.0 * s, top + 5.0 * s, width - 24.0 * s, 1.0 * s), &self.paint);
                }
                continue;
            }
            if self.hovered == Some(ri) {
                unsafe {
                    self.paint.SetColor(&white(0.1));
                    self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(left, top, width, row.height), radiusX: 10.0 * s, radiusY: 10.0 * s }, &self.paint);
                }
            }
            self.text_in(&item.icon, 14.0, true, left + 12.0 * s, top, 16.0 * s, row.height, true, white(1.0))?;
            self.text_in(&item.label, self.look.font_size, false, left + 40.0 * s, top, width - 40.0 * s, row.height, false, white(1.0))?;
        }
        Ok(())
    }

    /// Kasumi's calendar: the month, weekday initials, ISO week numbers, today in the accent.
    fn draw_calendar(&self, x: f32, y: f32) -> Result<()> {
        let s = self.s;
        let t = unsafe { GetLocalTime() };
        const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
        let (left, top, width) = (x + 16.0 * s, y + 10.0 * s, 228.0 * s);
        let title = format!("{} {}", MONTHS[(t.wMonth as usize).saturating_sub(1) % 12], t.wYear);
        self.text_in(&title, 13.0, false, left, top, width, 16.0 * s, true, white(1.0))?;
        let wk = 22.0 * s;
        let col = (width - wk) / 7.0;
        let grid = top + 24.0 * s;
        for (i, d) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
            self.text_in(d, 11.0, false, left + wk + col * i as f32, grid, col, 16.0 * s, true, white(self.look.dim))?;
        }
        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        let first = days_from_civil(t.wYear as i32, t.wMonth as u32, 1);
        let start = first - weekday(first);
        for week in 0..6 {
            let row_top = grid + 16.0 * s + 2.0 * s + week as f32 * 26.0 * s;
            let row_start = start + week * 7;
            self.text_in(&iso_week(row_start).to_string(), 10.0, false, left, row_top, wk, 24.0 * s, true, white(0.28))?;
            for i in 0..7 {
                let day = row_start + i;
                let (_, m, d) = civil_from_days(day);
                let cx = left + wk + col * i as f32;
                let mut c = if m != t.wMonth as u32 { white(0.25) } else { white(1.0) };
                if day == today {
                    unsafe {
                        self.paint.SetColor(&self.look.accent);
                        self.rt.FillRoundedRectangle(&D2D1_ROUNDED_RECT { rect: rect(cx, row_top, col, 24.0 * s), radiusX: 12.0 * s, radiusY: 12.0 * s }, &self.paint);
                    }
                    c = color(0.0, 0.0, 0.0, 1.0);
                }
                self.text_in(&d.to_string(), 12.0, false, cx, row_top, col, 24.0 * s, true, c)?;
            }
        }
        Ok(())
    }

    /// One frame of Kasumi's shape and content into the layered window (only as tall
    /// as what shows: the window grows with a panel and shrinks back).
    fn render(&mut self) -> Result<()> {
        self.place();
        let w = self.size.cx as f32;
        let h = self.look.g.expanded_height;
        let t = self.open.value;
        let frame = geometry::compute(&self.look.g, w, t, 1.0, &self.bumps());
        let deepest = frame.bumps.iter().map(|b| b.depth).fold(frame.base, f32::max).max(self.look.notch_height);
        let shown = SIZE { cx: self.size.cx, cy: ((deepest + self.look.g.screen_corner + 2.0).ceil() as i32).min(self.size.cy) };
        unsafe {
            let rc = RECT { left: 0, top: 0, right: shown.cx, bottom: shown.cy };
            self.rt.BindDC(self.memdc, &rc)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            self.rt.FillGeometry(&geometry_of(&self.d2d, &frame.path)?, &self.ink, None::<&ID2D1Brush>);
            if !frame.corners.is_empty() {
                self.rt.FillGeometry(&geometry_of(&self.d2d, &frame.corners)?, &self.ink, None::<&ID2D1Brush>);
            }
            // The notches' content, centred in their (whole-pixel) rows.
            let row = self.look.notch_height.round();
            for n in &self.notches {
                self.text_in(&n.text, n.size, n.icon, n.text_x, 0.0, n.text_w + 2.0, row, false, white(n.alpha))?;
            }
            // The open bar's content, as [animation] bar_content.
            let (offset, alpha) = match self.look.bar_content.as_str() {
                "reveal" => (0.0, (t * 5.0).clamp(0.0, 1.0)),
                "fade" => (0.0, ((t - 0.35) / 0.55).clamp(0.0, 1.0)),
                _ => (frame.base - h, ((t - 0.25) / 0.6).clamp(0.0, 1.0)),
            };
            if alpha > 0.0 {
                self.rt.PushAxisAlignedClip(&D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: frame.base }, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                let gap = self.look.padding;
                let n0 = &self.notches[0];
                let n2 = &self.notches[2];
                self.text_in("Kasumi native test · right-click a notch to quit", self.look.font_size, false, n0.text_x + n0.text_w + gap, offset, 800.0, h, false, white(alpha))?;
                let mw = self.measure(&self.memory, self.look.font_size, false)?;
                self.text_in(&self.memory, self.look.font_size, false, n2.text_x - gap - mw, offset, mw + 2.0, h, false, white(alpha * 0.7))?;
                self.rt.PopAxisAlignedClip();
            }
            // Panels: their content waits in place and the growing shape uncovers it.
            for p in &self.panels {
                if p.spring.value <= 0.001 {
                    continue;
                }
                let Some(bf) = self.panel_frame(&frame, p) else { continue };
                self.rt.PushAxisAlignedClip(&D2D_RECT_F { left: bf.left, top: h, right: bf.right, bottom: bf.depth }, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                match p.kind {
                    Kind::Menu => self.draw_menu(bf.panel_x, h, p)?,
                    Kind::Calendar => self.draw_calendar(bf.panel_x, h)?,
                }
                self.rt.PopAxisAlignedClip();
            }
            self.rt.EndDraw(None, None)?;
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            UpdateLayeredWindow(self.hwnd, None, Some(&self.pos), Some(&shown), Some(self.memdc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
        }
        Ok(())
    }

    fn kick(&mut self) {
        if !self.animating {
            self.animating = true;
            self.last = Instant::now();
        }
    }

    fn set_open(&mut self, open: bool) {
        let target = if open { 1.0 } else { 0.0 };
        if self.open.target == target {
            return;
        }
        self.open.target = target;
        self.dock_check();
        if open {
            self.memory = format!("{:.1} MB", private_mb());
            unsafe { SetTimer(Some(self.hwnd), MEMORY, 1000, None) };
        } else {
            let _ = unsafe { KillTimer(Some(self.hwnd), MEMORY) };
            for p in &mut self.panels {
                p.spring.target = 0.0;
            }
        }
        self.kick();
    }

    /// Opens a panel (closing the other), or closes it if it's the one open.
    fn toggle(&mut self, kind: Kind) {
        let open = self.panels.iter().any(|p| p.kind == kind && p.spring.target < 0.5);
        for p in &mut self.panels {
            p.spring.target = if open && p.kind == kind { 1.0 } else { 0.0 };
        }
        self.hovered = None;
        self.kick();
    }

    /// One animation step (the message loop calls it once per display refresh).
    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.05);
        self.last = now;
        // Each window is drawn only while something on it moves (and once after).
        let top_moving = !(self.open.settled() && self.panels.iter().all(|p| p.spring.settled()) && self.notches.iter().all(|n| n.moved.settled()));
        let dock_done = match self.dock.as_mut() {
            Some(d) => {
                let draw = d.dirty || d.springs_moving() || d.timed(now);
                let settled = d.step(dt);
                if draw {
                    let _ = d.render(&self.d2d, &self.rt, &self.ink, &self.paint);
                }
                settled && !d.timed(Instant::now())
            }
            None => true,
        };
        self.open.step(dt);
        let mut done = self.open.snap_if_settled();
        for p in &mut self.panels {
            p.spring.step(dt);
            done &= p.spring.snap_if_settled();
        }
        for n in &mut self.notches {
            n.moved.step(dt);
            done &= n.moved.snap_if_settled();
        }
        if done && dock_done {
            self.animating = false;
        }
        if top_moving {
            let _ = self.render();
        }
    }

    /// The open panel's box (where the cursor keeps the bar open).
    fn open_panel(&self) -> Option<(&Panel, D2D_RECT_F)> {
        let w = self.size.cx as f32;
        let h = self.look.g.expanded_height;
        let p = self.panels.iter().find(|p| p.spring.target > 0.5)?;
        let x = (p.anchor - p.width / 2.0).clamp(0.0, (w - p.width).max(0.0));
        Some((p, rect(x, h, p.width, p.height)))
    }

    /// Like Kasumi: a notch's trigger (80% of its visible width) or the top-left hot
    /// corner opens the bar; leaving the bar (and its open panel) closes it after
    /// [timing] grace_ms. Over the menu: its hovered row.
    fn hover(&mut self) {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let m = self.monitor;
        let on_monitor = pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom;
        let x = (pt.x - m.left) as f32;
        let y = (pt.y - m.top) as f32;
        let g = &self.look.g;
        let hot = 12.0 * self.s;
        let panel = self.open_panel().map(|(p, r)| (p.kind, p.shift, r));
        let in_panel = panel.is_some_and(|(_, _, r)| x >= r.left && x <= r.right && y >= r.top && y <= r.bottom);
        let inside = on_monitor
            && if self.open.target > 0.5 {
                y < g.expanded_height.max(self.look.notch_height) + 2.0 || in_panel
            } else {
                (x < hot && y < hot)
                    || self.notches.iter().any(|n| {
                        let visible = n.bump.width + 2.0 * g.concave_radius;
                        let tw = 0.8 * visible;
                        let tx = n.bump.x + (n.bump.width - tw) / 2.0;
                        x >= tx && x <= tx + tw && y < n.bump.height
                    })
            };
        if inside {
            self.away_since = None;
            self.set_open(true);
        } else if self.open.target > 0.5 {
            let since = *self.away_since.get_or_insert_with(Instant::now);
            if since.elapsed().as_millis() >= self.look.grace_ms {
                self.set_open(false);
            }
        }
        // The menu row under the cursor.
        let hovered = match panel {
            Some((Kind::Menu, _, r)) if in_panel => self.rows.iter().position(|row| {
                let item = &self.look.menu[row.item];
                !item.separator && y >= r.top + row.top && y < r.top + row.top + row.height
            }),
            _ => None,
        };
        if hovered != self.hovered {
            self.hovered = hovered;
            if !self.animating {
                let _ = self.render();
            }
        }
        if self.dock.as_mut().is_some_and(|d| d.hover(pt, self.look.grace_ms)) {
            self.kick();
        }
        let near = on_monitor && y < self.look.notch_height * 3.0;
        let fast = near || self.open.target > 0.5 || self.dock.as_ref().is_some_and(|d| d.near(pt));
        unsafe { SetTimer(Some(self.hwnd), HOVER, if fast { self.look.poll_ms } else { 100 }, None) };
    }

    /// A click: a notch opens its panel; a menu row closes the menu and hands back
    /// its item, run once the app is released (opening it pumps messages).
    fn click(&mut self, x: f32, y: f32) -> Option<MenuItem> {
        if let Some((p, r)) = self.open_panel() {
            if x >= r.left && x <= r.right && y >= r.top && y <= r.bottom {
                if p.kind == Kind::Menu {
                    let hit = self.rows.iter().find(|row| y >= r.top + row.top && y < r.top + row.top + row.height).map(|row| row.item);
                    if let Some(i) = hit.filter(|&i| !self.look.menu[i].separator) {
                        self.toggle(Kind::Menu);
                        let item = &self.look.menu[i];
                        return Some(MenuItem { label: item.label.clone(), icon: String::new(), action: item.action.clone(), target: item.target.clone(), separator: false });
                    }
                }
                return None;
            }
        }
        if y > self.look.notch_height.max(self.look.g.expanded_height) {
            return None;
        }
        let w = self.size.cx as f32;
        let hit = self.notches.iter().find(|n| x >= n.bump.x.max(0.0) && x <= (n.bump.x + n.bump.width).min(w)).and_then(|n| n.opens);
        if let Some(kind) = hit {
            self.set_open(true);
            self.toggle(kind);
        }
        None
    }
}

/// What a click asks for, run once the app is released (it may pump messages).
enum Clicked {
    Menu(MenuItem),
    Task(dock::Action),
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            APP.with(|a| {
                // Busy (a call that pumps messages): skip; timers come again.
                let Ok(mut a) = a.try_borrow_mut() else { return };
                let Some(app) = a.as_mut() else { return };
                match wp.0 {
                    HOVER => app.hover(),
                    DOCK => {
                        if quit_requested() {
                            unsafe { PostQuitMessage(0) };
                        }
                        app.dock_check();
                    }
                    dock::TASKS | dock::TASKS_ANYWAY => {
                        if let Some(d) = app.dock.as_mut() {
                            if d.refresh(&app.rt) {
                                app.kick();
                            }
                        }
                    }
                    CLOCK => {
                        let _ = app.layout();
                        let _ = app.render();
                        unsafe { SetTimer(Some(hwnd), CLOCK, to_next_minute(), None) };
                    }
                    MEMORY => {
                        app.memory = format!("{:.1} MB", private_mb());
                        if !app.animating {
                            let _ = app.render();
                        }
                    }
                    CYCLE => {
                        let open = app.open.target < 0.5;
                        app.set_open(open);
                    }
                    QUIT => unsafe { PostQuitMessage(0) },
                    _ => {}
                }
            });
            LRESULT(0)
        }
        dock::WM_ICONS => {
            APP.with(|a| {
                let Ok(mut a) = a.try_borrow_mut() else {
                    // Busy: ask again (the icons wait in their queue).
                    unsafe {
                        let _ = PostMessageW(Some(hwnd), msg, wp, lp);
                    }
                    return;
                };
                let Some(app) = a.as_mut() else { return };
                if app.dock.as_mut().is_some_and(|d| d.receive_icons(&app.rt)) {
                    app.kick();
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_MBUTTONUP => {
            let (x, y) = ((lp.0 & 0xffff) as i16 as f32, ((lp.0 >> 16) & 0xffff) as i16 as f32);
            let clicked = APP.with(|a| {
                let mut a = a.try_borrow_mut().ok()?;
                let app = a.as_mut()?;
                match app.dock.as_ref() {
                    Some(d) if d.hwnd == hwnd => d.click(x, y, msg == WM_MBUTTONUP).map(Clicked::Task),
                    _ if msg == WM_LBUTTONUP => app.click(x, y).map(Clicked::Menu),
                    _ => None,
                }
            });
            match clicked {
                Some(Clicked::Menu(item)) => run(&item),
                Some(Clicked::Task(action)) => dock::run(action),
                None => {}
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            // The top bar's notches quit the test (the dock's tasks panel isn't here).
            let on_dock = APP.with(|a| a.try_borrow().is_ok_and(|a| a.as_ref().and_then(|app| app.dock.as_ref()).is_some_and(|d| d.hwnd == hwnd)));
            if !on_dock {
                unsafe { PostQuitMessage(0) };
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
    }
}

/// A 32-bit premultiplied bitmap a layered window shows, selected into its own DC.
fn dib(size: SIZE) -> Result<(HDC, *mut u8)> {
    unsafe {
        let memdc = CreateCompatibleDC(None);
        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: size.cx,
                biHeight: -size.cy,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = std::ptr::null_mut();
        let bmp = CreateDIBSection(Some(memdc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)?;
        SelectObject(memdc, bmp.into());
        Ok((memdc, bits as *mut u8))
    }
}

fn save_bmp(bits: *const u8, size: SIZE, path: &std::path::Path) {
    let (w, h) = (size.cx as u32, size.cy as u32);
    let pixels = unsafe { std::slice::from_raw_parts(bits, (w * h * 4) as usize) };
    let mut out = Vec::with_capacity(54 + pixels.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + pixels.len() as u32).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(-(h as i32)).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(pixels);
    let _ = std::fs::write(path, out);
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let has = |f: &str| args.iter().any(|a| a == f);
    let value = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let seconds: Option<u32> = value("--seconds").and_then(|s| s.parse().ok());
    if args.get(1).is_some_and(|a| a == "--icons") {
        dock::icons_main(&args[2..]);
        return Ok(());
    }

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _running = if has("--offscreen") || has("--snapshot") { None } else { take_over() };
        // The shell (app icons and IDs, opening menu items) wants COM.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let s = dx as f32 / 96.0;
        let (look, expand, panel, slide) = read_config(s);
        let m = info.rcMonitor;
        // Room for the tallest panel; the window itself is only as tall as what shows.
        let tall = look.g.expanded_height + 420.0 * s + look.g.screen_corner + 2.0;
        let size = SIZE { cx: m.right - m.left, cy: tall.ceil() as i32 };
        let pos = if has("--offscreen") { POINT { x: -32000, y: -32000 } } else { POINT { x: m.left, y: m.top } };

        let instance = GetModuleHandleW(None)?;
        let class = w!("KasumiNativeTest");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() });
        let window = |title: windows::core::PCWSTR, pos: POINT, size: SIZE| {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class,
                title,
                WS_POPUP,
                pos.x,
                pos.y,
                size.cx,
                size.cy,
                None,
                None,
                Some(instance.into()),
                None,
            )
        };
        let hwnd = window(w!("Kasumi native test"), pos, size)?;

        // The drawing surface: a 32-bit premultiplied bitmap the layered window shows.
        let (memdc, bits) = dib(size)?;

        let d2d: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let props = D2D1_RENDER_TARGET_PROPERTIES {
            // Software: a bar this small draws in microseconds on the CPU, and a
            // graphics-card device costs tens of MB of driver memory.
            r#type: if has("--gpu") { D2D1_RENDER_TARGET_TYPE_DEFAULT } else { D2D1_RENDER_TARGET_TYPE_SOFTWARE },
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            usage: D2D1_RENDER_TARGET_USAGE_NONE,
            minLevel: D2D1_FEATURE_LEVEL_DEFAULT,
        };
        let rt = d2d.CreateDCRenderTarget(&props)?;
        // Grayscale text: ClearType needs an opaque background.
        rt.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
        let ink = rt.CreateSolidColorBrush(&color(0.0, 0.0, 0.0, 1.0), None)?;
        let paint = rt.CreateSolidColorBrush(&white(1.0), None)?;

        let mut app = App {
            hwnd,
            pos,
            size,
            monitor: m,
            s,
            look,
            d2d,
            dw,
            rt,
            ink,
            paint,
            formats: RefCell::new(HashMap::new()),
            memdc,
            open: expand,
            panels: vec![Panel { kind: Kind::Menu, spring: panel.clone(), width: 0.0, height: 0.0, anchor: 0.0, shift: 0.0 }],
            rows: Vec::new(),
            hovered: None,
            last: Instant::now(),
            animating: false,
            away_since: None,
            notches: Vec::new(),
            memory: String::new(),
            slide: slide.clone(),
            dock: None,
        };
        app.layout()?;
        app.render()?;
        // The bottom dock: as tall as it opens, plus the rounded screen corners above it.
        let dock_size = SIZE { cx: size.cx, cy: (app.look.dock.open_height() + app.look.g.screen_corner + 2.0).ceil() as i32 };
        let dock_pos = if has("--offscreen") { POINT { x: -32000, y: -32000 } } else { POINT { x: m.left, y: m.bottom - dock_size.cy } };
        let dock_hwnd = window(w!("Kasumi native test dock"), dock_pos, dock_size)?;
        let (dock_dc, dock_bits) = dib(dock_size)?;
        let setup = dock::Setup {
            hwnd: dock_hwnd,
            memdc: dock_dc,
            bits: dock_bits,
            size: dock_size,
            pos: dock_pos,
            monitor: m,
            hmon: mon,
            s,
            g: app.look.g.clone(),
            accent: app.look.accent,
            expand: app.open.clone(),
            slide,
            // Kasumi hides the Windows taskbar for its dock; only on screen, never for snapshots.
            hide_taskbar: app.look.dock.hide_taskbar && !has("--offscreen") && !has("--snapshot"),
            wait_icons: has("--snapshot"),
            look: app.look.dock.clone(),
        };
        let mut d = dock::Dock::new(setup, &app.rt);
        d.render(&app.d2d, &app.rt, &app.ink, &app.paint)?;
        app.dock = Some(d);
        if let Some(dir) = value("--snapshot") {
            let dir = std::path::PathBuf::from(dir);
            app.memory = format!("{:.1} MB", private_mb());
            let shots: [(&str, f32, Option<Kind>, f32); 5] =
                [("closed", 0.0, None, 0.0), ("open", 1.0, None, 0.0), ("menu-half", 1.0, Some(Kind::Menu), 0.5), ("menu", 1.0, Some(Kind::Menu), 1.0), ("calendar", 1.0, Some(Kind::Calendar), 1.0)];
            for (name, t, panel, p) in shots {
                app.open.value = t;
                for pn in &mut app.panels {
                    pn.spring.value = if Some(pn.kind) == panel { p } else { 0.0 };
                }
                app.hovered = if name == "menu" { Some(1) } else { None };
                app.render()?;
                save_bmp(bits as *const u8, size, &dir.join(format!("{name}.bmp")));
            }
            if let Some(d) = app.dock.as_mut() {
                for (name, e) in [("dock-rest", 0.0), ("dock-half", 0.5), ("dock-open", 1.0), ("dock-hover", 1.0)] {
                    d.open.value = e;
                    if name == "dock-hover" {
                        d.pose_hover();
                    }
                    d.render(&app.d2d, &app.rt, &app.ink, &app.paint)?;
                    save_bmp(d.bits, d.size, &dir.join(format!("{name}.bmp")));
                }
            }
            return Ok(());
        }
        APP.with(|a| *a.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = ShowWindow(dock_hwnd, SW_SHOWNOACTIVATE);

        SetTimer(Some(hwnd), HOVER, 100, None);
        SetTimer(Some(hwnd), DOCK, 250, None);
        SetTimer(Some(hwnd), CLOCK, to_next_minute(), None);
        if has("--cycle") {
            SetTimer(Some(hwnd), CYCLE, 2000, None);
        }
        if let Some(n) = seconds {
            SetTimer(Some(hwnd), QUIT, n * 1000, None);
        }

        // Idle: wait for messages. Animating: one frame per display refresh (DwmFlush
        // waits for the next composition), handling messages in between.
        let mut msg = MSG::default();
        'run: loop {
            let animating = APP.with(|a| a.try_borrow().is_ok_and(|a| a.as_ref().is_some_and(|app| app.animating)));
            if animating {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        break 'run;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                APP.with(|a| {
                    if let Ok(mut a) = a.try_borrow_mut() {
                        if let Some(app) = a.as_mut() {
                            app.frame();
                        }
                    }
                });
                let _ = DwmFlush();
            } else {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    break 'run;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        // Release Direct2D / DirectWrite now, not in the thread's late teardown.
        APP.with(|a| a.borrow_mut().take());
        dock::restore_taskbar();
    }
    Ok(())
}
