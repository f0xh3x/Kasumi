//! Scroll position pop-up ([osd] scroll): a bar at the right edge of the screen
//! while the window under the cursor scrolls. The keyboard helper signals the wheel
//! and the paging keys (keyhook.rs); this thread then reads where the view is:
//! a standard Windows scroll bar (lists, trees, classic apps) directly, anything
//! else (File Explorer's items, Settings, Store apps) through UI Automation.
//! Browsers, and apps built on them, are left alone: asking them switches on
//! their screen-reader mode, which slows them down.

use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HANDLE, HWND, POINT, RECT, WAIT_OBJECT_0};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::System::Threading::WaitForMultipleObjects;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetParent, GetScrollInfo, GetWindowLongW, GetWindowRect, WindowFromPoint, GA_ROOT, GWL_STYLE, SB_VERT, SCROLLINFO, SIF_ALL,
    WS_VSCROLL,
};

use crate::providers::SharedConfig;

/// Where the view is: the thumb's start and length, as shares of the track.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Pos {
    pub top: f32,
    pub size: f32,
}

/// After the last signal, how long the position is still read (smooth scrolling
/// keeps moving), and how often.
const FOLLOW: Duration = Duration::from_millis(450);
const EVERY: Duration = Duration::from_millis(25);

/// What scrolls: a window's standard bar, or a UI Automation element.
enum Source {
    Bar(HWND),
    Uia(IUIAutomationElement),
}

struct Found {
    /// The window under the cursor when it was found, and the scrolling part's box:
    /// while the cursor stays there, it's the same one.
    under: isize,
    rect: RECT,
    source: Source,
}

impl Found {
    fn key(&self) -> (isize, [i32; 4]) {
        (self.under, [self.rect.left, self.rect.top, self.rect.right, self.rect.bottom])
    }
}

struct Uia {
    uia: IUIAutomation,
    cache: IUIAutomationCacheRequest,
    walker: IUIAutomationTreeWalker,
}

#[derive(Default)]
struct Reader {
    uia: Option<Uia>,
    found: Option<(Found, Instant)>,
}

/// A browser's (or an app built on one: Electron, WebView2) or one of Kasumi's own.
fn left_alone(h: HWND) -> bool {
    let mut cur = h;
    for _ in 0..32 {
        let c = crate::win::class_name(cur);
        if c.starts_with("Chrome_") || c.starts_with("Mozilla") {
            return true;
        }
        match unsafe { GetParent(cur) } {
            Ok(p) if !p.is_invalid() => cur = p,
            _ => break,
        }
    }
    crate::win::window_pid(h) == std::process::id()
}

/// A window's own vertical scroll bar, if it has one with somewhere to go.
fn bar_pos(h: HWND) -> Option<Pos> {
    unsafe {
        if GetWindowLongW(h, GWL_STYLE) as u32 & WS_VSCROLL.0 == 0 {
            return None;
        }
        let mut si = SCROLLINFO { cbSize: std::mem::size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..Default::default() };
        GetScrollInfo(h, SB_VERT, &mut si).ok()?;
        let total = (si.nMax - si.nMin + 1) as f32;
        let page = si.nPage as f32;
        if page <= 0.0 || page >= total {
            return None;
        }
        Some(Pos { top: ((si.nPos - si.nMin) as f32 / total).clamp(0.0, 1.0 - page / total), size: page / total })
    }
}

/// An element's position from its cached scroll properties, if it scrolls up and down.
fn uia_pos(e: &IUIAutomationElement) -> Option<Pos> {
    unsafe {
        let flag = |id| e.GetCachedPropertyValue(id).ok().and_then(|v| bool::try_from(&v).ok()).unwrap_or(false);
        let num = |id| e.GetCachedPropertyValue(id).ok().and_then(|v| f64::try_from(&v).ok());
        if !flag(UIA_IsScrollPatternAvailablePropertyId) || !flag(UIA_ScrollVerticallyScrollablePropertyId) {
            return None;
        }
        let (pct, view) = (num(UIA_ScrollVerticalScrollPercentPropertyId)?, num(UIA_ScrollVerticalViewSizePropertyId)?);
        if !(0.0..=100.0).contains(&pct) || !(0.0..100.0).contains(&view) || view <= 0.0 {
            return None;
        }
        let size = (view / 100.0) as f32;
        Some(Pos { top: (pct / 100.0) as f32 * (1.0 - size), size })
    }
}

