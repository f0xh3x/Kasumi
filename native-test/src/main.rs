//! Kasumi native test: one top bar on the main monitor, drawn with Direct2D and
//! DirectWrite into a layered window (per-pixel alpha, so clicks pass through
//! everything but the shapes). No WebView2. The shapes are Kasumi's own
//! (geometry.rs is a port of src/geometry.ts) and the look and motion come from
//! your config.toml: [geometry], the expand spring, open / close speed, bar
//! content style, rounded screen corners, font. It redraws only when something
//! changes; while the bar moves, once per display refresh.
//!
//! Hover a notch or the top-left hot corner: the bar opens. Right-click a notch: quit.
//! `--offscreen`        drawn off screen (measuring without covering anything)
//! `--cycle`            opens and closes every 2 s by itself (measuring the animation)
//! `--seconds N`        quits after N seconds
//! `--snapshot DIR`     the closed, half-open and open bar as BMPs, then quits
//! `--gpu`              Direct2D on the graphics card (default: software, far less memory)

#![windows_subsystem = "windows"]

mod geometry;

use std::cell::RefCell;
use std::time::Instant;

use geometry::{Bump, Cmd, Geometry, Pt};
use windows::core::{w, Result, HSTRING};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_WINDING,
    D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1PathGeometry, ID2D1SolidColorBrush, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE,
    D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES,
    D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_METRICS,
};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, GetMonitorInfoW, MonitorFromPoint, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    BLENDFUNCTION, DIB_RGB_COLORS, HDC, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer, LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassW,
    SetTimer, ShowWindow, TranslateMessage, UpdateLayeredWindow, IDC_ARROW, MSG, PM_REMOVE, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_QUIT, WM_RBUTTONUP, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_numerics::Vector2;

const HOVER: usize = 2; // the cursor, near the top or a notch
const CLOCK: usize = 3; // the next minute
const MEMORY: usize = 4; // the memory readout (only while open)
const CYCLE: usize = 5; // --cycle
const QUIT: usize = 6; // --seconds
const NOTCH_PAD: f32 = 10.0; // as main.ts

/// Kasumi's spring (spring.ts): critically damped at least unless [animation] bounce,
/// with the open / close speed factors.
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
}

fn read_config(scale: f32) -> (Look, Spring) {
    let path = std::env::var("APPDATA").map(|a| std::path::PathBuf::from(a).join("Kasumi").join("config.toml")).unwrap_or_default();
    let cfg: toml::Value = std::fs::read_to_string(path).ok().and_then(|t| t.parse().ok()).unwrap_or(toml::Value::Table(Default::default()));
    let get = |table: &str, key: &str| cfg.get(table).and_then(|t| t.get(key)).cloned();
    let num = |table: &str, key: &str, d: f32| get(table, key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64))).map_or(d, |v| v as f32);
    let flag = |table: &str, key: &str, d: bool| get(table, key).and_then(|v| v.as_bool()).unwrap_or(d);
    let text = |table: &str, key: &str, d: &str| get(table, key).and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| d.to_string());
    let rounded = flag("general", "rounded_corners", false);
    let font = text("theme", "font", "");
    let look = Look {
        g: Geometry {
            expanded_height: num("geometry", "expanded_height", 30.0) * scale,
            concave_radius: num("geometry", "concave_radius", 10.0) * scale,
            notch_corner_radius: num("geometry", "notch_corner_radius", 12.0) * scale,
            panel_concave_radius: num("geometry", "panel_concave_radius", 10.0) * scale,
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
    };
    let spring = Spring {
        value: 0.0,
        velocity: 0.0,
        target: 0.0,
        stiffness: num("animation", "expand_stiffness", 420.0),
        damping: num("animation", "expand_damping", 32.0),
        mass: num("animation", "mass", 1.0).max(0.01),
        open: num("animation", "open_speed", 1.0).clamp(0.25, 4.0),
        close: num("animation", "close_speed", 1.0).clamp(0.25, 4.0),
        bounce: flag("animation", "bounce", false),
    };
    (look, spring)
}

