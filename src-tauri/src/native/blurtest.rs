//! Prototype (`Kasumi.exe --blur-test`): one notch with a live blur of what's behind it
//! (acrylic), to see whether Windows keeps the blur live for a window that never has the
//! focus. Not the bars' way of drawing: they're layered windows, which DWM can't blur.
//!
//! The window has no picture of its own (WS_EX_NOREDIRECTIONBITMAP); DirectComposition
//! shows a tree on it: the host backdrop (what's behind, blurred by the system) and a
//! tint, both clipped to the notch's shape (a Direct2D path), and the text on top. Drag it to move it; right-click: the tint's strength; middle-click: close.

use std::cell::RefCell;
use windows::core::{Interface, HSTRING};
use windows::Graphics::DirectX::{DirectXAlphaMode, DirectXPixelFormat};
use windows::Graphics::SizeInt32;
use windows::System::DispatcherQueueController;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Direct2D::Common::{D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_END_CLOSED, D2D_RECT_F, D2D_SIZE_F};
use windows::Win32::Graphics::Direct2D::{
    D2D1CreateFactory, ID2D1Device, ID2D1DeviceContext, ID2D1Factory1, ID2D1PathGeometry, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_SMALL, D2D1_DRAW_TEXT_OPTIONS_NONE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_SWEEP_DIRECTION_CLOCKWISE, D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE,
};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{D3D11CreateDevice, ID3D11Device, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION};
use windows::Win32::Graphics::DirectWrite::{
    DWriteCreateFactory, IDWriteFactory, IDWriteTextFormat, DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_WEIGHT,
    DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD, DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER, DWRITE_TEXT_ALIGNMENT_CENTER,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::WinRT::Composition::{ICompositionDrawingSurfaceInterop, ICompositorDesktopInterop, ICompositorInterop};
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Color;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionBrush, CompositionDrawingSurface, CompositionGraphicsDevice, Compositor};
use windows::core::{implement, Ref};
use windows::Graphics::IGeometrySource2D;
use windows::Win32::Graphics::Direct2D::{ID2D1Factory, ID2D1Geometry};
use windows::Win32::System::WinRT::Graphics::Direct2D::{IGeometrySource2DInterop, IGeometrySource2DInterop_Impl};
use windows::UI::Composition::CompositionPath;
use windows_numerics::{Matrix3x2, Vector2};

/// A Direct2D geometry handed to composition (a path for its clip).
#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct Shape {
    geom: ID2D1Geometry,
    /// Width, height, top (concave) and bottom radii: to build it again with
    /// composition's own factory when it asks for that.
    dims: (f32, f32, f32, f32),
}

impl windows::Graphics::IGeometrySource2D_Impl for Shape_Impl {}

impl IGeometrySource2DInterop_Impl for Shape_Impl {
    fn GetGeometry(&self) -> windows::core::Result<ID2D1Geometry> {
        note("composition asked for the shape");
        Ok(self.geom.clone())
    }
    fn TryGetGeometryUsingFactory(&self, factory: Ref<'_, ID2D1Factory>) -> windows::core::Result<ID2D1Geometry> {
        note("composition asked for the shape from its own factory");
        let (w, h, rc, r) = self.dims;
        let f = factory.ok()?;
        unsafe { notch(f, w, h, rc, r)?.cast() }
    }
}

/// The tint (black, Kasumi's background).
const TINT: f32 = 0.6;

