//! The menu's look: a window per level (the menu, then each submenu open), drawn
//! like Kasumi's panels and sized to its entries. The accent marks the row under the
//! cursor (or chosen with the keys), with text that reads on it.

use std::time::Instant;

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

use super::shell::{Entry, Icon};
use crate::native::gfx::{rect, rgba, white, with_alpha, Color, Font, Gfx};
use crate::native::surface::Dib;
use crate::native::widgets::icon::{AppIcon, Style as IconStyle};
use crate::native::widgets::{fit_text, Look};
use crate::win::Monitor;

pub const CLASS: PCWSTR = w!("KasumiMenu");

/// Around the rows; the menu's corners; a row's highlight corners.
const PAD: f32 = 6.0;
const RADIUS: f32 = 12.0;
const ROW_R: f32 = 8.0;
/// A row's text (or icon) from its left; the icon and the gap after it.
const LEAD: f32 = 10.0;
const ICON: f32 = 16.0;
const ICON_GAP: f32 = 10.0;
const SEP_H: f32 = 9.0;
/// Room on the right: for a submenu's chevron, or just padding.
const CHEVRON_W: f32 = 26.0;
const TRAIL: f32 = 14.0;
const MIN_W: f32 = 120.0;
const MAX_W: f32 = 480.0;
const FADE_MS: f32 = 110.0;

/// How every level looks ([theme], [context_menu] icons).
pub struct Style {
    pub look: Look,
    pub icons: bool,
    font: Font,
    keys_font: Font,
    row_h: f32,
}

impl Style {
    pub fn new(gfx: &Gfx, look: Look, icons: bool) -> Style {
        let mut font = Font::new(look.main.clone(), look.font_size.max(12.0));
        font.weight = 400;
        let mut keys_font = Font::new(look.secondary.clone(), (look.font_size - 1.0).max(11.0));
        keys_font.weight = 400;
        let th = gfx.text("Ag", &font).map_or(16.0, |t| t.height);
        Style { look, icons, font, keys_font, row_h: (th + 12.0).max(26.0).round() }
    }
}

/// Black or white, whichever reads on `c`.
fn on(c: Color) -> Color {
    let lum = 0.2126 * c.r + 0.7152 * c.g + 0.0722 * c.b;
    if lum > 0.6 {
        rgba(0.08, 0.08, 0.08, 1.0)
    } else {
        white(1.0)
    }
}

fn glyph(gfx: &Gfx, g: char, cx: f32, cy: f32, size: f32, c: Color) {
    let f = Font::icons(size);
    let s = g.to_string();
    let w = gfx.measure(&s, &f);
    gfx.text_at(&s, &f, cx - w / 2.0, cy, c);
}

pub struct Level {
    pub hwnd: HWND,
    dib: Option<Dib>,
    pub entries: Vec<Entry>,
    bitmaps: Vec<Option<AppIcon>>,
    /// Each entry's row: top and height (logical px).
    rows: Vec<(f32, f32)>,
    w: f32,
    h: f32,
    icon_col: f32,
    keys_col: f32,
    chevron: bool,
    pub pos: POINT,
    pub size: SIZE,
    pub scale: f32,
    pub hover: Option<usize>,
    /// The row of the level before that this one opened from.
    pub parent_row: Option<usize>,
    born: Instant,
    pub alpha: f32,
}

impl Drop for Level {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Level {
    pub fn new(gfx: &Gfx, style: &Style, entries: Vec<Entry>, parent_row: Option<usize>, scale: f32) -> Result<Level> {
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                CLASS,
                w!("Kasumi menu"),
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
        let bitmaps = entries
            .iter()
            .map(|e| match (&e.icon, style.icons) {
                (Some(Icon::Bitmap(px)), true) => AppIcon::new(gfx, px, IconStyle::Color),
                _ => None,
            })
            .collect();
        let mut l = Level {
            hwnd,
            dib: None,
            entries,
            bitmaps,
            rows: Vec::new(),
            w: 0.0,
            h: 0.0,
            icon_col: 0.0,
            keys_col: 0.0,
            chevron: false,
            pos: POINT::default(),
            size: SIZE::default(),
            scale,
            hover: None,
            parent_row,
            born: Instant::now(),
            alpha: 0.0,
        };
        l.layout(gfx, style);
        Ok(l)
    }

