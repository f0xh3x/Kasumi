//! Panels with lists to act on: clipboard history (extras.ts clipboardPanel),
//! notifications (notificationsPanel) and an app's menu in the tasks widget
//! (tasks.ts tasksPanel: its windows, a new window, pin, close all).

use std::collections::HashMap;

use serde_json::{json, Value};

use super::icon::{decode, AppIcon, Pixels, Style};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply};
use crate::native::gfx::{rect, white, with_alpha, Font, Gfx};

fn line(g: &Gfx, f: &Font) -> f32 {
    g.text("Ag", f).map_or(f.size * 1.21, |t| t.height)
}

/// A list that scrolls with the wheel once it's taller than `max` (the page's
/// max-height + overflow-y: auto).
#[derive(Default)]
struct Scroll {
    offset: f32,
    content: f32,
    max: f32,
}

impl Scroll {
    fn shown(&self) -> f32 {
        self.content.min(self.max)
    }
    fn wheel(&mut self, notches: f32) -> bool {
        let was = self.offset;
        self.offset = (self.offset - notches * 48.0).clamp(0.0, (self.content - self.max).max(0.0));
        (self.offset - was).abs() > 0.1
    }
    fn fit(&mut self, content: f32, max: f32) {
        self.content = content;
        self.max = max;
        self.offset = self.offset.clamp(0.0, (content - max).max(0.0));
    }
}

/// "now", "5 min", "3 h", or the date.
fn ago(unix: f64) -> String {
    let s = (super::extras::now_secs() - unix).max(0.0);
    if s < 60.0 {
        "now".into()
    } else if s < 3600.0 {
        format!("{} min", (s / 60.0).floor())
    } else if s < 86400.0 {
        format!("{} h", (s / 3600.0).floor())
    } else {
        const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
        let days = (unix / 86400.0).floor() as i64;
        let z = days + 719468;
        let era = z.div_euclid(146097);
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        format!("{d} {}", MONTHS[(m - 1) as usize])
    }
}

// ----------------------------------------------------------------------

struct Clip {
    id: u64,
    text: Option<String>,
    thumb: Option<Pixels>,
}

/// Recent copies to paste again; × removes one, "Clear" all of them.
pub struct ClipboardPanel {
    items: Vec<Clip>,
    thumbs: HashMap<u64, windows::Win32::Graphics::Direct2D::ID2D1Bitmap>,
    rows: Vec<(f32, f32)>,
    scroll: Scroll,
    hovered: Option<usize>,
    hover_x: bool,
    hover_clear: bool,
}

const CLIP_W: f32 = 300.0;

impl ClipboardPanel {
    pub fn new() -> ClipboardPanel {
        ClipboardPanel { items: Vec::new(), thumbs: HashMap::new(), rows: Vec::new(), scroll: Scroll::default(), hovered: None, hover_x: false, hover_clear: false }
    }
    fn text_w() -> f32 {
        CLIP_W - 16.0 - 8.0 - 14.0
    }
    /// The row and whether the × under a point, the "Clear" link.
    fn hit(&self, p: (f32, f32)) -> (Option<usize>, bool, bool) {
        let list_h = self.scroll.shown();
        if p.1 > list_h {
            let clear = !self.items.is_empty() && p.1 >= list_h + 8.0 && p.0 >= CLIP_W - 50.0;
            return (None, false, clear);
        }
        let y = p.1 + self.scroll.offset;
        let row = self.rows.iter().position(|(top, h)| y >= *top && y < top + h);
        (row, row.is_some() && p.0 >= CLIP_W - 8.0 - 14.0 - 4.0, false)
    }
}

