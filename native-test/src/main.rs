//! Kasumi native test: one top bar on the main monitor, drawn with Direct2D and
//! DirectWrite into a layered window (per-pixel alpha, so clicks pass through
//! everything but the shapes). No WebView2. It redraws only when something changes:
//! the clock's minute, or the bar opening / closing.
//!
//! Hover the top edge or a notch: the bar opens. Right-click a notch: quit.
//! `--offscreen`  drawn off screen (measuring without covering anything)
//! `--cycle`      opens and closes every 2 s by itself (measuring the animation)
//! `--seconds N`  quits after N seconds
//! `--gpu`        Direct2D on the graphics card (default: software, far less memory)

#![windows_subsystem = "windows"]

use std::cell::RefCell;
use std::time::Instant;

use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{
    D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_BEZIER_SEGMENT, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D1_PIXEL_FORMAT, D2D_RECT_F,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Brush, ID2D1DCRenderTarget, ID2D1Factory, ID2D1SolidColorBrush, D2D1_DRAW_TEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED,
    D2D1_FEATURE_LEVEL_DEFAULT, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT, D2D1_RENDER_TARGET_TYPE_SOFTWARE, D2D1_RENDER_TARGET_USAGE_NONE, D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE,
};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_TEXT_METRICS,
};
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
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetCursorPos, GetMessageW, KillTimer, LoadCursorW, PostQuitMessage, RegisterClassW, SetTimer,
    ShowWindow, TranslateMessage, UpdateLayeredWindow, IDC_ARROW, MSG, SW_SHOWNOACTIVATE, ULW_ALPHA, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_LAYERED,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows_numerics::Vector2;

const ANIM: usize = 1; // frames while the bar moves
const HOVER: usize = 2; // the cursor, near the edge or a notch
const CLOCK: usize = 3; // the next minute
const MEMORY: usize = 4; // the memory readout (only while open)
const CYCLE: usize = 5; // --cycle
const QUIT: usize = 6; // --seconds

/// A critically damped spring (no overshoot), like Kasumi's with Bounce off.
struct Spring {
    value: f32,
    velocity: f32,
    target: f32,
}

impl Spring {
    const K: f32 = 2000.0;
    fn step(&mut self, dt: f32) {
        let c = 2.0 * Self::K.sqrt();
        let mut left = dt;
        while left > 0.0 {
            let s = left.min(1.0 / 240.0);
            let a = -Self::K * (self.value - self.target) - c * self.velocity;
            self.velocity += a * s;
            self.value += self.velocity * s;
            left -= s;
        }
    }
    fn settled(&self) -> bool {
        (self.value - self.target).abs() < 5e-4 && self.velocity.abs() < 5e-3
    }
}

struct App {
    hwnd: HWND,
    pos: POINT,
    size: SIZE,
    /// Where the monitor is (the cursor is compared against it, even off screen).
    monitor: RECT,
    s: f32,
    d2d: ID2D1Factory,
    dw: IDWriteFactory,
    rt: ID2D1DCRenderTarget,
    ink: ID2D1SolidColorBrush,
    text: ID2D1SolidColorBrush,
    font: IDWriteTextFormat,
    memdc: HDC,
    open: Spring,
    last: Instant,
    animating: bool,
    away_since: Option<Instant>,
    /// Each notch's span (with its flares), for hovering.
    spans: Vec<(f32, f32)>,
    memory: String,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn color(r: f32, g: f32, b: f32, a: f32) -> D2D1_COLOR_F {
    D2D1_COLOR_F { r, g, b, a }
}

fn v(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
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
        let _ = GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
        );
    }
    pmc.PrivateUsage as f64 / (1024.0 * 1024.0)
}

impl App {
    fn text_width(&self, s: &str) -> Result<(f32, f32)> {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let layout = unsafe { self.dw.CreateTextLayout(&wide, &self.font, 4000.0, 200.0)? };
        let mut m = DWRITE_TEXT_METRICS::default();
        unsafe { layout.GetMetrics(&mut m)? };
        Ok((m.width, m.height))
    }

