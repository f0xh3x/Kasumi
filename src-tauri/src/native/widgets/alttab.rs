//! Kasumi's Alt + Tab ([alt_tab]): the windows to switch to, as cards out of the top
//! bar's centre notch: a live preview (Windows draws it, see Surface::sync_thumbs),
//! and / or the app's icon and the window's title. The native thread (mod.rs) keeps
//! what's open and chosen here; Alt + Tab's keys move the choice, letting go of Alt
//! switches to it.

use std::cell::RefCell;
use std::collections::HashMap;

use serde_json::Value;

use super::icon::{decode, AppIcon, Pixels, Style};
use super::{fit_text, Act, Button, Ctx, Panel, Reply};
use crate::native::gfx::{rect, white, with_alpha, Gfx};

/// A window to switch to: its handle, title and app.
#[derive(Clone)]
pub struct Entry {
    pub hwnd: i64,
    pub title: String,
    pub app: String,
}

/// The switcher: showing or not, its windows (most recent first), the chosen one,
/// the bar it shows on, and how its cards look ([alt_tab] previews / labels).
#[derive(Default)]
pub struct State {
    pub open: bool,
    pub list: Vec<Entry>,
    pub sel: usize,
    pub label: String,
    pub previews: bool,
    pub labels: String,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State::default());
}

pub fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    S.with(|s| f(&mut s.borrow_mut()))
}

/// Previews: a card's width and its preview's height; padding; the gap between cards.
const CARD_W: f32 = 208.0;
const THUMB_H: f32 = 117.0;
const PAD: f32 = 6.0;
const GAP: f32 = 8.0;

fn line(g: &Gfx, cx: &Ctx) -> f32 {
    g.text("Ag", &cx.secondary(12.0)).map_or(15.0, |t| t.height)
}

/// What's under each window: icon, title (without previews, at least one of them).
fn shows(previews: bool, labels: &str) -> (bool, bool) {
    match labels {
        "icons" => (true, false),
        "titles" => (false, true),
        "none" if previews => (false, false),
        _ => (true, true),
    }
}

pub struct AltTabPanel {
    pixels: HashMap<String, Pixels>,
    icons: HashMap<String, AppIcon>,
    /// The cards as last laid out (content px).
    cards: Vec<(f32, f32, f32, f32)>,
    hovered: Option<usize>,
}

impl AltTabPanel {
    pub fn new() -> AltTabPanel {
        AltTabPanel { pixels: HashMap::new(), icons: HashMap::new(), cards: Vec::new(), hovered: None }
    }
    fn hit(&self, p: (f32, f32)) -> Option<usize> {
        self.cards.iter().position(|(x, y, w, h)| p.0 >= *x && p.0 < x + w && p.1 >= *y && p.1 < y + h)
    }
}

