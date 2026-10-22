//! The menu's look: a window per level (the menu, then each submenu open), drawn
//! like Kasumi's panels and sized to its entries. The accent marks the row under the
//! cursor (or chosen with the keys), with text that reads on it.
//!
//! Opening ([animation] menu_open): a level grows out of where it opens (down from
//! the cursor, or sideways out of its row for a submenu) on a spring, its rows coming
//! one after another (menu_cascade); or it fades in. A submenu joins its menu: square
//! where they meet, with concave corners where the edges turn (like a notch's), each
//! window drawing its own part in a margin around its body.

use std::rc::Rc;
use std::time::Instant;

use windows::core::{w, Result, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::ID2D1Brush;
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, GetWindowLongW, SetWindowLongW, SetWindowPos, ShowWindow, UpdateLayeredWindow, GWL_EXSTYLE, HWND_TOPMOST,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};

use super::shell::{Entry, Icon};
use crate::native::geometry::{Cmd, Pt};
use crate::native::gfx::{rect, rgba, white, with_alpha, Color, Font, Gfx};
use crate::native::surface::Dib;
use crate::native::widgets::icon::{AppIcon, Style as IconStyle};
use crate::native::widgets::{fit_text, Look};
use crate::native::Spring;
use crate::win::Monitor;

pub const CLASS: PCWSTR = w!("KasumiMenu");

/// Around the rows; the menu's corners; a row's highlight corners.
const PAD: f32 = 6.0;
const RADIUS: f32 = 12.0;
const ROW_R: f32 = 8.0;
/// The concave corners where a submenu meets its menu; the margin they're drawn in.
const FILLET: f32 = 10.0;
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
/// Cubic Bézier circle-arc constant.
const K: f64 = 0.5523;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Grows out of where it opens.
    Expand,
    Fade,
    None,
}

/// Which way a level grows as it opens (and the side a submenu joins its menu on).
#[derive(Clone, Copy, PartialEq)]
pub enum Grow {
    Down,
    Up,
    Right,
    Left,
}

/// How every level looks and moves ([theme], [context_menu] icons, [animation] menu_*).
pub struct Style {
    pub look: Look,
    pub icons: bool,
    font: Font,
    keys_font: Font,
    row_h: f32,
    mode: Mode,
    cascade: bool,
    anim: crate::config::Animation,
}

