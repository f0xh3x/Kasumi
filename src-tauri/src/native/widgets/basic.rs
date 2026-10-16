//! Small widgets drawn as a row of pieces: temps, disk, buttons, language,
//! workspaces (src/widgets/temps.ts, disk.ts, buttons.ts, index.ts language,
//! workspaces.ts).

use std::time::{Duration, Instant};

use serde_json::Value;

use super::{row_draw, row_width, Act, Button, Ctx, Opts, Piece, Reply, Roll, Widget};
use crate::native::gfx::{rgba, white, with_alpha, Color};

/// Segoe Fluent Icons glyphs.
const TEMP: &str = "\u{E9CA}";
const BUTTON: &str = "\u{E8B7}";
const KEYBOARD: &str = "\u{E765}";

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// Meters' fill: the accent with accent details, else the text colour; past 90% the warning colour.
fn meter_fill(cx: &Ctx, high: bool) -> Color {
    if high {
        cx.look.warn
    } else if cx.opts.accent {
        cx.look.accent
    } else {
        cx.look.fg
    }
}

// ----------------------------------------------------------------------

/// CPU · GPU temperatures from LibreHardwareMonitor; `warn` (°C) highlights hot ones.
pub struct Temps {
    opts: Opts,
    cpu: Roll,
    gpu: Roll,
    hot: (bool, bool),
    ok: bool,
}

impl Temps {
    pub fn new(opts: Opts) -> Temps {
        Temps { opts, cpu: Roll::default(), gpu: Roll::default(), hot: (false, false), ok: false }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let body = cx.body();
        let color = |hot: bool| if !self.ok { cx.look.dimmed() } else if hot { cx.look.warn } else { cx.look.fg };
        vec![
            Piece::glyph(TEMP, 14.0, color(self.hot.0)),
            Piece::Gap(4.0),
            self.cpu.piece(body.clone(), color(self.hot.0)),
            Piece::Gap(6.0 + 2.0),
            Piece::text("·", body.clone(), cx.look.dimmed()),
            Piece::Gap(2.0 + 6.0),
            Piece::glyph(TEMP, 14.0, color(self.hot.1)),
            Piece::Gap(4.0),
            self.gpu.piece(body, color(self.hot.1)),
        ]
    }
}

impl Widget for Temps {
    fn keys(&self) -> &'static [&'static str] {
        &["temps"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let warn = self.opts.num("warn").unwrap_or(90.0);
        let read = |k: &str| v.get(k).and_then(num);
        let fmt = |t: Option<f64>| t.map_or("–".to_string(), |t| format!("{}°", t.round()));
        let (c, g) = (read("cpu"), read("gpu"));
        let ok = v.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
        let hot = (c.is_some_and(|t| t >= warn), g.is_some_and(|t| t >= warn));
        let mut changed = self.cpu.set(fmt(c), self.opts.roll);
        changed |= self.gpu.set(fmt(g), self.opts.roll);
        changed |= hot != self.hot || ok != self.ok;
        self.hot = hot;
        self.ok = ok;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn animating(&self, now: Instant) -> bool {
        self.cpu.animating(now) || self.gpu.animating(now)
    }
    fn click(&mut self, _: Button, _: (f32, f32)) -> Reply {
        Reply::TogglePanel
    }
}

// ----------------------------------------------------------------------

struct Drive {
    id: String,
    share: f64,
    text: Roll,
}

/// Disk usage per chosen drive: letter, meter (bar | circle | none), text
/// (percent | used | free | none). A click opens This PC.
pub struct Disk {
    opts: Opts,
    drives: Vec<Drive>,
}

/// Bytes as Explorer shows them (1024-based): "931 GB", "1.8 TB".
pub fn format_bytes(bytes: f64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut i) = (bytes, 0);
    while v >= 1024.0 && i < units.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if v >= 100.0 || i == 0 {
        format!("{} {}", v.round(), units[i])
    } else {
        format!("{v:.1} {}", units[i])
    }
}

