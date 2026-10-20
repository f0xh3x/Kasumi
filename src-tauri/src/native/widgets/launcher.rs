//! The app launcher (src/widgets/launcher.ts): a search field (the launcher
//! widget's, the dock's inline one, or the panel's own) and the matching apps.
//! Up / down choose, Enter launches, Esc closes; typing goes to the field. One
//! state for every field, as the page's `S`.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::icon::{decode, AppIcon, Pixels, Style};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply, Widget};
use crate::native::gfx::{rect, white, with_alpha, Font, Gfx};

struct State {
    open: bool,
    query: String,
    selected: usize,
    apps: Vec<(String, String)>,
    results: Vec<usize>,
    max: usize,
    /// When the caret last moved (it shows solid for a moment, then blinks).
    typed: Instant,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State { open: false, query: String::new(), selected: 0, apps: Vec::new(), results: Vec::new(), max: 8, typed: Instant::now() });
}

/// Higher is better; -1: no match (launcher.ts score).
fn score(name: &str, q: &str) -> f64 {
    let n = name.to_lowercase();
    if n.starts_with(q) {
        return 100.0 - n.len() as f64 * 0.01;
    }
    let words: Vec<&str> = n.split(|c: char| c.is_whitespace() || "-_.()".contains(c)).filter(|w| !w.is_empty()).collect();
    if words.iter().any(|w| w.starts_with(q)) {
        return 80.0 - n.len() as f64 * 0.01;
    }
    // Initials: "vsc" -> "Visual Studio Code".
    if words.iter().filter_map(|w| w.chars().next()).collect::<String>().starts_with(q) {
        return 70.0;
    }
    n.find(q).map_or(-1.0, |i| 60.0 - i as f64 * 0.1)
}

fn search(s: &mut State) {
    let q = s.query.trim().to_lowercase();
    let mut found: Vec<(usize, f64)> = if q.is_empty() { Vec::new() } else { s.apps.iter().enumerate().map(|(i, a)| (i, score(&a.1, &q))).filter(|(_, sc)| *sc >= 0.0).collect() };
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    s.results = found.into_iter().take(s.max).map(|(i, _)| i).collect();
    s.selected = 0;
}

/// Whether a search is showing (the fields grow, a dock notch's other widgets step aside).
pub fn searching() -> bool {
    S.with(|s| s.borrow().open)
}

/// Rust opened or closed the launcher: an empty query either way.
pub fn set_open(open: bool) {
    S.with(|s| {
        let mut s = s.borrow_mut();
        s.open = open;
        s.query.clear();
        s.typed = Instant::now();
        search(&mut s);
    });
}

fn set_max(opts: &Opts) {
    if let Some(n) = opts.num("results").filter(|n| *n > 0.0) {
        S.with(|s| s.borrow_mut().max = (n.round() as usize).clamp(3, 20));
    }
}

/// The apps to search (launcher_apps), asked for each time it opens.
pub fn load_apps() -> Act {
    Act::call(|_| {
        if let Ok(list) = tauri::async_runtime::block_on(crate::launcher::launcher_apps()) {
            crate::native::post(crate::native::Msg::Data("launcher:apps".into(), Value::Array(list)));
        }
    })
}

/// What a key did.
pub enum Out {
    None,
    Redraw,
    Launch(String),
    Close,
}

/// A key pressed in the field (virtual-key code). `bottom`: the list grows up from it.
pub fn key(vk: u16, ctrl: bool, bottom: bool) -> Out {
    S.with(|s| {
        let mut s = s.borrow_mut();
        if !s.open {
            return Out::None;
        }
        s.typed = Instant::now();
        let n = s.results.len();
        // On a bottom dock up moves away from the field.
        let (away, toward) = if bottom { (0x26, 0x28) } else { (0x28, 0x26) };
        match vk {
            v if v == away && n > 0 => s.selected = (s.selected + 1) % n,
            v if v == toward && n > 0 => s.selected = (s.selected + n - 1) % n,
            0x0D => return s.results.get(s.selected).map_or(Out::None, |&i| Out::Launch(s.apps[i].0.clone())),
            0x1B => return Out::Close,
            // Backspace: a letter, or (with Ctrl) the word before the caret.
            0x08 => {
                if ctrl {
                    let t = s.query.trim_end().to_string();
                    let cut = t.rfind(char::is_whitespace).map_or(0, |i| i + 1);
                    s.query.truncate(cut);
                } else {
                    s.query.pop();
                }
                search(&mut s);
            }
            _ => return Out::None,
        }
        Out::Redraw
    })
}

/// Typed text (WM_CHAR, control characters left out).
pub fn type_char(c: char) -> bool {
    if c.is_control() {
        return false;
    }
    S.with(|s| {
        let mut s = s.borrow_mut();
        if !s.open {
            return false;
        }
        s.query.push(c);
        s.typed = Instant::now();
        search(&mut s);
        true
    })
}

