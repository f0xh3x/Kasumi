//! The scroll position pop-up (osd/scroll.rs reads the position): a notch growing
//! out of the right edge of the screen (the bars' shape, turned on its side) with a
//! thin grey track and a white thumb, in a click-through window of its own. The
//! thumb glides to each new position; it goes back into the edge [osd] duration_ms
//! after the last scroll.

use std::time::{Duration, Instant};

use windows::core::Result;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Direct2D::ID2D1Brush;
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::geometry::{self, Bump, Cmd, Pt};
use super::gfx::{rect, white, with_alpha, Gfx};
use super::surface::Dib;
use super::widgets::Look;
use super::{ScrollShow, Spring};
use crate::win::Monitor;

/// The gap between the track's ends and the notch's (flat) ends; the thumb's
/// shortest length.
const END: f32 = 10.0;
const MIN_THUMB: f32 = 24.0;
/// Room for a spring's overshoot.
const ROOM: f32 = 1.5;

pub struct ScrollBar {
    hwnd: Option<HWND>,
    dib: Option<Dib>,
    monitor: Option<Monitor>,
    /// The track's length and the notch's depth from the edge ([osd] scroll_width;
    /// px, logical).
    length: f32,
    depth: f32,
    look: Option<Look>,
    geometry: crate::config::Geometry,
    /// Out (0..1: the notch grows from the edge), the thumb's start and length
    /// (shares of the track).
    shown: Spring,
    top: Spring,
    size: Spring,
    pub until: Option<Instant>,
}

impl ScrollBar {
    pub fn new() -> ScrollBar {
        let s = Spring::new(0.0, 300.0, 30.0);
        ScrollBar { hwnd: None, dib: None, monitor: None, length: 800.0, depth: 10.0, look: None, geometry: Default::default(), shown: s.clone(), top: s.clone(), size: s, until: None }
    }

    /// A new position: out (again) on that monitor, the thumb heading there.
    pub fn show(&mut self, s: &ScrollShow) {
        let a = &s.anim;
        self.shown.configure(a.expand_stiffness, a.expand_damping, a);
        for sp in [&mut self.top, &mut self.size] {
            sp.configure(a.panel_stiffness, a.panel_damping, a);
        }
        let moved = self.monitor.as_ref().is_none_or(|m| m.device != s.monitor.device);
        let hidden = self.shown.value < 0.02;
        self.top.target = s.top.clamp(0.0, 1.0) as f64;
        self.size.target = s.size.clamp(0.0, 1.0) as f64;
        // Coming out: the thumb is already where it should be (it only glides while out).
        if hidden || moved {
            self.top.snap();
            self.size.snap();
        }
        if moved && !hidden {
            self.shown.snap();
        }
        let tall = (s.monitor.bottom - s.monitor.top) as f64 / s.monitor.scale.max(0.5);
        self.length = s.length.clamp(100.0, tall * 0.8) as f32;
        self.depth = s.width.clamp(4.0, 40.0) as f32;
        self.monitor = Some(s.monitor.clone());
        self.look = Some(Look::from_theme(&s.theme, false));
        self.geometry = s.geometry.clone();
        self.shown.target = 1.0;
        self.until = Some(Instant::now() + Duration::from_millis(s.ms.max(300)));
    }

    /// Out long enough (the last show's timer): back into the edge.
    pub fn hide(&mut self) {
        self.until = None;
        self.shown.target = 0.0;
    }

    pub fn animating(&self) -> bool {
        !(self.shown.settled() && self.top.settled() && self.size.settled()) || (self.shown.target > 0.5 && self.hwnd.is_none())
    }

    /// One frame: the springs on, drawn, or hidden once it's back in the edge.
    pub fn frame(&mut self, gfx: &Gfx, dt: f64) {
        for sp in [&mut self.shown, &mut self.top, &mut self.size] {
            sp.step(dt);
            if sp.settled() {
                sp.snap();
            }
        }
        if self.shown.target < 0.5 && self.shown.value <= 0.001 {
            self.shown.snap();
            if let Some(h) = self.hwnd {
                unsafe {
                    let _ = ShowWindow(h, SW_HIDE);
                }
            }
            return;
        }
        if let Err(e) = self.present(gfx) {
            eprintln!("[kasumi] scroll bar: {e}");
        }
    }

    /// The notch's length along the edge (between its curves' middles) and the window's
    /// size (logical px).
    fn sizes(&self) -> (f32, f32, f32) {
        let g = &self.geometry;
        let k = self.depth as f64 / g.notch_height.max(1.0);
        let width = self.length + 2.0 * (END + (g.notch_corner_radius * k) as f32);
        let w = (self.depth * ROOM).ceil();
        let h = width + 2.0 * (g.concave_radius * k * ROOM as f64) as f32 + 2.0;
        (width, w, h)
    }

