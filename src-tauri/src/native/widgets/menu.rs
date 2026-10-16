//! The menu widget (its icon) and its panel (src/widgets/menu.ts).

use std::time::{Duration, Instant};

use super::{glyph, Act, Button, Ctx, Opts, Panel, Reply, Widget};
use crate::native::gfx::{rect, rgba, white, with_alpha, Font};

/// The menu button in the bar or a notch: the hamburger glyph.
#[derive(Default)]
pub struct MenuButton {
    width: f32,
}

impl Widget for MenuButton {
    fn width(&mut self, cx: &Ctx) -> f32 {
        self.width = cx.gfx.measure(glyph::MENU, &Font::icons(14.0));
        self.width
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        // .icon is nudged 1 px down (styles.css).
        cx.gfx.text_at(glyph::MENU, &Font::icons(14.0), x, cy + 1.0, with_alpha(cx.look.fg, alpha));
    }
    fn icon_x(&self) -> Option<f32> {
        Some(self.width / 2.0)
    }
}

struct Item {
    label: String,
    icon: String,
    action: String,
    target: Option<String>,
    confirm: bool,
    separator: bool,
}

/// The dropdown: `[widgets.menu] items`, destructive ones asking for a second click.
pub struct Menu {
    items: Vec<Item>,
    hovered: Option<usize>,
    /// The item armed by a first click (confirm), until when.
    armed: Option<(usize, Instant)>,
    /// Row height and content width as last laid out (hit tests and row fills).
    row_h: f32,
    width: f32,
}

// styles.css .menu-item: 8 px 12 px padding, 12 px gap, a 16 px icon column; .menu-sep: 1 px, 5 px 12 px margins.
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 8.0;
const ICON_W: f32 = 16.0;
const GAP: f32 = 12.0;
const SEP_H: f32 = 11.0;

impl Menu {
    pub fn new(opts: &Opts) -> Menu {
        let items = opts
            .table
            .get("items")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .map(|it| {
                        let s = |k: &str| it.get(k).and_then(|v| v.as_str()).map(String::from);
                        let action = s("action").unwrap_or_default();
                        Item {
                            label: s("label").unwrap_or_else(|| if action.is_empty() { "?".into() } else { action.clone() }),
                            icon: s("icon").unwrap_or_default(),
                            target: s("target"),
                            action,
                            confirm: it.get("confirm").and_then(|v| v.as_bool()).unwrap_or(false),
                            separator: it.get("separator").and_then(|v| v.as_bool()).unwrap_or(false),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();
        Menu { items, hovered: None, armed: None, row_h: 32.0, width: 0.0 }
    }

    /// Each item's top and height in the list.
    fn rows(&self) -> Vec<(f32, f32)> {
        let mut y = 0.0;
        self.items
            .iter()
            .map(|it| {
                let h = if it.separator { SEP_H } else { self.row_h };
                let r = (y, h);
                y += h;
                r
            })
            .collect()
    }

    fn label(&self, i: usize) -> String {
        let it = &self.items[i];
        if self.armed.is_some_and(|(a, _)| a == i) {
            format!("Click again to {}", it.label.to_lowercase())
        } else {
            it.label.clone()
        }
    }

    fn hit(&self, y: f32) -> Option<usize> {
        self.rows().iter().position(|&(top, h)| y >= top && y < top + h).filter(|&i| !self.items[i].separator)
    }
}

impl Panel for Menu {
    fn pad(&self) -> (f32, f32, f32) {
        (6.0, 10.0, 6.0)
    }
    fn wake(&self) -> Option<Instant> {
        self.armed.map(|(_, until)| until)
    }
    fn tick(&mut self, now: Instant) -> bool {
        if self.armed.is_some_and(|(_, until)| now >= until) {
            self.armed = None;
            return true;
        }
        false
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        self.row_h = cx.gfx.text("Ag", &cx.body()).map_or(16.0, |t| t.height) + 2.0 * PAD_Y;
        let body = cx.body();
        self.width = (0..self.items.len())
            .filter(|&i| !self.items[i].separator)
            .map(|i| PAD_X + ICON_W + GAP + cx.gfx.measure(&self.label(i), &body) + PAD_X)
            .fold(0.0, f32::max)
            .ceil();
        let h = self.rows().last().map_or(0.0, |(top, h)| top + h);
        (self.width, h)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let body = cx.body();
        let icon = Font::icons(14.0);
        let fg = with_alpha(cx.look.fg, alpha);
        for (i, (top, h)) in self.rows().into_iter().enumerate() {
            let it = &self.items[i];
            let top = y + top;
            if it.separator {
                g.fill_rect(rect(x + PAD_X, top + 5.0, self.width - 2.0 * PAD_X, 1.0), white(0.12 * alpha));
                continue;
            }
            if self.armed.is_some_and(|(a, _)| a == i) {
                g.fill_round(rect(x, top, self.width, h), 10.0, rgba(1.0, 80.0 / 255.0, 80.0 / 255.0, 0.22 * alpha));
            } else if self.hovered == Some(i) {
                g.fill_round(rect(x, top, self.width, h), 10.0, with_alpha(cx.hover_bg(0.1), alpha));
            }
            let iw = g.measure(&it.icon, &icon);
            g.text_at(&it.icon, &icon, x + PAD_X + (ICON_W - iw) / 2.0, top + h / 2.0 + 1.0, fg);
            g.text_at(&self.label(i), &body, x + PAD_X + ICON_W + GAP, top + h / 2.0, fg);
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|(_, y)| self.hit(y));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, _button: Button, at: (f32, f32)) -> Reply {
        let Some(i) = self.hit(at.1) else { return Reply::None };
        let it = &self.items[i];
        // Destructive items need a second click while armed (3 s).
        if it.confirm && !self.armed.is_some_and(|(a, _)| a == i) {
            self.armed = Some((i, Instant::now() + Duration::from_secs(3)));
            return Reply::Redraw;
        }
        self.armed = None;
        Reply::CloseAnd(Act::Run { action: it.action.clone(), target: it.target.clone() })
    }
    fn icon_x(&self) -> Option<f32> {
        Some(PAD_X + ICON_W / 2.0)
    }
    fn closed(&mut self) {
        self.hovered = None;
        self.armed = None;
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        self.rows().into_iter().map(|(top, h)| (0.0, top, self.width, h)).collect()
    }
}