/// Pasted text (Ctrl+V).
pub fn paste(text: &str) {
    S.with(|s| {
        let mut s = s.borrow_mut();
        s.query.push_str(&text.replace(['\r', '\n'], " "));
        search(&mut s);
    });
}

/// The caret shows: solid for half a second after typing, then on and off every 530 ms.
fn caret_on() -> bool {
    S.with(|s| {
        let t = s.borrow().typed.elapsed().as_millis();
        t < 500 || (t / 530) % 2 == 0
    })
}

/// A search field: the query (or "Search apps" dimmed) and the caret.
fn draw_field(g: &Gfx, cx: &Ctx, x: f32, cy: f32, w: f32, alpha: f32) {
    let f = cx.body();
    let (query, open) = S.with(|s| (s.borrow().query.clone(), s.borrow().open));
    let lh = g.text("Ag", &f).map_or(16.0, |t| t.height);
    g.clip(rect(x, cy - lh, w, lh * 2.0));
    let shown = if query.is_empty() { 0.0 } else { g.measure(&query, &f) };
    // Long queries scroll so the caret stays in view.
    let scroll = (shown + 2.0 - w).max(0.0);
    if query.is_empty() {
        g.text_at("Search apps", &f, x, cy, with_alpha(cx.look.dimmed(), alpha));
    } else {
        g.text_at(&query, &f, x - scroll, cy, with_alpha(cx.look.fg, alpha));
    }
    if open && caret_on() {
        g.fill_rect(rect(g.snap(x + shown - scroll), cy - lh / 2.0 + 1.0, 1.0, lh - 2.0), with_alpha(cx.look.fg, alpha));
    }
    g.unclip();
}

/// The search icon that becomes a field while the launcher is open (190 px).
/// `inline`: the dock's own field (no launcher widget placed), only there then.
pub struct LauncherWidget {
    inline: bool,
}

impl LauncherWidget {
    pub fn new(opts: &Opts, inline: bool) -> LauncherWidget {
        set_max(opts);
        LauncherWidget { inline }
    }
}

impl Widget for LauncherWidget {
    fn keys(&self) -> &'static [&'static str] {
        &["launcher:apps"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let apps = v.as_array().map(|a| a.iter().filter_map(|x| Some((x.get("id")?.as_str()?.to_string(), x.get("name")?.as_str()?.to_string()))).collect()).unwrap_or_default();
        S.with(|s| {
            let mut s = s.borrow_mut();
            s.apps = apps;
            search(&mut s);
        });
        true
    }
    fn wake(&self) -> Option<Instant> {
        searching().then(|| Instant::now() + Duration::from_millis(265))
    }
    fn tick(&mut self, _now: Instant) -> bool {
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        let on = searching();
        if self.inline && !on {
            return 0.0;
        }
        cx.gfx.measure("\u{E721}", &Font::icons(14.0)) + if on { 8.0 + 190.0 } else { 0.0 }
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let f = Font::icons(14.0);
        let iw = cx.gfx.text_at("\u{E721}", &f, x, cy + 1.0, with_alpha(cx.look.fg, alpha));
        if searching() {
            draw_field(cx.gfx, cx, x + iw + 8.0, cy, 190.0, alpha);
        }
    }
    fn click(&mut self, button: Button, _at: (f32, f32)) -> Reply {
        if button != Button::Left || searching() {
            return Reply::None;
        }
        Reply::Do(Act::Launcher { open: true, refocus: false })
    }
}

/// Power buttons the panel can show ([widgets.launcher] power): id -> (label, glyph, needs a second click).
const POWER: [(&str, &str, &str, bool); 6] = [
    ("lock", "Lock", "\u{E72E}", false),
    ("sleep", "Sleep", "\u{E708}", false),
    ("hibernate", "Hibernate", "\u{E823}", false),
    ("sign_out", "Sign out", "\u{E8F8}", true),
    ("restart", "Restart", "\u{E777}", true),
    ("shutdown", "Shut down", "\u{E7E8}", true),
];

#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Row(usize),
    Power(usize),
}

/// The matching apps (an icon and name each, the chosen one highlighted), with
/// a field of its own when nothing else has one, and the power buttons.
pub struct LauncherPanel {
    own_field: bool,
    power: Vec<usize>,
    icons: HashMap<String, AppIcon>,
    pixels: HashMap<String, Pixels>,
    asked: Vec<String>,
    pending: Option<Act>,
    width: f32,
    /// Rows as last laid out (content px): what, top, height.
    rows: Vec<(Hit, f32, f32)>,
    hovered: Option<Hit>,
    armed: Option<(usize, Instant)>,
}

const ROW_H: f32 = 34.0;

