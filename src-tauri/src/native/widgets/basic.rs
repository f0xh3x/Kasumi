//! Small widgets drawn as a row of pieces: temps, disk, buttons, language,
//! workspaces (src/widgets/temps.ts, disk.ts, buttons.ts, index.ts language,
//! workspaces.ts).

use std::time::{Duration, Instant};

use serde_json::Value;

use super::{row_draw, row_width, Act, Button, Ctx, Opts, Piece, Reply, Roll, Widget};
use crate::native::gfx::{mix, rgba, white, with_alpha, Color};

/// Segoe Fluent Icons glyphs.
const TEMP: &str = "\u{E9CA}";
const BUTTON: &str = "\u{E8B7}";
const KEYBOARD: &str = "\u{E765}";

fn num(v: &Value) -> Option<f64> {
    v.as_f64()
}

/// Meters' fill: the accent with accent details, else the text colour; past 90% the warning colour.
pub(super) fn meter_fill(cx: &Ctx, high: bool) -> Color {
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
/// `heat_colour` warms the numbers towards the warning colour as they near it,
/// `sparkline` draws the last minute beside each, `bar_shows` = "load" shows the
/// load instead.
pub struct Temps {
    opts: Opts,
    cpu: Roll,
    gpu: Roll,
    /// Latest readings: temperatures and loads.
    temp: (Option<f64>, Option<f64>),
    load: (Option<f64>, Option<f64>),
    /// The last minute of temperatures (for the sparklines).
    history: (Vec<f32>, Vec<f32>),
    ok: bool,
}

/// Readings the bar's sparkline keeps.
const SPARK: usize = 60;

impl Temps {
    pub fn new(opts: Opts) -> Temps {
        Temps { opts, cpu: Roll::default(), gpu: Roll::default(), temp: (None, None), load: (None, None), history: (Vec::new(), Vec::new()), ok: false }
    }
    fn load_shown(&self) -> bool {
        self.opts.str("bar_shows") == Some("load")
    }
    fn color(&self, cx: &Ctx, t: Option<f64>) -> Color {
        let warn = self.opts.num("warn").unwrap_or(90.0);
        match t {
            _ if !self.ok => cx.look.dimmed(),
            Some(t) if t >= warn => cx.look.warn,
            // Warming from 30° under the warning level up to it.
            Some(t) if self.opts.flag("heat_colour") == Some(true) => mix(cx.look.fg, cx.look.warn, ((t - (warn - 30.0)) / 30.0) as f32 * 0.85),
            _ => cx.look.fg,
        }
    }
    fn spark(&self, cx: &Ctx, h: &[f32], color: Color) -> Option<Piece> {
        if self.opts.flag("sparkline") != Some(true) || h.len() < 2 {
            return None;
        }
        let lo = h.iter().cloned().fold(f32::INFINITY, f32::min) - 2.0;
        let hi = h.iter().cloned().fold(f32::NEG_INFINITY, f32::max) + 2.0;
        let fill = if cx.opts.accent { cx.look.accent } else { color };
        Some(Piece::Spark { pts: h.iter().map(|v| (v - lo) / (hi - lo)).collect(), w: 26.0, color: with_alpha(fill, 0.85) })
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let body = cx.body();
        if self.load_shown() {
            let small = cx.secondary(cx.look.font_size - 2.0);
            let mut out = Vec::new();
            for (i, (name, l)) in [("CPU", self.load.0), ("GPU", self.load.1)].into_iter().enumerate() {
                if i > 0 {
                    out.push(Piece::Gap(12.0));
                }
                let v = l.unwrap_or(0.0);
                out.push(Piece::text(name, small.clone(), cx.look.dimmed()));
                out.push(Piece::Gap(6.0));
                out.push(Piece::Bar { len: 24.0, v: (v / 100.0) as f32, fill: meter_fill(cx, v >= 90.0) });
                out.push(Piece::Gap(6.0));
                out.push(Piece::text(l.map_or("–".into(), |l| format!("{}%", l.round())), body.clone(), if self.ok { cx.look.fg } else { cx.look.dimmed() }));
            }
            return out;
        }
        let (c, g) = (self.color(cx, self.temp.0), self.color(cx, self.temp.1));
        let mut out = vec![Piece::glyph(TEMP, 14.0, c), Piece::Gap(4.0), self.cpu.piece(body.clone(), c)];
        if let Some(sp) = self.spark(cx, &self.history.0, c) {
            out.push(Piece::Gap(6.0));
            out.push(sp);
        }
        out.extend([Piece::Gap(6.0 + 2.0), Piece::text("·", body.clone(), cx.look.dimmed()), Piece::Gap(2.0 + 6.0), Piece::glyph(TEMP, 14.0, g), Piece::Gap(4.0), self.gpu.piece(body, g)]);
        if let Some(sp) = self.spark(cx, &self.history.1, g) {
            out.push(Piece::Gap(6.0));
            out.push(sp);
        }
        out
    }
}

impl Widget for Temps {
    fn keys(&self) -> &'static [&'static str] {
        &["temps"]
    }
    fn tooltip(&self) -> Option<String> {
        let pct = |l: Option<f64>| l.map_or("–".into(), |l| format!("{}%", l.round()));
        Some(if !self.ok {
            "Temperatures (LibreHardwareMonitor not reachable)".into()
        } else if self.load_shown() {
            format!("CPU {} ({})  ·  GPU {} ({})", pct(self.load.0), self.cpu.text, pct(self.load.1), self.gpu.text)
        } else {
            format!("CPU {}  ·  GPU {}", self.cpu.text, self.gpu.text)
        })
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let read = |k: &str| v.get(k).and_then(num);
        let fmt = |t: Option<f64>| t.map_or("–".to_string(), |t| format!("{}°", t.round()));
        let (c, g) = (read("cpu"), read("gpu"));
        self.ok = v.get("ok").and_then(|o| o.as_bool()).unwrap_or(false);
        self.cpu.set(fmt(c), self.opts.roll);
        self.gpu.set(fmt(g), self.opts.roll);
        self.temp = (c, g);
        self.load = (read("cpuLoad"), read("gpuLoad"));
        for (h, t) in [(&mut self.history.0, c), (&mut self.history.1, g)] {
            if let Some(t) = t {
                h.push(t as f32);
                if h.len() > SPARK {
                    h.remove(0);
                }
            }
        }
        true
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
    fn click(&mut self, button: Button, _: (f32, f32)) -> Reply {
        if button == Button::Left { Reply::TogglePanel } else { Reply::None }
    }
}

// ----------------------------------------------------------------------

struct Drive {
    id: String,
    share: f64,
    text: Roll,
}

/// Disk usage per chosen drive: letter, meter (bar | circle | none), text
/// (percent | used | free | none). Drives past `warn` percent full turn the warning
/// colour; with `only_when_full` it takes no room until one is. A click opens This PC,
/// or its panel with a look (`style` = "drives" | "rings").
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
    /// How full counts as nearly full (0..1).
    fn warn(&self) -> f64 {
        self.opts.num("warn").unwrap_or(90.0) / 100.0
    }
    /// Shown at all: always, or (`only_when_full`) while a drive is nearly full.
    fn shown(&self) -> bool {
        self.opts.flag("only_when_full") != Some(true) || self.drives.iter().any(|d| d.share >= self.warn())
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
            let fill = meter_fill(cx, d.share >= self.warn());
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
    fn tooltip(&self) -> Option<String> {
        let list: Vec<String> = self.drives.iter().map(|d| format!("{} {}", d.id, d.text.text)).collect();
        Some(if list.is_empty() { "Disk usage".into() } else { list.join("\n") })
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
        if !self.shown() {
            return 0.0;
        }
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        if self.shown() {
            row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
        }
    }
    fn animating(&self, now: Instant) -> bool {
        self.drives.iter().any(|d| d.text.animating(now))
    }
    fn click(&mut self, button: Button, _: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        match self.opts.str("style") {
            Some("drives" | "rings") => Reply::TogglePanel,
            _ => Reply::Do(Act::open("shell:MyComputerFolder")),
        }
    }
}

// ----------------------------------------------------------------------

struct Item {
    icon: String,
    label: Option<String>,
    action: String,
    target: Option<String>,
    confirm: bool,
    /// A group's entries (`items`): they slide out of it when it's clicked.
    kids: Vec<Item>,
}

impl Item {
    /// From config: a button (`action`), or a group (`items`, any action ignored).
    fn parse(it: &toml::Value, nested: bool) -> Option<Item> {
        let s = |k: &str| it.get(k).and_then(|v| v.as_str()).map(String::from);
        let group = !nested && it.get("items").is_some();
        let kids: Vec<Item> = if group { it.get("items").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|k| Item::parse(k, true)).collect()).unwrap_or_default() } else { Vec::new() };
        Some(Item {
            label: s("label"),
            action: if group { "group".into() } else { s("action")? },
            icon: s("icon").unwrap_or_else(|| BUTTON.into()),
            target: s("target"),
            confirm: it.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false),
            kids,
        })
    }
}