impl Reader {
    fn uia(&mut self) -> Option<&Uia> {
        if self.uia.is_none() {
            unsafe {
                let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER).ok()?;
                // A hung app answers nothing: give up quickly (this thread only waits).
                if let Ok(u2) = windows::core::Interface::cast::<IUIAutomation2>(&uia) {
                    let _ = u2.SetConnectionTimeout(500);
                    let _ = u2.SetTransactionTimeout(500);
                }
                let cache = uia.CreateCacheRequest().ok()?;
                for id in [
                    UIA_IsScrollPatternAvailablePropertyId,
                    UIA_ScrollVerticallyScrollablePropertyId,
                    UIA_ScrollVerticalScrollPercentPropertyId,
                    UIA_ScrollVerticalViewSizePropertyId,
                    UIA_NativeWindowHandlePropertyId,
                    UIA_BoundingRectanglePropertyId,
                ] {
                    let _ = cache.AddProperty(id);
                }
                let walker = uia.RawViewWalker().ok()?;
                self.uia = Some(Uia { uia, cache, walker });
            }
        }
        self.uia.as_ref()
    }

    /// The scrolling part under `pt` and where it is (None: nothing there scrolls, or
    /// it's left alone).
    fn read(&mut self, pt: POINT) -> Option<((isize, [i32; 4]), Pos)> {
        let under = unsafe { WindowFromPoint(pt) };
        if under.is_invalid() || left_alone(under) {
            self.found = None;
            return None;
        }
        // The same part as a moment ago: only its position again.
        if let Some((f, at)) = &self.found {
            let inside = pt.x >= f.rect.left && pt.x < f.rect.right && pt.y >= f.rect.top && pt.y < f.rect.bottom;
            if f.under == under.0 as isize && inside && at.elapsed() < Duration::from_secs(5) {
                if let Some(p) = self.again() {
                    return Some((self.found.as_ref()?.0.key(), p));
                }
            }
        }
        self.found = None;
        let root = unsafe { GetAncestor(under, GA_ROOT) };
        // A standard scroll bar on the window or one it sits in.
        let mut cur = under;
        loop {
            if let Some(p) = bar_pos(cur) {
                let mut rect = RECT::default();
                unsafe {
                    let _ = GetWindowRect(cur, &mut rect);
                }
                eprintln!("[kasumi] scroll: {} ({}): its scroll bar", crate::win::class_name(cur), crate::win::window_process(root));
                let f = Found { under: under.0 as isize, rect, source: Source::Bar(cur) };
                let key = f.key();
                self.found = Some((f, Instant::now()));
                return Some((key, p));
            }
            if cur == root {
                break;
            }
            match unsafe { GetParent(cur) } {
                Ok(p) if !p.is_invalid() => cur = p,
                _ => break,
            }
        }
        // UI Automation: the element under the cursor, then up until one scrolls (not
        // past the top-level window).
        let u = self.uia()?;
        unsafe {
            let mut e = u.uia.ElementFromPointBuildCache(pt, &u.cache).ok()?;
            for _ in 0..24 {
                if let Some(p) = uia_pos(&e) {
                    let rect = e.CachedBoundingRectangle().unwrap_or_default();
                    eprintln!("[kasumi] scroll: {} ({}): UI Automation", crate::win::class_name(under), crate::win::window_process(root));
                    let f = Found { under: under.0 as isize, rect, source: Source::Uia(e) };
                    let key = f.key();
                    self.found = Some((f, Instant::now()));
                    return Some((key, p));
                }
                if e.CachedNativeWindowHandle().is_ok_and(|h| h == root) {
                    break;
                }
                e = u.walker.GetParentElementBuildCache(&e, &u.cache).ok()?;
            }
        }
        None
    }

    /// The found part's position now.
    fn again(&mut self) -> Option<Pos> {
        let (f, at) = self.found.as_mut()?;
        *at = Instant::now();
        match &f.source {
            Source::Bar(h) => bar_pos(*h),
            Source::Uia(e) => {
                let cache = &self.uia.as_ref()?.cache;
                let fresh = unsafe { e.BuildUpdatedCache(cache).ok()? };
                let p = uia_pos(&fresh);
                f.source = Source::Uia(fresh);
                p
            }
        }
    }
}

