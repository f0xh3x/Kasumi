//! Native widgets: the ports of src/widgets/*. A widget measures itself, draws
//! itself in a row (centred vertically on it), and answers the cursor; a panel
//! is what drops out of the bar or a notch for it. Both may take data keys the
//! providers publish (the same JSON the pages got).

mod basic;
mod clock;
mod extras;
pub mod icon;
mod menu;
mod shell;
mod tasks;
mod weather;

use std::time::Instant;

use serde_json::Value;

use super::gfx::{hex, rect, rgba, white, with_alpha, Color, Family, Font, Gfx};
use crate::engine::Layout;

/// Segoe Fluent Icons glyphs (src/widgets/index.ts `Glyph`).
pub mod glyph {
    pub const MENU: &str = "\u{E80F}";
}

/// The look of one surface: the theme, resolved.
#[derive(Clone)]
pub struct Look {
    pub bg: Color,
    /// Opacity of the shape ([theme] opacity), 0..1.
    pub opacity: f32,
    pub fg: Color,
    pub dim: f32,
    pub accent: Color,
    pub warn: Color,
    pub font_size: f32,
    pub main: Family,
    pub secondary: Family,
    /// A bottom surface (things that grow from the edge grow up).
    pub bottom: bool,
}

impl Look {
    pub fn from_layout(l: &Layout) -> Look {
        let t = &l.theme;
        Look {
            bg: hex(&t.background).unwrap_or(rgba(0.0, 0.0, 0.0, 1.0)),
            opacity: (t.opacity as f32 / 100.0).clamp(0.0, 1.0),
            fg: hex(&t.text).unwrap_or(white(1.0)),
            dim: t.dim_opacity as f32,
            accent: hex(&t.accent).unwrap_or(white(1.0)),
            warn: hex(&t.warn).unwrap_or(rgba(1.0, 0.62, 0.26, 1.0)),
            font_size: t.font_size as f32,
            main: Family::from_config(&t.font),
            secondary: if t.font_secondary.trim().is_empty() { Family::from_config(&t.font) } else { Family::from_config(&t.font_secondary) },
            bottom: l.edge == crate::config::Edge::Bottom,
        }
    }

    pub fn dimmed(&self) -> Color {
        with_alpha(self.fg, self.dim)
    }
}

/// A widget's options: its `[widgets.<name>]` table plus what the overlay
/// resolves for it (main.ts `opts()`).
#[derive(Clone, Default)]
pub struct Opts {
    pub table: toml::Table,
    /// Effects ([animation] or the widget's own `anim_<key>`).
    pub roll: bool,
    pub magnify: bool,
    pub glide: bool,
    pub cascade: bool,
    pub nudge: bool,
    /// Details in the accent colour.
    pub accent: bool,
    /// Its own fonts ("" = the theme's).
    pub font: String,
    pub font2: String,
    /// The monitor's device name.
    pub device: String,
}

impl Opts {
    pub fn str(&self, key: &str) -> Option<&str> {
        self.table.get(key).and_then(|v| v.as_str())
    }
    pub fn num(&self, key: &str) -> Option<f64> {
        self.table.get(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
    }
    pub fn flag(&self, key: &str) -> Option<bool> {
        self.table.get(key).and_then(|v| v.as_bool())
    }
}

/// What a widget or panel draws with.
pub struct Ctx<'a> {
    pub gfx: &'a Gfx,
    pub look: &'a Look,
    pub opts: &'a Opts,
    /// Docks: open progress (0 rest, 1 open) and the notch row's current height
    /// (the pages' --t / --h); bars: 1 and the row height.
    pub t: f32,
    pub h: f32,
    /// How open the bar is for open-only parts (media keys): 0 closed, 1 open (the pages' bar-open / --bar-fade).
    pub open: f32,
}

