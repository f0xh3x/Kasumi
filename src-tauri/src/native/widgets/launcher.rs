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
use crate::native::gfx::{rect, rgba, white, with_alpha, Font, Gfx};
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
    /// When the caret last moved (it shows solid for a moment, then blinks).
    typed: Instant,
    /// The surface it's open on (every surface draws on this thread: the others keep
    /// their search icon).
    owner: String,
}

thread_local! {
    static S: RefCell<State> = RefCell::new(State { open: false, query: String::new(), selected: 0, apps: Vec::new(), results: Vec::new(), max: 8, calc: None, calculator: false, settings: false, pins: Vec::new(), pinned: false, user: false, clip: Vec::new(), clipboard: false, typed: Instant::now(), owner: String::new() });
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
/// pins' indices begin.
const CALC: usize = usize::MAX;
const PIN: usize = 1 << 40;
const CLIP: usize = 1 << 41;

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

/// A result's id and name: an app (its index in the list), the calculator's row or a
/// pin of Start's.
fn entry(s: &State, i: usize) -> (String, String) {
    if i == CALC {
        let v = s.calc.clone().unwrap_or_default();
        return (format!("calc:{v}"), format!("= {v}"));
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
    found.sort_by(|a, b| b.1.total_cmp(&a.1));
    // A sum: its answer first.
    s.calc = if s.calculator { calc(&s.query) } else { None };
    let first = s.calc.as_ref().map(|_| CALC);
    let room = s.max - first.is_some() as usize;
    s.results = first.into_iter().chain(found.into_iter().take(room).map(|(i, _)| i)).collect();
    // Nothing typed yet: Start's pins.
    if q.is_empty() && s.pinned {
        s.results = (0..s.pins.len().min(s.max)).map(|k| PIN + k).collect();
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

/// The app list or Start's pins arrived (whichever of the launcher's widgets and panels
/// gets it: they share one state).
fn take_list(key: &str, v: &Value) {
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
            _ => return,
        }
        search(&mut s);
    });
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
    surface: String,
}

impl LauncherWidget {
    pub fn new(opts: &Opts, inline: bool) -> LauncherWidget {
        set_opts(opts);
        LauncherWidget { inline, surface: opts.surface.clone() }
    }
    fn searching(&self) -> bool {
        searching_on(&self.surface)
    }
}

impl Widget for LauncherWidget {
    fn keys(&self) -> &'static [&'static str] {
        &["launcher:apps", "launcher:pinned", "clipboard"]
    }
    fn tooltip(&self) -> Option<String> {
        (!self.searching()).then(|| "Search apps".into())
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        take_list(key, v);
        true
    }
    fn wake(&self) -> Option<Instant> {
        self.searching().then(|| Instant::now() + Duration::from_millis(265))
    }
    fn tick(&mut self, _now: Instant) -> bool {
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
        cx.gfx.measure("\u{E721}", &Font::icons(14.0)) + if on { 8.0 + 190.0 } else { 0.0 }
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let f = Font::icons(14.0);
        let iw = cx.gfx.text_at("\u{E721}", &f, x, cy + 1.0, with_alpha(cx.look.fg, alpha));
        if self.searching() {
            draw_field(cx.gfx, cx, x + iw + 8.0, cy, 190.0, alpha);
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
}

const ROW_H: f32 = 34.0;
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
        for i in 0..results {
            parts.push((Part::Row(i), y, ROW_H));
            y += ROW_H + 2.0;
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
        &["appIcons", "launcher:apps", "launcher:pinned", "launcher:user", "clipboard"]
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        match key {
            "appIcons" => {
                if let Some(map) = v.as_object() {
                    for (id, icon) in map {
                        if !self.icons.contains_key(id) {
                            if let Some(px) = decode(icon) {
                                self.pixels.insert(id.clone(), px);
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
            _ => take_list(key, v),
        }
        true
    }
    fn act(&mut self) -> Option<Act> {
        self.pending.take()
    }
    fn wake(&self) -> Option<Instant> {
        // The own field's caret blinks; an armed button disarms after 3 s.
        let caret = (self.own_field && searching_on(&self.surface)).then(|| Instant::now() + Duration::from_millis(265));
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
        if self.user_pic.is_none() {
            if let Some(bytes) = self.user.as_ref().and_then(|u| u.1.as_ref()) {
                self.user_pic = g.picture(bytes).ok();
            }
        }
        let (results, query, ids) = S.with(|s| {
            let s = s.borrow();
            (s.results.len(), s.query.clone(), s.results.iter().filter(|&&i| i != CALC && !(CLIP..CALC).contains(&i)).map(|&i| entry(&s, i).0).collect::<Vec<_>>())
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
            .filter_map(|&(part, top, ht)| match part {
                Part::Row(i) => Some((Hit::Row(i), [lay.list_x, top, lay.list_w, ht])),
                _ => None,
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
        let (results, selected, query) = S.with(|s| {
            let s = s.borrow();
            (s.results.iter().map(|&i| entry(&s, i)).collect::<Vec<_>>(), s.selected, s.query.clone())
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
                    let (id, name) = &results[i];
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
                    } else if let Some(icon) = self.icons.get(id) {
                        icon.draw(g, lx + 8.0, top + 6.0, 22.0, alpha);
                    }
                    // What it is, dimmed at the end: a Settings page, the sum's answer (copied).
                    let tag = if id.starts_with("ms-settings:") {
                        "Settings"
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
        // A right-click on an app: its menu (the native thread opens it, take_menu).
        if button == Button::Right {
            if let Some(Hit::Row(i)) = self.hit(at) {
                let id = S.with(|s| {
                    let s = s.borrow();
                    s.results.get(i).map(|&k| entry(&s, k).0)
                });
                MENU.with(|m| *m.borrow_mut() = id.filter(|id| !id.starts_with("calc:") && !id.starts_with("clip:")));
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
    fn closed(&mut self) {
        self.hovered = None;
        self.armed = None;
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

/// Starts an app (or copies the sum's answer) and closes the launcher.
pub fn launch(id: String) -> Act {
    if let Some(v) = id.strip_prefix("calc:").map(String::from) {
        return Act::call(move |_| crate::win::set_clipboard_text(&v)).then(Act::Launcher { open: false, refocus: false });
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