    /// Its size from its entries: the widest text, an icon column when any shows, the
    /// shortcuts' column, a chevron's room.
    fn layout(&mut self, gfx: &Gfx, style: &Style) {
        let e = &self.entries;
        let any_icon = style.icons && e.iter().any(|e| e.icon.is_some());
        let any_check = e.iter().any(|e| e.checked);
        self.icon_col = if any_icon || any_check { ICON + ICON_GAP } else { 0.0 };
        let text_w = e.iter().filter(|e| !e.separator).map(|e| gfx.measure(&e.text, &style.font)).fold(0.0, f32::max);
        let keys_w = e.iter().map(|e| if e.keys.is_empty() { 0.0 } else { gfx.measure(&e.keys, &style.keys_font) }).fold(0.0, f32::max);
        self.keys_col = if keys_w > 0.0 { keys_w + 24.0 } else { 0.0 };
        self.chevron = e.iter().any(|e| e.sub.is_some());
        let trail = if self.chevron { CHEVRON_W } else { TRAIL };
        self.w = (2.0 * PAD + LEAD + self.icon_col + text_w + self.keys_col + trail).clamp(MIN_W, MAX_W).ceil();
        let mut y = PAD;
        self.rows = e
            .iter()
            .map(|e| {
                let h = if e.separator { SEP_H } else { style.row_h };
                let r = (y, h);
                y += h;
                r
            })
            .collect();
        self.h = y + PAD;
        self.size = SIZE { cx: (self.w * self.scale).ceil() as i32, cy: (self.h * self.scale).ceil() as i32 };
    }

    /// The menu at the cursor: below and right of it, else above or left (as Windows'
    /// menus do), kept on the monitor.
    pub fn place_root(&mut self, at: POINT, mon: &Monitor) {
        let (w, h) = (self.size.cx, self.size.cy);
        let x = if at.x + w <= mon.right { at.x } else { at.x - w };
        let y = if at.y + h <= mon.bottom { at.y } else { at.y - h };
        self.pos = POINT { x: x.clamp(mon.left, (mon.right - w).max(mon.left)), y: y.clamp(mon.top, (mon.bottom - h).max(mon.top)) };
    }

    /// A submenu beside its row in `parent`: to the right, else the left.
    pub fn place_sub(&mut self, parent: &Level, row: usize, mon: &Monitor) {
        let (w, h) = (self.size.cx, self.size.cy);
        let overlap = (PAD * self.scale) as i32;
        let right = parent.pos.x + parent.size.cx - overlap;
        let x = if right + w <= mon.right { right } else { parent.pos.x - w + overlap };
        let y = parent.pos.y + ((parent.rows[row].0 - PAD) * parent.scale) as i32;
        self.pos = POINT { x: x.clamp(mon.left, (mon.right - w).max(mon.left)), y: y.clamp(mon.top, (mon.bottom - h).max(mon.top)) };
    }

    /// The row at a point in the window (device px), if it's one that can be pointed at.
    pub fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let (x, y) = (x as f32 / self.scale, y as f32 / self.scale);
        if x < PAD || x > self.w - PAD {
            return None;
        }
        self.rows.iter().position(|(top, h)| y >= *top && y < top + h).filter(|&i| !self.entries[i].separator)
    }

    pub fn contains(&self, pt: POINT) -> bool {
        pt.x >= self.pos.x && pt.x < self.pos.x + self.size.cx && pt.y >= self.pos.y && pt.y < self.pos.y + self.size.cy
    }