/// Hands the bar to the native thread: on the cursor's monitor, out for `ms`.
fn show(app: &AppHandle, pt: POINT, pos: Pos) {
    let (on, ms, length, width, anim, theme, geometry) = {
        let cfg = app.state::<SharedConfig>();
        let cfg = cfg.read().unwrap();
        (cfg.osd.enabled && cfg.osd.scroll, cfg.osd.duration_ms, cfg.osd.scroll_length, cfg.osd.scroll_width, cfg.animation.clone(), cfg.theme.clone(), cfg.geometry.clone())
    };
    if !on {
        return;
    }
    let Some(monitor) = crate::win::monitors().into_iter().find(|m| pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom) else { return };
    crate::native::post(crate::native::Msg::Scroll(Box::new(crate::native::ScrollShow { monitor, top: pos.top, size: pos.size, ms, length, width, anim, theme, geometry })));
}

fn held(vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY) -> bool {
    unsafe { GetAsyncKeyState(vk.0 as i32) as u16 & 0x8000 != 0 }
}

pub fn start(app: &AppHandle) {
    if !crate::native::active() {
        crate::keyhook::set_scroll(false);
        return;
    }
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-scroll".into())
        .spawn(move || {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
            let signals = crate::keyhook::scroll_signals();
            let mut reader = Reader::default();
            // Following one part: it, until when, where it was last.
            let mut follow: Option<((isize, [i32; 4]), Instant, Pos)> = None;
            // The last position read of a part (a paging key shows only a move).
            let mut known: Option<((isize, [i32; 4]), Pos)> = None;
            loop {
                let on = {
                    let cfg = app.state::<SharedConfig>();
                    let cfg = cfg.read().unwrap();
                    cfg.osd.enabled && cfg.osd.scroll
                };
                crate::keyhook::set_scroll(on);
                let wait = if follow.is_some() { EVERY } else { Duration::from_secs(1) };
                let handles = signals.map(|h| HANDLE(h as *mut _));
                let r = unsafe { WaitForMultipleObjects(&handles, false, wait.as_millis() as u32) };
                let signal = match r.0.wrapping_sub(WAIT_OBJECT_0.0) {
                    0 => Some(true),
                    1 => Some(false),
                    _ => None,
                };
                if !on {
                    follow = None;
                    continue;
                }
                let Some((x, y)) = crate::win::cursor_pos() else { continue };
                let pt = POINT { x, y };
                let now = Instant::now();
                match signal {
                    Some(wheel) => {
                        // Ctrl + wheel zooms, Shift + wheel scrolls sideways.
                        if wheel && (held(VK_CONTROL) || held(VK_SHIFT)) {
                            continue;
                        }
                        let Some((key, pos)) = reader.read(pt) else {
                            follow = None;
                            continue;
                        };
                        // The wheel over something that scrolls shows where it is (at its end
                        // too); a paging key only when the view moved.
                        let moved = known.is_some_and(|(k, p)| k == key && p != pos);
                        if wheel || moved {
                            show(&app, pt, pos);
                        }
                        known = Some((key, pos));
                        follow = Some((key, now + FOLLOW, pos));
                    }
                    None => {
                        let Some((key, until, last)) = follow else { continue };
                        if now >= until {
                            follow = None;
                            continue;
                        }
                        // Smooth scrolling still moving the view.
                        match reader.read(pt) {
                            Some((k, pos)) if k == key => {
                                if pos != last {
                                    show(&app, pt, pos);
                                }
                                known = Some((k, pos));
                                follow = Some((key, until, pos));
                            }
                            _ => follow = None,
                        }
                    }
                }
            }
        })
        .expect("spawn scroll thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What scrolls at the middle of each visible window, read only (no input):
    /// `cargo test scroll_reads -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scroll_reads() {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }
        use windows::core::BOOL;
        use windows::Win32::Foundation::LPARAM;
        use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, IsIconic, IsWindowVisible};
        unsafe extern "system" fn each(h: HWND, data: LPARAM) -> BOOL {
            let list = &mut *(data.0 as *mut Vec<HWND>);
            if IsWindowVisible(h).as_bool() && !IsIconic(h).as_bool() {
                list.push(h);
            }
            true.into()
        }
        let mut list: Vec<HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), LPARAM(&mut list as *mut _ as isize));
        }
        let mut reader = Reader::default();
        for hwnd in list {
            let mut r = RECT::default();
            unsafe {
                let _ = GetWindowRect(hwnd, &mut r);
            }
            let pt = POINT { x: (r.left + r.right) / 2, y: (r.top + r.bottom) / 2 };
            let under = unsafe { WindowFromPoint(pt) };
            let t = Instant::now();
            let got = reader.read(pt);
            println!(
                "{:<28} under {:<28} skip={} -> {:?} ({} ms)",
                crate::win::class_name(hwnd),
                crate::win::class_name(under),
                left_alone(under),
                got.map(|(_, p)| p),
                t.elapsed().as_millis()
            );
        }
    }
}