impl Ctx<'_> {
    /// The widget's main text font at a size ([theme] font, or its own).
    pub fn main(&self, size: f32) -> Font {
        Font::new(if self.opts.font.is_empty() { self.look.main.clone() } else { Family::from_config(&self.opts.font) }, size)
    }
    /// Secondary text (labels, details).
    pub fn secondary(&self, size: f32) -> Font {
        let family = if !self.opts.font2.is_empty() {
            Family::from_config(&self.opts.font2)
        } else if !self.opts.font.is_empty() {
            Family::from_config(&self.opts.font)
        } else {
            self.look.secondary.clone()
        };
        Font::new(family, size)
    }
    /// Body text: the theme's size.
    pub fn body(&self) -> Font {
        self.main(self.look.font_size)
    }
    /// A hovered row's or button's background: white at `plain`, or the accent at
    /// 30% with accent details (styles.css `--hover`).
    pub fn hover_bg(&self, plain: f32) -> Color {
        if self.opts.accent {
            with_alpha(self.look.accent, 0.3)
        } else {
            white(plain)
        }
    }
}

/// Which mouse button.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// Something to do once the surface is released (it may pump messages).
pub enum Act {
    /// actions.rs `run_action`.
    Run { action: String, target: Option<String> },
    /// Anything else the pages invoked: run with the app, on a thread of its own.
    Call(Box<dyn FnOnce(tauri::AppHandle) + Send>),
    /// A hosted tray icon clicked ("left" | "right"), at a point local to the widget
    /// (the surface makes it a screen point: apps put their menus there).
    Tray { id: String, button: &'static str, at: (f32, f32) },
}

impl Act {
    pub fn call(f: impl FnOnce(tauri::AppHandle) + Send + 'static) -> Act {
        Act::Call(Box::new(f))
    }
    pub fn open(target: &str) -> Act {
        Act::Run { action: "open".into(), target: Some(target.into()) }
    }
}

/// What a click asks for.
pub enum Reply {
    None,
    /// The widget's own panel: open or close it.
    TogglePanel,
    /// Its panel too, and something to do (the notifications bell marks them seen).
    ToggleAnd(Act),
    /// Close the open panel and do something.
    CloseAnd(Act),
    /// Do something, leaving panels as they are.
    Do(Act),
    /// Redraw (it changed how it looks).
    Redraw,
}

// ----------------------------------------------------------------------
// A widget as a row of pieces (most are: icons, texts, meters), measured and
// drawn left to right, centred on the row (the pages' flex rows).

/// Where a rolling number is (the pages' setText with `roll`).
#[derive(Clone)]
pub struct RollAt {
    pub prev: String,
    pub up: bool,
    /// Eased progress, 0 → 1.
    pub k: f32,
}

pub enum Piece {
    Text { s: String, font: Font, color: Color, roll: Option<RollAt> },
    /// An icon-font glyph; `dy` nudges it (.icon sits 1 px low).
    Glyph { s: String, size: f32, color: Color, dy: f32 },
    /// A Fluent SVG icon (20 px view box).
    Svg { d: &'static str, size: f32, color: Color },
    Gap(f32),
    /// A meter bar `len` long, `v` full.
    Bar { len: f32, v: f32, fill: Color },
    /// A meter ring (15 px), `v` round.
    Ring { v: f32, fill: Color },
    Dot { d: f32, color: Color },
    /// A rounded box (workspace dots).
    Pill { w: f32, h: f32, color: Color },
    /// A square button with a glyph in it (`.btn`): side, glyph size, background.
    Button { side: f32, glyph: String, size: f32, color: Color, bg: Option<Color> },
    /// Text in a padded box (workspace numbers): padding x / y, radius, background.
    Label { s: String, font: Font, color: Color, pad: (f32, f32), bg: Option<Color> },
}

impl Piece {
    pub fn text(s: impl Into<String>, font: Font, color: Color) -> Piece {
        Piece::Text { s: s.into(), font, color, roll: None }
    }
    pub fn glyph(s: impl Into<String>, size: f32, color: Color) -> Piece {
        Piece::Glyph { s: s.into(), size, color, dy: 1.0 }
    }

    pub fn width(&self, g: &Gfx) -> f32 {
        match self {
            Piece::Text { s, font, .. } => g.measure(s, font),
            Piece::Glyph { s, size, .. } => g.measure(s, &Font::icons(*size)),
            Piece::Svg { size, .. } => *size,
            Piece::Gap(w) => *w,
            Piece::Bar { len, .. } => *len,
            Piece::Ring { .. } => 15.0,
            Piece::Dot { d, .. } => *d,
            Piece::Pill { w, .. } => *w,
            Piece::Button { side, .. } => *side,
            Piece::Label { s, font, pad, .. } => g.measure(s, font) + 2.0 * pad.0,
        }
    }