impl Disk {
    pub fn new(opts: Opts) -> Disk {
        Disk { opts, drives: Vec::new() }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let kind = self.opts.str("indicator").unwrap_or("bar");
        let show = self.opts.str("show").unwrap_or("percent");
        let letters = self.opts.flag("letters") != Some(false);
        let len = self.opts.num("bar_length").unwrap_or(30.0) as f32;
        let mut out = Vec::new();
        for (i, d) in self.drives.iter().enumerate() {
            if i > 0 {
                out.push(Piece::Gap(12.0));
            }
            let mut parts = Vec::new();
            if letters {
                parts.push(Piece::text(d.id.clone(), cx.secondary(cx.look.font_size), cx.look.dimmed()));
            }
            let fill = meter_fill(cx, d.share >= 0.9);
            match kind {
                "none" => {}
                "circle" => parts.push(Piece::Ring { v: d.share as f32, fill }),
                _ => parts.push(Piece::Bar { len, v: d.share as f32, fill }),
            }
            if show != "none" {
                parts.push(d.text.piece(cx.body(), cx.look.fg));
            }
            for (j, p) in parts.into_iter().enumerate() {
                if j > 0 {
                    out.push(Piece::Gap(6.0));
                }
                out.push(p);
            }
        }
        out
    }
}