struct State {
    _queue: DispatcherQueueController,
    _compositor: Compositor,
    _target: DesktopWindowTarget,
    _device: CompositionGraphicsDevice,
    text: CompositionDrawingSurface,
    /// The blur and tint's layer, cut to the notch or a plain rectangle (right-click).
    back: windows::UI::Composition::ContainerVisual,
    shape_clip: windows::UI::Composition::CompositionClip,
    rect_clip: windows::UI::Composition::CompositionClip,
    clipped: bool,
    /// The notch's outline, drawn with the text (always visible: where the shape is).
    outline: ID2D1Geometry,
    big: IDWriteTextFormat,
    small: IDWriteTextFormat,
    size: (f32, f32),
    scale: f32,
    /// Why there's no blur, if there isn't.
    backdrop: Result<(), String>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

pub fn run() {
    if let Err(e) = unsafe { start() } {
        unsafe {
            let _ = MessageBoxW(None, &HSTRING::from(format!("The blur test couldn't start:\n{e:?}")), windows::core::w!("Kasumi blur test"), MB_OK);
        }
        return;
    }
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// What came of the blur, in %TEMP%\kasumi-blur-test.txt.
/// A line added to the report.
fn note(line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(std::env::temp_dir().join("kasumi-blur-test.txt")) {
        let _ = writeln!(f, "{line}");
    }
}

fn report(r: &Result<(), String>) {
    note(&match r {
        Ok(()) => "blur: ok".to_string(),
        Err(e) => format!("blur: failed, {e}"),
    });
}

fn black(a: f32) -> Color {
    Color { A: (a * 255.0).round() as u8, R: 0, G: 0, B: 0 }
}

unsafe fn start() -> windows::core::Result<()> {
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        let _ = std::fs::write(std::env::temp_dir().join("kasumi-blur-test.txt"), "");
        // Composition needs a dispatcher queue on its thread.
        let queue = CreateDispatcherQueueController(DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        })?;

        let inst = GetModuleHandleW(None)?;
        let class = windows::core::w!("KasumiBlurTest");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: inst.into(), lpszClassName: class, hCursor: LoadCursorW(None, IDC_ARROW)?, ..Default::default() });
        let hwnd = CreateWindowExW(
            WS_EX_NOREDIRECTIONBITMAP | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class,
            windows::core::w!("Kasumi blur test"),
            WS_POPUP,
            0,
            0,
            10,
            10,
            None,
            None,
            Some(inst.into()),
            None,
        )?;

        // 460 x 120 (logical px), a quarter down the primary screen, centred (or far
        // off screen with --offscreen: only the report).
        let scale = GetDpiForWindow(hwnd) as f32 / 96.0;
        let (w, h) = ((460.0 * scale).round(), (120.0 * scale).round());
        let mon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut mi = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        let _ = GetMonitorInfoW(mon, &mut mi);
        let r = mi.rcWork;
        let x = r.left + ((r.right - r.left) - w as i32) / 2;
        let y = r.top + (r.bottom - r.top) / 4;
        let (x, y) = if std::env::args().any(|a| a == "--offscreen") { (-30000, -30000) } else { (x, y) };
        SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w as i32, h as i32, SWP_NOACTIVATE)?;
        // Windows 11 shows a host backdrop only for windows that ask for it.
        let on = windows::Win32::Foundation::TRUE;
        let dwm = windows::Win32::Graphics::Dwm::DwmSetWindowAttribute(hwnd, windows::Win32::Graphics::Dwm::DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as *const _, 4);

        // Direct3D → Direct2D → composition's drawing device.
        let mut d3d: Option<ID3D11Device> = None;
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, Default::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut d3d), None, None)?;
        let dxgi: IDXGIDevice = d3d.ok_or_else(|| windows::core::Error::from_win32())?.cast()?;
        let d2f: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
        let d2d: ID2D1Device = d2f.CreateDevice(&dxgi)?;

        let compositor = Compositor::new()?;
        let target = compositor.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)?;
        let device = compositor.cast::<ICompositorInterop>()?.CreateGraphicsDevice(&d2d)?;
        let px = SizeInt32 { Width: w as i32, Height: h as i32 };
        let full = Vector2 { X: w, Y: h };

        // The notch's shape: a clip on the layers under the text (a mask brush won't take
        // the backdrop as its source: "Unsupported source brush type").
        let dims = (w, h, 18.0 * scale, 22.0 * scale);
        let geom = notch(&d2f, dims.0, dims.1, dims.2, dims.3)?;
        let source: IGeometrySource2D = Shape { geom: geom.cast()?, dims }.into();

        let clip = compositor.CreateGeometricClipWithGeometry(&compositor.CreatePathGeometryWithPath(&CompositionPath::Create(&source)?)?)?;

        let root = compositor.CreateContainerVisual()?;
        root.SetSize(full)?;
        let back = compositor.CreateContainerVisual()?;
        back.SetSize(full)?;
        back.SetClip(&clip)?;
        root.Children()?.InsertAtTop(&back)?;
        let layer = |brush: &CompositionBrush| -> windows::core::Result<()> {
            let v = compositor.CreateSpriteVisual()?;
            v.SetSize(full)?;
            v.SetBrush(brush)?;
            back.Children()?.InsertAtTop(&v)
        };
        // What's behind, blurred by the system (needs Windows' transparency effects on).
        let backdrop = match compositor.CreateHostBackdropBrush() {
            Ok(b) => b.cast().and_then(|b: CompositionBrush| layer(&b)).map_err(|e| format!("showing the backdrop: {e}")),
            Err(e) => Err(format!("the host backdrop: {e}")),
        };
        report(&backdrop);
        note(&format!("host backdrop asked for: {dwm:?}"));
        let tint = compositor.CreateColorBrushWithColor(black(TINT))?;
        layer(&tint.cast()?)?;
        // The text on its own surface (it stays inside the notch: no clip needed).
        let text = device.CreateDrawingSurface2(px, DirectXPixelFormat::B8G8R8A8UIntNormalized, DirectXAlphaMode::Premultiplied)?;
        let tv = compositor.CreateSpriteVisual()?;
        tv.SetSize(full)?;
        tv.SetBrush(&compositor.CreateSurfaceBrushWithSurface(&text)?)?;
        root.Children()?.InsertAtTop(&tv)?;
        target.SetRoot(&root)?;

        let dw: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
        let font = |weight: DWRITE_FONT_WEIGHT, size: f32| -> windows::core::Result<IDWriteTextFormat> {
            let f = dw.CreateTextFormat(windows::core::w!("Segoe UI Variable Text"), None, weight, DWRITE_FONT_STYLE_NORMAL, DWRITE_FONT_STRETCH_NORMAL, size * scale, windows::core::w!("en-us"))?;
            f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_CENTER)?;
            f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
            Ok(f)
        };
        let big = font(DWRITE_FONT_WEIGHT_SEMI_BOLD, 22.0)?;
        let small = font(DWRITE_FONT_WEIGHT_NORMAL, 11.5)?;

        let rect_clip = compositor.CreateInsetClip()?.cast()?;
        let shape_clip = clip.cast()?;
        let outline = geom.cast()?;
        STATE.with(|s| {
            *s.borrow_mut() = Some(State {
                _queue: queue,
                _target: target,
                _device: device,
                text,
                rect_clip,
                _compositor: compositor,
                back,
                shape_clip,
                clipped: true,
                outline,
                big,
                small,
                size: (w, h),
                scale,
                backdrop,
            })
        });
        redraw();
        SetTimer(Some(hwnd), 1, 1000, None);
        // A forgotten test closes itself after 15 minutes.
        SetTimer(Some(hwnd), 2, 15 * 60 * 1000, None);
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        Ok(())
    }
}