/// A button's place: its index, and its index among its group's entries.
type At = (usize, Option<usize>);

/// Where a button is drawn (local x) and how far it has come out (a group's entries).
struct Slot {
    at: At,
    x: f32,
    alpha: f32,
}

/// A group coming out or going back in.
const SLIDE: Duration = Duration::from_millis(220);
/// The cursor gone this long: an open group goes back in.
const LINGER: Duration = Duration::from_millis(1200);

/// A row of icon buttons running menu actions (`items`); destructive ones take a second
/// click. A group (an item with `items`) slides its entries out beside it when clicked,
/// and back in when clicked again, when one of them runs, or when the cursor has left.
pub struct Buttons {
    items: Vec<Item>,
    hovered: Option<At>,
    armed: Option<(At, Instant)>,
    /// The row height as last drawn (buttons size with it).
    h: f32,
    /// The group out (or going back in), and its slide: from, to, since.
    group: Option<usize>,
    slide: (f32, f32, Instant),
    /// The slide's last frame drawn.
    settled: bool,
    /// The cursor left at (an open group goes back in after LINGER).
    left: Option<Instant>,
    /// The next frame while it slides.
    beat: Instant,
}

impl Buttons {
    pub fn new(opts: Opts) -> Buttons {
        let items = opts.table.get("items").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|it| Item::parse(it, false)).collect()).unwrap_or_default();
        let now = Instant::now();
        Buttons { items, hovered: None, armed: None, h: 30.0, group: None, slide: (0.0, 0.0, now), settled: true, left: None, beat: now }
    }

    fn item(&self, at: At) -> Option<&Item> {
        let it = self.items.get(at.0)?;
        match at.1 {
            Some(k) => it.kids.get(k),
            None => Some(it),
        }
    }

    /// How far the group is out (0..1, eased).
    fn out(&self, now: Instant) -> f32 {
        let (from, to, since) = self.slide;
        let k = (now.saturating_duration_since(since).as_secs_f32() / SLIDE.as_secs_f32()).min(1.0);
        let k = 1.0 - (1.0 - k).powi(3);
        from + (to - from) * k
    }

    fn sliding(&self, now: Instant) -> bool {
        now < self.slide.2 + SLIDE && self.slide.0 != self.slide.1
    }

    /// Sends the group `g` out (`true`) or back in, from where it is now.
    fn slide_to(&mut self, g: usize, open: bool) {
        let now = Instant::now();
        let from = if self.group == Some(g) { self.out(now) } else { 0.0 };
        self.group = Some(g);
        self.slide = (from, if open { 1.0 } else { 0.0 }, now);
        self.settled = false;
        self.beat = now;
        self.left = None;
    }

    /// The group that's out (or coming out).
    fn open(&self) -> Option<usize> {
        self.group.filter(|_| self.slide.1 > 0.5)
    }

    /// The buttons' places now, and the row's width. A group's entries spread out from
    /// behind it (fading in) as it opens; the buttons after it make room.
    fn slots(&self, h: f32, now: Instant) -> (Vec<Slot>, f32) {
        let (side, gap) = (h * 0.8, h * 0.1);
        let e = self.out(now);
        let mut out = Vec::new();
        let mut x = 0.0;
        for (i, it) in self.items.iter().enumerate() {
            if i > 0 {
                x += gap;
            }
            out.push(Slot { at: (i, None), x, alpha: 1.0 });
            x += side;
            if self.group == Some(i) && e > 0.001 {
                for k in 0..it.kids.len() {
                    out.push(Slot { at: (i, Some(k)), x: x + (gap + k as f32 * (side + gap)) * e, alpha: e });
                }
                x += it.kids.len() as f32 * (side + gap) * e;
            }
        }
        (out, x)
    }

    /// The button under a local x (not one still mostly hidden).
    fn at(&self, x: f32) -> Option<At> {
        let side = self.h * 0.8;
        let (slots, _) = self.slots(self.h, Instant::now());
        // The last drawn wins (entries over their group while they come out).
        slots.iter().rev().find(|s| s.alpha > 0.5 && x >= s.x && x < s.x + side).map(|s| s.at)
    }
}

