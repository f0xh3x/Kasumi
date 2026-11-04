//! The menu widget (its icon) and its panel (src/widgets/menu.ts), in a look
//! ([widgets.menu] style): a list, a grid, a strip of icons, or a greeting over the list.

use std::time::{Duration, Instant};

use super::{glyph, Act, Button, Ctx, Opts, Panel, Reply, Widget};
use crate::native::gfx::{rect, rgba, white, with_alpha, Font};

/// The menu button in the bar or a notch: the hamburger glyph.
#[derive(Default)]
pub struct MenuButton {
    width: f32,
}

impl Widget for MenuButton {
    fn tooltip(&self) -> Option<String> {
        Some("Menu".into())
    }
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

impl Item {
    /// Lock, sleep, sign out, restart...: apart from the places in the grid.
    fn power(&self) -> bool {
        matches!(self.action.as_str(), "lock" | "sleep" | "hibernate" | "sign_out" | "restart" | "shutdown")
    }

    /// Its Windows shortcut, where it has one (the greeting's list shows it).
    fn shortcut(&self) -> Option<&'static str> {
        let target = self.target.as_deref().unwrap_or("");
        match self.action.as_str() {
            "lock" => Some("Win L"),
            "settings" => Some("Win I"),
            "desktop" => Some("Win D"),
            _ if target.eq_ignore_ascii_case("shell:MyComputerFolder") || target.eq_ignore_ascii_case("explorer.exe") => Some("Win E"),
            _ if target.starts_with("ms-settings:") => Some("Win I"),
            _ if target.eq_ignore_ascii_case("taskmgr.exe") => Some("Ctrl Shift Esc"),
            _ => None,
        }
    }
}

/// The dropdown's look ([widgets.menu] style).
#[derive(Clone, Copy, PartialEq, Debug)]
enum MenuStyle {
    /// A list with icons, lines between groups.
    List,
    /// Places as tiles, the power actions as a row of icons under them.
    Grid,
    /// Icons in one row, the hovered one's name under it.
    Strip,
    /// A greeting with the date and the PC's time on, then the list with shortcuts.
    Greeting,
}

/// The dropdown: `[widgets.menu] items`, destructive ones asking for a second click,
/// in its look ([widgets.menu] style).
pub struct Menu {
    items: Vec<Item>,
    style: MenuStyle,
    hovered: Option<usize>,
    /// The item armed by a first click (confirm), until when.
    armed: Option<(usize, Instant)>,
    /// Row height and content width as last laid out (row fills), the items' boxes and
    /// the lines between groups (content px), the greeting's height.
    row_h: f32,
    width: f32,
    height: f32,
    boxes: Vec<(usize, [f32; 4])>,
    seps: Vec<[f32; 4]>,
    head_h: f32,
}

// styles.css .menu-item: 8 px 12 px padding, 12 px gap, a 16 px icon column; .menu-sep: 1 px, 5 px 12 px margins.
const PAD_X: f32 = 12.0;
const PAD_Y: f32 = 8.0;
const ICON_W: f32 = 16.0;
const GAP: f32 = 12.0;
const SEP_H: f32 = 11.0;
/// Grid: a tile; a power button. Strip: a button.
const CELL_W: f32 = 72.0;
const CELL_H: f32 = 58.0;
const POWER_W: f32 = 30.0;
const POWER_H: f32 = 28.0;
const STRIP: f32 = 32.0;

/// "Good morning" / "Good afternoon" / "Good evening", the date, how long Windows has been on.
fn greeting() -> (String, String) {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let hello = match t.wHour {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    };
    const DAYS: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    const MONTHS: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];
    let mins = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() } / 60_000;
    let up = match mins {
        m if m < 60 => format!("up {m} min"),
        m if m < 48 * 60 => format!("up {} h", m / 60),
        m => format!("up {} days", m / (24 * 60)),
    };
    (hello.to_string(), format!("{} {} {} · {up}", DAYS[t.wDayOfWeek as usize % 7], t.wDay, MONTHS[(t.wMonth as usize).saturating_sub(1) % 12]))
}

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
        let style = match opts.str("style") {
            Some("grid") => MenuStyle::Grid,
            Some("strip") => MenuStyle::Strip,
            Some("greeting") => MenuStyle::Greeting,
            _ => MenuStyle::List,
        };
        Menu { items, style, hovered: None, armed: None, row_h: 32.0, width: 0.0, height: 0.0, boxes: Vec::new(), seps: Vec::new(), head_h: 0.0 }
    }

    /// An item's text; armed, a short "Confirm?" (a longer ask widened the menu).
    fn label(&self, i: usize) -> String {
        if self.armed.is_some_and(|(a, _)| a == i) {
            "Confirm?".to_string()
        } else {
            self.items[i].label.clone()
        }
    }

    fn hit(&self, p: (f32, f32)) -> Option<usize> {
        self.boxes.iter().find(|(_, b)| p.0 >= b[0] && p.0 < b[0] + b[2] && p.1 >= b[1] && p.1 < b[1] + b[3]).map(|(i, _)| *i)
    }

    /// The list (and the greeting's, under its header): rows, a line between groups, at
    /// least `min_w` wide.
    fn lay_list(&mut self, cx: &Ctx, top: f32, min_w: f32) {
        let body = cx.body();
        let hints = self.style == MenuStyle::Greeting;
        let hint_f = cx.secondary(10.5);
        self.width = (0..self.items.len())
            .filter(|&i| !self.items[i].separator)
            .map(|i| {
                let hint = self.items[i].shortcut().filter(|_| hints).map_or(0.0, |h| 16.0 + cx.gfx.measure(h, &hint_f));
                PAD_X + ICON_W + GAP + cx.gfx.measure(&self.label(i), &body) + hint + PAD_X
            })
            .fold(min_w, f32::max)
            .ceil();
        let mut y = top;
        for (i, it) in self.items.iter().enumerate() {
            if it.separator {
                self.seps.push([PAD_X, y + 5.0, self.width - 2.0 * PAD_X, 1.0]);
                y += SEP_H;
            } else {
                self.boxes.push((i, [0.0, y, self.width, self.row_h]));
                y += self.row_h;
            }
        }
        self.height = y;
    }
}