    fn draw(&self, g: &Gfx, x: f32, cy: f32, alpha: f32) {
        let a = |c: &Color| with_alpha(*c, alpha);
        match self {
            Piece::Text { s, font, color, roll: None } => {
                g.text_at(s, font, x, cy, a(color));
            }
            Piece::Text { s, font, color, roll: Some(r) } => draw_rolling(g, s, r, font, x, cy, a(color)),
            Piece::Glyph { s, size, color, dy } => {
                g.text_at(s, &Font::icons(*size), x, cy + dy, a(color));
            }
            Piece::Svg { d, size, color } => g.draw_svg(d, 20.0, x, (cy - size / 2.0).round(), *size, a(color)),
            Piece::Gap(_) => {}
            Piece::Bar { len, v, fill } => {
                let r = rect(x, cy - 2.5, *len, 5.0);
                g.fill_round(r, 2.5, white(0.18 * alpha));
                if *v > 0.0 {
                    g.fill_round(rect(x, cy - 2.5, len * v.clamp(0.0, 1.0), 5.0), 2.5, a(fill));
                }
            }
            Piece::Ring { v, fill } => g.ring(x + 7.5, cy, 6.0 * 15.0 / 16.0, 2.5 * 15.0 / 16.0, *v, white(0.18 * alpha), a(fill)),
            Piece::Dot { d, color } => g.fill_round(rect(x, cy - d / 2.0, *d, *d), d / 2.0, a(color)),
            Piece::Pill { w, h, color } => g.fill_round(rect(x, cy - h / 2.0, *w, *h), h / 2.0, a(color)),
            Piece::Button { side, glyph, size, color, bg } => {
                if let Some(bg) = bg {
                    g.fill_round(rect(x, cy - side / 2.0, *side, *side), side * 0.25, a(bg));
                }
                let f = Font::icons(*size);
                let w = g.measure(glyph, &f);
                g.text_at(glyph, &f, x + (side - w) / 2.0, cy, a(color));
            }
            Piece::Label { s, font, color, pad, bg } => {
                let w = g.measure(s, font);
                if let Some(bg) = bg {
                    let h = g.text(s, font).map_or(16.0, |t| t.height) + 2.0 * pad.1;
                    g.fill_round(rect(x, cy - h / 2.0, w + 2.0 * pad.0, h), 6.0, a(bg));
                }
                g.text_at(s, font, x + pad.0, cy, a(color));
            }
        }
    }
}

pub fn row_width(g: &Gfx, pieces: &[Piece]) -> f32 {
    pieces.iter().map(|p| p.width(g)).sum()
}

pub fn row_draw(g: &Gfx, pieces: &[Piece], x: f32, cy: f32, alpha: f32) {
    let mut x = x;
    for p in pieces {
        p.draw(g, x, cy, alpha);
        x += p.width(g);
    }
}

/// The piece under a local x (gaps don't count).
pub fn row_hit(g: &Gfx, pieces: &[Piece], lx: f32) -> Option<usize> {
    let mut x = 0.0;
    for (i, p) in pieces.iter().enumerate() {
        let w = p.width(g);
        if !matches!(p, Piece::Gap(_)) && lx >= x && lx < x + w {
            return Some(i);
        }
        x += w;
    }
    None
}

/// Text cut to fit `max` px, with an ellipsis (text-overflow: ellipsis).
pub fn fit_text(g: &Gfx, s: &str, font: &Font, max: f32) -> String {
    if g.measure(s, font) <= max {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let t: String = chars[..mid].iter().collect::<String>() + "…";
        if g.measure(&t, font) <= max {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    chars[..lo].iter().collect::<String>().trim_end().to_string() + "…"
}

/// CSS cubic-bezier easing (x1, y1, x2, y2) at time t.
pub fn ease(c: (f32, f32, f32, f32), t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let bez = |a: f32, b: f32, s: f32| 3.0 * a * s * (1.0 - s) * (1.0 - s) + 3.0 * b * s * s * (1.0 - s) + s * s * s;
    // Solve x(s) = t by bisection (monotonic for CSS curves), then y(s).
    let (mut lo, mut hi) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.0;
        if bez(c.0, c.2, mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    bez(c.1, c.3, (lo + hi) / 2.0)
}

/// A number's text that rolls its changed digits to the new value ([animation] rolling).
#[derive(Default)]
pub struct Roll {
    pub text: String,
    prev: String,
    up: bool,
    start: Option<Instant>,
}

const ROLL_MS: f32 = 340.0;

impl Roll {
    /// New text; true if it changed. With `on`, digits roll (same length only).
    pub fn set(&mut self, s: String, on: bool) -> bool {
        if s == self.text {
            return false;
        }
        let num = |t: &str| t.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse::<f64>().unwrap_or(0.0);
        if on && !self.text.is_empty() && self.text.chars().count() == s.chars().count() {
            self.up = num(&s) >= num(&self.text);
            self.prev = std::mem::take(&mut self.text);
            self.start = Some(Instant::now());
        } else {
            self.start = None;
        }
        self.text = s;
        true
    }

    pub fn at(&self, now: Instant) -> Option<RollAt> {
        let start = self.start?;
        let t = now.saturating_duration_since(start).as_secs_f32() * 1000.0 / ROLL_MS;
        (t < 1.0).then(|| RollAt { prev: self.prev.clone(), up: self.up, k: ease((0.2, 0.8, 0.2, 1.0), t) })
    }

    pub fn animating(&self, now: Instant) -> bool {
        self.start.is_some_and(|s| now.saturating_duration_since(s).as_secs_f32() * 1000.0 < ROLL_MS + 20.0)
    }

    /// The text as a piece (rolling while it does).
    pub fn piece(&self, font: Font, color: Color) -> Piece {
        Piece::Text { s: self.text.clone(), font, color, roll: self.at(Instant::now()) }
    }
}

/// Text whose changed digits roll: up (the new one comes from below) or down.
fn draw_rolling(g: &Gfx, s: &str, r: &RollAt, font: &Font, x: f32, cy: f32, c: Color) {
    let line = g.text("0", font).map_or(16.0, |t| t.height);
    let prev: Vec<char> = r.prev.chars().collect();
    let mut cx = x;
    let mut run = String::new();
    let mut run_x = x;
    let flush = |run: &mut String, run_x: f32| {
        if !run.is_empty() {
            g.text_at(run, font, run_x, cy, c);
            run.clear();
        }
    };
    for (i, ch) in s.chars().enumerate() {
        let w = g.measure(&ch.to_string(), font);
        let old = prev.get(i).copied().unwrap_or(ch);
        if old != ch && old.is_ascii_digit() && ch.is_ascii_digit() {
            flush(&mut run, run_x);
            let top = (cy - line / 2.0).round();
            g.clip(rect(cx, top, w + 1.0, line));
            let (from, to) = if r.up { (-r.k * line, line - r.k * line) } else { (r.k * line, -line + r.k * line) };
            g.text_at(&old.to_string(), font, cx, cy + from, c);
            g.text_at(&ch.to_string(), font, cx, cy + to, c);
            g.unclip();
            run_x = cx + w;
        } else {
            if run.is_empty() {
                run_x = cx;
            }
            run.push(ch);
        }
        cx += w;
    }
    flush(&mut run, run_x);
}

pub trait Widget {
    /// Data keys it reads.
    fn keys(&self) -> &'static [&'static str] {
        &[]
    }
    /// New data under one of its keys. True: it looks different now.
    fn data(&mut self, _key: &str, _v: &Value) -> bool {
        false
    }
    /// When it next wants `tick` (a clock's next minute).
    fn wake(&self) -> Option<Instant> {
        None
    }
    /// True: it looks different now.
    fn tick(&mut self, _now: Instant) -> bool {
        false
    }
    /// Whether what `tick` changes can change its width (false: only redraw; a
    /// scrolling title moving a pixel).
    fn tick_resizes(&self) -> bool {
        true
    }
    /// Its width at the context's size (0: nothing to show, it takes no room).
    fn width(&mut self, cx: &Ctx) -> f32;
    /// Draws itself from x, centred on the row's middle `cy`.
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32);
    /// The cursor over it (local px from its left / the row's middle), or gone.
    fn hover(&mut self, _at: Option<(f32, f32)>) -> bool {
        false
    }
    fn click(&mut self, _button: Button, _at: (f32, f32)) -> Reply {
        Reply::TogglePanel
    }
    /// Its icon's centre x from its left (a menu panel lines its icons up under it).
    fn icon_x(&self) -> Option<f32> {
        None
    }
    /// Still moving (time-driven effects): keep drawing frames.
    fn animating(&self, _now: Instant) -> bool {
        false
    }
}

pub trait Panel {
    fn keys(&self) -> &'static [&'static str] {
        &[]
    }
    fn data(&mut self, _key: &str, _v: &Value) -> bool {
        false
    }
    fn wake(&self) -> Option<Instant> {
        None
    }
    fn tick(&mut self, _now: Instant) -> bool {
        false
    }
    /// Padding toward the bar, away from it, and on the sides (the page's .panel padding).
    fn pad(&self) -> (f32, f32, f32) {
        (10.0, 14.0, 16.0)
    }
    /// Its content's size (without padding).
    fn size(&mut self, cx: &Ctx) -> (f32, f32);
    /// Draws its content with its top-left at (x, y).
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32);
    fn hover(&mut self, _at: Option<(f32, f32)>) -> bool {
        false
    }
    fn click(&mut self, _button: Button, _at: (f32, f32)) -> Reply {
        Reply::None
    }
    /// Panels whose icon column lines up under the widget's icon (the menu): its
    /// centre's x in the content.
    fn icon_x(&self) -> Option<f32> {
        None
    }
    /// The panel has shut (forget armed buttons and hover).
    fn closed(&mut self) {}
}

/// `buttons#2` → `buttons`.
pub fn widget_type(name: &str) -> &str {
    name.split('#').next().unwrap_or(name)
}

/// The panel a widget opens, wherever it's placed (main.ts panelFor).
pub fn panel_for(name: &str) -> Option<String> {
    match widget_type(name) {
        "clock" => Some(name.replacen("clock", "calendar", 1)),
        "menu" | "tray" | "media" | "weather" | "temps" | "tasks" | "quick" | "launcher" | "privacy" | "claude" | "clipboard" | "notifications" => Some(name.to_string()),
        _ => None,
    }
}

pub fn create(name: &str, opts: Opts) -> Box<dyn Widget> {
    match widget_type(name) {
        "clock" => Box::new(clock::Clock::new(opts)),
        "menu" => Box::new(menu::MenuButton::default()),
        "temps" => Box::new(basic::Temps::new(opts)),
        "disk" => Box::new(basic::Disk::new(opts)),
        "buttons" => Box::new(basic::Buttons::new(opts)),
        "language" => Box::new(basic::Language::new(opts)),
        "workspaces" => Box::new(basic::Workspaces::new(opts)),
        "weather" => Box::new(weather::Weather::new(opts)),
        "focus" => Box::new(extras::Focus::new(opts)),
        "netspeed" => Box::new(extras::Netspeed::new(opts)),
        "privacy" => Box::new(extras::Privacy::new(opts)),
        "clipboard" => Box::new(extras::Clipboard::new(opts)),
        "notifications" => Box::new(extras::Notifications::new(opts)),
        "claude" => Box::new(extras::Claude::new(opts)),
        "tasks" => Box::new(tasks::Tasks::new(opts)),
        "media" => Box::new(shell::Media::new(opts)),
        "quick" => Box::new(shell::Quick::new(opts)),
        "tray" => Box::new(shell::Tray::new(opts)),
        _ => Box::new(Unknown(name.to_string())),
    }
}

pub fn create_panel(name: &str, opts: &Opts) -> Option<Box<dyn Panel>> {
    Some(match widget_type(name) {
        "calendar" => Box::new(clock::Calendar),
        "menu" => Box::new(menu::Menu::new(opts)),
        _ => return None,
    })
}

/// A widget not ported yet: its name, dimmed (as the page's `.widget.unknown`).
struct Unknown(String);

impl Widget for Unknown {
    fn width(&mut self, cx: &Ctx) -> f32 {
        cx.gfx.measure(&self.0, &cx.body())
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        cx.gfx.text_at(&self.0, &cx.body(), x, cy, with_alpha(cx.look.dimmed(), alpha));
    }
    fn click(&mut self, _: Button, _: (f32, f32)) -> Reply {
        Reply::None
    }
}
