//! Blurred backgrounds ([theme] backdrop = "acrylic" | "mica"): a window right under a
//! bar's, showing only the blur, cut to the bar's shape; the bar's own fill on top is the
//! tint ([theme] backdrop_tint). Layered windows (the bars) can't be blurred by DWM, so the
//! blur is DirectComposition's: Acrylic is the host backdrop (what's behind, blurred by
//! Windows), Mica the blurred wallpaper (Windows 11; Acrylic's blur where there's none).
//!
//! The window takes no clicks (layered + transparent), no focus, no taskbar or Alt + Tab
//! entry, and stays right under its bar: the bar's window procedure calls `below`
//! whenever the bar moves in the z-order.

use std::cell::RefCell;
use std::collections::HashMap;
use windows::core::{implement, w, Interface, Ref, Result};
use windows::Graphics::IGeometrySource2D;
use windows::System::DispatcherQueueController;
use windows::Win32::Foundation::{COLORREF, E_NOTIMPL, HWND, TRUE};
use windows::Win32::Graphics::Direct2D::{ID2D1Factory, ID2D1Geometry};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_HOSTBACKDROPBRUSH};
use windows::Win32::System::WinRT::Composition::ICompositorDesktopInterop;
use windows::Win32::System::WinRT::Graphics::Direct2D::{IGeometrySource2DInterop, IGeometrySource2DInterop_Impl};
use windows::Win32::System::WinRT::{CreateDispatcherQueueController, DispatcherQueueOptions, DQTAT_COM_NONE, DQTYPE_THREAD_CURRENT};
use windows::Win32::UI::WindowsAndMessaging::*;
use windows::UI::Composition::Desktop::DesktopWindowTarget;
use windows::UI::Composition::{CompositionBrush, CompositionPath, CompositionPathGeometry, Compositor, SpriteVisual};
use windows_numerics::Vector2;

/// What's blurred.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    /// What's behind the bar.
    Acrylic,
    /// The wallpaper.
    Mica,
}

impl Kind {
    /// [theme] backdrop: None for "solid" (and anything else).
    pub fn from_config(s: &str) -> Option<Kind> {
        match s {
            "acrylic" => Some(Kind::Acrylic),
            "mica" => Some(Kind::Mica),
            _ => None,
        }
    }
}

thread_local! {
    /// The thread's compositor (it needs a dispatcher queue on its thread).
    static COMPOSITOR: RefCell<Option<(DispatcherQueueController, Compositor)>> = const { RefCell::new(None) };
    /// A bar's window → its backdrop's (for the bar's window procedure).
    static BELOW: RefCell<HashMap<isize, isize>> = RefCell::new(HashMap::new());
}

fn compositor() -> Result<Compositor> {
    COMPOSITOR.with(|c| {
        if let Some((_, comp)) = c.borrow().as_ref() {
            return Ok(comp.clone());
        }
        // The thread's COM is already up (an STA): the queue leaves it alone.
        let queue = unsafe {
            CreateDispatcherQueueController(DispatcherQueueOptions {
                dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
                threadType: DQTYPE_THREAD_CURRENT,
                apartmentType: DQTAT_COM_NONE,
            })?
        };
        let comp = Compositor::new()?;
        *c.borrow_mut() = Some((queue, comp.clone()));
        Ok(comp)
    })
}

/// A Direct2D geometry handed to composition (the clip's path). Composition takes it as
/// it's made (GetGeometry), so the geometry's own factory is fine.
#[implement(IGeometrySource2D, IGeometrySource2DInterop)]
struct Shape(ID2D1Geometry);

impl windows::Graphics::IGeometrySource2D_Impl for Shape_Impl {}

impl IGeometrySource2DInterop_Impl for Shape_Impl {
    fn GetGeometry(&self) -> Result<ID2D1Geometry> {
        Ok(self.0.clone())
    }
    fn TryGetGeometryUsingFactory(&self, _factory: Ref<'_, ID2D1Factory>) -> Result<ID2D1Geometry> {
        Err(E_NOTIMPL.into())
    }
}

pub struct Backdrop {
    hwnd: HWND,
    /// The bar's window it stays under.
    above: HWND,
    _target: DesktopWindowTarget,
    visual: SpriteVisual,
    path: CompositionPathGeometry,
    /// Position and size (physical px) as last placed.
    rect: (i32, i32, i32, i32),
    shown: bool,
}