/// Draws on a composition surface with Direct2D (in pixels, from its top left).
unsafe fn draw(surface: &CompositionDrawingSurface, f: impl FnOnce(&ID2D1DeviceContext) -> windows::core::Result<()>) -> windows::core::Result<()> {
    unsafe {
        let interop: ICompositionDrawingSurfaceInterop = surface.cast()?;
        let mut off = POINT::default();
        let dc: ID2D1DeviceContext = interop.BeginDraw(None, &mut off)?;
        dc.SetDpi(96.0, 96.0);
        dc.SetTransform(&Matrix3x2::translation(off.x as f32, off.y as f32));
        dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
        let r = f(&dc);
        interop.EndDraw()?;
        r
    }
}

/// A notch hanging from its top edge: concave corners up there, round ones below.
unsafe fn notch(f: &ID2D1Factory, w: f32, h: f32, rc: f32, r: f32) -> windows::core::Result<ID2D1PathGeometry> {
    unsafe {
        let geom = f.CreatePathGeometry()?;
        let sink = geom.Open()?;
        let v = |x: f32, y: f32| Vector2 { X: x, Y: y };
        let arc = |x: f32, y: f32, rr: f32, cw: bool| D2D1_ARC_SEGMENT {
            point: v(x, y),
            size: D2D_SIZE_F { width: rr, height: rr },
            rotationAngle: 0.0,
            sweepDirection: if cw { D2D1_SWEEP_DIRECTION_CLOCKWISE } else { D2D1_SWEEP_DIRECTION_COUNTER_CLOCKWISE },
            arcSize: D2D1_ARC_SIZE_SMALL,
        };
        sink.BeginFigure(v(0.0, 0.0), D2D1_FIGURE_BEGIN_FILLED);
        sink.AddArc(&arc(rc, rc, rc, true));
        sink.AddLine(v(rc, h - r));
        sink.AddArc(&arc(rc + r, h, r, false));
        sink.AddLine(v(w - rc - r, h));
        sink.AddArc(&arc(w - rc, h - r, r, false));
        sink.AddLine(v(w - rc, rc));
        sink.AddArc(&arc(w, 0.0, rc, true));
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        geom.cast()
    }
}