impl Panel for Menu {
    fn pad(&self) -> (f32, f32, f32) {
        match self.style {
            MenuStyle::Greeting => (12.0, 10.0, 6.0),
            MenuStyle::Grid => (10.0, 12.0, 10.0),
            _ => (6.0, 10.0, 6.0),
        }
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
        let g = cx.gfx;
        self.row_h = g.text("Ag", &cx.body()).map_or(16.0, |t| t.height) + 2.0 * PAD_Y;
        self.boxes.clear();
        self.seps.clear();
        match self.style {
            MenuStyle::List => self.lay_list(cx, 0.0, 0.0),
            MenuStyle::Greeting => {
                let h = |f: &Font| g.text("Ag", f).map_or(f.size * 1.21, |t| t.height);
                self.head_h = 6.0 + h(&cx.main(18.0)) + 2.0 + h(&cx.secondary(12.0)) + 10.0;
                // As wide as the greeting and its date line too.
                let (hello, when) = greeting();
                let head_w = g.measure(&hello, &cx.main(18.0)).max(g.measure(&when, &cx.secondary(12.0))) + 2.0 * PAD_X;
                self.lay_list(cx, self.head_h + 7.0, head_w.ceil());
                self.seps.insert(0, [PAD_X - 6.0, self.head_h, 0.0, 1.0]);
                let w = self.width;
                self.seps[0][2] = w - 2.0 * (PAD_X - 6.0);
            }
            MenuStyle::Grid => {
                let places: Vec<usize> = (0..self.items.len()).filter(|&i| !self.items[i].separator && !self.items[i].power()).collect();
                let power: Vec<usize> = (0..self.items.len()).filter(|&i| self.items[i].power()).collect();
                let cols = places.len().clamp(1, 3);
                self.width = (cols as f32 * CELL_W + (cols as f32 - 1.0) * 6.0).max(power.len() as f32 * (POWER_W + 4.0));
                for (k, &i) in places.iter().enumerate() {
                    self.boxes.push((i, [(k % cols) as f32 * (CELL_W + 6.0), (k / cols) as f32 * (CELL_H + 6.0), CELL_W, CELL_H]));
                }
                let mut y = places.len().div_ceil(cols) as f32 * (CELL_H + 6.0) - 6.0;
                if !power.is_empty() {
                    if !places.is_empty() {
                        self.seps.push([0.0, y + 10.0, self.width, 1.0]);
                        y += 21.0;
                    }
                    let mut x = self.width - power.len() as f32 * (POWER_W + 4.0) + 4.0;
                    for &i in &power {
                        self.boxes.push((i, [x, y, POWER_W, POWER_H]));
                        x += POWER_W + 4.0;
                    }
                    y += POWER_H;
                }
                self.height = y.max(0.0);
            }
            MenuStyle::Strip => {
                let mut x = 0.0;
                for (i, it) in self.items.iter().enumerate() {
                    if it.separator {
                        self.seps.push([x + 3.0, 6.0, 1.0, STRIP - 12.0]);
                        x += 7.0;
                    } else {
                        self.boxes.push((i, [x, 0.0, STRIP, STRIP]));
                        x += STRIP + 4.0;
                    }
                }
                let lh = g.text("Ag", &cx.secondary(11.0)).map_or(14.0, |t| t.height);
                self.width = (x - 4.0).max(140.0);
                self.height = STRIP + 6.0 + lh;
            }
        }
        (self.width, self.height)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let body = cx.body();
        let icon = Font::icons(14.0);
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        for s in &self.seps {
            g.fill_rect(rect(x + s[0], y + s[1], s[2], s[3]), white(0.12 * alpha));
        }
        if self.style == MenuStyle::Greeting {
            let (hello, when) = greeting();
            let (hf, wf) = (cx.main(18.0), cx.secondary(12.0));
            let h = |f: &Font| g.text("Ag", f).map_or(f.size * 1.21, |t| t.height);
            g.text_at(&hello, &hf, x + PAD_X, y + 6.0 + h(&hf) / 2.0, fg);
            g.text_at(&when, &wf, x + PAD_X, y + 6.0 + h(&hf) + 2.0 + h(&wf) / 2.0, dim);
        }
        for &(i, b) in &self.boxes {
            let it = &self.items[i];
            let (bx, by) = (x + b[0], y + b[1]);
            let armed = self.armed.is_some_and(|(a, _)| a == i);
            let on = self.hovered == Some(i);
            let radius = if self.style == MenuStyle::List || self.style == MenuStyle::Greeting { 10.0 } else { 9.0 };
            let tile = self.style == MenuStyle::Grid && !it.power();
            if armed {
                g.fill_round(rect(bx, by, b[2], b[3]), radius, rgba(1.0, 80.0 / 255.0, 80.0 / 255.0, 0.22 * alpha));
            } else if on || tile {
                g.fill_round(rect(bx, by, b[2], b[3]), radius, if on { with_alpha(cx.hover_bg(0.14), alpha) } else { white(0.07 * alpha) });
            }
            let iw = g.measure(&it.icon, &icon);
            match self.style {
                MenuStyle::List | MenuStyle::Greeting => {
                    g.text_at(&it.icon, &icon, bx + PAD_X + (ICON_W - iw) / 2.0, by + b[3] / 2.0 + 1.0, fg);
                    g.text_at(&self.label(i), &body, bx + PAD_X + ICON_W + GAP, by + b[3] / 2.0, fg);
                    if let Some(hint) = it.shortcut().filter(|_| self.style == MenuStyle::Greeting && !armed) {
                        let f = cx.secondary(10.5);
                        let hw = g.measure(hint, &f);
                        g.text_at(hint, &f, bx + b[2] - PAD_X - hw, by + b[3] / 2.0, dim);
                    }
                }
                MenuStyle::Grid if tile => {
                    g.text_at(&it.icon, &icon, bx + (b[2] - iw) / 2.0, by + 20.0, fg);
                    let f = cx.secondary(10.5);
                    let t = super::fit_text(g, &self.label(i), &f, b[2] - 8.0);
                    let tw = g.measure(&t, &f);
                    g.text_at(&t, &f, bx + (b[2] - tw) / 2.0, by + 40.0, if on || armed { fg } else { dim });
                }
                _ => {
                    // Icon buttons (the grid's power row, the strip).
                    g.text_at(&it.icon, &icon, bx + (b[2] - iw) / 2.0, by + b[3] / 2.0 + 1.0, if on || armed { fg } else { dim });
                }
            }
        }
        if self.style == MenuStyle::Strip {
            // The hovered (or armed) button's name under the row.
            let shown = self.armed.map(|(a, _)| (a, true)).or(self.hovered.map(|h| (h, false)));
            if let Some((i, ask)) = shown {
                let f = cx.secondary(11.0);
                let text = if ask { format!("Click again to {}", self.items[i].label.to_lowercase()) } else { self.items[i].label.clone() };
                let lh = g.text("Ag", &f).map_or(14.0, |t| t.height);
                g.text_at(&text, &f, x + 4.0, y + STRIP + 6.0 + lh / 2.0, if ask { fg } else { dim });
            }
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.hit(p));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, _button: Button, at: (f32, f32)) -> Reply {
        let Some(i) = self.hit(at) else { return Reply::None };
        let it = &self.items[i];
        // Destructive items need a second click while armed (3 s).
        if it.confirm && !self.armed.is_some_and(|(a, _)| a == i) {
            self.armed = Some((i, Instant::now() + Duration::from_secs(3)));
            return Reply::Redraw;
        }
        self.armed = None;
        Reply::CloseAnd(Act::Run { action: it.action.clone(), target: it.target.clone() })
    }
    fn tooltip(&self) -> Option<String> {
        // The grid's power buttons have no names of their own.
        let i = self.hovered?;
        (self.style == MenuStyle::Grid && self.items[i].power()).then(|| if self.armed.is_some_and(|(a, _)| a == i) { format!("Click again to {}", self.items[i].label.to_lowercase()) } else { self.items[i].label.clone() })
    }
    fn icon_x(&self) -> Option<f32> {
        // The first icon under the menu button.
        Some(match self.style {
            MenuStyle::Grid => CELL_W / 2.0,
            MenuStyle::Strip => STRIP / 2.0,
            _ => PAD_X + ICON_W / 2.0,
        })
    }
    fn closed(&mut self) {
        self.hovered = None;
        self.armed = None;
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        let mut out: Vec<(f32, f32, f32, f32)> = self.boxes.iter().map(|(_, b)| (b[0], b[1], b[2], b[3])).collect();
        if self.style == MenuStyle::Greeting {
            out.insert(0, (0.0, 0.0, self.width, self.head_h));
        }
        out
    }
}
