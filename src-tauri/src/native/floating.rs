//! Alt + Tab in the middle of the screen ([alt_tab] position = "center"): the same
//! cards as out of a notch, on a rounded panel in a window of its own, with the
//! windows' live previews placed on them (DWM thumbnails, as Surface::sync_thumbs).

use windows::core::Result;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, RECT, SIZE};
use windows::Win32::Graphics::Dwm::*;
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::gfx::{rect, with_alpha, Gfx};
use super::surface::Dib;
use super::widgets::alttab::AltTabPanel;
use super::widgets::{Button, Ctx, Look, Opts, Panel, Reply};
use crate::win::Monitor;

/// The panel's padding around the cards, and its corners.
pub const PAD: f32 = 16.0;
const RADIUS: f32 = 22.0;

pub struct Floating {
    pub hwnd: Option<HWND>,
    dib: Option<Dib>,
    pub panel: AltTabPanel,
    /// The monitor's scale as last shown (clicks come in device px).
    scale: f32,
    /// Live previews: window, DWM thumbnail, its box (device px).
    thumbs: Vec<(isize, isize, [i32; 4])>,
}

impl Floating {
    pub fn new() -> Floating {
        Floating { hwnd: None, dib: None, panel: AltTabPanel::new(), scale: 1.0, thumbs: Vec::new() }
    }

    /// Draws the switcher centred on `mon` (again after the choice moves).
    pub fn show(&mut self, gfx: &Gfx, look: &Look, opts: &Opts, mon: &Monitor) -> Result<()> {
        let scale = mon.scale as f32;
        self.scale = scale;
        let cx = Ctx { gfx, look, opts, h: 30.0, open: 1.0 };
        let (cw, ch) = self.panel.size(&cx);
        let (w, h) = (cw + 2.0 * PAD, ch + 2.0 * PAD);
        let (w_px, h_px) = ((w * scale).ceil() as i32, (h * scale).ceil() as i32);
        let hwnd = match self.hwnd {
            Some(h) => h,
            None => {
                let h = unsafe {
                    CreateWindowExW(
                        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                        super::SURFACE_CLASS,
                        windows::core::w!("Kasumi"),
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
        let pos = POINT { x: mon.left + (mon.width() - w_px) / 2, y: mon.top + (mon.bottom - mon.top - h_px) / 2 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&SIZE { cx: w_px, cy: h_px }), Some(dib.dc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        self.sync_thumbs(hwnd);
        Ok(())
    }

    /// The previews on their cards: new ones registered, moved ones placed again, gone
    /// ones let go.
    fn sync_thumbs(&mut self, hwnd: HWND) {
        let s = self.scale;
        let want: Vec<(isize, [i32; 4])> = self
            .panel
            .thumbs()
            .into_iter()
            .map(|(h, (x, y, w, hh))| (h, [((PAD + x) * s) as i32, ((PAD + y) * s) as i32, ((PAD + x + w) * s) as i32, ((PAD + y + hh) * s) as i32]))
            .collect();
        self.thumbs.retain(|(h, id, r)| {
            let keep = want.iter().any(|(wh, wr)| wh == h && wr == r);
            if !keep {
                unsafe {
                    let _ = DwmUnregisterThumbnail(*id);
                }
            }
            keep
        });
        for (h, r) in want {
            if self.thumbs.iter().any(|(th, _, _)| *th == h) {
                continue;
            }
            unsafe {
                let Ok(id) = DwmRegisterThumbnail(hwnd, HWND(h as _)) else { continue };
                let (bw, bh) = ((r[2] - r[0]) as f32, (r[3] - r[1]) as f32);
                let (mut fw, mut fh) = (bw, bh);
                if let Ok(src) = DwmQueryThumbnailSourceSize(id) {
                    if src.cx > 0 && src.cy > 0 {
                        let k = (bw / src.cx as f32).min(bh / src.cy as f32);
                        (fw, fh) = (src.cx as f32 * k, src.cy as f32 * k);
                    }
                }
                let (x0, y0) = (r[0] + ((bw - fw) / 2.0) as i32, r[1] + ((bh - fh) / 2.0) as i32);
                let props = DWM_THUMBNAIL_PROPERTIES {
                    dwFlags: DWM_TNP_RECTDESTINATION | DWM_TNP_VISIBLE | DWM_TNP_OPACITY | DWM_TNP_SOURCECLIENTAREAONLY,
                    rcDestination: RECT { left: x0, top: y0, right: x0 + fw as i32, bottom: y0 + fh as i32 },
                    opacity: 255,
                    fVisible: true.into(),
                    fSourceClientAreaOnly: false.into(),
                    ..Default::default()
                };
                let _ = DwmUpdateThumbnailProperties(id, &props);
                self.thumbs.push((h, id, r));
            }
        }
    }

    pub fn hide(&mut self) {
        for (_, id, _) in self.thumbs.drain(..) {
            unsafe {
                let _ = DwmUnregisterThumbnail(id);
            }
        }
        if let Some(h) = self.hwnd {
            unsafe {
                let _ = ShowWindow(h, SW_HIDE);
            }
        }
        self.panel.closed();
    }

    /// The mouse over it (client device px). True: the choice moved (draw it again).
    pub fn hover(&mut self, (x, y): (i32, i32)) -> bool {
        let p = (x as f32 / self.scale - PAD, y as f32 / self.scale - PAD);
        super::widgets::Panel::hover(&mut self.panel, Some(p))
    }

    /// A click on it (client device px): the card's window, to switch to (or a card's
    /// close button: it stays).
    pub fn click(&mut self, button: Button, (x, y): (i32, i32)) -> Option<super::widgets::Act> {
        let p = (x as f32 / self.scale - PAD, y as f32 / self.scale - PAD);
        match self.panel.click(button, p) {
            Reply::CloseAnd(act) => {
                self.hide();
                Some(act)
            }
            Reply::Do(act) => Some(act),
            _ => None,
        }
    }
}

impl Drop for Floating {
    fn drop(&mut self) {
        self.hide();
        if let Some(h) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
    }
}
