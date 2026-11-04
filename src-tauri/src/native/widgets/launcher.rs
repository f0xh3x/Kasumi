//! The app launcher (src/widgets/launcher.ts): a search field (the launcher
//! widget's, the dock's inline one, or the panel's own) and the matching apps
//! (and open windows, [widgets.launcher] windows). Up / down or the wheel choose,
//! Enter launches, Esc closes; typing goes to the field. One state for every
//! field, as the page's `S`. Its look ([widgets.launcher] style): a list, a box in
//! the middle of the screen (centre.rs draws it), a grid of pins, or a prompt.

use std::cell::RefCell;
use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::icon::{decode, AppIcon, Pixels, Style};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply, Widget};
use crate::native::gfx::{rect, rgba, white, with_alpha, Family, Font, Gfx};
use windows::Win32::Graphics::Direct2D::ID2D1Bitmap;

struct State {
    open: bool,
    query: String,
    selected: usize,
    /// Id, name, search words (Settings pages'), lower case.
    apps: Vec<(String, String, String)>,
    results: Vec<usize>,
    max: usize,
    /// The sum the query is, worked out ([widgets.launcher] calculator).
    calc: Option<String>,
    calculator: bool,
    /// Windows Settings pages among the results ([widgets.launcher] settings).
    settings: bool,
    /// Start's pinned apps (id, name), listed while nothing is typed ([widgets.launcher]
    /// pinned); the user's button ([widgets.launcher] user: its name and picture are
    /// asked for as the launcher opens).
    pins: Vec<(String, String)>,
    pinned: bool,
    user: bool,
    /// The clipboard's history (id, text; images "Image"), searched after a comma
    /// ([widgets.launcher] clipboard).
    clip: Vec<(u64, String)>,
    clipboard: bool,
    /// The open windows (handle, title, app key, app name; most recent first), found by
    /// their title or app ([widgets.launcher] windows): Enter switches to one.
    windows: Vec<(isize, String, String, String)>,
    window_search: bool,
    /// The look ([widgets.launcher] style): "" (a list), "centre", "grid", "prompt".
    style: String,
    /// Grid: how many pins a row holds, as last laid out (up / down move by a row).
    grid_cols: usize,
    /// When the caret last moved (it shows solid for a moment, then blinks).
    typed: Instant,
    /// The surface it's open on (every surface draws on this thread: the others keep
    /// their search icon).
    owner: String,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State { open: false, query: String::new(), selected: 0, apps: Vec::new(), results: Vec::new(), max: 8, calc: None, calculator: false, settings: false, pins: Vec::new(), pinned: false, user: false, clip: Vec::new(), clipboard: false, windows: Vec::new(), window_search: false, style: String::new(), grid_cols: 0, typed: Instant::now(), owner: String::new() });
    /// Something shown changed since the box in the middle of the screen was drawn.
    static CHANGED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// What it shows changed (the centred box draws again).
pub fn touch() {
    CHANGED.with(|c| c.set(true));
}

/// Whether what it shows changed since this was last asked.
pub fn take_changed() -> bool {
    CHANGED.with(|c| c.replace(false))
}

/// It opens in the middle of the screen ([widgets.launcher] style = "centre").
pub fn centred() -> bool {
    S.with(|s| s.borrow().style == "centre")
}

/// The surface it's open on, if it is.
pub fn open_on() -> Option<String> {
    S.with(|s| {
        let s = s.borrow();
        s.open.then(|| s.owner.clone())
    })
}


/// A line of text's height in a font.
fn line_h(g: &Gfx, f: &Font) -> f32 {
    g.text("Ag", f).map_or(f.size * 1.21, |t| t.height)
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

/// A program on the PATH (launcher.rs `run:` ids): found only by its whole name
/// (`notepad`, `notepad.exe`), as Start does, under any app that matches as well.
fn command_score(name: &str, q: &str) -> f64 {
    if q.strip_suffix(".exe").unwrap_or(q) == name {
        90.0
    } else {
        -1.0
    }
}

/// The calculator's row in the results (its index among them), and where Start's
/// pins', the clipboard's and the open windows' indices begin.
const CALC: usize = usize::MAX;
const PIN: usize = 1 << 40;
const CLIP: usize = 1 << 41;
const WIN: usize = 1 << 42;

/// A sum typed into the field ([widgets.launcher] calculator): `24*7`, `(3+4)^2`,
/// `1,5 / 3`, `2 x 8`, `sqrt(2)`, `pi*2`. None unless it's a whole sum with at least one
/// operation (a lone number isn't one).
fn calc(text: &str) -> Option<String> {
    struct P<'a> {
        c: &'a [char],
        i: usize,
        ops: usize,
    }
    impl P<'_> {
        fn skip(&mut self) {
            while self.c.get(self.i).is_some_and(|c| c.is_whitespace()) {
                self.i += 1;
            }
        }
        fn eat(&mut self, set: &str) -> Option<char> {
            self.skip();
            let c = *self.c.get(self.i)?;
            set.contains(c).then(|| {
                self.i += 1;
                c
            })
        }
        fn word(&mut self, w: &str) -> bool {
            self.skip();
            let n = w.chars().count();
            let here: String = self.c.iter().skip(self.i).take(n).collect::<String>().to_lowercase();
            if here == w {
                self.i += n;
                true
            } else {
                false
            }
        }
        fn sum(&mut self) -> Option<f64> {
            let mut v = self.product()?;
            while let Some(op) = self.eat("+-−") {
                self.ops += 1;
                let r = self.product()?;
                v = if op == '+' { v + r } else { v - r };
            }
            Some(v)
        }
        fn product(&mut self) -> Option<f64> {
            let mut v = self.power()?;
            while let Some(op) = self.eat("*/x×÷") {
                self.ops += 1;
                let r = self.power()?;
                v = if matches!(op, '/' | '÷') { v / r } else { v * r };
            }
            Some(v)
        }
        fn power(&mut self) -> Option<f64> {
            let v = self.unary()?;
            if self.eat("^").is_some() {
                self.ops += 1;
                return Some(v.powf(self.power()?));
            }
            Some(v)
        }
        fn unary(&mut self) -> Option<f64> {
            if let Some(op) = self.eat("-−+") {
                let v = self.unary()?;
                return Some(if op == '+' { v } else { -v });
            }
            self.atom()
        }
        fn atom(&mut self) -> Option<f64> {
            if self.eat("(").is_some() {
                let v = self.sum()?;
                self.eat(")")?;
                return Some(v);
            }
            if self.word("sqrt") || self.word("√") {
                self.ops += 1;
                return Some(self.atom()?.sqrt());
            }
            if self.word("pi") || self.word("π") {
                return Some(std::f64::consts::PI);
            }
            self.skip();
            let start = self.i;
            while self.c.get(self.i).is_some_and(|c| c.is_ascii_digit() || *c == '.' || *c == ',') {
                self.i += 1;
            }
            let n: String = self.c[start..self.i].iter().map(|&c| if c == ',' { '.' } else { c }).collect();
            n.parse::<f64>().ok()
        }
    }
    let text = text.trim().trim_start_matches('=');
    let chars: Vec<char> = text.chars().collect();
    let mut p = P { c: &chars, i: 0, ops: 0 };
    let v = p.sum()?;
    p.skip();
    if p.i != chars.len() || p.ops == 0 || !v.is_finite() {
        return None;
    }
    // Ten decimals at most, without trailing zeros.
    let v = if v == 0.0 { 0.0 } else { v };
    if v.abs() >= 1e15 {
        return Some(format!("{v:e}"));
    }
    let s = format!("{v:.10}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    Some(if s == "-0" { "0".to_string() } else { s.to_string() })
}

