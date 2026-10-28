//! A widget's hint ([general] tooltips): a pill with its text in a small window of
//! its own, just past the bar (below a top bar, above a bottom one), shown once the
//! cursor has rested on the widget a moment. The pages used the browser's `title`s.

use windows::core::Result;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::gfx::{rect, with_alpha, Font, Gfx};
use super::surface::Dib;
use super::widgets::Look;

/// Padding inside the pill, its gap from the bar, and its text size.
const PAD_X: f32 = 10.0;
const PAD_Y: f32 = 5.0;
pub const GAP: f32 = 6.0;
const SIZE_PX: f32 = 12.0;

#[derive(Default)]
pub struct Tooltip {
    hwnd: Option<HWND>,
    dib: Option<Dib>,
    /// What it shows (empty: hidden).
    pub text: String,
}

impl Tooltip {
    /// Shows `text` centred on `x` (screen px), its near edge at `y` (below it on a
    /// top bar; above it when `up`), kept within `left..right`.
    pub fn show(&mut self, gfx: &Gfx, look: &Look, text: &str, (x, y): (i32, i32), up: bool, (left, right): (i32, i32), scale: f32) -> Result<()> {
        let Some((w_px, h_px)) = self.render(gfx, look, text, (0.0, 0.0), false, scale)? else { return Ok(()) };
        let px = (x - w_px / 2).clamp(left, (right - w_px).max(left));
        let py = if up { y - h_px } else { y };
        self.place(px, py, w_px, h_px)
    }

    /// Another app's tooltip in Kasumi's look: drawn over its box (screen px), at least
    /// as large, so the original under it doesn't show (apptips.rs).
    pub fn show_over(&mut self, gfx: &Gfx, look: &Look, text: &str, cover: [i32; 4], scale: f32) -> Result<()> {
        // A couple of px past it each way: its rounded corners and edge stay covered.
        let min = ((cover[2] - cover[0]) as f32 / scale + 4.0, (cover[3] - cover[1]) as f32 / scale + 4.0);
        let Some((w_px, h_px)) = self.render(gfx, look, text, min, true, scale)? else { return Ok(()) };
        let (cx, cy) = ((cover[0] + cover[2]) / 2, (cover[1] + cover[3]) / 2);
        self.place(cx - w_px / 2, cy - h_px / 2, w_px, h_px)
    }

    /// Draws the pill (at least `min` px, logical) into its bitmap: its size in device
    /// px, None (hidden) without text. `boxy`: rounded corners rather than round ends.
    fn render(&mut self, gfx: &Gfx, look: &Look, text: &str, min: (f32, f32), boxy: bool, scale: f32) -> Result<Option<(i32, i32)>> {
        let font = Font::new(look.main.clone(), SIZE_PX);
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.is_empty() {
            self.hide();
            return Ok(None);
        }
        let lh = gfx.text("Ag", &font).map_or(15.0, |t| t.height);
        let tw = lines.iter().map(|l| gfx.measure(l, &font)).fold(0.0, f32::max);
        let (w, h) = ((tw + 2.0 * PAD_X).max(min.0), (lines.len() as f32 * lh + 2.0 * PAD_Y).max(min.1));
        let (w_px, h_px) = ((w * scale).ceil() as i32, (h * scale).ceil() as i32);
        if self.hwnd.is_none() {
            let h = unsafe {
                CreateWindowExW(
                    WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT,
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
        }
        if self.dib.as_ref().is_none_or(|d| d.w < w_px || d.h < h_px) {
            self.dib = Some(Dib::new(w_px.max(64), h_px.max(16))?);
        }
        let dib = self.dib.as_ref().unwrap();
        gfx.begin(dib.dc, w_px, h_px, scale)?;
        // A pill: fully round ends on one line, rounded corners on several (and over
        // another app's box, whose corners must stay covered).
        let radius = if boxy { 6.0 } else if lines.len() == 1 { h / 2.0 } else { 10.0 };
        // Over another app's tooltip the fill is opaque: it mustn't show through.
        let fill = if boxy { with_alpha(look.bg, 1.0) } else { with_alpha(look.bg, look.opacity.max(0.92)) };
        gfx.fill_round(rect(0.0, 0.0, w, h), radius, fill);
        let top = (h - lines.len() as f32 * lh) / 2.0;
        for (i, l) in lines.iter().enumerate() {
            let lw = gfx.measure(l, &font);
            gfx.text_at(l, &font, (w - lw) / 2.0, top + (i as f32 + 0.5) * lh, look.fg);
        }
        gfx.end()?;
        self.text = text.to_string();
        Ok(Some((w_px, h_px)))
    }

    /// Puts the drawn bitmap on screen at (x, y), w × h device px, on top.
    fn place(&mut self, x: i32, y: i32, w_px: i32, h_px: i32) -> Result<()> {
        let (Some(hwnd), Some(dib)) = (self.hwnd, self.dib.as_ref()) else { return Ok(()) };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            UpdateLayeredWindow(hwnd, None, Some(&POINT { x, y }), Some(&SIZE { cx: w_px, cy: h_px }), Some(dib.dc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(())
    }

    pub fn hide(&mut self) {
        if let Some(h) = self.hwnd {
            if !self.text.is_empty() {
                unsafe {
                    let _ = ShowWindow(h, SW_HIDE);
                }
            }
        }
        self.text.clear();
    }

    pub fn shown(&self) -> bool {
        !self.text.is_empty()
    }
}

impl Drop for Tooltip {
    fn drop(&mut self) {
        if let Some(h) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
    }
}