impl Panel for AltTabPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["taskIcons"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        if let Some(map) = v.as_object() {
            for (k, icon) in map {
                if let Some(px) = decode(icon) {
                    self.icons.remove(k);
                    self.pixels.insert(k.clone(), px);
                }
            }
        }
        true
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        let (n, apps, previews, labels) = with(|s| (s.list.len(), s.list.iter().map(|e| e.app.clone()).collect::<Vec<_>>(), s.previews, s.labels.clone()));
        for app in apps {
            if !self.icons.contains_key(&app) {
                if let Some(i) = self.pixels.get(&app).and_then(|px| AppIcon::new(g, px, Style::Color)) {
                    self.icons.insert(app, i);
                }
            }
        }
        let (icon, title) = shows(previews, &labels);
        let lh = line(g, cx);
        // A card: its preview and a label row under it; without previews, the icon over the title.
        let (w, h, per_row) = if previews {
            let label = if icon || title { PAD + lh.max(18.0) } else { 0.0 };
            (CARD_W, PAD + THUMB_H + label + PAD, 5)
        } else {
            match (icon, title) {
                (true, false) => (64.0, 64.0, 10),
                (false, true) => (170.0, 12.0 + lh + 12.0, 6),
                _ => (116.0, 12.0 + 32.0 + 8.0 + lh + 12.0, 8),
            }
        };
        let per_row = n.clamp(1, per_row);
        self.cards = (0..n).map(|i| ((i % per_row) as f32 * (w + GAP), (i / per_row) as f32 * (h + GAP), w, h)).collect();
        let rows = n.div_ceil(per_row).max(1) as f32;
        (per_row as f32 * w + (per_row as f32 - 1.0) * GAP, rows * h + (rows - 1.0) * GAP)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (list, sel, previews, labels) = with(|s| (s.list.clone(), s.sel, s.previews, s.labels.clone()));
        let (show_icon, show_title) = shows(previews, &labels);
        let f = cx.secondary(12.0);
        let lh = line(g, cx);
        for (i, (e, card)) in list.iter().zip(&self.cards).enumerate() {
            let (left, top, w, h) = (x + card.0, y + card.1, card.2, card.3);
            if i == sel {
                g.fill_round(rect(left, top, w, h), 12.0, with_alpha(cx.hover_bg(0.16), alpha));
            } else if self.hovered == Some(i) {
                g.fill_round(rect(left, top, w, h), 12.0, with_alpha(cx.hover_bg(0.08), alpha));
            }
            let fg = with_alpha(if i == sel { cx.look.fg } else { cx.look.dimmed() }, alpha);
            let icon = self.icons.get(&e.app);
            if previews {
                // Behind the live preview: shows for minimized windows (Windows draws none).
                let (tx, ty, tw) = (left + PAD, top + PAD, w - 2.0 * PAD);
                g.fill_round(rect(tx, ty, tw, THUMB_H), 8.0, white(0.05 * alpha));
                if let Some(ic) = icon {
                    ic.draw(g, tx + tw / 2.0 - 16.0, ty + THUMB_H / 2.0 - 16.0, 32.0, 0.5 * alpha);
                }
                if show_icon || show_title {
                    let cy = ty + THUMB_H + PAD + lh.max(18.0) / 2.0;
                    let iw = if show_icon { 16.0 + 6.0 } else { 0.0 };
                    let text = if show_title { fit_text(g, &e.title, &f, tw - iw - 4.0) } else { String::new() };
                    let total = iw + g.measure(&text, &f) - if show_title { 0.0 } else { 6.0 };
                    let mut lx = tx + (tw - total) / 2.0;
                    if show_icon {
                        if let Some(ic) = icon {
                            ic.draw(g, lx, cy - 8.0, 16.0, alpha);
                        }
                        lx += iw;
                    }
                    if show_title {
                        g.text_at(&text, &f, lx, cy, fg);
                    }
                }
            } else {
                let cxm = left + w / 2.0;
                let mut ty = top + 12.0;
                if show_icon {
                    let size = 32.0;
                    let iy = if show_title { ty } else { top + (h - size) / 2.0 };
                    if let Some(ic) = icon {
                        ic.draw(g, cxm - size / 2.0, iy, size, alpha);
                    }
                    ty += size + 8.0;
                }
                if show_title {
                    let text = fit_text(g, &e.title, &f, w - 16.0);
                    let tw = g.measure(&text, &f);
                    g.text_at(&text, &f, cxm - tw / 2.0, ty + lh / 2.0, fg);
                }
            }
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.hit(p));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        let Some(i) = self.hit(at) else { return Reply::None };
        let hwnd = with(|s| {
            s.open = false;
            s.list.get(i).map(|e| e.hwnd as isize)
        });
        match hwnd {
            Some(h) => Reply::CloseAnd(Act::call(move |_| {
                let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_activate(h, false));
            })),
            None => Reply::None,
        }
    }
    fn closed(&mut self) {
        self.hovered = None;
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        self.cards.clone()
    }
    fn thumbs(&self) -> Vec<(isize, (f32, f32, f32, f32))> {
        let (list, previews) = with(|s| (s.list.iter().map(|e| e.hwnd).collect::<Vec<_>>(), s.previews));
        if !previews {
            return Vec::new();
        }
        list.iter().zip(&self.cards).map(|(h, (x, y, w, _))| (*h as isize, (x + PAD, y + PAD, w - 2.0 * PAD, THUMB_H))).collect()
    }
}