/// A Settings page's search words matching (two letters or more: a word starting with
/// the query, or the whole query inside them).
fn keys_score(keys: &str, q: &str) -> f64 {
    if q.chars().count() < 2 || keys.is_empty() {
        return -1.0;
    }
    if keys.split_whitespace().any(|w| w.starts_with(q)) {
        50.0
    } else if q.contains(' ') && keys.contains(q) {
        45.0
    } else {
        -1.0
    }
}

/// A result's id and name: an app (its index in the list), the calculator's row, an
/// open window, a clipboard item or a pin of Start's.
fn entry(s: &State, i: usize) -> (String, String) {
    if i == CALC {
        let v = s.calc.clone().unwrap_or_default();
        return (format!("calc:{v}"), format!("= {v}"));
    }
    if i >= WIN {
        let (h, title, _, _) = &s.windows[i - WIN];
        return (format!("win:{h}"), title.clone());
    }
    if i >= CLIP {
        let (id, text) = &s.clip[i - CLIP];
        return (format!("clip:{id}"), text.clone());
    }
    if i >= PIN {
        return s.pins[i - PIN].clone();
    }
    (s.apps[i].0.clone(), s.apps[i].1.clone())
}

fn search(s: &mut State) {
    let q = s.query.trim().to_lowercase();
    let settings = s.settings;
    let rate = |a: &(String, String, String)| {
        if a.0.starts_with("run:") {
            command_score(&a.1, &q)
        } else if a.0.starts_with("ms-settings:") {
            if settings { score(&a.1, &q).max(keys_score(&a.2, &q)) } else { -1.0 }
        } else {
            score(&a.1, &q)
        }
    };
    let mut found: Vec<(usize, f64)> = if q.is_empty() { Vec::new() } else { s.apps.iter().enumerate().map(|(i, a)| (i, rate(a))).filter(|(_, sc)| *sc >= 0.0).collect() };
    // Open windows: by their title, or by their app's name a little below the app itself
    // (typing an app's name still starts it first; its windows follow).
    if s.window_search && !q.is_empty() {
        let wins = s.windows.iter().enumerate().map(|(k, w)| (WIN + k, score(&w.1, &q).max(score(&w.3, &q) - 5.0)));
        found.extend(wins.filter(|(_, sc)| *sc >= 0.0));
    }
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    // A sum: its answer first.
    s.calc = if s.calculator { calc(&s.query) } else { None };
    let first = s.calc.as_ref().map(|_| CALC);
    let room = s.max - first.is_some() as usize;
    s.results = first.into_iter().chain(found.into_iter().take(room).map(|(i, _)| i)).collect();
    // Nothing typed yet: Start's pins.
    if q.is_empty() && s.pinned {
        let room = if s.style == "grid" { 16 } else { s.max };
        s.results = (0..s.pins.len().min(room)).map(|k| PIN + k).collect();
    }
    // A comma first: the clipboard's history (newest first), what follows it searched.
    if let Some(rest) = q.strip_prefix(',').filter(|_| s.clipboard) {
        let rest = rest.trim();
        s.calc = None;
        s.results = (0..s.clip.len()).filter(|&k| rest.is_empty() || s.clip[k].1.to_lowercase().contains(rest)).take(s.max).map(|k| CLIP + k).collect();
    }
    s.selected = 0;
}

/// Whether a search is showing on `surface` (its field grows, a dock notch's other
/// widgets step aside).
pub fn searching_on(surface: &str) -> bool {
    S.with(|s| {
        let s = s.borrow();
        s.open && s.owner == surface
    })
}

/// Whether `surface` shows the search field (not while it's in the middle of the screen).
pub fn field_on(surface: &str) -> bool {
    searching_on(surface) && !centred()
}

/// The rest of the chosen result's name after what's typed (the prompt's hint), when
/// the name starts with it.
fn ghost(s: &State) -> String {
    let q = s.query.trim_start();
    let Some(&i) = s.results.get(s.selected) else { return String::new() };
    if q.is_empty() || i == CALC || i >= CLIP {
        return String::new();
    }
    let name = entry(s, i).1;
    if !name.to_lowercase().starts_with(&q.to_lowercase()) {
        return String::new();
    }
    name.chars().skip(q.chars().count()).collect()
}

/// Rust opened or closed the launcher on `surface`: an empty query either way.
pub fn set_open(open: bool, surface: &str) {
    S.with(|s| {
        let mut s = s.borrow_mut();
        if !open && s.owner != surface {
            return; // another surface's launcher is the open one
        }
        s.open = open;
        s.owner = surface.to_string();
        s.query.clear();
        s.typed = Instant::now();
        search(&mut s);
    });
    touch();
}

/// [widgets.launcher]: how many results, the calculator, Settings pages.
fn set_opts(opts: &Opts) {
    let on = |k: &str| opts.table.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    S.with(|s| {
        let mut s = s.borrow_mut();
        if let Some(n) = opts.num("results").filter(|n| *n > 0.0) {
            s.max = (n.round() as usize).clamp(3, 20);
        }
        s.calculator = on("calculator");
        s.settings = on("settings");
        s.pinned = on("pinned");
        s.user = on("user");
        s.clipboard = on("clipboard");
        s.window_search = on("windows");
        s.style = opts.str("style").unwrap_or("").to_string();
        // The grid is made of Start's pins.
        s.pinned = s.pinned || s.style == "grid";
    });
}