impl Style {
    pub fn new(gfx: &Gfx, look: Look, icons: bool, anim: crate::config::Animation) -> Rc<Style> {
        let mut font = Font::new(look.main.clone(), look.font_size.max(12.0));
        font.weight = 400;
        let mut keys_font = Font::new(look.secondary.clone(), (look.font_size - 1.0).max(11.0));
        keys_font.weight = 400;
        let th = gfx.text("Ag", &font).map_or(16.0, |t| t.height);
        let mode = match anim.menu_open.as_str() {
            "fade" => Mode::Fade,
            "none" => Mode::None,
            _ => Mode::Expand,
        };
        Rc::new(Style { look, icons, font, keys_font, row_h: (th + 12.0).max(26.0).round(), mode, cascade: anim.menu_cascade && mode != Mode::None, anim })
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

/// A rectangle with its own radius at each corner (top left, top right, bottom
/// right, bottom left).
fn body_path(x: f32, y: f32, w: f32, h: f32, r: [f32; 4]) -> Vec<Cmd> {
    let (x, y, w, h) = (x as f64, y as f64, w as f64, h as f64);
    let lim = (w / 2.0).min(h / 2.0).max(0.0);
    let [tl, tr, br, bl] = r.map(|v| (v as f64).min(lim));
    vec![
        Cmd::M(Pt(x + tl, y)),
        Cmd::L(Pt(x + w - tr, y)),
        Cmd::C(Pt(x + w - tr + K * tr, y), Pt(x + w, y + tr - K * tr), Pt(x + w, y + tr)),
        Cmd::L(Pt(x + w, y + h - br)),
        Cmd::C(Pt(x + w, y + h - br + K * br), Pt(x + w - br + K * br, y + h), Pt(x + w - br, y + h)),
        Cmd::L(Pt(x + bl, y + h)),
        Cmd::C(Pt(x + bl - K * bl, y + h), Pt(x, y + h - bl + K * bl), Pt(x, y + h - bl)),
        Cmd::L(Pt(x, y + tl)),
        Cmd::C(Pt(x, y + tl - K * tl), Pt(x + tl - K * tl, y), Pt(x + tl, y)),
        Cmd::Z,
    ]
}

/// A concave corner: at (x, y) where two edges meet, reaching `sx` along one and
/// `sy` along the other (each ±1), `r` across.
fn fillet_path(x: f32, y: f32, sx: f32, sy: f32, r: f32) -> Vec<Cmd> {
    let (x, y, sx, sy, r) = (x as f64, y as f64, sx as f64, sy as f64, r as f64);
    vec![
        Cmd::M(Pt(x, y + sy * r)),
        Cmd::L(Pt(x, y)),
        Cmd::L(Pt(x + sx * r, y)),
        Cmd::C(Pt(x + sx * (r - K * r), y), Pt(x, y + sy * (r - K * r)), Pt(x, y + sy * r)),
        Cmd::Z,
    ]
}

/// How long `n` rows take to come in, one after another (as a panel's cascade).
fn cascade_ms(n: usize) -> f32 {
    let n = n.max(1) as f32;
    (n - 1.0) * (240.0 / n).min(25.0) + 160.0
}

pub struct Level {
    pub hwnd: HWND,
    dib: Option<Dib>,
    style: Rc<Style>,
    pub entries: Vec<Entry>,
    bitmaps: Vec<Option<AppIcon>>,
    /// Each entry's row in the body: top and height (logical px).
    rows: Vec<(f32, f32)>,
    /// The body (logical, whole device px) and its place on screen (device px).
    bw: f32,
    bh: f32,
    pub body: POINT,
    body_px: SIZE,
    icon_col: f32,
    keys_col: f32,
    chevron: bool,
    pub scale: f32,
    pub hover: Option<usize>,
    /// The row of the level before that this one opened from.
    pub parent_row: Option<usize>,
    /// Corner radii (top left, top right, bottom right, bottom left) and concave
    /// corners (x, y in the body; directions; radius): where a submenu joins.
    corners: [f32; 4],
    fillets: Vec<(f32, f32, f32, f32, f32)>,
    pub grow: Grow,
    open: Spring,
    born: Instant,
    /// Moving (drawing frames); on its way out.
    active: bool,
    closing: bool,
}

impl Drop for Level {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.hwnd);
        }
    }
}

impl Level {
    pub fn new(gfx: &Gfx, style: Rc<Style>, entries: Vec<Entry>, parent_row: Option<usize>, scale: f32) -> Result<Level> {
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
        let a = &style.anim;
        let mut open = Spring::new(0.0, a.menu_stiffness, a.menu_damping);
        open.configure(a.menu_stiffness, a.menu_damping, a);
        open.target = 1.0;
        if style.mode == Mode::None {
            open.snap();
        }
        let mut l = Level {
            hwnd,
            dib: None,
            style,
            entries,
            bitmaps,
            rows: Vec::new(),
            bw: 0.0,
            bh: 0.0,
            body: POINT::default(),
            body_px: SIZE::default(),
            icon_col: 0.0,
            keys_col: 0.0,
            chevron: false,
            scale,
            hover: None,
            parent_row,
            corners: [RADIUS; 4],
            fillets: Vec::new(),
            grow: Grow::Down,
            open,
            born: Instant::now(),
            active: true,
            closing: false,
        };
        l.layout(gfx);
        Ok(l)
    }

    /// Its size from its entries: the widest text, an icon column when any shows, the
    /// shortcuts' column, a chevron's room.
    fn layout(&mut self, gfx: &Gfx) {
        let style = self.style.clone();
        let e = &self.entries;
        let any_icon = style.icons && e.iter().any(|e| e.icon.is_some());
        let any_check = e.iter().any(|e| e.checked);
        self.icon_col = if any_icon || any_check { ICON + ICON_GAP } else { 0.0 };
        let text_w = e.iter().filter(|e| !e.separator).map(|e| gfx.measure(&e.text, &style.font)).fold(0.0, f32::max);
        let keys_w = e.iter().map(|e| if e.keys.is_empty() { 0.0 } else { gfx.measure(&e.keys, &style.keys_font) }).fold(0.0, f32::max);
        self.keys_col = if keys_w > 0.0 { keys_w + 24.0 } else { 0.0 };
        self.chevron = e.iter().any(|e| e.sub.is_some());
        let trail = if self.chevron { CHEVRON_W } else { TRAIL };
        let w = (2.0 * PAD + LEAD + self.icon_col + text_w + self.keys_col + trail).clamp(MIN_W, MAX_W);
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
        // Whole device pixels, so a submenu meets its menu without a seam.
        self.body_px = SIZE { cx: (w * self.scale).ceil() as i32, cy: ((y + PAD) * self.scale).ceil() as i32 };
        self.bw = self.body_px.cx as f32 / self.scale;
        self.bh = self.body_px.cy as f32 / self.scale;
    }