impl LauncherPanel {
    pub fn new(opts: &Opts, own_field: bool) -> LauncherPanel {
        set_max(opts);
        let chosen: Vec<String> = opts.table.get("power").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let power = (0..POWER.len()).filter(|&i| chosen.iter().any(|c| c == POWER[i].0)).collect();
        let width = opts.num("panel_width").map_or(340.0, |w| w.clamp(240.0, 700.0) as f32);
        LauncherPanel { own_field, power, icons: HashMap::new(), pixels: HashMap::new(), asked: Vec::new(), pending: None, width, rows: Vec::new(), hovered: None, armed: None }
    }
    fn hit(&self, p: (f32, f32)) -> Option<Hit> {
        self.rows.iter().find(|(h, top, ht)| {
            let in_row = p.1 >= *top && p.1 < top + ht;
            match h {
                Hit::Row(_) => in_row,
                Hit::Power(k) => in_row && {
                    let x = self.width - 4.0 - (self.power.len() - k) as f32 * 32.0;
                    p.0 >= x && p.0 < x + 30.0
                },
            }
        }).map(|r| r.0)
    }
}

/// A piece of the launcher panel.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Field,
    Line,
    Row(usize),
    Hint,
    Power,
}

impl LauncherPanel {
    /// Its parts top to bottom (what, top, height) and the total height. A bottom dock
    /// mirrors them (the page's column-reverse): the field by the dock, the best match
    /// next to it, the power buttons furthest away.
    fn parts(&self, results: usize, no_match: bool, lh: f32, bottom: bool) -> (Vec<(Part, f32, f32)>, f32) {
        let mut v = Vec::new();
        let mut y = 0.0;
        if self.own_field {
            v.push((Part::Field, 4.0, 26.0));
            v.push((Part::Line, 4.0 + 26.0 + 8.0, 1.0));
            y += 4.0 + 26.0 + 8.0 + 1.0 + 4.0;
        }
        for i in 0..results {
            v.push((Part::Row(i), y, ROW_H));
            y += ROW_H + 2.0;
        }
        if no_match {
            v.push((Part::Hint, y, 12.0 + lh));
            y += 12.0 + lh;
        }
        if !self.power.is_empty() {
            v.push((Part::Line, y + 4.0, 1.0));
            v.push((Part::Power, y + 4.0 + 1.0 + 6.0, 28.0));
            y += 4.0 + 1.0 + 6.0 + 28.0;
        }
        if bottom {
            for p in &mut v {
                p.1 = y - p.1 - p.2;
            }
        }
        (v, y)
    }
}