impl Panel for ClipboardPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["clipboard"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.items = v
            .as_array()
            .map(|a| {
                a.iter()
                    .map(|it| Clip {
                        id: it.get("id").and_then(|x| x.as_u64()).unwrap_or(0),
                        text: (it.get("kind").and_then(|k| k.as_str()) != Some("image")).then(|| it.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string()),
                        thumb: it.get("thumb").and_then(decode),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let ids: Vec<u64> = self.items.iter().map(|c| c.id).collect();
        self.thumbs.retain(|id, _| ids.contains(id));
        true
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        for c in &self.items {
            if let (Some(px), false) = (&c.thumb, self.thumbs.contains_key(&c.id)) {
                if let Ok(b) = g.bitmap(px.0, px.1, &px.2) {
                    self.thumbs.insert(c.id, b);
                }
            }
        }
        let f = cx.secondary(12.0);
        if self.items.is_empty() {
            self.rows.clear();
            self.scroll.fit(line(g, &f), 360.0);
            return (CLIP_W, line(g, &f));
        }
        let mut y = 0.0;
        self.rows = self
            .items
            .iter()
            .map(|c| {
                let body = match (&c.text, &c.thumb) {
                    (Some(t), _) => g.paragraph(t, &f, Self::text_w(), 2).map_or(line(g, &f), |p| p.height),
                    (None, Some(px)) => {
                        let k = (160.0 / px.0 as f32).min(90.0 / px.1 as f32).min(1.0);
                        px.1 as f32 * k
                    }
                    (None, None) => line(g, &f),
                };
                let r = (y, body + 14.0);
                y += body + 14.0 + 4.0;
                r
            })
            .collect();
        self.scroll.fit(y - 4.0, 360.0);
        (CLIP_W, self.scroll.shown() + 8.0 + line(g, &f) + 4.0)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let f = cx.secondary(12.0);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        if self.items.is_empty() {
            g.text_at("Copy something and it shows up here.", &f, x, y + line(g, &f) / 2.0, dim);
            return;
        }
        g.clip(rect(x - 2.0, y, CLIP_W + 4.0, self.scroll.shown()));
        for (i, (c, (top, h))) in self.items.iter().zip(&self.rows).enumerate() {
            let ry = y + top - self.scroll.offset;
            if ry + h < y || ry > y + self.scroll.shown() {
                continue;
            }
            let hovered = self.hovered == Some(i);
            g.fill_round(rect(x, ry, CLIP_W, *h), 8.0, if hovered { with_alpha(cx.hover_bg(0.12), alpha) } else { white(0.05 * alpha) });
            match (&c.text, self.thumbs.get(&c.id), &c.thumb) {
                (Some(t), _, _) => {
                    if let Some(p) = g.paragraph(t, &f, Self::text_w(), 2) {
                        g.draw_text(&p, g.snap(x + 8.0), g.snap(ry + 7.0), with_alpha(cx.look.fg, alpha));
                    }
                }
                (None, Some(b), Some(px)) => {
                    let k = (160.0 / px.0 as f32).min(90.0 / px.1 as f32).min(1.0);
                    g.fill_picture(b, rect(x + 8.0, ry + 7.0, px.0 as f32 * k, px.1 as f32 * k), 5.0, alpha);
                }
                _ => {
                    g.text_at("Image", &f, x + 8.0, ry + h / 2.0, dim);
                }
            }
            if hovered {
                let xf = Font::icons(10.0);
                let (bx, by) = (x + CLIP_W - 8.0 - 14.0, ry + h / 2.0 - 7.0);
                if self.hover_x {
                    g.fill_round(rect(bx, by, 14.0, 14.0), 5.0, white(0.12 * alpha));
                }
                let w = g.measure("\u{E711}", &xf);
                g.text_at("\u{E711}", &xf, bx + (14.0 - w) / 2.0, ry + h / 2.0, white(if self.hover_x { alpha } else { 0.7 * alpha }));
            }
        }
        g.unclip();
        // "Clear", right-aligned under the list.
        let cyy = y + self.scroll.shown() + 8.0 + line(g, &f) / 2.0;
        let w = g.measure("Clear", &f);
        if self.hover_clear {
            g.fill_round(rect(x + CLIP_W - w - 12.0, cyy - line(g, &f) / 2.0 - 2.0, w + 12.0, line(g, &f) + 4.0), 6.0, white(0.1 * alpha));
        }
        g.text_at("Clear", &f, x + CLIP_W - w - 6.0, cyy, if self.hover_clear { with_alpha(cx.look.fg, alpha) } else { dim });
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let (row, x, clear) = at.map_or((None, false, false), |p| self.hit(p));
        let changed = row != self.hovered || x != self.hover_x || clear != self.hover_clear;
        (self.hovered, self.hover_x, self.hover_clear) = (row, x, clear);
        changed
    }
    fn wheel(&mut self, _at: (f32, f32), notches: f32) -> Reply {
        if self.scroll.wheel(notches) { Reply::Redraw } else { Reply::None }
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        match self.hit(at) {
            (Some(i), true, _) => {
                let id = self.items[i].id;
                Reply::Do(Act::call(move |_| {
                    let _ = tauri::async_runtime::block_on(crate::providers::clipboard::clipboard_remove(id));
                }))
            }
            (Some(i), false, _) => {
                let id = self.items[i].id;
                Reply::CloseAnd(Act::call(move |_| {
                    let _ = tauri::async_runtime::block_on(crate::providers::clipboard::clipboard_paste(id));
                }))
            }
            (None, _, true) => Reply::Do(Act::call(|_| {
                let _ = tauri::async_runtime::block_on(crate::providers::clipboard::clipboard_clear());
            })),
            _ => Reply::None,
        }
    }
    fn closed(&mut self) {
        (self.hovered, self.hover_x, self.hover_clear) = (None, false, false);
        self.scroll.offset = 0.0;
    }
}

// ----------------------------------------------------------------------

struct Note {
    id: i64,
    app: String,
    aumid: String,
    title: String,
    body: String,
    time: f64,
}

/// Recent notifications in the app's style: its icon and name, when, title and
/// text. A click opens the app (`click` = "open") ; with `keep`, × dismisses one
/// and "Clear all" all of them; `clear_on_close` dismisses what was shown once shut.
pub struct NotificationsPanel {
    opts: Opts,
    notes: Vec<Note>,
    /// App icons by AppUserModelID (asked for once each).
    icons: HashMap<String, AppIcon>,
    pixels: HashMap<String, Pixels>,
    asked: Vec<String>,
    pending: Option<Act>,
    rows: Vec<(f32, f32)>,
    scroll: Scroll,
    hovered: Option<(usize, bool)>,
    hovered_action: Option<usize>,
    /// The action buttons as last drawn (x from the content's left, width).
    actions_x: std::cell::RefCell<Vec<(f32, f32)>>,
}

const NOTE_W: f32 = 320.0;

impl NotificationsPanel {
    pub fn new(opts: &Opts) -> NotificationsPanel {
        NotificationsPanel { opts: opts.clone(), notes: Vec::new(), icons: HashMap::new(), pixels: HashMap::new(), asked: Vec::new(), pending: None, rows: Vec::new(), scroll: Scroll::default(), hovered: None, hovered_action: None, actions_x: std::cell::RefCell::new(Vec::new()) }
    }
    fn keeps(&self) -> bool {
        self.opts.flag("keep") == Some(true)
    }
    fn opens(&self) -> bool {
        self.opts.str("click") != Some("nothing")
    }
    fn actions(&self) -> Vec<&'static str> {
        let mut a = Vec::new();
        if self.keeps() && !self.notes.is_empty() {
            a.push("Clear all");
        }
        a.push("Notification settings");
        a
    }
    fn body_w() -> f32 {
        NOTE_W - 20.0
    }
    fn dismiss(ids: Option<Vec<i64>>) -> Act {
        Act::call(move |app| {
            let _ = tauri::async_runtime::block_on(crate::providers::notifications::notifications_dismiss(app, ids));
        })
    }
    fn hit_row(&self, p: (f32, f32)) -> Option<(usize, bool)> {
        if p.1 > self.scroll.shown() {
            return None;
        }
        let y = p.1 + self.scroll.offset;
        let i = self.rows.iter().position(|(top, h)| y >= *top && y < top + h)?;
        // The × in the head (keep): its right end, the head's height.
        let on_x = self.keeps() && p.0 >= NOTE_W - 10.0 - 18.0 && y - self.rows[i].0 < 8.0 + 18.0;
        Some((i, on_x))
    }
    fn hit_action(&self, p: (f32, f32)) -> Option<usize> {
        if p.1 <= self.scroll.shown() + 4.0 {
            return None;
        }
        self.actions_x.borrow().iter().position(|(ax, aw)| p.0 >= *ax && p.0 < ax + aw)
    }
}

impl Panel for NotificationsPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["notifications", "appIcons"]
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        if key == "appIcons" {
            if let Some(map) = v.as_object() {
                for (id, icon) in map {
                    if !self.icons.contains_key(id) {
                        if let Some(px) = decode(icon) {
                            self.pixels.insert(id.clone(), px);
                        }
                    }
                }
            }
            return true;
        }
        self.notes = v
            .get("items")
            .and_then(|i| i.as_array())
            .map(|a| {
                a.iter()
                    .map(|n| {
                        let s = |k: &str| n.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                        Note { id: n.get("id").and_then(|x| x.as_i64()).unwrap_or(0), app: s("app"), aumid: s("aumid"), title: s("title"), body: s("body"), time: n.get("time").and_then(|x| x.as_f64()).unwrap_or(0.0) }
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Icons not asked for yet: from the launcher's index (as the pages' appIcon()).
        let wanted: Vec<String> = self.notes.iter().map(|n| n.aumid.clone()).filter(|a| !a.is_empty() && !self.asked.contains(a)).collect();
        if !wanted.is_empty() {
            self.asked.extend(wanted.iter().cloned());
            self.pending = Some(Act::call(move |_| {
                let mut found = serde_json::Map::new();
                for id in wanted {
                    if let Ok(icon) = tauri::async_runtime::block_on(crate::launcher::launcher_icon(id.clone())) {
                        found.insert(id, icon);
                    }
                }
                if !found.is_empty() {
                    crate::native::post(crate::native::Msg::Entries("appIcons".into(), found));
                }
            }));
        }
        true
    }
    fn act(&mut self) -> Option<Act> {
        self.pending.take()
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        for (id, px) in self.pixels.drain() {
            if let Some(i) = AppIcon::new(g, &px, Style::Color) {
                self.icons.insert(id, i);
            }
        }
        let (head, title, body) = (line(g, &cx.secondary(11.0)).max(14.0), cx.body(), cx.secondary(12.0));
        let mut y = 0.0;
        self.rows = self
            .notes
            .iter()
            .map(|n| {
                let mut h = 8.0 + head;
                if !n.title.is_empty() {
                    h += 2.0 + line(g, &title);
                }
                if !n.body.is_empty() {
                    h += 2.0 + g.paragraph(&n.body, &body, Self::body_w(), 3).map_or(line(g, &body), |p| p.height);
                }
                h += 8.0;
                let r = (y, h);
                y += h + 6.0;
                r
            })
            .collect();
        let empty = line(g, &cx.body());
        self.scroll.fit(if self.notes.is_empty() { empty } else { y - 6.0 }, 380.0);
        let af = cx.secondary(12.0);
        (NOTE_W, self.scroll.shown() + 8.0 + line(g, &af) + 6.0)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        if self.notes.is_empty() {
            g.text_at("No notifications.", &cx.body(), x, y + line(g, &cx.body()) / 2.0, dim);
        } else {
            g.clip(rect(x - 2.0, y, NOTE_W + 4.0, self.scroll.shown()));
            let hf = cx.secondary(11.0);
            let head = line(g, &hf).max(14.0);
            for (i, (n, (top, h))) in self.notes.iter().zip(&self.rows).enumerate() {
                let ry = y + top - self.scroll.offset;
                if ry + h < y || ry > y + self.scroll.shown() {
                    continue;
                }
                let hovered = self.hovered.is_some_and(|(r, _)| r == i);
                let bg = if hovered && self.opens() { cx.hover_bg(0.1) } else { white(0.05) };
                g.fill_round(rect(x, ry, NOTE_W, *h), 9.0, with_alpha(bg, alpha));
                let (lx, mut yy) = (x + 10.0, ry + 8.0);
                let hc = yy + head / 2.0;
                match self.icons.get(&n.aumid) {
                    Some(icon) => icon.draw(g, lx, hc - 7.0, 14.0, alpha),
                    None => {
                        let f = Font::icons(11.0);
                        let w = g.measure("\u{EA8F}", &f);
                        g.text_at("\u{EA8F}", &f, lx + (14.0 - w) / 2.0, hc, dim);
                    }
                }
                let time = ago(n.time);
                let tw = g.measure(&time, &hf);
                let right = x + NOTE_W - 10.0 - if self.keeps() { 18.0 + 6.0 } else { 0.0 };
                g.text_at(&time, &hf, right - tw, hc, dim);
                g.text_at(&fit_text(g, &n.app, &hf, right - tw - 6.0 - (lx + 20.0)), &hf, lx + 20.0, hc, dim);
                if self.keeps() {
                    let xf = Font::icons(11.0);
                    let on = self.hovered == Some((i, true));
                    if on {
                        g.fill_round(rect(x + NOTE_W - 10.0 - 18.0, hc - 9.0, 18.0, 18.0), 5.0, white(0.12 * alpha));
                    }
                    let w = g.measure("\u{E711}", &xf);
                    g.text_at("\u{E711}", &xf, x + NOTE_W - 10.0 - 9.0 - w / 2.0, hc, if on { fg } else { dim });
                }
                yy += head;
                if !n.title.is_empty() {
                    yy += 2.0;
                    let f = cx.body();
                    g.text_at(&fit_text(g, &n.title, &f, Self::body_w()), &f, lx, yy + line(g, &f) / 2.0, fg);
                    yy += line(g, &f);
                }
                if !n.body.is_empty() {
                    yy += 2.0;
                    if let Some(p) = g.paragraph(&n.body, &cx.secondary(12.0), Self::body_w(), 3) {
                        g.draw_text(&p, g.snap(lx), g.snap(yy), white(0.75 * alpha));
                    }
                }
            }
            g.unclip();
        }
        // The actions, right-aligned under the list.
        let af = cx.secondary(12.0);
        let cyy = y + self.scroll.shown() + 8.0 + 3.0 + line(g, &af) / 2.0;
        let mut ax = x + NOTE_W;
        let mut boxes = Vec::new();
        for (i, a) in self.actions().iter().enumerate().rev() {
            let w = g.measure(a, &af) + 16.0;
            ax -= w;
            let on = self.hovered_action == Some(i);
            if on {
                g.fill_round(rect(ax, cyy - line(g, &af) / 2.0 - 3.0, w, line(g, &af) + 6.0), 6.0, white(0.1 * alpha));
            }
            g.text_at(a, &af, ax + 8.0, cyy, if on { fg } else { dim });
            boxes.push((ax - x, w));
            ax -= 6.0;
        }
        boxes.reverse();
        *self.actions_x.borrow_mut() = boxes;
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let row = at.and_then(|p| self.hit_row(p));
        let action = at.and_then(|p| self.hit_action(p));
        let changed = row != self.hovered || action != self.hovered_action;
        self.hovered = row;
        self.hovered_action = action;
        changed
    }
    fn wheel(&mut self, _at: (f32, f32), notches: f32) -> Reply {
        if self.scroll.wheel(notches) { Reply::Redraw } else { Reply::None }
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        if let Some(i) = self.hit_action(at) {
            return match self.actions()[i] {
                "Clear all" => Reply::Do(Self::dismiss(None)),
                _ => Reply::CloseAnd(Act::open("ms-settings:notifications")),
            };
        }
        match self.hit_row(at) {
            Some((i, true)) => Reply::Do(Self::dismiss(Some(vec![self.notes[i].id]))),
            Some((i, false)) if self.opens() && !self.notes[i].aumid.is_empty() => {
                let id = self.notes[i].aumid.clone();
                Reply::CloseAnd(Act::call(move |_| {
                    let _ = tauri::async_runtime::block_on(crate::launcher::launcher_launch(id));
                }))
            }
            _ => Reply::None,
        }
    }
    fn closed(&mut self) {
        // [widgets.notifications] clear_on_close: what the panel showed goes once it's shut.
        if self.opts.flag("clear_on_close") == Some(true) && !self.notes.is_empty() {
            self.pending = Some(Self::dismiss(Some(self.notes.iter().map(|n| n.id).collect())));
        }
        self.hovered = None;
        self.hovered_action = None;
        self.scroll.offset = 0.0;
    }
}

// ----------------------------------------------------------------------

/// An app's menu (right click on it in the tasks widget): its windows (click one
/// to front it, × to close it), a new window, pin / unpin, close all.
pub struct TasksPanel {
    opts: Opts,
    tasks: Value,
    pixels: HashMap<String, Pixels>,
    icons: HashMap<String, AppIcon>,
    /// Rows as last laid out: (what, top, height).
    rows: Vec<(TaskRow, f32, f32)>,
    hovered: Option<(usize, bool)>,
    width: f32,
}

#[derive(Clone, PartialEq, Debug)]
enum TaskRow {
    Head,
    Window(i64, String, bool),
    Sep,
    Launch(bool),
    Pin(bool),
    CloseAll(Vec<i64>),
    Empty,
}

/// A window title without the app's name at its end ("Docs - File Explorer" →
/// "Docs"); a folder path shows just the folder.
fn short_title(title: &str, app: &str) -> String {
    let mut t = title.to_string();
    for sep in [" - ", " — ", " – "] {
        let tail = format!("{sep}{app}");
        if t.to_lowercase().ends_with(&tail.to_lowercase()) {
            t.truncate(t.len() - tail.len());
        }
    }
    let b = t.as_bytes();
    if (b.len() > 2 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\') || t.starts_with("\\\\") {
        if let Some(last) = t.split('\\').filter(|p| !p.is_empty()).next_back() {
            t = last.to_string();
        }
    }
    let t = t.trim();
    if t.is_empty() { title.to_string() } else { t.to_string() }
}

impl TasksPanel {
    pub fn new(opts: &Opts) -> TasksPanel {
        TasksPanel { opts: opts.clone(), tasks: Value::Null, pixels: HashMap::new(), icons: HashMap::new(), rows: Vec::new(), hovered: None, width: 220.0 }
    }
    fn app(&self) -> Option<&Value> {
        let key = super::tasks::menu_target();
        self.tasks.get("apps").and_then(|a| a.as_array()).and_then(|a| a.iter().find(|x| x.get("key").and_then(|k| k.as_str()) == Some(key.as_str())))
    }
    /// The pin list as written in the config (app keys / dropped files' paths).
    fn pins(&self) -> Vec<String> {
        self.tasks.get("apps").and_then(|a| a.as_array()).map(|a| a.iter().filter(|x| x.get("pinned").and_then(|p| p.as_bool()) == Some(true)).filter_map(|x| x.get("pin").and_then(|p| p.as_str()).or(x.get("key").and_then(|k| k.as_str())).map(String::from)).collect()).unwrap_or_default()
    }
    fn list(&self) -> Vec<TaskRow> {
        let Some(app) = self.app() else { return vec![TaskRow::Empty] };
        let key = app.get("key").and_then(|k| k.as_str()).unwrap_or("");
        let name = app.get("name").and_then(|k| k.as_str()).unwrap_or("");
        let wins: Vec<(i64, String, bool)> = self
            .tasks
            .get("windows")
            .and_then(|w| w.as_array())
            .map(|w| w.iter().filter(|x| x.get("app").and_then(|a| a.as_str()) == Some(key)).map(|x| (x.get("hwnd").and_then(|h| h.as_i64()).unwrap_or(0), short_title(x.get("title").and_then(|t| t.as_str()).unwrap_or(""), name), x.get("active").and_then(|a| a.as_bool()).unwrap_or(false))).collect())
            .unwrap_or_default();
        let mut out = vec![TaskRow::Head];
        out.extend(wins.iter().map(|(h, t, a)| TaskRow::Window(*h, t.clone(), *a)));
        out.push(TaskRow::Sep);
        out.push(TaskRow::Launch(!wins.is_empty()));
        if self.opts.flag("pinning") != Some(false) && self.opts.flag("pinned") != Some(false) {
            out.push(TaskRow::Pin(app.get("pinned").and_then(|p| p.as_bool()).unwrap_or(false)));
        }
        if !wins.is_empty() {
            out.push(TaskRow::CloseAll(wins.iter().map(|w| w.0).collect()));
        }
        out
    }
    fn label(row: &TaskRow, count: usize) -> (&'static str, String) {
        match row {
            TaskRow::Launch(running) => ("\u{E710}", if *running { "New window".into() } else { "Open".into() }),
            TaskRow::Pin(pinned) => if *pinned { ("\u{E77A}", "Unpin".into()) } else { ("\u{E718}", "Pin".into()) },
            TaskRow::CloseAll(_) => ("\u{E711}", if count > 1 { "Close all windows".into() } else { "Close window".into() }),
            _ => ("", String::new()),
        }
    }
    fn hit(&self, p: (f32, f32)) -> Option<(usize, bool)> {
        let i = self.rows.iter().position(|(_, top, h)| p.1 >= *top && p.1 < top + h)?;
        let on_x = matches!(self.rows[i].0, TaskRow::Window(..)) && p.0 >= self.width - 8.0 - 18.0;
        Some((i, on_x))
    }
}

impl Panel for TasksPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["tasks", "taskIcons"]
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        if key == "taskIcons" {
            if let Some(map) = v.as_object() {
                for (k, icon) in map {
                    if let Some(px) = decode(icon) {
                        self.icons.remove(k);
                        self.pixels.insert(k.clone(), px);
                    }
                }
            }
        } else {
            self.tasks = v.clone();
        }
        true
    }
    fn pad(&self) -> (f32, f32, f32) {
        (10.0, 14.0, 16.0)
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        let body = cx.body();
        let lh = line(g, &body);
        let list = self.list();
        let count = list.iter().filter(|r| matches!(r, TaskRow::Window(..))).count();
        let name = self.app().and_then(|a| a.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string();
        let mut widest: f32 = 220.0;
        let mut y = 0.0;
        self.rows = list
            .into_iter()
            .map(|r| {
                let h = match &r {
                    TaskRow::Head => {
                        widest = widest.max(6.0 + 20.0 + 8.0 + g.measure(&name, &body) + 6.0);
                        2.0 + 20.0f32.max(lh) + 8.0
                    }
                    TaskRow::Window(_, t, _) => {
                        widest = widest.max(8.0 + g.measure(t, &cx.secondary(cx.look.font_size)) + 10.0 + 18.0 + 8.0);
                        12.0 + lh
                    }
                    TaskRow::Sep => 13.0,
                    TaskRow::Empty => 12.0 + lh,
                    other => {
                        widest = widest.max(8.0 + 14.0 + 10.0 + g.measure(&Self::label(other, count).1, &body) + 8.0);
                        12.0 + lh
                    }
                };
                let row = (r, y, h);
                y += h + 2.0;
                row
            })
            .collect();
        self.width = widest.min(360.0).ceil();
        // The app's icon in the head.
        let key = super::tasks::menu_target();
        if !self.icons.contains_key(&key) {
            if let Some(i) = self.pixels.get(&key).and_then(|px| AppIcon::new(g, px, Style::Color)) {
                self.icons.insert(key, i);
            }
        }
        (self.width, (y - 2.0).max(0.0))
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let body = cx.body();
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let count = self.rows.iter().filter(|r| matches!(r.0, TaskRow::Window(..))).count();
        for (i, (r, top, h)) in self.rows.iter().enumerate() {
            let (ry, cy) = (y + top, y + top + h / 2.0);
            let hovered = self.hovered.is_some_and(|(k, _)| k == i);
            match r {
                TaskRow::Head => {
                    let key = super::tasks::menu_target();
                    let hy = ry + 2.0 + 10.0;
                    let mut lx = x + 6.0;
                    if let Some(icon) = self.icons.get(&key) {
                        icon.draw(g, lx, hy - 10.0, 20.0, alpha);
                        lx += 28.0;
                    }
                    let name = self.app().and_then(|a| a.get("name")).and_then(|n| n.as_str()).unwrap_or("").to_string();
                    g.text_at(&fit_text(g, &name, &body, self.width - (lx - x) - 6.0), &body, lx, hy, fg);
                }
                TaskRow::Sep => g.fill_rect(rect(x + 4.0, ry + 6.0, self.width - 8.0, 1.0), white(0.1 * alpha)),
                TaskRow::Empty => {
                    g.text_at("No windows", &cx.secondary(cx.look.font_size), x + 8.0, cy, dim);
                }
                TaskRow::Window(_, title, active) => {
                    if hovered {
                        g.fill_round(rect(x, ry, self.width, *h), 8.0, with_alpha(cx.hover_bg(0.1), alpha));
                    }
                    let f = cx.secondary(cx.look.font_size);
                    g.text_at(&fit_text(g, title, &f, self.width - 16.0 - 28.0), &f, x + 8.0, cy, if *active { fg } else { dim });
                    let on_x = self.hovered == Some((i, true));
                    let xf = Font::icons(10.0);
                    let bx = x + self.width - 8.0 - 18.0;
                    if on_x {
                        g.fill_round(rect(bx, cy - 9.0, 18.0, 18.0), 6.0, white(0.12 * alpha));
                    }
                    let w = g.measure("\u{E711}", &xf);
                    g.text_at("\u{E711}", &xf, bx + (18.0 - w) / 2.0, cy, white(if on_x { alpha } else { 0.6 * alpha }));
                }
                other => {
                    if hovered {
                        g.fill_round(rect(x, ry, self.width, *h), 8.0, with_alpha(cx.hover_bg(0.1), alpha));
                    }
                    let (glyph, label) = Self::label(other, count);
                    g.text_at(glyph, &Font::icons(14.0), x + 8.0, cy + 1.0, fg);
                    g.text_at(&label, &body, x + 8.0 + 14.0 + 10.0, cy, fg);
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
        let Some((i, on_x)) = self.hit(at) else { return Reply::None };
        let key = super::tasks::menu_target();
        let save_pins = |pins: Vec<String>| {
            Act::call(move |_| {
                let path = vec![crate::config_edit::Seg::Key("widgets".into()), crate::config_edit::Seg::Key("tasks".into()), crate::config_edit::Seg::Key("pins".into())];
                if let Err(e) = tauri::async_runtime::block_on(crate::config_edit::config_write(path, Some(json!(pins)))) {
                    eprintln!("[kasumi] pins: {e}");
                }
            })
        };
        match self.rows[i].0.clone() {
            TaskRow::Window(hwnd, _, _) if on_x => Reply::Do(Act::call(move |_| {
                let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_close(hwnd as isize));
            })),
            TaskRow::Window(hwnd, _, _) => Reply::CloseAnd(Act::call(move |_| {
                let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_activate(hwnd as isize, false));
            })),
            TaskRow::Launch(_) => Reply::CloseAnd(Act::call(move |_| {
                let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_launch(key));
            })),
            TaskRow::Pin(pinned) => {
                let mut pins = self.pins();
                let mine = self.app().and_then(|a| a.get("pin").and_then(|p| p.as_str()).map(String::from)).unwrap_or_else(|| key.clone());
                if pinned {
                    pins.retain(|k| *k != mine);
                } else {
                    pins.push(key.clone());
                }
                Reply::CloseAnd(save_pins(pins))
            }
            TaskRow::CloseAll(hwnds) => Reply::CloseAnd(Act::call(move |_| {
                for h in hwnds {
                    let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_close(h as isize));
                }
            })),
            _ => Reply::None,
        }
    }
    fn closed(&mut self) {
        self.hovered = None;
    }
}