    /// Draws it into its bitmap: the window's size (device px).
    fn render(&mut self, gfx: &Gfx) -> Result<(i32, i32)> {
        let Some(mon) = self.monitor.as_ref() else { return Ok((0, 0)) };
        let scale = mon.scale as f32;
        let (width, w, h) = self.sizes();
        let (w_px, h_px) = ((w * scale).ceil() as i32, (h * scale).ceil() as i32);
        if self.dib.as_ref().is_none_or(|d| d.w < w_px || d.h < h_px) {
            self.dib = Some(Dib::new(w_px.max(16), h_px.max(64))?);
        }
        let dib = self.dib.as_ref().unwrap();
        let look = self.look.clone().unwrap_or_else(|| Look::from_theme(&Default::default(), false));
        let out = self.shown.value.max(0.0);
        // The notch, as the bars draw theirs, standing on the right edge: its depth (and
        // the size of its curves) grows from nothing.
        let depth = self.depth as f64 * out;
        let k = depth / self.geometry.notch_height.max(1.0);
        let left = (h - width) as f64 / 2.0;
        let bump = Bump { x: left, width: width as f64, height: depth, in_hull: true, scale: k, ..Default::default() };
        let frame = geometry::compute(&self.geometry, h as f64, 0.0, false, 0.0, 0.0, &[bump]);
        let turn = |p: Pt| Pt(w as f64 - p.1, p.0);
        let cmds: Vec<Cmd> = frame
            .path
            .iter()
            .map(|c| match *c {
                Cmd::M(p) => Cmd::M(turn(p)),
                Cmd::L(p) => Cmd::L(turn(p)),
                Cmd::C(a, b, p) => Cmd::C(turn(a), turn(b), turn(p)),
                Cmd::Z => Cmd::Z,
            })
            .collect();
        gfx.begin(dib.dc, w_px, h_px, scale)?;
        let shape = gfx.path(&cmds, None)?;
        unsafe {
            gfx.brush.SetColor(&with_alpha(look.bg, look.opacity));
            gfx.rt.FillGeometry(&shape, &gfx.brush, None::<&ID2D1Brush>);
        }
        // The track in the middle of the notch's depth: the level pop-ups' empty track
        // (white at 20%) with a white thumb, fading in as the notch comes out.
        let a = ((out as f32 - 0.35) / 0.65).clamp(0.0, 1.0);
        if a > 0.0 {
            // The track: under a third of the notch's depth (4 px in a 14 px notch), on
            // whole device pixels (a track between pixels has soft edges).
            let track =((self.depth / 3.5).clamp(2.0, 10.0) * scale).round() / scale;
            let x = gfx.snap(w - depth as f32 / 2.0 - track / 2.0);
            let y0 = (h - self.length) / 2.0;
            gfx.fill_round(rect(x, y0, track, self.length), track / 2.0, white(0.2 * a));
            let size = self.size.value as f32;
            let len = (size * self.length).clamp(MIN_THUMB.min(self.length), self.length);
            let at = (self.top.value as f32 / (1.0 - size).max(1e-3)).clamp(0.0, 1.0) * (self.length - len);
            gfx.fill_round(rect(x, y0 + at, track, len), track / 2.0, white(a));
        }
        gfx.end()?;
        Ok((w_px, h_px))
    }

    /// Drawn and put on screen, against the monitor's right edge, centred.
    fn present(&mut self, gfx: &Gfx) -> Result<()> {
        let hwnd = match self.hwnd {
            Some(h) => h,
            None => {
                let h = unsafe {
                    CreateWindowExW(
                        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
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
        let (w_px, h_px) = self.render(gfx)?;
        let (Some(mon), Some(dib)) = (self.monitor.as_ref(), self.dib.as_ref()) else { return Ok(()) };
        let pos = POINT { x: mon.right - w_px, y: mon.top + (mon.bottom - mon.top - h_px) / 2 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&SIZE { cx: w_px, cy: h_px }), Some(dib.dc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(())
    }

    /// Off screen (snapshot.rs): drawn `out` of the way out; its pixels (premultiplied
    /// BGRA, `stride` px a row), width and height.
    pub fn snapshot(gfx: &Gfx, s: &ScrollShow, out: f64) -> Option<(Vec<u8>, i32, i32, i32)> {
        let mut b = ScrollBar::new();
        b.show(s);
        b.shown.value = out;
        b.shown.target = out;
        let (w, h) = b.render(gfx).ok()?;
        let dib = b.dib.as_ref()?;
        Some((dib.pixels().to_vec(), w, h, dib.w))
    }
}

impl Drop for ScrollBar {
    fn drop(&mut self) {
        if let Some(h) = self.hwnd.take() {
            unsafe {
                let _ = DestroyWindow(h);
            }
        }
    }
}