/// The apps to search (launcher_apps), asked for each time it opens; with their options
/// on, the user (name, picture) and Start's pins too (those take half a second).
pub fn load_apps() -> Act {
    let (pinned, user) = S.with(|s| {
        let s = s.borrow();
        (s.pinned, s.user)
    });
    Act::call(move |_| {
        let post = |key: &str, v: Value| crate::native::post(crate::native::Msg::Data(key.into(), v));
        if let Ok(list) = tauri::async_runtime::block_on(crate::launcher::launcher_apps()) {
            post("launcher:apps", Value::Array(list));
        }
        if user {
            post("launcher:user", tauri::async_runtime::block_on(crate::launcher::launcher_user()));
        }
        if pinned {
            let pins = crate::launcher::start_pins().into_iter().map(|(id, name)| serde_json::json!({ "id": id, "name": name })).collect();
            post("launcher:pinned", Value::Array(pins));
        }
    })
}

/// The app list, Start's pins, the clipboard or the open windows arrived (whichever of
/// the launcher's widgets and panels gets it: they share one state). False: nothing
/// shown changed.
fn take_list(key: &str, v: &Value) -> bool {
    let list = |keys: bool| -> Vec<(String, String, String)> {
        v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| {
                        let k = if keys { x.get("keys").and_then(|k| k.as_str()).unwrap_or("").to_lowercase() } else { String::new() };
                        Some((x.get("id")?.as_str()?.to_string(), x.get("name")?.as_str()?.to_string(), k))
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    S.with(|s| {
        let mut s = s.borrow_mut();
        match key {
            "launcher:apps" => s.apps = list(true),
            "launcher:pinned" => s.pins = list(false).into_iter().map(|(id, name, _)| (id, name)).collect(),
            "clipboard" => {
                s.clip = v
                    .as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| {
                                let id = x.get("id")?.as_u64()?;
                                let text = match x.get("kind").and_then(|k| k.as_str()) {
                                    Some("image") => "Image".to_string(),
                                    _ => x.get("text").and_then(|t| t.as_str()).unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" "),
                                };
                                Some((id, text))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
            }
            "tasks" => {
                let names: HashMap<&str, &str> = v
                    .get("apps")
                    .and_then(|a| a.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|a| Some((a.get("key")?.as_str()?, a.get("name")?.as_str()?)))
                    .collect();
                let mut wins: Vec<&Value> = v.get("windows").and_then(|w| w.as_array()).into_iter().flatten().collect();
                wins.sort_by_key(|w| w.get("z").and_then(|z| z.as_i64()).unwrap_or(i64::MAX));
                let text = |w: &Value, k: &str| w.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                s.windows = wins
                    .iter()
                    .filter_map(|w| {
                        let app = text(w, "app");
                        let name = names.get(app.as_str()).map_or_else(String::new, |n| n.to_string());
                        Some((w.get("hwnd")?.as_i64()? as isize, text(w, "title"), app, name))
                    })
                    .collect();
                if !s.open || !s.window_search {
                    return false;
                }
                // Windows change (titles do all the time): the choice stays where it is.
                let sel = s.selected;
                search(&mut s);
                s.selected = sel.min(s.results.len().saturating_sub(1));
                touch();
                return true;
            }
            _ => return false,
        }
        search(&mut s);
        touch();
        true
    })
}

/// The choice moves one result away from the field (or toward it), stopping at the
/// ends. False: it couldn't move.
fn step(away: bool) -> bool {
    S.with(|s| {
        let mut s = s.borrow_mut();
        let n = s.results.len();
        if !s.open || n == 0 {
            return false;
        }
        let to = if away { (s.selected + 1).min(n - 1) } else { s.selected.saturating_sub(1) };
        let moved = to != s.selected;
        s.selected = to;
        if moved {
            touch();
        }
        moved
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
        touch();
        let n = s.results.len();
        // On a bottom dock up moves away from the field (the centred box's list is below it).
        let bottom = bottom && s.style != "centre";
        let (away, toward) = if bottom { (0x26, 0x28) } else { (0x28, 0x26) };
        // The grid (pins, nothing typed): the arrows move in it; the prompt's chips: left
        // and right too.
        let grid = s.style == "grid" && s.query.trim().is_empty() && s.grid_cols > 0;
        let across = grid || s.style == "prompt";
        let cols = s.grid_cols.max(1);
        match vk {
            0x25 if across && n > 0 => s.selected = (s.selected + n - 1) % n,
            0x27 if across && n > 0 => s.selected = (s.selected + 1) % n,
            0x26 if grid && n > 0 => s.selected = s.selected.saturating_sub(cols),
            0x28 if grid && n > 0 => s.selected = (s.selected + cols).min(n - 1),
            // Tab: the prompt takes the rest of the chosen name.
            0x09 if s.style == "prompt" => {
                let rest = ghost(&s);
                if rest.is_empty() {
                    return Out::None;
                }
                let q = s.query.trim_start().to_string();
                s.query = q + &rest;
                search(&mut s);
            }
            v if v == away && n > 0 => s.selected = (s.selected + 1) % n,
            v if v == toward && n > 0 => s.selected = (s.selected + n - 1) % n,
            0x0D => return s.results.get(s.selected).map_or(Out::None, |&i| Out::Launch(entry(&s, i).0)),
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
        touch();
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
    touch();
}

/// The caret shows: solid for half a second after typing, then on and off every 530 ms.
fn caret_on() -> bool {
    S.with(|s| {
        let t = s.borrow().typed.elapsed().as_millis();
        t < 500 || (t / 530) % 2 == 0
    })
}

/// A search field: the query (or "Search apps" dimmed) and the caret; the prompt's in a
/// fixed-width face after "›", the rest of the chosen name faint after the caret.
fn draw_field(g: &Gfx, cx: &Ctx, x: f32, cy: f32, w: f32, alpha: f32) {
    let (query, open, prompt, rest) = S.with(|s| {
        let s = s.borrow();
        (s.query.clone(), s.open, s.style == "prompt", ghost(&s))
    });
    if prompt {
        let f = Font::new(Family::from_config("Cascadia Mono"), cx.look.font_size);
        let lh = g.text("Ag", &f).map_or(16.0, |t| t.height);
        g.clip(rect(x, cy - lh, w, lh * 2.0));
        let px = x + g.text_at("› ", &f, x, cy, with_alpha(cx.look.dimmed(), alpha));
        let shown = if query.is_empty() { 0.0 } else { g.measure(&query, &f) };
        let scroll = (shown + 2.0 - (x + w - px)).max(0.0);
        if query.is_empty() {
            g.text_at("type an app", &f, px, cy, with_alpha(cx.look.dimmed(), alpha * 0.6));
        } else {
            g.text_at(&query, &f, px - scroll, cy, with_alpha(cx.look.fg, alpha));
            g.text_at(&rest, &f, px - scroll + shown + 1.0, cy, with_alpha(cx.look.dimmed(), alpha));
        }
        if open && caret_on() {
            g.fill_rect(rect(g.snap(px + shown - scroll), cy - lh / 2.0 + 1.0, 1.0, lh - 2.0), with_alpha(cx.look.fg, alpha));
        }
        g.unclip();
        return;
    }
    let f = cx.body();
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

/// The search icon that becomes a field while the launcher is open, as wide as the panel
/// under it (its text starts where the results' icons do).
/// `inline`: the dock's own field (no launcher widget placed), only there then.
pub struct LauncherWidget {
    inline: bool,
    surface: String,
    /// The panel's width ([widgets.launcher] panel_width).
    panel_w: f32,
    /// The caret's next blink.
    beat: Instant,
}

impl LauncherWidget {
    pub fn new(opts: &Opts, inline: bool) -> LauncherWidget {
        set_opts(opts);
        let panel_w = opts.num("panel_width").map_or(340.0, |w| w.clamp(240.0, 700.0) as f32);
        LauncherWidget { inline, surface: opts.surface.clone(), panel_w, beat: Instant::now() }
    }
    /// Its field shows (searching here, not in the middle of the screen).
    fn searching(&self) -> bool {
        field_on(&self.surface)
    }
    /// The field's width after the icon (`iw` wide): the panel's, less the rows' inset.
    fn field_w(&self, iw: f32) -> f32 {
        self.panel_w - 2.0 * 12.0 - iw - 8.0
    }
}

impl Widget for LauncherWidget {
    fn keys(&self) -> &'static [&'static str] {
        &["launcher:apps", "launcher:pinned", "clipboard", "tasks"]
    }
    fn tooltip(&self) -> Option<String> {
        (!self.searching()).then(|| "Search apps".into())
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        take_list(key, v)
    }
    fn wake(&self) -> Option<Instant> {
        self.searching().then_some(self.beat)
    }
    fn tick(&mut self, now: Instant) -> bool {
        self.beat = now + Duration::from_millis(265);
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        let on = self.searching();
        if self.inline && !on {
            return 0.0;
        }
        let iw = cx.gfx.measure("\u{E721}", &Font::icons(14.0));
        iw + if on { 8.0 + self.field_w(iw) } else { 0.0 }
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let f = Font::icons(14.0);
        let iw = cx.gfx.text_at("\u{E721}", &f, x, cy + 1.0, with_alpha(cx.look.fg, alpha));
        if self.searching() {
            draw_field(cx.gfx, cx, x + iw + 8.0, cy, self.field_w(iw), alpha);
        }
    }
    fn click(&mut self, button: Button, _at: (f32, f32)) -> Reply {
        if button != Button::Left || self.searching() {
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
    User,
    Button(usize),
    Power(usize),
}

/// A button of the user's ([widgets.launcher] buttons, as the Buttons widget's items).
struct Custom {
    icon: String,
    label: String,
    action: String,
    target: Option<String>,
    confirm: bool,
}

/// The matching apps (an icon and name each, the chosen one highlighted), with a field of
/// its own when nothing else has one; the user's button, buttons of their own and the
/// power buttons in a row at the end, like a start menu's, or in a column on the left
/// ([widgets.launcher] buttons_position = "left").
pub struct LauncherPanel {
    own_field: bool,
    surface: String,
    power: Vec<usize>,
    buttons: Vec<Custom>,
    show_user: bool,
    /// The user's name and account picture (JPEG) as read, the picture once drawable.
    user: Option<(String, Option<Vec<u8>>)>,
    user_pic: Option<ID2D1Bitmap>,
    side: bool,
    /// The row's buttons: "left", "center" or "right" ([widgets.launcher] power_align).
    align: String,
    icons: HashMap<String, AppIcon>,
    pixels: HashMap<String, Pixels>,
    asked: Vec<String>,
    pending: Option<Act>,
    width: f32,
    /// What can be clicked as last laid out (content px): what, x, y, w, h.
    boxes: Vec<(Hit, [f32; 4])>,
    hovered: Option<Hit>,
    armed: Option<(Hit, Instant)>,
    /// Its own field's caret's next blink.
    beat: Instant,
    /// On a bottom surface (the list grows up from the field); the wheel's part notch.
    bottom: bool,
    wheel_left: f32,
}

const ROW_H: f32 = 34.0;
/// The grid's cells (at least this wide) and the prompt's chips.
const CELL_W: f32 = 64.0;
const CELL_H: f32 = 62.0;
const CHIP_H: f32 = 28.0;
/// An icon button, and the column's width.
const BTN_W: f32 = 30.0;
const BTN_H: f32 = 28.0;
const COL: f32 = 40.0;

/// A piece of the list.
#[derive(Clone, Copy, PartialEq)]
enum Part {
    Field,
    Line,
    Row(usize),
    /// The pins as a grid, so many to a row.
    Grid(usize),
    /// The prompt's results in a row.
    Chips,
    Hint,
}

/// The panel laid out: the list's pieces (what, top, height) in the list's area, the
/// total height, the list's area, the buttons' boxes, the row's note box (x, centre y,
/// width) and the dividing line (x, y, w, h).
struct Lay {
    parts: Vec<(Part, f32, f32)>,
    h: f32,
    list_x: f32,
    list_w: f32,
    boxes: Vec<(Hit, [f32; 4])>,
    note: Option<[f32; 3]>,
    divider: Option<[f32; 4]>,
}

impl LauncherPanel {
    pub fn new(opts: &Opts, own_field: bool) -> LauncherPanel {
        set_opts(opts);
        let chosen: Vec<String> = opts.table.get("power").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let power = (0..POWER.len()).filter(|&i| chosen.iter().any(|c| c == POWER[i].0)).collect();
        let buttons = opts
            .table
            .get("buttons")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|it| {
                        let s = |k: &str| it.get(k).and_then(|v| v.as_str()).map(String::from);
                        let action = s("action")?;
                        Some(Custom { icon: s("icon").unwrap_or_else(|| "\u{E8B7}".into()), label: s("label").unwrap_or_else(|| action.clone()), action, target: s("target"), confirm: it.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let width = opts.num("panel_width").map_or(340.0, |w| w.clamp(240.0, 700.0) as f32);
        let align = match opts.str("power_align") {
            Some(a @ ("left" | "center")) => a.to_string(),
            _ => "right".to_string(),
        };
        LauncherPanel {
            own_field,
            surface: opts.surface.clone(),
            power,
            buttons,
            show_user: opts.table.get("user").and_then(|v| v.as_bool()).unwrap_or(false),
            user: None,
            user_pic: None,
            side: opts.str("buttons_position") == Some("left"),
            align,
            icons: HashMap::new(),
            pixels: HashMap::new(),
            asked: Vec::new(),
            pending: None,
            width,
            boxes: Vec::new(),
            hovered: None,
            armed: None,
            beat: Instant::now(),
            bottom: false,
            wheel_left: 0.0,
        }
    }

    fn hit(&self, p: (f32, f32)) -> Option<Hit> {
        self.boxes.iter().find(|(_, b)| p.0 >= b[0] && p.0 < b[0] + b[2] && p.1 >= b[1] && p.1 < b[1] + b[3]).map(|(h, _)| *h)
    }

    fn controls(&self) -> bool {
        self.show_user || !self.buttons.is_empty() || !self.power.is_empty()
    }

    /// The user's name as shown in the row (fitted), and the width it takes.
    fn user_name(&self, g: &Gfx, cx: &Ctx) -> (String, f32) {
        let name = self.user.as_ref().map(|u| u.0.clone()).unwrap_or_default();
        let f = cx.secondary(12.0);
        let text = fit_text(g, &name, &f, self.width * 0.4);
        let w = g.measure(&text, &f);
        (text, w)
    }

    fn lay(&self, g: &Gfx, cx: &Ctx, results: usize, no_match: bool, lh: f32) -> Lay {
        let bottom = cx.look.bottom;
        let side = self.side && self.controls();
        let list_x = if side { COL + 1.0 + 6.0 } else { 0.0 };
        let list_w = self.width - list_x;
        let mut parts = Vec::new();
        let mut y = 0.0;
        if self.own_field {
            parts.push((Part::Field, 4.0, 26.0));
            parts.push((Part::Line, 4.0 + 26.0 + 8.0, 1.0));
            y += 4.0 + 26.0 + 8.0 + 1.0 + 4.0;
        }
        let (style, empty) = S.with(|s| {
            let s = s.borrow();
            (s.style.clone(), s.query.trim().is_empty())
        });
        if style == "grid" && empty && results > 0 {
            let cols = (((list_w + 4.0) / (CELL_W + 4.0)).floor() as usize).max(1);
            S.with(|s| s.borrow_mut().grid_cols = cols);
            let h = results.div_ceil(cols) as f32 * (CELL_H + 4.0) - 4.0;
            parts.push((Part::Grid(cols), y, h));
            y += h + 2.0;
        } else if style == "prompt" {
            if results > 0 {
                parts.push((Part::Chips, y + 2.0, CHIP_H));
                y += 2.0 + CHIP_H + 4.0;
            }
        } else {
            for i in 0..results {
                parts.push((Part::Row(i), y, ROW_H));
                y += ROW_H + 2.0;
            }
        }
        if no_match {
            parts.push((Part::Hint, y, 12.0 + lh));
            y += 12.0 + lh;
        }
        let list_h = y;
        let (nb, np) = (self.buttons.len(), self.power.len());
        let mut boxes = Vec::new();
        let (mut note, mut divider) = (None, None);
        let h;
        if !self.controls() {
            h = list_h;
        } else if side {
            // The column: the user and the buttons from the top, power buttons at the end.
            let bx = (COL - BTN_W) / 2.0;
            let mut top = 4.0;
            if self.show_user {
                boxes.push((Hit::User, [(COL - 30.0) / 2.0, top, 30.0, 30.0]));
                top += 30.0 + 6.0;
            }
            for k in 0..nb {
                boxes.push((Hit::Button(k), [bx, top, BTN_W, BTN_H]));
                top += BTN_H + 2.0;
            }
            let power_h = np as f32 * (BTN_H + 2.0);
            h = list_h.max(top + if np > 0 && top > 4.0 { 8.0 } else { 0.0 } + power_h + 2.0);
            let mut py = h - 2.0 - power_h;
            for k in 0..np {
                boxes.push((Hit::Power(k), [bx, py, BTN_W, BTN_H]));
                py += BTN_H + 2.0;
            }
            divider = Some([COL, 4.0, 1.0, (h - 8.0).max(0.0)]);
        } else {
            // The row at the end: the user, then the buttons (aligned as asked).
            parts.push((Part::Line, y + 4.0, 1.0));
            let band = y + 4.0 + 1.0 + 6.0;
            h = band + BTN_H;
            let user_w = if self.show_user { 3.0 + 24.0 + 8.0 + self.user_name(g, cx).1 + 8.0 } else { 0.0 };
            if self.show_user {
                boxes.push((Hit::User, [0.0, band, user_w, BTN_H]));
            }
            let n = (nb + np) as f32;
            let gw = n * (BTN_W + 2.0) - 2.0;
            let after_user = if self.show_user { user_w + 4.0 } else { 4.0 };
            let gx = match self.align.as_str() {
                "left" => after_user,
                "center" => ((self.width - gw) / 2.0).max(after_user),
                _ => self.width - 4.0 - gw,
            };
            for k in 0..nb + np {
                let hit = if k < nb { Hit::Button(k) } else { Hit::Power(k - nb) };
                boxes.push((hit, [gx + k as f32 * (BTN_W + 2.0), band, BTN_W, BTN_H]));
            }
            // The note (a button's name, "Click again to ...") in the free side.
            let (nx, nr) = if self.align == "left" { (gx + gw + 6.0, self.width - 6.0) } else { (after_user + 2.0, gx - 6.0) };
            if nr - nx > 30.0 {
                note = Some([nx, band + BTN_H / 2.0, nr - nx]);
            }
        }
        // A bottom dock mirrors the list (the field by the dock, the best match next to
        // it) and the row (furthest away); the column stays as it is.
        if bottom {
            for p in &mut parts {
                p.1 = h - p.1 - p.2;
            }
            if !side {
                for (_, b) in &mut boxes {
                    b[1] = h - b[1] - b[3];
                }
                if let Some(n) = note.as_mut() {
                    n[1] = h - n[1];
                }
            }
        }
        Lay { parts, h, list_x, list_w, boxes, note, divider }
    }

    /// The grid's cells: each result's box (list px: x from the list's left, y from the grid's top).
    fn cells(n: usize, cols: usize, list_w: f32) -> Vec<[f32; 4]> {
        let w = (list_w - (cols as f32 - 1.0) * 4.0) / cols as f32;
        (0..n).map(|i| [(i % cols) as f32 * (w + 4.0), (i / cols) as f32 * (CELL_H + 4.0), w, CELL_H]).collect()
    }

    /// The prompt's chips: each result's x and width (from the list's left), as many as fit.
    fn chips(g: &Gfx, cx: &Ctx, names: &[String], list_w: f32) -> Vec<(f32, f32)> {
        let f = cx.secondary(12.0);
        let mut out = Vec::new();
        let mut x = 6.0;
        for name in names {
            let w = 4.0 + 18.0 + 6.0 + g.measure(&fit_text(g, name, &f, 160.0), &f) + 10.0;
            if x + w > list_w - 6.0 {
                break;
            }
            out.push((x, w));
            x += w + 6.0;
        }
        out
    }

    /// What the note says: a confirmation asked for, else the hovered button's name.
    fn note_text(&self) -> Option<(String, bool)> {
        let label = |hit: Hit| match hit {
            Hit::Button(k) => self.buttons.get(k).map(|b| b.label.clone()),
            Hit::Power(k) => self.power.get(k).map(|&p| POWER[p].1.to_string()),
            Hit::User => self.user.as_ref().map(|u| u.0.clone()).filter(|_| self.side),
            Hit::Row(_) => None,
        };
        let (hit, _) = self.armed?;
        label(hit).map(|l| (format!("Click again to {}", l.to_lowercase()), true))
    }

    /// The account picture (round), or a person glyph until it's read.
    fn draw_user_pic(&self, g: &Gfx, cx: &Ctx, x: f32, y: f32, d: f32, alpha: f32) {
        match &self.user_pic {
            Some(bmp) => g.fill_picture(bmp, rect(x, y, d, d), d / 2.0, alpha),
            None => {
                g.fill_round(rect(x, y, d, d), d / 2.0, white(0.12 * alpha));
                let f = Font::icons(d / 2.0);
                let w = g.measure("\u{E77B}", &f);
                g.text_at("\u{E77B}", &f, x + (d - w) / 2.0, y + d / 2.0, with_alpha(cx.look.fg, alpha));
            }
        }
    }

    /// A click on a button: a second one first if it asks for that, then its action and
    /// the launcher closes.
    fn press(&mut self, hit: Hit) -> Reply {
        let close = Act::Launcher { open: false, refocus: false };
        let (act, confirm) = match hit {
            Hit::User => (Act::Run { action: "open".into(), target: Some("ms-settings:yourinfo".into()) }, false),
            Hit::Button(k) => {
                let b = &self.buttons[k];
                (Act::Run { action: b.action.clone(), target: b.target.clone() }, b.confirm)
            }
            Hit::Power(k) => {
                let (action, _, _, confirm) = POWER[self.power[k]];
                let act = action.to_string();
                (
                    Act::call(move |app| {
                        let _ = tauri::async_runtime::block_on(crate::actions::run_action(app, act, None));
                    }),
                    confirm,
                )
            }
            Hit::Row(_) => return Reply::None,
        };
        if confirm && !self.armed.is_some_and(|a| a.0 == hit) {
            self.armed = Some((hit, Instant::now() + Duration::from_secs(3)));
            return Reply::Redraw;
        }
        self.armed = None;
        Reply::Do(act.then(close))
    }
}

impl Panel for LauncherPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["appIcons", "taskIcons", "launcher:apps", "launcher:pinned", "launcher:user", "clipboard", "tasks"]
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        match key {
            // Results' icons by id; the open windows' apps' (Tasks') as "task:<app>".
            "appIcons" | "taskIcons" => {
                let prefix = if key == "taskIcons" { "task:" } else { "" };
                if let Some(map) = v.as_object() {
                    for (id, icon) in map {
                        let id = format!("{prefix}{id}");
                        if !self.icons.contains_key(&id) {
                            if let Some(px) = decode(icon) {
                                self.pixels.insert(id, px);
                            }
                        }
                    }
                }
            }
            "launcher:user" => {
                use base64::Engine;
                let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
                let picture = v
                    .get("picture")
                    .and_then(|p| p.as_str())
                    .and_then(|url| url.split_once(','))
                    .and_then(|(_, b64)| base64::engine::general_purpose::STANDARD.decode(b64).ok());
                if self.user.as_ref().is_none_or(|u| u.0 != name || u.1 != picture) {
                    self.user = Some((name, picture));
                    self.user_pic = None;
                }
            }
            _ => return take_list(key, v),
        }
        true
    }
    fn act(&mut self) -> Option<Act> {
        self.pending.take()
    }
    fn wake(&self) -> Option<Instant> {
        // The own field's caret blinks; an armed button disarms after 3 s.
        let caret = (self.own_field && searching_on(&self.surface)).then_some(self.beat);
        [caret, self.armed.map(|a| a.1)].into_iter().flatten().min()
    }
    fn tick(&mut self, now: Instant) -> bool {
        if self.armed.is_some_and(|a| now >= a.1) {
            self.armed = None;
        }
        if now >= self.beat {
            self.beat = now + Duration::from_millis(265);
        }
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        self.bottom = cx.look.bottom;
        for (id, px) in self.pixels.drain() {
            if let Some(i) = AppIcon::new(g, &px, Style::Color) {
                self.icons.insert(id, i);
            }
        }
        if self.user_pic.is_none() {
            if let Some(bytes) = self.user.as_ref().and_then(|u| u.1.as_ref()) {
                self.user_pic = g.picture(bytes).ok();
            }
        }
        let (results, query, ids, names) = S.with(|s| {
            let s = s.borrow();
            let ids = s.results.iter().filter(|&&i| i != CALC && !(CLIP..CALC).contains(&i)).map(|&i| entry(&s, i).0).collect::<Vec<_>>();
            (s.results.len(), s.query.clone(), ids, s.results.iter().map(|&i| entry(&s, i).1).collect::<Vec<_>>())
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
        let lay = self.lay(g, cx, results, !query.trim().is_empty() && results == 0, lh);
        let mut boxes: Vec<(Hit, [f32; 4])> = lay
            .parts
            .iter()
            .flat_map(|&(part, top, ht)| match part {
                Part::Row(i) => vec![(Hit::Row(i), [lay.list_x, top, lay.list_w, ht])],
                Part::Grid(cols) => Self::cells(results, cols, lay.list_w).into_iter().enumerate().map(|(i, c)| (Hit::Row(i), [lay.list_x + c[0], top + c[1], c[2], c[3]])).collect(),
                Part::Chips => Self::chips(g, cx, &names, lay.list_w).into_iter().enumerate().map(|(i, (x, w))| (Hit::Row(i), [lay.list_x + x, top, w, CHIP_H])).collect(),
                _ => vec![],
            })
            .collect();
        boxes.extend(lay.boxes);
        self.boxes = boxes;
        (self.width, lay.h.max(1.0))
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let body = cx.body();
        let small = cx.secondary(12.0);
        let lh = g.text("Ag", &body).map_or(16.0, |t| t.height);
        // Each result: its id, name and the key its icon is under (an open window's: its app's).
        let (results, selected, query) = S.with(|s| {
            let s = s.borrow();
            let row = |i: usize| {
                let (id, name) = entry(&s, i);
                let icon = if (WIN..CALC).contains(&i) { format!("task:{}", s.windows[i - WIN].2) } else { id.clone() };
                (id, name, icon)
            };
            (s.results.iter().map(|&i| row(i)).collect::<Vec<_>>(), s.selected, s.query.clone())
        });
        let lay = self.lay(g, cx, results.len(), !query.trim().is_empty() && results.is_empty(), lh);
        let (lx, lw) = (x + lay.list_x, lay.list_w);
        for &(part, top, _) in &lay.parts {
            let top = y + top;
            match part {
                Part::Field => {
                    let cy = top + 13.0;
                    let iw = g.text_at("\u{E721}", &Font::icons(14.0), lx + 8.0, cy + 1.0, fg);
                    draw_field(g, cx, lx + 8.0 + iw + 10.0, cy, lw - 16.0 - iw - 10.0, alpha);
                }
                Part::Line => g.fill_rect(rect(lx, top, lw, 1.0), white(0.08 * alpha)),
                Part::Row(i) => {
                    let (id, name, icon_key) = &results[i];
                    if i == selected {
                        g.fill_round(rect(lx, top, lw, ROW_H), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
                    }
                    let cy = top + ROW_H / 2.0;
                    let glyph = if id.starts_with("calc:") {
                        Some("\u{E8EF}")
                    } else if id.starts_with("clip:") {
                        Some(if name == "Image" { "\u{EB9F}" } else { "\u{E77F}" })
                    } else {
                        None
                    };
                    if let Some(gl) = glyph {
                        let f = Font::icons(16.0);
                        let w = g.measure(gl, &f);
                        g.text_at(gl, &f, lx + 8.0 + (22.0 - w) / 2.0, cy, fg);
                    } else if let Some(icon) = self.icons.get(icon_key) {
                        icon.draw(g, lx + 8.0, top + 6.0, 22.0, alpha);
                    }
                    // What it is, dimmed at the end: a Settings page, the sum's answer (copied),
                    // an open window.
                    let tag = if id.starts_with("ms-settings:") {
                        "Settings"
                    } else if id.starts_with("win:") {
                        "Switch to"
                    } else if id.starts_with("calc:") {
                        "Copy"
                    } else if id.starts_with("clip:") {
                        "Paste"
                    } else {
                        ""
                    };
                    let tw = if tag.is_empty() { 0.0 } else { g.measure(tag, &small) + 12.0 };
                    if !tag.is_empty() {
                        g.text_at(tag, &small, lx + lw - 10.0 - (tw - 12.0), cy, dim);
                    }
                    g.text_at(&fit_text(g, name, &body, lw - 48.0 - tw), &body, lx + 40.0, cy, fg);
                }
                Part::Grid(cols) => {
                    let f = cx.secondary(11.0);
                    for (i, c) in Self::cells(results.len(), cols, lw).into_iter().enumerate() {
                        let (id, name, icon_key) = &results[i];
                        let (cx0, cy0) = (lx + c[0], top + c[1]);
                        let on = i == selected;
                        if on {
                            g.fill_round(rect(cx0, cy0, c[2], c[3]), 10.0, with_alpha(cx.hover_bg(0.12), alpha));
                        }
                        let ix = cx0 + (c[2] - 30.0) / 2.0;
                        match self.icons.get(icon_key) {
                            Some(icon) => icon.draw(g, ix, cy0 + 8.0, 30.0, alpha),
                            None if id.starts_with("calc:") || id.starts_with("clip:") => {}
                            None => g.fill_round(rect(ix, cy0 + 8.0, 30.0, 30.0), 8.0, white(0.1 * alpha)),
                        }
                        let t = fit_text(g, name, &f, c[2] - 6.0);
                        let tw = g.measure(&t, &f);
                        g.text_at(&t, &f, cx0 + (c[2] - tw) / 2.0, cy0 + 8.0 + 30.0 + 6.0 + line_h(g, &f) / 2.0, if on { fg } else { dim });
                    }
                }
                Part::Chips => {
                    let names: Vec<String> = results.iter().map(|r| r.1.clone()).collect();
                    let f = cx.secondary(12.0);
                    for (i, (x0, w)) in Self::chips(g, cx, &names, lw).into_iter().enumerate() {
                        let (_, name, icon_key) = &results[i];
                        let bg = if i == selected { cx.hover_bg(0.16) } else { white(0.08) };
                        g.fill_round(rect(lx + x0, top, w, CHIP_H), 8.0, with_alpha(bg, alpha));
                        if let Some(icon) = self.icons.get(icon_key) {
                            icon.draw(g, lx + x0 + 5.0, top + (CHIP_H - 18.0) / 2.0, 18.0, alpha);
                        }
                        g.text_at(&fit_text(g, name, &f, 160.0), &f, lx + x0 + 4.0 + 18.0 + 6.0, top + CHIP_H / 2.0, fg);
                    }
                }
                Part::Hint => {
                    let clip = S.with(|s| s.borrow().clipboard) && query.trim_start().starts_with(',');
                    let hint = if clip { "Nothing copied matches" } else { "No apps found" };
                    g.text_at(hint, &cx.secondary(cx.look.font_size), lx + 8.0, top + 6.0 + lh / 2.0, dim);
                }
            }
        }
        if let Some(d) = lay.divider {
            g.fill_rect(rect(x + d[0], y + d[1], d[2], d[3]), white(0.08 * alpha));
        }
        for &(hit, b) in &lay.boxes {
            let (bx, by) = (x + b[0], y + b[1]);
            let on = self.hovered == Some(hit);
            let armed = self.armed.is_some_and(|a| a.0 == hit);
            if armed || on {
                g.fill_round(rect(bx, by, b[2], b[3]), 8.0, white(if armed { 0.16 } else { 0.1 } * alpha));
            }
            match hit {
                Hit::User if self.side => self.draw_user_pic(g, cx, bx + 1.0, by + 1.0, 28.0, alpha),
                Hit::User => {
                    self.draw_user_pic(g, cx, bx + 3.0, by + (b[3] - 24.0) / 2.0, 24.0, alpha);
                    let (name, _) = self.user_name(g, cx);
                    g.text_at(&name, &small, bx + 3.0 + 24.0 + 8.0, by + b[3] / 2.0, fg);
                }
                Hit::Button(_) | Hit::Power(_) => {
                    let glyph = match hit {
                        Hit::Button(k) => self.buttons[k].icon.as_str(),
                        Hit::Power(k) => POWER[self.power[k]].2,
                        _ => "",
                    };
                    let f = Font::icons(14.0);
                    let w = g.measure(glyph, &f);
                    g.text_at(glyph, &f, bx + (b[2] - w) / 2.0, by + b[3] / 2.0, if on || armed { fg } else { dim });
                }
                Hit::Row(_) => {}
            }
        }
        // A button's name or a confirmation: in the row's free side, or (column) as a
        // label beside the button.
        if let Some((text, ask)) = self.note_text() {
            let anchor = self.armed.map(|a| a.0).or(self.hovered);
            if let Some(n) = lay.note {
                let t = fit_text(g, &text, &small, n[2]);
                let tx = if self.align == "left" { x + n[0] + n[2] - g.measure(&t, &small) } else { x + n[0] };
                g.text_at(&t, &small, tx, y + n[1], if ask { fg } else { dim });
            } else if let Some(b) = anchor.and_then(|h| lay.boxes.iter().find(|(bh, _)| *bh == h)).map(|(_, b)| *b).filter(|_| lay.divider.is_some()) {
                let tw = g.measure(&text, &small).min(lay.list_w - 12.0);
                let t = fit_text(g, &text, &small, tw);
                let (px, cy) = (x + COL + 6.0, y + b[1] + b[3] / 2.0);
                g.fill_round(rect(px, cy - 12.0, tw + 16.0, 24.0), 8.0, with_alpha(rgba(0.16, 0.16, 0.16, 1.0), alpha));
                g.text_at(&t, &small, px + 8.0, cy, fg);
            }
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.hit(p));
        let mut changed = h != self.hovered;
        self.hovered = h;
        // Onto a row: it becomes the chosen one (moving within it leaves a choice the
        // wheel or the keys made).
        if let (Some(Hit::Row(i)), true) = (h, changed) {
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
        // A right-click on an app: its menu (the native thread opens it, take_menu).
        if button == Button::Right {
            if let Some(Hit::Row(i)) = self.hit(at) {
                let id = S.with(|s| {
                    let s = s.borrow();
                    s.results.get(i).map(|&k| entry(&s, k).0)
                });
                MENU.with(|m| *m.borrow_mut() = id.filter(|id| !id.starts_with("calc:") && !id.starts_with("clip:") && !id.starts_with("win:")));
            }
            return Reply::None;
        }
        if button != Button::Left {
            return Reply::None;
        }
        match self.hit(at) {
            Some(Hit::Row(i)) => {
                let id = S.with(|s| {
                    let s = s.borrow();
                    s.results.get(i).map(|&k| entry(&s, k).0)
                });
                match id {
                    Some(id) => Reply::Do(launch(id)),
                    None => Reply::None,
                }
            }
            Some(hit) => self.press(hit),
            None => Reply::None,
        }
    }
    fn wheel(&mut self, _at: (f32, f32), notches: f32) -> Reply {
        // Each whole notch moves the choice one result: away from the field when rolled
        // that way (down under a top bar's field, up over a bottom dock's). Touchpads
        // send parts of a notch: they add up.
        self.wheel_left += notches;
        let mut moved = false;
        while self.wheel_left.abs() >= 1.0 {
            let up = self.wheel_left > 0.0;
            self.wheel_left -= if up { 1.0 } else { -1.0 };
            moved |= step(up == self.bottom);
        }
        if moved {
            Reply::Redraw
        } else {
            Reply::None
        }
    }
    fn closed(&mut self) {
        self.hovered = None;
        self.armed = None;
        self.wheel_left = 0.0;
    }
    fn tooltip(&self) -> Option<String> {
        match self.hovered? {
            Hit::Button(k) => self.buttons.get(k).map(|b| b.label.clone()),
            Hit::Power(k) => self.power.get(k).map(|&p| POWER[p].1.to_string()),
            Hit::User => Some(self.user.as_ref().map_or("Your account".into(), |u| format!("{} (your account)", u.0))),
            Hit::Row(_) => None,
        }
    }
}

thread_local! {
    /// An app right-clicked in the launcher: its menu, once the click is done.
    static MENU: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The app whose menu a right-click asked for (ctxmenu::launcher_menu shows it).
pub fn take_menu() -> Option<String> {
    MENU.with(|m| m.borrow_mut().take())
}

/// Starts an app (copies the sum's answer, switches to the window) and closes the launcher.
pub fn launch(id: String) -> Act {
    if let Some(v) = id.strip_prefix("calc:").map(String::from) {
        return Act::call(move |_| crate::win::set_clipboard_text(&v)).then(Act::Launcher { open: false, refocus: false });
    }
    if let Some(h) = id.strip_prefix("win:").and_then(|n| n.parse::<isize>().ok()) {
        return Act::call(move |_| {
            let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_activate(h, false));
        })
        .then(Act::Launcher { open: false, refocus: false });
    }
    // A clipboard item: the launcher hands the focus back, then it's pasted there.
    if let Some(clip) = id.strip_prefix("clip:").and_then(|n| n.parse::<u64>().ok()) {
        return Act::Launcher { open: false, refocus: true }.then(Act::call(move |_| {
            std::thread::sleep(Duration::from_millis(250));
            let _ = tauri::async_runtime::block_on(crate::providers::clipboard::clipboard_paste(clip));
        }));
    }
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

#[cfg(test)]
mod tests {
    use super::calc;

    #[test]
    fn sums() {
        assert_eq!(calc("24*7").as_deref(), Some("168"));
        assert_eq!(calc("(3+4)^2").as_deref(), Some("49"));
        assert_eq!(calc("1,5 / 3").as_deref(), Some("0.5"));
        assert_eq!(calc("2 x 8").as_deref(), Some("16"));
        assert_eq!(calc("= -2 + 5").as_deref(), Some("3"));
        assert_eq!(calc("10/3").as_deref(), Some("3.3333333333"));
        assert_eq!(calc("sqrt(16)").as_deref(), Some("4"));
        assert_eq!(calc("2^-1").as_deref(), Some("0.5"));
    }

    #[test]
    fn not_sums() {
        for t in ["42", "notepad", "xbox", "1/0", "2+", "(2+3", "3.1.4+1", ""] {
            assert_eq!(calc(t), None, "{t}");
        }
    }
}