    /// The margin around the body (device px) the concave corners are drawn in.
    fn margin(&self) -> i32 {
        (FILLET * self.scale).ceil() as i32
    }

    /// The menu at the cursor: below and right of it, else above or left (as Windows'
    /// menus do), kept on the monitor. It grows down (or up, when above).
    pub fn place_root(&mut self, at: POINT, mon: &Monitor) {
        let (w, h) = (self.body_px.cx, self.body_px.cy);
        let x = if at.x + w <= mon.right { at.x } else { at.x - w };
        let up = at.y + h > mon.bottom;
        let y = if up { at.y - h } else { at.y };
        self.grow = if up { Grow::Up } else { Grow::Down };
        self.body = POINT { x: x.clamp(mon.left, (mon.right - w).max(mon.left)), y: y.clamp(mon.top, (mon.bottom - h).max(mon.top)) };
    }

    /// A submenu beside its row in `parent`, touching it: to the right, else the left.
    pub fn place_sub(&mut self, parent: &Level, row: usize, mon: &Monitor) {
        let (w, h) = (self.body_px.cx, self.body_px.cy);
        let right = parent.body.x + parent.body_px.cx;
        let (x, grow) = if right + w <= mon.right { (right, Grow::Right) } else { (parent.body.x - w, Grow::Left) };
        self.grow = grow;
        let y = parent.body.y + ((parent.rows[row].0 - PAD) * parent.scale).round() as i32;
        self.body = POINT { x, y: y.clamp(mon.top, (mon.bottom - h).max(mon.top)) };
    }

    /// Back to its own corners (its submenu closed).
    pub fn reset_shape(&mut self) {
        self.corners = [RADIUS; 4];
        self.fillets.clear();
    }

    fn add_fillet(&mut self, x: i32, y: i32, sx: f32, sy: f32, r_px: f32) {
        let s = self.scale;
        self.fillets.push(((x - self.body.x) as f32 / s, (y - self.body.y) as f32 / s, sx, sy, r_px / s));
    }

    /// The row at a point in the window (device px), if it's one that can be pointed at.
    pub fn row_at(&self, x: i32, y: i32) -> Option<usize> {
        let m = self.margin() as f32 / self.scale;
        let (x, y) = (x as f32 / self.scale - m, y as f32 / self.scale - m);
        if x < PAD || x > self.bw - PAD {
            return None;
        }
        self.rows.iter().position(|(top, h)| y >= *top && y < top + h).filter(|&i| !self.entries[i].separator)
    }

    pub fn contains(&self, pt: POINT) -> bool {
        pt.x >= self.body.x && pt.x < self.body.x + self.body_px.cx && pt.y >= self.body.y && pt.y < self.body.y + self.body_px.cy
    }

    /// Opening or closing: the spring, and the rows coming in. True: it moved (draw it).
    pub fn tick(&mut self, dt: f64) -> bool {
        if !self.active {
            return false;
        }
        self.open.step(dt);
        let cascading = !self.closing && self.style.cascade && self.born.elapsed().as_secs_f32() * 1000.0 < cascade_ms(self.rows.len());
        if self.open.settled() && !cascading {
            self.open.snap();
            self.active = false;
        }
        true
    }

    /// On its way out: closing as it opened, letting clicks through.
    pub fn retire(&mut self) {
        self.closing = true;
        self.open.target = 0.0;
        self.active = self.style.mode != Mode::None;
        unsafe {
            let ex = GetWindowLongW(self.hwnd, GWL_EXSTYLE);
            SetWindowLongW(self.hwnd, GWL_EXSTYLE, ex | WS_EX_TRANSPARENT.0 as i32);
        }
    }

    /// Closed (it can go).
    pub fn gone(&self) -> bool {
        self.closing && (!self.active || self.open.value <= 0.02)
    }