const CLASS: windows::core::PCWSTR = w!("KasumiBackdrop");

impl Backdrop {
    /// A backdrop for the bar's window `above` (hidden until its first `update`).
    pub fn new(kind: Kind, above: HWND) -> Result<Backdrop> {
        let comp = compositor()?;
        unsafe {
            let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None)?;
            // A second registration fails harmlessly.
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(proc_), hInstance: inst.into(), lpszClassName: CLASS, ..Default::default() });
            let hwnd = CreateWindowExW(
                WS_EX_NOREDIRECTIONBITMAP | WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS,
                w!("Kasumi backdrop"),
                WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(inst.into()),
                None,
            )?;
            // Layered (for the clicks to pass) but shown as composition draws it.
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
            // Windows 11 shows a host backdrop only for windows that ask for it.
            let on = TRUE;
            let _ = DwmSetWindowAttribute(hwnd, DWMWA_USE_HOSTBACKDROPBRUSH, &on as *const _ as *const _, 4);
            let made = (|| -> Result<(DesktopWindowTarget, SpriteVisual, CompositionPathGeometry)> {
                let target = comp.cast::<ICompositorDesktopInterop>()?.CreateDesktopWindowTarget(hwnd, true)?;
                let brush: CompositionBrush = match kind {
                    Kind::Mica => comp.TryCreateBlurredWallpaperBackdropBrush().or_else(|_| comp.CreateHostBackdropBrush())?.cast()?,
                    Kind::Acrylic => comp.CreateHostBackdropBrush()?.cast()?,
                };
                let visual = comp.CreateSpriteVisual()?;
                visual.SetBrush(&brush)?;
                let path = comp.CreatePathGeometry()?;
                visual.SetClip(&comp.CreateGeometricClipWithGeometry(&path)?)?;
                target.SetRoot(&visual)?;
                Ok((target, visual, path))
            })();
            let (target, visual, path) = match made {
                Ok(m) => m,
                Err(e) => {
                    let _ = DestroyWindow(hwnd);
                    return Err(e);
                }
            };
            BELOW.with(|b| b.borrow_mut().insert(above.0 as isize, hwnd.0 as isize));
            Ok(Backdrop { hwnd, above, _target: target, visual, path, rect: (0, 0, 0, 0), shown: false })
        }
    }

    /// Where the bar's window is (physical px) and its shape there (px, from its top left).
    pub fn update(&mut self, x: i32, y: i32, w: i32, h: i32, shape: &ID2D1Geometry, shown: bool) {
        let source: IGeometrySource2D = Shape(shape.clone()).into();
        if let Ok(path) = CompositionPath::Create(&source) {
            let _ = self.path.SetPath(&path);
        }
        let rect = (x, y, w, h);
        if rect != self.rect {
            self.rect = rect;
            let _ = self.visual.SetSize(Vector2 { X: w as f32, Y: h as f32 });
            unsafe {
                let _ = SetWindowPos(self.hwnd, Some(self.above), x, y, w, h, SWP_NOACTIVATE);
            }
        }
        self.set_shown(shown);
    }

    pub fn set_shown(&mut self, shown: bool) {
        if shown == self.shown {
            return;
        }
        self.shown = shown;
        unsafe {
            let _ = ShowWindow(self.hwnd, if shown { SW_SHOWNOACTIVATE } else { SW_HIDE });
        }
        if shown {
            restack(self.above, self.hwnd);
        }
    }
}

unsafe extern "system" fn proc_(hwnd: HWND, msg: u32, wp: windows::Win32::Foundation::WPARAM, lp: windows::Win32::Foundation::LPARAM) -> windows::Win32::Foundation::LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

fn restack(above: HWND, hwnd: HWND) {
    unsafe {
        let _ = SetWindowPos(hwnd, Some(above), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
    }
}

/// The bar's window `above` moved in the z-order: its backdrop follows (from its window
/// procedure's WM_WINDOWPOSCHANGED; no borrow of the native state).
pub fn below(above: HWND) {
    if let Some(h) = BELOW.with(|b| b.borrow().get(&(above.0 as isize)).copied()) {
        restack(above, HWND(h as *mut _));
    }
}

impl Drop for Backdrop {
    fn drop(&mut self) {
        BELOW.with(|b| b.borrow_mut().remove(&(self.above.0 as isize)));
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}
