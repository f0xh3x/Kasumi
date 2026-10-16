//! Native widgets: the ports of src/widgets/*. A widget measures itself, draws
//! itself in a row (centred vertically on it), and answers the cursor; a panel
//! is what drops out of the bar or a notch for it. Both may take data keys the
//! providers publish (the same JSON the pages got).

mod clock;
mod menu;

use std::time::Instant;

use serde_json::Value;

use super::gfx::{hex, rgba, white, with_alpha, Color, Family, Font, Gfx};
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
#[derive(Clone, Debug)]
pub enum Act {
    /// actions.rs `run_action`.
    Run { action: String, target: Option<String> },
}

/// What a click asks for.
#[derive(Clone, Debug)]
pub enum Reply {
    None,
    /// The widget's own panel: open or close it.
    TogglePanel,
    /// Close the open panel and do something.
    CloseAnd(Act),
    /// Redraw (it changed how it looks).
    Redraw,
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
        "menu" => Some(name.to_string()),
        _ => None,
    }
}

pub fn create(name: &str, opts: Opts) -> Box<dyn Widget> {
    match widget_type(name) {
        "clock" => Box::new(clock::Clock::new(opts)),
        "menu" => Box::new(menu::MenuButton::default()),
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