/// A notch and its one line of text.
struct Notch {
    bump: Bump,
    text: String,
    text_x: f32,
    big: bool,
    dim: bool,
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
    text: ID2D1SolidColorBrush,
    font: IDWriteTextFormat,
    clock_font: IDWriteTextFormat,
    memdc: HDC,
    open: Spring,
    last: Instant,
    animating: bool,
    away_since: Option<Instant>,
    notches: Vec<Notch>,
    memory: String,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn v(p: Pt) -> Vector2 {
    Vector2 { X: p.0, Y: p.1 }
}

fn now_text() -> (String, String) {
    let t = unsafe { GetLocalTime() };
    const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    (
        format!("{:02}:{:02}", t.wHour, t.wMinute),
        format!("{} {} {}", DAYS[t.wDayOfWeek as usize % 7], t.wDay, MONTHS[(t.wMonth as usize).saturating_sub(1) % 12]),
    )
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

impl App {
    fn measure(&self, s: &str, big: bool) -> Result<(f32, f32)> {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe { self.dw.CreateTextLayout(&wide, if big { &self.clock_font } else { &self.font }, 4000.0, 200.0)? };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        Ok((m.width, m.height))
    }

    fn draw_text(&self, s: &str, x: f32, row_top: f32, row: f32, big: bool, alpha: f32) -> Result<()> {
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let layout = self.dw.CreateTextLayout(&wide, if big { &self.clock_font } else { &self.font }, 4000.0, 200.0)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            self.text.SetOpacity(alpha);
            self.rt.DrawTextLayout(Vector2 { X: x.round(), Y: (row_top + (row - m.height) / 2.0).round() }, &layout, &self.text, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    /// The notches, sized like Kasumi's: content + both round-offs + padding (corner
    /// notches add the bar's edge padding), corner ones hugging their corners.
    fn layout(&mut self) -> Result<()> {
        let w = self.size.cx as f32;
        let nc = self.look.g.notch_corner_radius;
        let extra = (self.look.padding - NOTCH_PAD * self.s).max(0.0);
        let pad = NOTCH_PAD * self.s;
        let (clock, date) = now_text();
        let items = [(date, false, false), (clock, true, false), ("native".to_string(), false, true)];
        let mut notches = Vec::new();
        for (i, (text, big, dim)) in items.into_iter().enumerate() {
            let (cw, _) = self.measure(&text, big)?;
            let corner = i != 1;
            let width = cw + 2.0 * nc + 2.0 * pad + if corner { extra } else { 0.0 };
            let (x, text_x) = match i {
                0 => (-nc, extra + pad),
                1 => (w / 2.0 - width / 2.0, w / 2.0 - cw / 2.0),
                _ => (w + nc - width, w - extra - pad - cw),
            };
            notches.push(Notch { bump: Bump { x, width, height: self.look.notch_height }, text, text_x, big, dim });
        }
        self.notches = notches;
        Ok(())
    }

    /// One frame of Kasumi's shape and content into the layered window.
    fn render(&mut self) -> Result<()> {
        let w = self.size.cx as f32;
        let h = self.look.g.expanded_height;
        let t = self.open.value;
        let bumps: Vec<Bump> = self.notches.iter().map(|n| Bump { x: n.bump.x, width: n.bump.width, height: n.bump.height }).collect();
        let frame = geometry::compute(&self.look.g, w, t, 1.0, &bumps);
        unsafe {
            let rc = RECT { left: 0, top: 0, right: self.size.cx, bottom: self.size.cy };
            self.rt.BindDC(self.memdc, &rc)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            self.rt.FillGeometry(&geometry_of(&self.d2d, &frame.path)?, &self.ink, None::<&ID2D1Brush>);
            if !frame.corners.is_empty() {
                self.rt.FillGeometry(&geometry_of(&self.d2d, &frame.corners)?, &self.ink, None::<&ID2D1Brush>);
            }
            // The notches' own content, centred in their (whole-pixel) rows.
            for (n, row) in self.notches.iter().zip(&frame.rows) {
                self.draw_text(&n.text, n.text_x, 0.0, row.round(), n.big, if n.dim { 0.45 } else { 1.0 })?;
            }
            // The open bar's content, as [animation] bar_content: slide rides the bar's
            // far edge; reveal / fade wait in place.
            let ex = t;
            let (offset, alpha) = match self.look.bar_content.as_str() {
                "reveal" => (0.0, (ex * 5.0).clamp(0.0, 1.0)),
                "fade" => (0.0, ((ex - 0.35) / 0.55).clamp(0.0, 1.0)),
                _ => (frame.base - h, ((ex - 0.25) / 0.6).clamp(0.0, 1.0)),
            };
            if alpha > 0.0 {
                self.rt.PushAxisAlignedClip(&D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: frame.base }, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
                let gap = self.look.padding;
                let left_end = self.notches[0].text_x + self.measure(&self.notches[0].text, false)?.0;
                self.draw_text("Kasumi native test · right-click a notch to quit", left_end + gap, offset, h, false, alpha)?;
                let (mw, _) = self.measure(&self.memory, false)?;
                self.draw_text(&self.memory, self.notches[2].text_x - gap - mw, offset, h, false, alpha * 0.7)?;
                self.rt.PopAxisAlignedClip();
            }
            self.rt.EndDraw(None, None)?;
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            UpdateLayeredWindow(self.hwnd, None, Some(&self.pos), Some(&self.size), Some(self.memdc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
        }
        Ok(())
    }

    fn set_open(&mut self, open: bool) {
        let target = if open { 1.0 } else { 0.0 };
        if self.open.target == target {
            return;
        }
        self.open.target = target;
        if open {
            self.memory = format!("{:.1} MB", private_mb());
            unsafe { SetTimer(Some(self.hwnd), MEMORY, 1000, None) };
        } else {
            let _ = unsafe { KillTimer(Some(self.hwnd), MEMORY) };
        }
        if !self.animating {
            self.animating = true;
            self.last = Instant::now();
        }
    }

    /// One animation step (the message loop calls it once per display refresh).
    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.05);
        self.last = now;
        self.open.step(dt);
        if self.open.settled() {
            self.open.value = self.open.target;
            self.open.velocity = 0.0;
            self.animating = false;
        }
        let _ = self.render();
    }

    /// Like Kasumi: a notch's trigger (80% of its visible width) or the top-left hot
    /// corner opens it; leaving the bar closes it after [timing] grace_ms.
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
        let inside = on_monitor
            && if self.open.target > 0.5 {
                y < g.expanded_height.max(self.look.notch_height) + 2.0
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
        let near = on_monitor && y < self.look.notch_height * 3.0;
        unsafe { SetTimer(Some(self.hwnd), HOVER, if near || self.open.target > 0.5 { self.look.poll_ms } else { 100 }, None) };
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            APP.with(|a| {
                let mut a = a.borrow_mut();
                let Some(app) = a.as_mut() else { return };
                match wp.0 {
                    HOVER => app.hover(),
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
        WM_RBUTTONUP => {
            unsafe { PostQuitMessage(0) };
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wp, lp) },
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

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let s = dx as f32 / 96.0;
        let (look, spring) = read_config(s);
        let m = info.rcMonitor;
        let tall = look.g.expanded_height.max(look.notch_height) + look.g.screen_corner + 2.0;
        let size = SIZE { cx: m.right - m.left, cy: tall.ceil() as i32 };
        let pos = if has("--offscreen") { POINT { x: -32000, y: -32000 } } else { POINT { x: m.left, y: m.top } };

        let instance = GetModuleHandleW(None)?;
        let class = w!("KasumiNativeTest");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() });
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            w!("Kasumi native test"),
            WS_POPUP,
            pos.x,
            pos.y,
            size.cx,
            size.cy,
            None,
            None,
            Some(instance.into()),
            None,
        )?;

        // The drawing surface: a 32-bit premultiplied bitmap the layered window shows.
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
        let text = rt.CreateSolidColorBrush(&color(1.0, 1.0, 1.0, 1.0), None)?;
        // Kasumi's text: Inter (or [theme] font), semibold; the clock a size up.
        let family = HSTRING::from(look.font.as_str());
        let font = dw.CreateTextFormat(&family, None, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, look.font_size * s, w!("en-us"))?;
        let clock_font = dw.CreateTextFormat(&family, None, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, (look.font_size + 1.0) * s, w!("en-us"))?;

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
            text,
            font,
            clock_font,
            memdc,
            open: spring,
            last: Instant::now(),
            animating: false,
            away_since: None,
            notches: Vec::new(),
            memory: String::new(),
        };
        app.layout()?;
        app.render()?;
        if let Some(dir) = value("--snapshot") {
            let dir = std::path::PathBuf::from(dir);
            app.memory = format!("{:.1} MB", private_mb());
            for (name, t) in [("closed", 0.0), ("half", 0.5), ("open", 1.0)] {
                app.open.value = t;
                app.render()?;
                save_bmp(bits as *const u8, size, &dir.join(format!("{name}.bmp")));
            }
            return Ok(());
        }
        APP.with(|a| *a.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        SetTimer(Some(hwnd), HOVER, 100, None);
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
        loop {
            let animating = APP.with(|a| a.borrow().as_ref().is_some_and(|app| app.animating));
            if animating {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        return Ok(());
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                APP.with(|a| {
                    if let Some(app) = a.borrow_mut().as_mut() {
                        app.frame();
                    }
                });
                let _ = DwmFlush();
            } else {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    }
}
