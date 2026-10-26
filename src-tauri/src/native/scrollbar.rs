//! The scroll position pop-up (osd/scroll.rs reads the position): a dark grey bar
//! at the right edge of the screen with a white thumb, in a click-through window of
//! its own. It fades and slides in, the thumb glides to each new position, and it
//! goes again [osd] duration_ms after the last scroll.

use std::time::{Duration, Instant};

use windows::core::Result;
use windows::Win32::Foundation::{COLORREF, HWND, POINT, SIZE};
use windows::Win32::Graphics::Gdi::{AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, SetWindowPos, ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SW_HIDE, SW_SHOWNOACTIVATE, ULW_ALPHA, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use super::gfx::{rect, rgba, white, Gfx};
use super::surface::Dib;
use super::{ScrollShow, Spring};
use crate::win::Monitor;

/// The bar's width, its gap to the screen's edge, how far it slides in, the thumb's
/// inset and shortest length.
const WIDTH: f32 = 10.0;
const MARGIN: f32 = 12.0;
const SLIDE: f32 = 8.0;
const INSET: f32 = 2.0;
const MIN_THUMB: f32 = 24.0;

pub struct ScrollBar {
    hwnd: Option<HWND>,
    dib: Option<Dib>,
    monitor: Option<Monitor>,
    /// The bar's length (px, logical).
    length: f32,
    /// Shown (0..1), the thumb's start and length (shares of the track).
    shown: Spring,
    top: Spring,
    size: Spring,
    pub until: Option<Instant>,
}

impl ScrollBar {
    pub fn new() -> ScrollBar {
        let s = Spring::new(0.0, 300.0, 30.0);
        ScrollBar { hwnd: None, dib: None, monitor: None, length: 800.0, shown: s.clone(), top: s.clone(), size: s, until: None }
    }

    /// A new position: out (again) on that monitor, the thumb heading there.
    pub fn show(&mut self, s: &ScrollShow) {
        let a = &s.anim;
        for sp in [&mut self.shown, &mut self.top, &mut self.size] {
            sp.configure(a.expand_stiffness, a.expand_damping, a);
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
        self.monitor = Some(s.monitor.clone());
        self.shown.target = 1.0;
        self.until = Some(Instant::now() + Duration::from_millis(s.ms.max(300)));
    }

    /// Time's up: it goes.
    pub fn tick(&mut self, now: Instant) {
        if self.until.is_some_and(|t| now >= t) {
            self.until = None;
            self.shown.target = 0.0;
        }
    }

    pub fn animating(&self) -> bool {
        !(self.shown.settled() && self.top.settled() && self.size.settled()) || (self.shown.target > 0.5 && self.hwnd.is_none())
    }

    /// One frame: the springs on, drawn, or hidden once it has faded out.
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
        if let Err(e) = self.draw(gfx) {
            eprintln!("[kasumi] scroll bar: {e}");
        }
    }

    fn draw(&mut self, gfx: &Gfx) -> Result<()> {
        let Some(mon) = self.monitor.clone() else { return Ok(()) };
        let scale = mon.scale as f32;
        let (w, h) = (WIDTH + MARGIN, self.length);
        let (w_px, h_px) = ((w * scale).ceil() as i32, (h * scale).ceil() as i32);
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
        if self.dib.as_ref().is_none_or(|d| d.w < w_px || d.h < h_px) {
            self.dib = Some(Dib::new(w_px.max(16), h_px.max(64))?);
        }
        let dib = self.dib.as_ref().unwrap();
        let a = (self.shown.value as f32).clamp(0.0, 1.0);
        let x = (1.0 - a) * SLIDE;
        gfx.begin(dib.dc, w_px, h_px, scale)?;
        // The track: the grey of the level pop-ups' empty track (white at 20% over black).
        gfx.fill_round(rect(x, 0.0, WIDTH, h), WIDTH / 2.0, rgba(0.2, 0.2, 0.2, a));
        let track = h - 2.0 * INSET;
        let len = (self.size.value as f32 * track).clamp(MIN_THUMB.min(track), track);
        let top = INSET + (self.top.value as f32 / (1.0 - self.size.value as f32).max(1e-3)).clamp(0.0, 1.0) * (track - len);
        let tw = WIDTH - 2.0 * INSET;
        gfx.fill_round(rect(x + INSET, top, tw, len), tw / 2.0, white(a));
        gfx.end()?;
        let pos = POINT { x: mon.right - w_px, y: mon.top + (mon.bottom - mon.top - h_px) / 2 };
        let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
        unsafe {
            UpdateLayeredWindow(hwnd, None, Some(&pos), Some(&SIZE { cx: w_px, cy: h_px }), Some(dib.dc), Some(&POINT { x: 0, y: 0 }), COLORREF(0), Some(&blend), ULW_ALPHA)?;
            let _ = SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        }
        Ok(())
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