fn redraw() {
    STATE.with(|s| {
        let s = s.borrow();
        let Some(s) = s.as_ref() else { return };
        let t = unsafe { GetLocalTime() };
        let clock = format!("{:02}:{:02}:{:02}", t.wHour, t.wMinute, t.wSecond);
        let line = match &s.backdrop {
            Ok(()) => format!("{}  ·  blur + {}% black", if s.clipped { "Cut to the notch" } else { "Plain rectangle" }, (TINT * 100.0).round()),
            Err(e) => format!("No blur: {e}"),
        };
        let hint = "drag: move  ·  right-click: shape on / off  ·  middle-click: close";
        let (w, h, k) = (s.size.0, s.size.1, s.scale);
        let _ = unsafe {
            draw(&s.text, |dc| {
                let white = dc.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }, None)?;
                let dim = dc.CreateSolidColorBrush(&D2D1_COLOR_F { r: 1.0, g: 1.0, b: 1.0, a: 0.62 }, None)?;
                dc.DrawGeometry(&s.outline, &dim, 1.0, None);
                let wide = |t: &str| t.encode_utf16().collect::<Vec<u16>>();
                let row = |top: f32, bottom: f32| D2D_RECT_F { left: 30.0 * k, top, right: w - 30.0 * k, bottom };
                dc.DrawText(&wide(&clock), &s.big, &row(8.0 * k, h * 0.5), &white, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                dc.DrawText(&wide(&line), &s.small, &row(h * 0.5, h * 0.5 + 22.0 * k), &dim, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                dc.DrawText(&wide(hint), &s.small, &row(h * 0.5 + 20.0 * k, h - 8.0 * k), &dim, D2D1_DRAW_TEXT_OPTIONS_NONE, DWRITE_MEASURING_MODE_NATURAL);
                Ok(())
            })
        };
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            // Anywhere on it moves it.
            WM_NCHITTEST => LRESULT(HTCAPTION as isize),
            WM_NCRBUTTONUP => {
                STATE.with(|s| {
                    if let Some(s) = s.borrow_mut().as_mut() {
                        s.clipped = !s.clipped;
                        let _ = s.back.SetClip(if s.clipped { &s.shape_clip } else { &s.rect_clip });
                    }
                });
                redraw();
                LRESULT(0)
            }
            // No window menu on a right-click, no maximising on a double-click.
            WM_CONTEXTMENU | WM_NCRBUTTONDOWN | WM_NCLBUTTONDBLCLK => LRESULT(0),
            WM_NCMBUTTONUP => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER if wp.0 == 1 => {
                redraw();
                LRESULT(0)
            }
            WM_TIMER if wp.0 == 2 => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