impl Panel for LauncherPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["appIcons"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        if let Some(map) = v.as_object() {
            for (id, icon) in map {
                if !self.icons.contains_key(id) {
                    if let Some(px) = decode(icon) {
                        self.pixels.insert(id.clone(), px);
                    }
                }
            }
        }
        true
    }
    fn act(&mut self) -> Option<Act> {
        self.pending.take()
    }
    fn wake(&self) -> Option<Instant> {
        // The own field's caret blinks; an armed power button disarms after 3 s.
        let caret = (self.own_field && searching()).then(|| Instant::now() + Duration::from_millis(265));
        [caret, self.armed.map(|a| a.1)].into_iter().flatten().min()
    }
    fn tick(&mut self, now: Instant) -> bool {
        if self.armed.is_some_and(|a| now >= a.1) {
            self.armed = None;
        }
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        for (id, px) in self.pixels.drain() {
            if let Some(i) = AppIcon::new(g, &px, Style::Color) {
                self.icons.insert(id, i);
            }
        }
        let (results, query, ids) = S.with(|s| {
            let s = s.borrow();
            (s.results.len(), s.query.clone(), s.results.iter().map(|&i| s.apps[i].0.clone()).collect::<Vec<_>>())
        });
        // Icons for results not asked for yet.
        let wanted: Vec<String> = ids.into_iter().filter(|id| !self.asked.contains(id)).collect();
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
        let lh = g.text("Ag", &cx.body()).map_or(16.0, |t| t.height);
        let (parts, h) = self.parts(results, !query.trim().is_empty() && results == 0, lh, cx.look.bottom);
        let mut rows = Vec::new();
        for &(part, top, ht) in &parts {
            match part {
                Part::Row(i) => rows.push((Hit::Row(i), top, ht)),
                Part::Power => rows.extend((0..self.power.len()).map(|k| (Hit::Power(k), top, ht))),
                _ => {}
            }
        }
        self.rows = rows;
        (self.width, h.max(1.0))
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let body = cx.body();
        let lh = g.text("Ag", &body).map_or(16.0, |t| t.height);
        let (results, selected, query) = S.with(|s| {
            let s = s.borrow();
            (s.results.iter().map(|&i| s.apps[i].clone()).collect::<Vec<_>>(), s.selected, s.query.clone())
        });
        let (parts, _) = self.parts(results.len(), !query.trim().is_empty() && results.is_empty(), lh, cx.look.bottom);
        for &(part, top, _) in &parts {
            let top = y + top;
            match part {
                Part::Field => {
                    let cy = top + 13.0;
                    let iw = g.text_at("\u{E721}", &Font::icons(14.0), x + 8.0, cy + 1.0, fg);
                    draw_field(g, cx, x + 8.0 + iw + 10.0, cy, self.width - 16.0 - iw - 10.0, alpha);
                }
                Part::Line => g.fill_rect(rect(x, top, self.width, 1.0), white(0.08 * alpha)),
                Part::Row(i) => {
                    let (id, name) = &results[i];
                    if i == selected {
                        g.fill_round(rect(x, top, self.width, ROW_H), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
                    }
                    if let Some(icon) = self.icons.get(id) {
                        icon.draw(g, x + 8.0, top + 6.0, 22.0, alpha);
                    }
                    g.text_at(&fit_text(g, name, &body, self.width - 48.0), &body, x + 40.0, top + ROW_H / 2.0, fg);
                }
                Part::Hint => {
                    g.text_at("No apps found", &cx.secondary(cx.look.font_size), x + 8.0, top + 6.0 + lh / 2.0, dim);
                }
                Part::Power => {
                    let cy = top + 14.0;
                    let n = self.power.len();
                    if let Some((k, _)) = self.armed {
                        let label = POWER[self.power[k]].1.to_lowercase();
                        g.text_at(&format!("Click again to {label}"), &cx.secondary(12.0), x + 6.0, cy, dim);
                    }
                    for (k, &p) in self.power.iter().enumerate() {
                        let bx = x + self.width - 4.0 - (n - k) as f32 * 32.0;
                        let on = self.hovered == Some(Hit::Power(k));
                        if on {
                            g.fill_round(rect(bx, cy - 14.0, 30.0, 28.0), 8.0, white(0.1 * alpha));
                        }
                        let f = Font::icons(14.0);
                        let w = g.measure(POWER[p].2, &f);
                        g.text_at(POWER[p].2, &f, bx + (30.0 - w) / 2.0, cy, if on { fg } else { dim });
                    }
                }
            }
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.hit(p));
        let mut changed = h != self.hovered;
        self.hovered = h;
        // Over a row: it becomes the chosen one.
        if let Some(Hit::Row(i)) = h {
            S.with(|s| {
                let mut s = s.borrow_mut();
                if s.selected != i {
                    s.selected = i;
                    changed = true;
                }
            });
        }
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        match self.hit(at) {
            Some(Hit::Row(i)) => {
                let id = S.with(|s| {
                    let s = s.borrow();
                    s.results.get(i).map(|&k| s.apps[k].0.clone())
                });
                match id {
                    Some(id) => Reply::Do(launch(id)),
                    None => Reply::None,
                }
            }
            Some(Hit::Power(k)) => {
                let (action, _, _, confirm) = POWER[self.power[k]];
                if confirm && !self.armed.is_some_and(|a| a.0 == k) {
                    self.armed = Some((k, Instant::now() + Duration::from_secs(3)));
                    return Reply::Redraw;
                }
                self.armed = None;
                let act = action.to_string();
                Reply::Do(
                    Act::call(move |app| {
                        let _ = tauri::async_runtime::block_on(crate::actions::run_action(app, act, None));
                    })
                    .then(Act::Launcher { open: false, refocus: false }),
                )
            }
            None => Reply::None,
        }
    }
    fn closed(&mut self) {
        self.hovered = None;
        self.armed = None;
    }
}

/// Starts an app and closes the launcher.
pub fn launch(id: String) -> Act {
    Act::call(move |_| {
        let _ = tauri::async_runtime::block_on(crate::launcher::launcher_launch(id));
    })
    .then(Act::Launcher { open: false, refocus: false })
}

/// The text on the clipboard (Ctrl+V into the field).
pub fn clipboard_text() -> Option<String> {
    use windows::Win32::Foundation::{HANDLE, HGLOBAL};
    use windows::Win32::System::DataExchange::{CloseClipboard, GetClipboardData, OpenClipboard};
    use windows::Win32::System::Memory::{GlobalLock, GlobalUnlock};
    const CF_UNICODETEXT: u32 = 13;
    unsafe {
        OpenClipboard(None).ok()?;
        let text = (|| {
            let h: HANDLE = GetClipboardData(CF_UNICODETEXT).ok()?;
            let p = GlobalLock(HGLOBAL(h.0)) as *const u16;
            if p.is_null() {
                return None;
            }
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            let _ = GlobalUnlock(HGLOBAL(h.0));
            Some(s)
        })();
        let _ = CloseClipboard();
        text
    }
}
