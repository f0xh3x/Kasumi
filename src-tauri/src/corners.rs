//! Rounded screen corners (`[general] rounded_corners`): small black, anti-aliased
//! quarter-circle masks in the monitors' corners, drawn by tiny layered windows
//! (no web views) that are click-through and never take focus. Where a notch or
//! an always-on bar fills a corner, the engine moves that corner's mask just
//! below (top) or above (bottom) it, so the screen still looks rounded there.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject, AC_SRC_ALPHA, AC_SRC_OVER,
    BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, RegisterClassW, SetTimer, SetWindowPos,
    ShowWindow, UpdateLayeredWindow, HWND_TOPMOST, MSG, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNA, ULW_ALPHA, WM_TIMER,
    WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

/// Which corner of the screen a mask rounds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// One mask: its window's top-left (physical px) and size (= radius).
#[derive(Debug, Clone, PartialEq)]
pub struct Corner {
    pub x: i32,
    pub y: i32,
    pub size: i32,
    pub which: Which,
}

static WANT: Mutex<Vec<Corner>> = Mutex::new(Vec::new());
static DIRTY: AtomicBool = AtomicBool::new(false);

/// Sets the masks to show (empty: none).
pub fn set(list: Vec<Corner>) {
    let mut want = WANT.lock().unwrap();
    if *want != list {
        *want = list;
        DIRTY.store(true, Ordering::Relaxed);
    }
}

pub fn start() {
    std::thread::Builder::new().name("statusnotch-corners".into()).spawn(run).expect("spawn corners thread");
}

unsafe extern "system" fn wndproc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    DefWindowProcW(h, msg, w, l)
}

/// Opaque black outside the quarter circle, clear inside, anti-aliased (premultiplied BGRA).
fn mask(size: i32, which: Which) -> Vec<u8> {
    let r = size as f64;
    let (cx, cy) = match which {
        Which::TopLeft => (r, r),
        Which::TopRight => (0.0, r),
        Which::BottomLeft => (r, 0.0),
        Which::BottomRight => (0.0, 0.0),
    };
    let mut px = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let d = ((x as f64 + 0.5 - cx).powi(2) + (y as f64 + 0.5 - cy).powi(2)).sqrt();
            let a = ((d - r + 0.5).clamp(0.0, 1.0) * 255.0).round() as u8;
            px[((y * size + x) * 4 + 3) as usize] = a; // black: color channels stay 0
        }
    }
    px
}

/// Draws a mask into a layered window and places it.
fn paint(h: HWND, c: &Corner) {
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: c.size,
            biHeight: -c.size,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        if let Ok(bmp) = CreateDIBSection(Some(dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0) {
            let px = mask(c.size, c.which);
            std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u8, px.len());
            let old = SelectObject(dc, HGDIOBJ(bmp.0));
            let blend = BLENDFUNCTION { BlendOp: AC_SRC_OVER as u8, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: AC_SRC_ALPHA as u8 };
            let pos = POINT { x: c.x, y: c.y };
            let size = SIZE { cx: c.size, cy: c.size };
            let src = POINT { x: 0, y: 0 };
            let _ = UpdateLayeredWindow(h, Some(screen), Some(&pos), Some(&size), Some(dc), Some(&src), COLORREF(0), Some(&blend), ULW_ALPHA);
            SelectObject(dc, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
        }
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
    }
}

fn run() {
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        let class = w!("StatusNotchCorner");
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wndproc), hInstance: instance.into(), lpszClassName: class, ..Default::default() });
        SetTimer(None, 0, 100, None);
        let mut shown: Vec<(HWND, Corner)> = Vec::new();
        let mut ticks = 0u32;
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.hwnd.is_invalid() && msg.message == WM_TIMER {
                ticks = ticks.wrapping_add(1);
                if DIRTY.swap(false, Ordering::Relaxed) {
                    let want = WANT.lock().unwrap().clone();
                    while shown.len() > want.len() {
                        let (h, _) = shown.pop().unwrap();
                        let _ = DestroyWindow(h);
                    }
                    for (i, c) in want.iter().enumerate() {
                        if i >= shown.len() {
                            let ex = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE;
                            let Ok(h) = CreateWindowExW(ex, class, w!(""), WS_POPUP, c.x, c.y, c.size, c.size, None, None, Some(instance.into()), None) else {
                                continue;
                            };
                            paint(h, c);
                            let _ = ShowWindow(h, SW_SHOWNA);
                            shown.push((h, c.clone()));
                        } else if shown[i].1 != *c {
                            paint(shown[i].0, c);
                            shown[i].1 = c.clone();
                        }
                    }
                } else if ticks.is_multiple_of(20) {
                    // Stay above other topmost windows that came up since.
                    for (h, _) in &shown {
                        let _ = SetWindowPos(*h, Some(HWND_TOPMOST), 0, 0, 0, 0, SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE);
                    }
                }
                continue;
            }
            DispatchMessageW(&msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_are_clear_inside_and_black_in_the_corner() {
        let m = mask(10, Which::TopLeft);
        let alpha = |x: i32, y: i32| m[((y * 10 + x) * 4 + 3) as usize];
        assert_eq!(alpha(0, 0), 255, "the very corner is covered");
        assert_eq!(alpha(9, 9), 0, "next to the circle's centre it's clear");
        let m = mask(10, Which::BottomRight);
        assert_eq!(m[((9 * 10 + 9) * 4 + 3) as usize], 255);
    }
}
