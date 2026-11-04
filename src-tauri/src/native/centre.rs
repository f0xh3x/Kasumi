//! The launcher in the middle of the screen ([widgets.launcher] style = "centre"): its
//! panel, with a field of its own, in a window of its own, the screen lightly dimmed
//! behind it ([widgets.launcher] dim, on by default).
//! The bar it opened from keeps the focus, so typing, Esc and closing when the focus
//! moves on work as they do there; this window only takes the mouse.

use windows::core::{w, Result};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, AC_SRC_ALPHA, AC_SRC_OVER, BLACK_BRUSH, BLENDFUNCTION, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassW, SetLayeredWindowAttributes, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, LWA_ALPHA,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, ULW_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use super::gfx::{rect, with_alpha, Gfx};
use super::surface::Dib;
use super::widgets::launcher::LauncherPanel;
use super::widgets::{Act, Button, Ctx, Look, Opts, Panel, Reply};
use crate::win::Monitor;

/// The box's padding around the panel, and its corners.
const PAD: f32 = 10.0;
const RADIUS: f32 = 16.0;
/// How far down the screen the box's top sits (it grows down as results come).
const TOP: f32 = 0.22;
/// How dark the screen behind it goes (0..255).
const VEIL: u8 = 64;

pub struct Centre {
    pub hwnd: Option<HWND>,
    veil: Option<HWND>,
    dib: Option<Dib>,
    pub panel: LauncherPanel,
    /// The monitor's scale as last shown (the mouse comes in device px).
    scale: f32,
    /// The screen behind it dims.
    dim: bool,
}

unsafe extern "system" fn veil_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(h, m, wp, lp)
}

impl Centre {
    pub fn new(opts: &Opts) -> Centre {
        Centre { hwnd: None, veil: None, dib: None, panel: LauncherPanel::new(opts, true), scale: 1.0, dim: opts.flag("dim") != Some(false) }
    }

    /// Draws the box (again) on `mon`, the screen dimmed behind it.
    pub fn show(&mut self, gfx: &Gfx, look: &Look, opts: &Opts, mon: &Monitor) -> Result<()> {
        let scale = mon.scale as f32;
        self.scale = scale;
        // Its list hangs under its field, whichever edge the bar is on.
        let mut look = look.clone();
        look.bottom = false;
        let cx = Ctx { gfx, look: &look, opts, h: 30.0, open: 1.0 };
        let (cw, ch) = self.panel.size(&cx);
        let (w, h) = (cw + 2.0 * PAD, ch + 2.0 * PAD);
        let (w_px, h_px) = ((w * scale).ceil() as i32, (h * scale).ceil() as i32);
        if self.dim {
            self.show_veil(mon)?;
        }
        let hwnd = match self.hwnd {
            Some(h) => h,
            None => {
                let h = unsafe {
                    CreateWindowExW(
                        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                        super::SURFACE_CLASS,
                        w!("Kasumi"),
                        WS_POPUP,
                        0,
                        0,
                        1,
                        1,
                        None,
                        None,
                        Some(GetModuleHandleW(None)?.into()),
                        None,
                    )?
                };
                self.hwnd = Some(h);
                h
            }
        };
        if self.dib.as_ref().is_none_or(|d| d.w < w_px || d.h < h_px) {
            self.dib = Some(Dib::new(w_px.max(64), h_px.max(64))?);
        }
        let dib = self.dib.as_ref().unwrap();
        gfx.begin(dib.dc, w_px, h_px, scale)?;
        gfx.fill_round(rect(0.0, 0.0, w, h), RADIUS, with_alpha(look.bg, look.opacity.max(0.92)));
        self.panel.draw(&cx, PAD, PAD, 1.0);
        gfx.end()?;
        let pos = POINT { x: mon.left + (mon.width() - w_px) / 2, y: mon.top + ((mon.bottom - mon.top) as f32 * TOP) as i32 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&SIZE { cx: w_px, cy: h_px }), Some(dib.dc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(())
    }

    /// The dimmed screen: black, mostly see-through, letting every click through (a
    /// click there lands on the app, which takes the focus and closes the launcher).
    fn show_veil(&mut self, mon: &Monitor) -> Result<()> {
        static CLASS: std::sync::Once = std::sync::Once::new();
        let veil = match self.veil {
            Some(h) => h,
            None => unsafe {
                let instance = GetModuleHandleW(None)?;
                CLASS.call_once(|| {
                    RegisterClassW(&WNDCLASSW {
                        lpfnWndProc: Some(veil_proc),
                        hInstance: instance.into(),
                        lpszClassName: w!("KasumiVeil"),
                        hbrBackground: HBRUSH(GetStockObject(BLACK_BRUSH).0),
                        ..Default::default()
                    });
                });
                let h = CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                    w!("KasumiVeil"),
                    w!("Kasumi"),
                    WS_POPUP,
                    0,
                    0,
                    1,
                    1,
                    None,
                    None,
                    Some(instance.into()),
                    None,
                )?;
                SetLayeredWindowAttributes(h, COLORREF(0), VEIL, LWA_ALPHA)?;
                self.veil = Some(h);
                h
            },
        };
        unsafe {
            let _ = SetWindowPos(veil, Some(HWND_TOPMOST), mon.left, mon.top, mon.width(), mon.bottom - mon.top, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        }
        Ok(())
    }

    /// Client device px → the panel's content px.
    fn local(&self, (x, y): (i32, i32)) -> (f32, f32) {
        (x as f32 / self.scale - PAD, y as f32 / self.scale - PAD)
    }

    /// The mouse over it. True: it changed (draw it again).
    pub fn hover(&mut self, at: (i32, i32)) -> bool {
        let p = self.local(at);
        self.panel.hover(Some(p))
    }

    /// The wheel over it. True: the choice moved.
    pub fn wheel(&mut self, at: (i32, i32), notches: f32) -> bool {
        let p = self.local(at);
        matches!(self.panel.wheel(p, notches), Reply::Redraw)
    }

    /// A click on it: what to do, and whether it changed.
    pub fn click(&mut self, button: Button, at: (i32, i32)) -> (Option<Act>, bool) {
        let p = self.local(at);
        match self.panel.click(button, p) {
            Reply::Do(act) | Reply::CloseAnd(act) | Reply::ToggleAnd(act) => (Some(act), true),
            Reply::Redraw => (None, true),
            _ => (None, false),
        }
    }
}

impl Drop for Centre {
    fn drop(&mut self) {
        for h in [self.hwnd.take(), self.veil.take()].into_iter().flatten() {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
    }
}