    fn draw_text(&self, s: &str, x: f32, row: f32, alpha: f32) -> Result<()> {
        let wide: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let layout = self.dw.CreateTextLayout(&wide, &self.font, 4000.0, 200.0)?;
            let mut m = DWRITE_TEXT_METRICS::default();
            layout.GetMetrics(&mut m)?;
            self.text.SetOpacity(alpha);
            self.rt.DrawTextLayout(v(x.round(), ((row - m.height) / 2.0).round()), &layout, &self.text, D2D1_DRAW_TEXT_OPTIONS_NONE);
        }
        Ok(())
    }

    /// One frame: the notches (and the bar, as it opens) into the layered window.
    fn render(&mut self) -> Result<()> {
        let s = self.s;
        let w = self.size.cx as f32;
        let row = 30.0 * s; // notch and bar height, as Kasumi's
        let pad = 14.0 * s;
        let edge = 10.0 * s;
        let r = 16.0 * s; // a notch's flare
        let t = self.open.value.clamp(0.0, 1.0);
        let (clock, date) = now_text();
        let label = "native";
        let (dw, _) = self.text_width(&date)?;
        let (cw, _) = self.text_width(&clock)?;
        let (lw, _) = self.text_width(label)?;
        // The notches' bottoms: left and right hug their corners, the clock is centred.
        let left = (-r, edge + dw + pad);
        let centre = (w / 2.0 - cw / 2.0 - pad, w / 2.0 + cw / 2.0 + pad);
        let right = (w - (edge + lw + pad), w + r);
        self.spans = vec![(left.0 - r, left.1 + r), (centre.0 - r, centre.1 + r), (right.0 - r, right.1 + r)];

        unsafe {
            let rc = RECT { left: 0, top: 0, right: self.size.cx, bottom: self.size.cy };
            self.rt.BindDC(self.memdc, &rc)?;
            self.rt.BeginDraw();
            self.rt.Clear(Some(&color(0.0, 0.0, 0.0, 0.0)));
            // The shape: one figure per notch, plus the bar's strip as it drops.
            let geom = self.d2d.CreatePathGeometry()?;
            let sink = geom.Open()?;
            for (x0, x1) in [left, centre, right] {
                sink.BeginFigure(v(x0 - r, 0.0), D2D1_FIGURE_BEGIN_FILLED);
                sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v(x0 - r * 0.45, 0.0), point2: v(x0 - r * 0.25, row), point3: v(x0 + r * 0.2, row) });
                sink.AddLine(v(x1 - r * 0.2, row));
                sink.AddBezier(&D2D1_BEZIER_SEGMENT { point1: v(x1 + r * 0.25, row), point2: v(x1 + r * 0.45, 0.0), point3: v(x1 + r, 0.0) });
                sink.EndFigure(D2D1_FIGURE_END_CLOSED);
            }
            sink.Close()?;
            self.rt.FillGeometry(&geom, &self.ink, None::<&ID2D1Brush>);
            if t > 0.001 {
                self.rt.FillRectangle(&D2D_RECT_F { left: 0.0, top: 0.0, right: w, bottom: row * t }, &self.ink);
            }
            self.draw_text(&date, edge, row, 1.0)?;
            self.draw_text(&clock, w / 2.0 - cw / 2.0, row, 1.0)?;
            self.draw_text(label, w - edge - lw, row, 0.6)?;
            // The open bar's own content: in late, as Kasumi's.
            let fade = ((t - 0.6) / 0.4).clamp(0.0, 1.0);
            if fade > 0.0 {
                self.draw_text("Kasumi native test · right-click a notch to quit", left.1 + 2.0 * pad, row, fade * 0.85)?;
                let (mw, _) = self.text_width(&self.memory)?;
                self.draw_text(&self.memory, right.0 - 2.0 * pad - mw, row, fade * 0.85)?;
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
            unsafe { SetTimer(Some(self.hwnd), ANIM, 8, None) };
        }
    }

    fn frame(&mut self) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32().min(0.05);
        self.last = now;
        self.open.step(dt);
        if self.open.settled() {
            self.open.value = self.open.target;
            self.open.velocity = 0.0;
            self.animating = false;
            let _ = unsafe { KillTimer(Some(self.hwnd), ANIM) };
        }
        let _ = self.render();
    }

    /// Open while the cursor is on the top edge, a notch or the open bar; close 300 ms after it leaves.
    fn hover(&mut self) {
        let mut pt = POINT::default();
        if unsafe { GetCursorPos(&mut pt) }.is_err() {
            return;
        }
        let m = self.monitor;
        let on_monitor = pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom;
        let x = (pt.x - m.left) as f32;
        let y = (pt.y - m.top) as f32;
        let row = 30.0 * self.s;
        let inside = on_monitor
            && (y <= 1.0 || (y < row && (self.open.target > 0.5 || self.spans.iter().any(|&(a, b)| x >= a && x <= b))));
        if inside {
            self.away_since = None;
            self.set_open(true);
        } else if self.open.target > 0.5 {
            let since = *self.away_since.get_or_insert_with(Instant::now);
            if since.elapsed().as_millis() >= 300 {
                self.set_open(false);
            }
        }
        // Poll faster near the top, slower elsewhere.
        let near = on_monitor && y < row * 3.0;
        unsafe { SetTimer(Some(self.hwnd), HOVER, if near || self.open.target > 0.5 { 30 } else { 120 }, None) };
    }
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TIMER => {
            APP.with(|a| {
                let mut a = a.borrow_mut();
                let Some(app) = a.as_mut() else { return };
                match wp.0 {
                    ANIM => app.frame(),
                    HOVER => app.hover(),
                    CLOCK => {
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

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let offscreen = args.iter().any(|a| a == "--offscreen");
    let cycle = args.iter().any(|a| a == "--cycle");
    let gpu = args.iter().any(|a| a == "--gpu");
    let seconds: Option<u32> = args.iter().position(|a| a == "--seconds").and_then(|i| args.get(i + 1)).and_then(|s| s.parse().ok());

    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut info);
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        let s = dx as f32 / 96.0;
        let m = info.rcMonitor;
        let size = SIZE { cx: m.right - m.left, cy: (30.0 * s).ceil() as i32 };
        let pos = if offscreen { POINT { x: -32000, y: -32000 } } else { POINT { x: m.left, y: m.top } };

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
            r#type: if gpu { D2D1_RENDER_TARGET_TYPE_DEFAULT } else { D2D1_RENDER_TARGET_TYPE_SOFTWARE },
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
        let font = dw.CreateTextFormat(w!("Segoe UI"), None, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, 12.5 * s, w!("en-us"))?;

        let mut app = App {
            hwnd,
            pos,
            size,
            monitor: m,
            s,
            d2d,
            dw,
            rt,
            ink,
            text,
            font,
            memdc,
            open: Spring { value: 0.0, velocity: 0.0, target: 0.0 },
            last: Instant::now(),
            animating: false,
            away_since: None,
            spans: Vec::new(),
            memory: String::new(),
        };
        app.render()?;
        // --snapshot <dir>: the closed and the open bar as closed.bmp / open.bmp, then quit.
        if let Some(dir) = args.iter().position(|a| a == "--snapshot").and_then(|i| args.get(i + 1)) {
            let save = |name: &str| {
                let (w, h) = (size.cx as u32, size.cy as u32);
                let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
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
                let _ = std::fs::write(std::path::Path::new(dir).join(name), out);
            };
            save("closed.bmp");
            app.open.value = 1.0;
            app.memory = format!("{:.1} MB", private_mb());
            app.render()?;
            save("open.bmp");
            return Ok(());
        }
        APP.with(|a| *a.borrow_mut() = Some(app));
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);

        SetTimer(Some(hwnd), HOVER, 120, None);
        SetTimer(Some(hwnd), CLOCK, to_next_minute(), None);
        if cycle {
            SetTimer(Some(hwnd), CYCLE, 2000, None);
        }
        if let Some(n) = seconds {
            SetTimer(Some(hwnd), QUIT, n * 1000, None);
        }

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    Ok(())
}