impl Widget for Buttons {
    fn tooltip(&self) -> Option<String> {
        let at = self.hovered?;
        let it = self.item(at)?;
        let label = it.label.clone().unwrap_or_else(|| it.action.clone());
        Some(if self.armed.is_some_and(|(a, _)| a == at) { format!("Click again: {label}") } else { label })
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        self.h = cx.h;
        self.slots(cx.h, Instant::now()).1
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        // .btn: 0.8 h square, 0.45 h icon.
        let h = cx.h;
        let (slots, _) = self.slots(h, Instant::now());
        for s in &slots {
            let Some(it) = self.item(s.at) else { continue };
            let bg = if self.armed.is_some_and(|(a, _)| a == s.at) {
                Some(rgba(1.0, 80.0 / 255.0, 80.0 / 255.0, 0.35))
            } else if self.hovered == Some(s.at) || (s.at.1.is_none() && self.open() == Some(s.at.0)) {
                // Hovered, or a group that's out.
                Some(cx.hover_bg(0.12))
            } else {
                None
            };
            let piece = Piece::Button { side: h * 0.8, glyph: it.icon.clone(), size: h * 0.45, color: cx.look.fg, bg };
            row_draw(cx.gfx, &[piece], x + s.x, cy, alpha * s.alpha);
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|(x, _)| self.at(x));
        // Gone: an open group goes back in a little later (back in time: it stays).
        match at {
            None if self.open().is_some() && self.left.is_none() => self.left = Some(Instant::now()),
            Some(_) => self.left = None,
            _ => {}
        }
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn wake(&self) -> Option<Instant> {
        let slide = (!self.settled).then_some(self.beat);
        let linger = self.left.filter(|_| self.open().is_some()).map(|t| t + LINGER);
        [self.armed.map(|(_, t)| t), slide, linger].into_iter().flatten().min()
    }
    fn tick(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.armed.is_some_and(|(_, t)| now >= t) {
            self.armed = None;
            changed = true;
        }
        if let (Some(t), Some(g)) = (self.left, self.open()) {
            if now >= t + LINGER {
                self.slide_to(g, false);
            }
        }
        if !self.settled {
            // A frame of the slide; at its end, the last one.
            if self.sliding(now) {
                self.beat = now + Duration::from_millis(15);
            } else {
                self.settled = true;
            }
            changed = true;
        }
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let Some(a) = self.at(at.0).filter(|_| button == Button::Left) else { return Reply::None };
        let Some(it) = self.item(a) else { return Reply::None };
        // A group: out, or back in (another one out goes in at once).
        if a.1.is_none() && it.action == "group" {
            let open = self.open() != Some(a.0);
            self.slide_to(a.0, open);
            return Reply::Redraw;
        }
        if it.confirm && !self.armed.is_some_and(|(x, _)| x == a) {
            self.armed = Some((a, Instant::now() + Duration::from_secs(3)));
            return Reply::Redraw;
        }
        let act = Act::Run { action: it.action.clone(), target: it.target.clone() };
        self.armed = None;
        // One of a group's entries: the group goes back in.
        if a.1.is_some() {
            self.slide_to(a.0, false);
        }
        Reply::Do(act)
    }
}

// ----------------------------------------------------------------------

/// The keyboard layout's code (`format` "two": EN, else ENG); a click switches to the next one.
pub struct Language {
    opts: Opts,
    code: String,
    /// The layout's full name (its tooltip).
    name: String,
}

impl Language {
    pub fn new(opts: Opts) -> Language {
        Language { opts, code: String::new(), name: String::new() }
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
        self.name = v.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        changed
    }
    fn tooltip(&self) -> Option<String> {
        Some(if self.name.is_empty() { "Keyboard layout".into() } else { format!("{} (click to switch)", self.name) })
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
    fn tooltip(&self) -> Option<String> {
        let w = self.spaces.get(self.hovered?)?;
        Some(match w.windows {
            0 => format!("{} · empty", w.name),
            n => format!("{} · {n} window{}", w.name, if n == 1 { "" } else { "s" }),
        })
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