impl Widget for Disk {
    fn keys(&self) -> &'static [&'static str] {
        &["disks"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let chosen: Vec<String> = self.opts.table.get("disks").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        let show = self.opts.str("show").unwrap_or("percent").to_string();
        let list: Vec<&Value> = v.as_array().map(|a| a.iter().collect()).unwrap_or_default();
        let list: Vec<&Value> = list.into_iter().filter(|d| chosen.is_empty() || d.get("id").and_then(|i| i.as_str()).is_some_and(|i| chosen.iter().any(|c| c == i))).collect();
        let ids: Vec<String> = list.iter().map(|d| d.get("id").and_then(|i| i.as_str()).unwrap_or("").to_string()).collect();
        let mut changed = false;
        if ids != self.drives.iter().map(|d| d.id.clone()).collect::<Vec<_>>() {
            self.drives = ids.into_iter().map(|id| Drive { id, share: 0.0, text: Roll::default() }).collect();
            changed = true;
        }
        for (d, v) in self.drives.iter_mut().zip(&list) {
            let total = v.get("total").and_then(num).unwrap_or(0.0);
            let free = v.get("free").and_then(num).unwrap_or(0.0);
            let used = total - free;
            let share = if total > 0.0 { used / total } else { 0.0 };
            let text = match show.as_str() {
                "used" => format!("{} / {}", format_bytes(used), format_bytes(total)),
                "free" => format!("{} free", format_bytes(free)),
                _ => format!("{}%", (share * 100.0).round()),
            };
            changed |= d.text.set(text, self.opts.roll) || (d.share - share).abs() > 1e-4;
            d.share = share;
        }
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn animating(&self, now: Instant) -> bool {
        self.drives.iter().any(|d| d.text.animating(now))
    }
    fn click(&mut self, _: Button, _: (f32, f32)) -> Reply {
        Reply::Do(Act::open("shell:MyComputerFolder"))
    }
}

// ----------------------------------------------------------------------

struct Item {
    icon: String,
    action: String,
    target: Option<String>,
    confirm: bool,
}

/// A row of icon buttons running menu actions (`items`); destructive ones take a second click.
pub struct Buttons {
    items: Vec<Item>,
    hovered: Option<usize>,
    armed: Option<(usize, Instant)>,
    /// The row height as last drawn (buttons size with it).
    h: f32,
}

impl Buttons {
    pub fn new(opts: Opts) -> Buttons {
        let items = opts
            .table
            .get("items")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|it| {
                        let s = |k: &str| it.get(k).and_then(|v| v.as_str()).map(String::from);
                        Some(Item { action: s("action")?, icon: s("icon").unwrap_or_else(|| BUTTON.into()), target: s("target"), confirm: it.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        Buttons { items, hovered: None, armed: None, h: 30.0 }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        // .btn: 0.8 h square, 0.45 h icon, 0.1 h apart.
        let h = cx.h;
        let mut out = Vec::new();
        for (i, it) in self.items.iter().enumerate() {
            if i > 0 {
                out.push(Piece::Gap(h * 0.1));
            }
            let bg = if self.armed.is_some_and(|(a, _)| a == i) {
                Some(rgba(1.0, 80.0 / 255.0, 80.0 / 255.0, 0.35))
            } else if self.hovered == Some(i) {
                Some(cx.hover_bg(0.12))
            } else {
                None
            };
            out.push(Piece::Button { side: h * 0.8, glyph: it.icon.clone(), size: h * 0.45, color: cx.look.fg, bg });
        }
        out
    }
    /// The button under a local x.
    fn at(&self, x: f32) -> Option<usize> {
        let (side, gap) = (self.h * 0.8, self.h * 0.1);
        let i = (x / (side + gap)).floor();
        (i >= 0.0 && (i as usize) < self.items.len() && x - i * (side + gap) < side).then_some(i as usize)
    }
}

impl Widget for Buttons {
    fn width(&mut self, cx: &Ctx) -> f32 {
        self.h = cx.h;
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|(x, _)| self.at(x));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn wake(&self) -> Option<Instant> {
        self.armed.map(|(_, t)| t)
    }
    fn tick(&mut self, now: Instant) -> bool {
        if self.armed.is_some_and(|(_, t)| now >= t) {
            self.armed = None;
            return true;
        }
        false
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let Some(i) = self.at(at.0).filter(|_| button == Button::Left) else { return Reply::None };
        let it = &self.items[i];
        if it.confirm && !self.armed.is_some_and(|(a, _)| a == i) {
            self.armed = Some((i, Instant::now() + Duration::from_secs(3)));
            return Reply::Redraw;
        }
        self.armed = None;
        Reply::Do(Act::Run { action: it.action.clone(), target: it.target.clone() })
    }
}

// ----------------------------------------------------------------------

/// The keyboard layout's code (`format` "two": EN, else ENG); a click switches to the next one.
pub struct Language {
    opts: Opts,
    code: String,
}

impl Language {
    pub fn new(opts: Opts) -> Language {
        Language { opts, code: String::new() }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let mut out = Vec::new();
        if self.opts.flag("icon") == Some(true) {
            out.push(Piece::glyph(KEYBOARD, 14.0, cx.look.fg));
            out.push(Piece::Gap(6.0));
        }
        out.push(Piece::text(self.code.clone(), cx.main(12.0), cx.look.fg));
        out
    }
}

/// A keyboard layout's code as the language widget shows it ([widgets.language] format).
pub fn language_code(k: &Value, format: Option<&str>) -> String {
    let code = k.get("code").and_then(|c| c.as_str()).unwrap_or("");
    let code2 = k.get("code2").and_then(|c| c.as_str()).unwrap_or("");
    if format == Some("two") && !code2.is_empty() { code2 } else { code }.to_string()
}

impl Widget for Language {
    fn keys(&self) -> &'static [&'static str] {
        &["keyboard"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let code = if v.is_null() { String::new() } else { language_code(v, self.opts.str("format")) };
        let changed = code != self.code;
        self.code = code;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        if self.code.is_empty() {
            return 0.0;
        }
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn click(&mut self, button: Button, _: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        Reply::Do(Act::call(|_| {
            if let Err(e) = crate::win::next_keyboard_layout() {
                eprintln!("[kasumi] keyboard layout: {e}");
            }
        }))
    }
}

// ----------------------------------------------------------------------

struct Space {
    name: String,
    windows: u64,
}

/// Komorebi workspaces of this monitor: dots (the focused one a pill), numbers
/// or names; click one to show it. Hidden while komorebi isn't running.
pub struct Workspaces {
    opts: Opts,
    monitor: Option<usize>,
    focused: usize,
    spaces: Vec<Space>,
    hovered: Option<usize>,
    /// The pieces' widths as last measured (hit tests).
    last: Vec<f32>,
}

impl Workspaces {
    pub fn new(opts: Opts) -> Workspaces {
        Workspaces { opts, monitor: None, focused: 0, spaces: Vec::new(), hovered: None, last: Vec::new() }
    }

    /// The workspace under a local x (every other piece is a gap).
    fn at(&self, x: f32) -> Option<usize> {
        let mut left = 0.0;
        for (i, w) in self.last.iter().enumerate() {
            if i % 2 == 0 && x >= left && x < left + w {
                return self.shown().get(i / 2).copied();
            }
            left += w;
        }
        None
    }
    fn show(&self) -> &str {
        match self.opts.str("show") {
            Some("numbers") => "numbers",
            Some("names") => "names",
            _ => "dots",
        }
    }
    /// The workspaces shown (hide_empty leaves out empty ones but the focused), by index.
    fn shown(&self) -> Vec<usize> {
        let hide = self.opts.flag("hide_empty") == Some(true);
        (0..self.spaces.len()).filter(|&i| !(hide && self.spaces[i].windows == 0 && i != self.focused)).collect()
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let mut out = Vec::new();
        for (n, i) in self.shown().into_iter().enumerate() {
            if n > 0 {
                out.push(Piece::Gap(5.0));
            }
            let s = &self.spaces[i];
            let focused = i == self.focused;
            let busy = s.windows > 0;
            let hovered = self.hovered == Some(i);
            if self.show() == "dots" {
                let a = if focused { 1.0 } else if hovered { 0.85 } else if busy { 0.6 } else { 0.3 };
                let color = if focused && cx.opts.accent { cx.look.accent } else { with_alpha(cx.look.fg, a) };
                out.push(Piece::Pill { w: if focused { 18.0 } else { 6.0 }, h: 6.0, color });
            } else {
                let text = if self.show() == "numbers" { (i + 1).to_string() } else { s.name.clone() };
                let bg = if focused {
                    Some(if cx.opts.accent { cx.hover_bg(0.16) } else { white(0.16) })
                } else if hovered {
                    Some(white(0.1))
                } else {
                    None
                };
                let color = if focused || busy { cx.look.fg } else { cx.look.dimmed() };
                out.push(Piece::Label { s: text, font: cx.body(), color, pad: (6.0, 1.0), bg });
            }
        }
        out
    }
}

impl Widget for Workspaces {
    fn keys(&self) -> &'static [&'static str] {
        &["workspaces"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        // Our monitor as komorebi names it: "\\.\DISPLAY4" -> "DISPLAY4".
        let me = self.opts.device.rsplit('\\').next().unwrap_or("").to_string();
        let monitors = v.get("monitors").and_then(|m| m.as_array()).cloned().unwrap_or_default();
        let i = monitors.iter().position(|m| m.get("name").and_then(|n| n.as_str()) == Some(me.as_str()));
        // A single monitor komorebi names differently is still this one.
        self.monitor = i.or(if monitors.len() == 1 { Some(0) } else { None });
        let Some(m) = self.monitor.and_then(|i| monitors.get(i)) else {
            let had = !self.spaces.is_empty();
            self.spaces.clear();
            return had;
        };
        self.focused = m.get("focused").and_then(|f| f.as_u64()).unwrap_or(0) as usize;
        self.spaces = m
            .get("workspaces")
            .and_then(|w| w.as_array())
            .map(|a| a.iter().map(|w| Space { name: w.get("name").and_then(|n| n.as_str()).unwrap_or("").into(), windows: w.get("windows").and_then(|n| n.as_u64()).unwrap_or(0) }).collect())
            .unwrap_or_default();
        true
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        if self.spaces.is_empty() {
            return 0.0;
        }
        let pieces = self.pieces(cx);
        self.last = pieces.iter().map(|p| p.width(cx.gfx)).collect();
        row_width(cx.gfx, &pieces)
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|(x, _)| self.at(x));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let Some(monitor) = self.monitor else { return Reply::None };
        let Some(ws) = self.at(at.0) else { return Reply::None };
        if button != Button::Left || ws == self.focused {
            return Reply::None;
        }
        Reply::Do(Act::call(move |_| {
            tauri::async_runtime::block_on(async {
                if let Err(e) = crate::providers::komorebi::komorebi_focus(monitor, ws).await {
                    eprintln!("[kasumi] komorebi: {e}");
                }
            })
        }))
    }
}