    pub fn draw(&mut self, gfx: &Gfx, style: &Style) -> Result<()> {
        let (w_px, h_px) = (self.size.cx, self.size.cy);
        if self.dib.as_ref().is_none_or(|d| d.w < w_px || d.h < h_px) {
            self.dib = Some(Dib::new(w_px.max(32), h_px.max(32))?);
        }
        let dc = self.dib.as_ref().unwrap().dc;
        let look = &style.look;
        let (fg, accent) = (look.fg, look.accent);
        let trail = if self.chevron { CHEVRON_W } else { TRAIL };
        gfx.begin(dc, w_px, h_px, self.scale)?;
        gfx.fill_round(rect(0.0, 0.0, self.w, self.h), RADIUS, with_alpha(look.bg, look.opacity.max(0.9)));
        for (i, (e, &(y, rh))) in self.entries.iter().zip(&self.rows).enumerate() {
            if e.separator {
                gfx.fill_rect(rect(PAD + LEAD, (y + rh / 2.0).floor(), self.w - 2.0 * (PAD + LEAD), 1.0 / self.scale), with_alpha(fg, 0.12));
                continue;
            }
            let sel = self.hover == Some(i);
            if sel {
                gfx.fill_round(rect(PAD, y, self.w - 2.0 * PAD, rh), ROW_R, if e.disabled { with_alpha(fg, 0.08) } else { accent });
            }
            let c = if e.disabled {
                with_alpha(fg, 0.38)
            } else if sel {
                on(accent)
            } else {
                fg
            };
            let cy = y + rh / 2.0;
            let mut x = PAD + LEAD;
            if self.icon_col > 0.0 {
                if e.checked {
                    let (g, size) = if e.radio { ('\u{E915}', 12.0) } else { ('\u{E73E}', 14.0) };
                    glyph(gfx, g, x + ICON / 2.0, cy, size, c);
                } else if style.icons {
                    match (&e.icon, &self.bitmaps[i]) {
                        (_, Some(b)) => b.draw(gfx, x, cy - ICON / 2.0, ICON, if e.disabled { 0.4 } else { 1.0 }),
                        (Some(Icon::Glyph(g)), _) => glyph(gfx, *g, x + ICON / 2.0, cy, ICON, c),
                        _ => {}
                    }
                }
                x += self.icon_col;
            }
            let room = self.w - PAD - trail - self.keys_col - x;
            gfx.text_at(&fit_text(gfx, &e.text, &style.font, room), &style.font, x, cy, c);
            if !e.keys.is_empty() {
                let kw = gfx.measure(&e.keys, &style.keys_font);
                gfx.text_at(&e.keys, &style.keys_font, self.w - PAD - trail - kw, cy, with_alpha(c, 0.6));
            }
            if e.sub.is_some() {
                glyph(gfx, '\u{E76C}', self.w - PAD - CHEVRON_W / 2.0, cy, 10.0, with_alpha(c, if sel { 1.0 } else { 0.6 }));
            }
        }
        gfx.end()?;
        self.present(true)
    }

    /// On screen (with its pixels again, or just its fade).
    fn present(&self, pixels: bool) -> Result<()> {
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: (self.alpha.clamp(0.0, 1.0) * 255.0) as u8,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        unsafe {
            if pixels {
                let dc = self.dib.as_ref().map(|d| d.dc);
                UpdateLayeredWindow(self.hwnd, None, Some(&self.pos), Some(&self.size), dc, Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            } else {
                UpdateLayeredWindow(self.hwnd, None, None, None, None, None, COLORREF(0), Some(&blend), ULW_ALPHA)?;
            }
        }
        Ok(())
    }

    /// Fading in: true while it still is.
    pub fn fade(&mut self) -> bool {
        if self.alpha >= 1.0 {
            return false;
        }
        self.alpha = (self.born.elapsed().as_secs_f32() * 1000.0 / FADE_MS).min(1.0);
        let _ = self.present(false);
        self.alpha < 1.0
    }

    /// The next row that can be chosen from `from` going `step` (wrapping).
    pub fn next_usable(&self, from: Option<usize>, step: isize) -> Option<usize> {
        let n = self.entries.len() as isize;
        if n == 0 {
            return None;
        }
        let mut i = from.map_or(if step > 0 { -1 } else { n }, |i| i as isize);
        for _ in 0..n {
            i = (i + step).rem_euclid(n);
            if self.entries[i as usize].usable() {
                return Some(i as usize);
            }
        }
        None
    }
}
