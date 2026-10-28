//! Batteries (providers/batteries.rs): the laptop's and the Bluetooth devices'. In the
//! bar the lowest of them, or each ([widgets.batteries] show = "all"); its panel lists
//! them with a meter each, the laptop's charge and time left first.

use std::time::Instant;

use serde_json::Value;

use super::shell::battery_glyph;
use super::{fit_text, row_draw, row_width, Button, Ctx, Opts, Panel, Piece, Reply, Widget};
use crate::native::gfx::{rect, white, with_alpha, Font, Gfx};

/// A battery: name, percent, charging, and what's said of it (the laptop's time left).
#[derive(Clone, PartialEq)]
struct Cell {
    name: String,
    percent: f64,
    charging: bool,
    note: String,
    laptop: bool,
}

/// What the data says: the laptop first (if it has a battery), then the devices.
fn cells(v: &Value) -> Vec<Cell> {
    let mut out = Vec::new();
    if let Some(b) = v.get("laptop").filter(|b| !b.is_null()) {
        let charging = b.get("charging").and_then(|c| c.as_bool()).unwrap_or(false);
        let plugged = b.get("plugged").and_then(|c| c.as_bool()).unwrap_or(false);
        let note = if charging {
            "Charging".to_string()
        } else if plugged {
            "Plugged in".to_string()
        } else {
            b.get("secondsLeft").and_then(|s| s.as_f64()).filter(|s| *s > 0.0).map_or(String::new(), |s| {
                let (h, m) = ((s / 3600.0).floor(), ((s % 3600.0) / 60.0).round());
                if h > 0.0 { format!("{h} h {m} min left") } else { format!("{m} min left") }
            })
        };
        if let Some(p) = b.get("percent").and_then(|p| p.as_f64()) {
            out.push(Cell { name: "This PC".into(), percent: p, charging, note, laptop: true });
        }
    }
    for d in v.get("devices").and_then(|d| d.as_array()).into_iter().flatten() {
        let name = d.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        if let Some(p) = d.get("percent").and_then(|p| p.as_f64()) {
            out.push(Cell { name, percent: p, charging: false, note: String::new(), laptop: false });
        }
    }
    out
}

/// At or under this, a battery shows in the warning colour.
const LOW: f64 = 20.0;

pub struct Batteries {
    opts: Opts,
    cells: Vec<Cell>,
}

impl Batteries {
    pub fn new(opts: Opts) -> Batteries {
        Batteries { opts, cells: Vec::new() }
    }
    /// The ones shown in the bar: all of them, or the lowest.
    fn shown(&self) -> Vec<&Cell> {
        let laptop = self.opts.flag("laptop") != Some(false);
        let list: Vec<&Cell> = self.cells.iter().filter(|c| laptop || !c.laptop).collect();
        if self.opts.str("show") == Some("all") {
            return list;
        }
        list.into_iter().min_by(|a, b| a.percent.total_cmp(&b.percent)).into_iter().collect()
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let body = cx.body();
        let mut out = Vec::new();
        for (i, c) in self.shown().into_iter().enumerate() {
            if i > 0 {
                out.push(Piece::Gap(12.0));
            }
            let color = if c.percent <= LOW && !c.charging { cx.look.warn } else { cx.look.fg };
            out.push(Piece::glyph(battery_glyph(c.percent, c.charging), 14.0, color));
            out.push(Piece::Gap(4.0));
            out.push(Piece::text(format!("{}%", c.percent.round()), body.clone(), color));
        }
        if out.is_empty() {
            out.push(Piece::glyph(battery_glyph(0.0, false), 14.0, cx.look.dimmed()));
        }
        out
    }
}

impl Widget for Batteries {
    fn keys(&self) -> &'static [&'static str] {
        &["batteries"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let cells = cells(v);
        let changed = cells != self.cells;
        self.cells = cells;
        changed
    }
    fn tooltip(&self) -> Option<String> {
        let list: Vec<String> = self.cells.iter().map(|c| format!("{} {}%", c.name, c.percent.round())).collect();
        Some(if list.is_empty() { "No batteries found".into() } else { list.join("\n") })
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn click(&mut self, button: Button, _: (f32, f32)) -> Reply {
        if button == Button::Left { Reply::TogglePanel } else { Reply::None }
    }
}

// ----------------------------------------------------------------------

const ROW_H: f32 = 34.0;
const W: f32 = 320.0;

pub struct BatteriesPanel {
    cells: Vec<Cell>,
}

impl BatteriesPanel {
    pub fn new() -> BatteriesPanel {
        BatteriesPanel { cells: Vec::new() }
    }
}

fn line(g: &Gfx, f: &Font) -> f32 {
    g.text("Ag", f).map_or(f.size * 1.21, |t| t.height)
}

impl Panel for BatteriesPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["batteries"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.cells = cells(v);
        true
    }
    fn pad(&self) -> (f32, f32, f32) {
        (12.0, 12.0, 14.0)
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        if self.cells.is_empty() {
            return (W, line(cx.gfx, &cx.secondary(12.0)));
        }
        (W, self.cells.len() as f32 * ROW_H - 6.0)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let small = cx.secondary(12.0);
        if self.cells.is_empty() {
            g.text_at("No batteries found (laptop or Bluetooth devices)", &small, x, y + line(g, &small) / 2.0, dim);
            return;
        }
        let body = cx.body();
        let fill = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
        for (i, c) in self.cells.iter().enumerate() {
            let top = y + i as f32 * ROW_H;
            let low = c.percent <= LOW && !c.charging;
            let color = with_alpha(if low { cx.look.warn } else { cx.look.fg }, alpha);
            // Icon, name (and the laptop's note) above a meter, the percent at the end.
            let glyph = if c.laptop { battery_glyph(c.percent, c.charging) } else { "\u{E702}".to_string() };
            let f = Font::icons(14.0);
            let gw = g.measure(&glyph, &f);
            g.text_at(&glyph, &f, x + (18.0 - gw) / 2.0, top + 9.0, if c.laptop { color } else { fg });
            let pct = format!("{}%", c.percent.round());
            let pw = g.measure(&pct, &body);
            g.text_at(&pct, &body, x + W - pw, top + 9.0, color);
            let tx = x + 28.0;
            let room = W - 28.0 - pw - 12.0;
            let nw = g.measure(&c.name, &body).min(room);
            g.text_at(&fit_text(g, &c.name, &body, room), &body, tx, top + 9.0, fg);
            if !c.note.is_empty() && room - nw > 40.0 {
                g.text_at(&fit_text(g, &format!("· {}", c.note), &small, room - nw - 8.0), &small, tx + nw + 8.0, top + 9.0, dim);
            }
            let mw = W - 28.0;
            g.fill_round(rect(tx, top + 21.0, mw, 4.0), 2.0, white(0.15 * alpha));
            let k = (c.percent / 100.0).clamp(0.0, 1.0) as f32;
            if k > 0.0 {
                g.fill_round(rect(tx, top + 21.0, mw * k, 4.0), 2.0, if low { color } else { with_alpha(fill, alpha) });
            }
        }
    }
    fn tick(&mut self, _now: Instant) -> bool {
        false
    }
}