    pub fn draw(&mut self, gfx: &Gfx) -> Result<()> {
        let style = self.style.clone();
        let mpx = self.margin();
        let (w_px, h_px) = (self.body_px.cx + 2 * mpx, self.body_px.cy + 2 * mpx);
        if self.dib.as_ref().is_none_or(|d| d.w < w_px || d.h < h_px) {
            self.dib = Some(Dib::new(w_px.max(32), h_px.max(32))?);
        }
        let dc = self.dib.as_ref().unwrap().dc;
        let look = &style.look;
        let (fg, accent) = (look.fg, look.accent);
        let trail = if self.chevron { CHEVRON_W } else { TRAIL };
        let m = mpx as f32 / self.scale;
        let e = self.open.value.clamp(0.0, 1.0) as f32;
        // The body as far as it's grown (anchored where it opens from).
        let (vx, vy, vw, vh) = match (style.mode, self.grow) {
            (Mode::Expand, Grow::Down) => (0.0, 0.0, self.bw, self.bh * e),
            (Mode::Expand, Grow::Up) => (0.0, self.bh * (1.0 - e), self.bw, self.bh * e),
            (Mode::Expand, Grow::Right) => (0.0, 0.0, self.bw * e, self.bh),
            (Mode::Expand, Grow::Left) => (self.bw * (1.0 - e), 0.0, self.bw * e, self.bh),
            _ => (0.0, 0.0, self.bw, self.bh),
        };
        gfx.begin(dc, w_px, h_px, self.scale)?;
        if vw > 0.5 && vh > 0.5 {
            let mut shape = body_path(m + vx, m + vy, vw, vh, self.corners);
            for &(fx, fy, sx, sy, r) in &self.fillets {
                let r = r.min(vw).min(vh);
                if r > 0.5 {
                    shape.extend(fillet_path(m + fx, m + fy, sx, sy, r));
                }
            }
            if let Ok(g) = gfx.path(&shape, None) {
                unsafe {
                    gfx.brush.SetColor(&with_alpha(look.bg, look.opacity.max(0.9)));
                    gfx.rt.FillGeometry(&g, &gfx.brush, None::<&ID2D1Brush>);
                }
            }
            gfx.clip(rect(m + vx, m + vy, vw, vh));
            let t = self.born.elapsed().as_secs_f32() * 1000.0;
            let n = self.rows.len().max(1) as f32;
            let step = (240.0 / n).min(25.0);
            for (i, (en, &(y, rh))) in self.entries.iter().zip(&self.rows).enumerate() {
                // One after another: each fades in and slides a little into place.
                let (ra, dx, dy) = if style.cascade && !self.closing {
                    let k = ((t - i as f32 * step) / 160.0).clamp(0.0, 1.0);
                    let k = 1.0 - (1.0 - k).powi(3);
                    let off = (1.0 - k) * 6.0;
                    match self.grow {
                        Grow::Down => (k, 0.0, -off),
                        Grow::Up => (k, 0.0, off),
                        Grow::Right => (k, -off, 0.0),
                        Grow::Left => (k, off, 0.0),
                    }
                } else {
                    (1.0, 0.0, 0.0)
                };
                if ra <= 0.0 {
                    continue;
                }
                let (ox, oy) = (m + dx, m + dy);
                if en.separator {
                    gfx.fill_rect(rect(ox + PAD + LEAD, oy + (y + rh / 2.0).floor(), self.bw - 2.0 * (PAD + LEAD), 1.0 / self.scale), with_alpha(fg, 0.12 * ra));
                    continue;
                }
                let sel = self.hover == Some(i);
                if sel {
                    let hl = if en.disabled { with_alpha(fg, 0.08) } else { accent };
                    gfx.fill_round(rect(ox + PAD, oy + y, self.bw - 2.0 * PAD, rh), ROW_R, with_alpha(hl, ra));
                }
                let c = if en.disabled {
                    with_alpha(fg, 0.38 * ra)
                } else if sel {
                    with_alpha(on(accent), ra)
                } else {
                    with_alpha(fg, ra)
                };
                let cy = oy + y + rh / 2.0;
                let mut x = ox + PAD + LEAD;
                if self.icon_col > 0.0 {
                    if en.checked {
                        let (g, size) = if en.radio { ('\u{E915}', 12.0) } else { ('\u{E73E}', 14.0) };
                        glyph(gfx, g, x + ICON / 2.0, cy, size, c);
                    } else if style.icons {
                        match (&en.icon, &self.bitmaps[i]) {
                            (_, Some(b)) => b.draw(gfx, x, cy - ICON / 2.0, ICON, ra * if en.disabled { 0.4 } else { 1.0 }),
                            (Some(Icon::Glyph(g)), _) => glyph(gfx, *g, x + ICON / 2.0, cy, ICON, c),
                            _ => {}
                        }
                    }
                    x += self.icon_col;
                }
                let room = ox + self.bw - PAD - trail - self.keys_col - x;
                gfx.text_at(&fit_text(gfx, &en.text, &style.font, room), &style.font, x, cy, c);
                if !en.keys.is_empty() {
                    let kw = gfx.measure(&en.keys, &style.keys_font);
                    gfx.text_at(&en.keys, &style.keys_font, ox + self.bw - PAD - trail - kw, cy, with_alpha(c, 0.6));
                }
                if en.sub.is_some() {
                    glyph(gfx, '\u{E76C}', ox + self.bw - PAD - CHEVRON_W / 2.0, cy, 10.0, with_alpha(c, if sel { 1.0 } else { 0.6 }));
                }
            }
            gfx.unclip();
        }
        gfx.end()?;
        let alpha = match style.mode {
            Mode::Fade => e,
            Mode::Expand => (e * 4.0).min(1.0),
            Mode::None => 1.0,
        };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: (alpha.clamp(0.0, 1.0) * 255.0) as u8, AlphaFormat: AC_SRC_ALPHA as u8 };
        let pos = POINT { x: self.body.x - mpx, y: self.body.y - mpx };
        unsafe {
            UpdateLayeredWindow(self.hwnd, None, Some(&pos), Some(&SIZE { cx: w_px, cy: h_px }), Some(dc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
            if !self.closing {
                let _ = SetWindowPos(self.hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                let _ = ShowWindow(self.hwnd, SW_SHOWNOACTIVATE);
            }
        }
        Ok(())
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

/// A submenu `c` joined to its menu `p`: square where they meet; where one's edge
/// runs past the other's, a concave corner turns into it (drawn by the one whose side
/// it's on), and the corner beside it rounds only as far as there's room.
pub fn join(p: &mut Level, c: &mut Level) {
    p.reset_shape();
    c.reset_shape();
    let s = c.scale;
    let right = c.grow == Grow::Right;
    let x = if right { p.body.x + p.body_px.cx } else { p.body.x };
    let (pt, pb) = (p.body.y, p.body.y + p.body_px.cy);
    let (ct, cb) = (c.body.y, c.body.y + c.body_px.cy);
    let (rr, rf) = (RADIUS * s, FILLET * s);
    // Corners on the side they meet: the menu's, the submenu's (top, bottom).
    let (p_top, p_bot, c_top, c_bot) = if right { (1, 2, 0, 3) } else { (0, 3, 1, 2) };
    // From the line they meet on: towards the submenu (+1 right).
    let side = if right { 1.0 } else { -1.0 };
    let room = |d: f32, f: f32| ((d - f) / s).clamp(0.0, RADIUS);

    let d = (ct - pt) as f32;
    if d.abs() < 1.0 {
        p.corners[p_top] = 0.0;
        c.corners[c_top] = 0.0;
    } else if d > 0.0 {
        // The submenu starts lower: a corner turns from the menu's side into its top.
        c.corners[c_top] = 0.0;
        let f = rf.min(d);
        c.add_fillet(x, ct, side, -1.0, f);
        if d < rr + f {
            p.corners[p_top] = room(d, f);
        }
    } else {
        p.corners[p_top] = 0.0;
        let f = rf.min(-d);
        p.add_fillet(x, pt, -side, -1.0, f);
        if -d < rr + f {
            c.corners[c_top] = room(-d, f);
        }
    }

    let d = (pb - cb) as f32;
    if d.abs() < 1.0 {
        p.corners[p_bot] = 0.0;
        c.corners[c_bot] = 0.0;
    } else if d > 0.0 {
        // The submenu ends higher: a corner turns from the menu's side into its bottom.
        c.corners[c_bot] = 0.0;
        let f = rf.min(d);
        c.add_fillet(x, cb, side, 1.0, f);
        if d < rr + f {
            p.corners[p_bot] = room(d, f);
        }
    } else {
        p.corners[p_bot] = 0.0;
        let f = rf.min(-d);
        p.add_fillet(x, pb, -side, 1.0, f);
        if -d < rr + f {
            c.corners[c_bot] = room(-d, f);
        }
    }
}
