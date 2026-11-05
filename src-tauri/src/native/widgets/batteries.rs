//! Batteries (providers/batteries.rs): the laptop's and the Bluetooth devices'. In the
//! bar the lowest of them, or each ([widgets.batteries] show = "all"), as a percent, a
//! small bar or just the icon (`level_as`), devices by kind (`device_icons`), or only
//! while one is low (`only_when_low`); its panel lists them with a meter each, the
//! laptop's charge and time left first, devices not connected greyed (`keep_disconnected`).

use std::time::Instant;

use serde_json::Value;

use super::shell::battery_glyph;
use super::{fit_text, row_draw, row_width, Button, Ctx, Opts, Panel, Piece, Reply, Widget};
use crate::native::gfx::{rect, white, with_alpha, Font, Gfx};

/// A battery: its id ("laptop", a device's), name, percent, charging, and what's said
/// of it (the laptop's time left); a device not connected now (`keep_disconnected`)
/// with when it was last seen.
#[derive(Clone, PartialEq)]
struct Cell {
    id: String,
    name: String,
    percent: f64,
    charging: bool,
    note: String,
    /// The laptop's time left, short ("3 h 10 min"), for the bar.
    left: String,
    laptop: bool,
    connected: bool,
    seen: Option<f64>,
}

/// What the data says: the laptop first (if it has a battery), then the devices, with
/// your own names ([widgets.batteries] rename).
fn cells(v: &Value, opts: &Opts) -> Vec<Cell> {
    let rename = opts.table.get("rename").and_then(|r| r.as_table());
    let name_of = |id: &str, name: String| rename.and_then(|r| r.get(id)).and_then(|n| n.as_str()).map(str::trim).filter(|n| !n.is_empty()).map(String::from).unwrap_or(name);
    let mut out = Vec::new();
    if let Some(b) = v.get("laptop").filter(|b| !b.is_null()) {
        let charging = b.get("charging").and_then(|c| c.as_bool()).unwrap_or(false);
        let plugged = b.get("plugged").and_then(|c| c.as_bool()).unwrap_or(false);
        let left = if charging || plugged {
            String::new()
        } else {
            b.get("secondsLeft").and_then(|s| s.as_f64()).filter(|s| *s > 0.0).map_or(String::new(), |s| {
                let (h, m) = ((s / 3600.0).floor(), ((s % 3600.0) / 60.0).round());
                if h > 0.0 { format!("{h} h {m} min") } else { format!("{m} min") }
            })
        };
        let note = if charging {
            "Charging".to_string()
        } else if plugged {
            "Plugged in".to_string()
        } else if left.is_empty() {
            String::new()
        } else {
            format!("{left} left")
        };
        if let Some(p) = b.get("percent").and_then(|p| p.as_f64()) {
            out.push(Cell { id: "laptop".into(), name: "This PC".into(), percent: p, charging, note, left, laptop: true, connected: true, seen: None });
        }
    }
    for d in v.get("devices").and_then(|d| d.as_array()).into_iter().flatten() {
        let name = d.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        let id = d.get("id").and_then(|n| n.as_str()).unwrap_or("").to_string();
        if let Some(p) = d.get("percent").and_then(|p| p.as_f64()) {
            let connected = d.get("connected").and_then(|c| c.as_bool()) != Some(false);
            let seen = d.get("seen").and_then(|s| s.as_f64());
            out.push(Cell { name: name_of(&id, name), id, percent: p, charging: false, note: String::new(), left: String::new(), laptop: false, connected, seen });
        }
    }
    out
}

/// A device's icon by what its name says it is (a mouse, headphones, a controller...),
/// else the Bluetooth sign.
fn kind_glyph(name: &str) -> &'static str {
    let n = name.to_lowercase();
    let any = |words: &[&str]| words.iter().any(|w| n.contains(w));
    if any(&["controller", "gamepad", "xbox", "dualsense", "dualshock", "joy-con", "8bitdo"]) {
        "\u{E7FC}"
    } else if any(&["buds", "airpods", "headphone", "headset", "earbud", "earphone", "wh-", "wf-", "jabra", "bose", "beats", "soundcore", "liberty", "freebuds"]) {
        "\u{E7F6}"
    } else if any(&["keyboard", "keys", "keychron", "k380", "k780"]) {
        "\u{E765}"
    } else if any(&["mouse", "mx master", "mx anywhere", "mx ergo", "trackball", "deathadder", "viper", "basilisk", "superlight", "g305", "g502", "g903"]) {
        "\u{E962}"
    } else if any(&["speaker", "soundlink", "jbl", "boom"]) {
        "\u{E7F5}"
    } else if any(&["phone", "iphone", "galaxy s", "pixel"]) {
        "\u{E8EA}"
    } else {
        "\u{E702}"
    }
}

/// How low counts as low ([widgets.batteries] low_at, 20 by default).
fn low_at(opts: &Opts) -> f64 {
    opts.num("low_at").unwrap_or(20.0)
}

/// How long ago, short: "5 min", "2 h", "3 days".
fn ago(unix: f64) -> String {
    let s = (super::extras::now_secs() - unix).max(0.0);
    if s < 3600.0 {
        format!("{} min", (s / 60.0).round().max(1.0))
    } else if s < 86400.0 {
        format!("{} h", (s / 3600.0).floor())
    } else {
        let d = (s / 86400.0).floor();
        format!("{d} day{}", if d == 1.0 { "" } else { "s" })
    }
}

pub struct Batteries {
    opts: Opts,
    cells: Vec<Cell>,
}

impl Batteries {
    pub fn new(opts: Opts) -> Batteries {
        Batteries { opts, cells: Vec::new() }
    }
    /// The ones shown in the bar: all of them, the ones picked ([widgets.batteries]
    /// show = "chosen", their ids in `shown`), or the lowest; connected ones only.
    fn shown(&self) -> Vec<&Cell> {
        let on = self.cells.iter().filter(|c| c.connected);
        match self.opts.str("show") {
            Some("chosen") => {
                let picked: Vec<&str> = self.opts.table.get("shown").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
                on.filter(|c| picked.contains(&c.id.as_str())).collect()
            }
            show => {
                let laptop = self.opts.flag("laptop") != Some(false);
                let list: Vec<&Cell> = on.filter(|c| laptop || !c.laptop).collect();
                if show == Some("all") {
                    return list;
                }
                list.into_iter().min_by(|a, b| a.percent.total_cmp(&b.percent)).into_iter().collect()
            }
        }
    }
    fn low(&self, c: &Cell) -> bool {
        c.percent <= low_at(&self.opts) && !c.charging
    }
    /// In the bar at all: always, or (`only_when_low`) while a battery is low.
    fn visible(&self) -> bool {
        self.opts.flag("only_when_low") != Some(true) || self.shown().iter().any(|c| self.low(c))
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let body = cx.body();
        let level_as = self.opts.str("level_as").unwrap_or("percent");
        let kinds = self.opts.str("device_icons") == Some("kind");
        let mut out = Vec::new();
        for (i, c) in self.shown().into_iter().enumerate() {
            if i > 0 {
                out.push(Piece::Gap(12.0));
            }
            let low = self.low(c);
            let color = if low { cx.look.warn } else { cx.look.fg };
            let glyph = if kinds && !c.laptop && level_as != "icon" { kind_glyph(&c.name).to_string() } else { battery_glyph(c.percent, c.charging) };
            out.push(Piece::glyph(glyph, 14.0, color));
            // Its name before the level ([widgets.batteries] names).
            if self.opts.flag("names") == Some(true) {
                out.push(Piece::Gap(5.0));
                out.push(Piece::text(c.name.clone(), body.clone(), cx.look.dimmed()));
            }
            match level_as {
                "icon" => {}
                "bar" => {
                    out.push(Piece::Gap(6.0));
                    out.push(Piece::Bar { len: 26.0, v: (c.percent / 100.0) as f32, fill: if low { cx.look.warn } else { super::basic::meter_fill(cx, false) } });
                }
                _ => {
                    out.push(Piece::Gap(4.0));
                    out.push(Piece::text(format!("{}%", c.percent.round()), body.clone(), color));
                }
            }
            // The laptop's time left ([widgets.batteries] time_left).
            if c.laptop && !c.left.is_empty() && self.opts.flag("time_left") == Some(true) {
                out.push(Piece::Gap(6.0));
                out.push(Piece::text(c.left.clone(), cx.secondary(cx.look.font_size - 1.0), cx.look.dimmed()));
            }
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
        let cells = cells(v, &self.opts);
        let changed = cells != self.cells;
        self.cells = cells;
        changed
    }
    fn tooltip(&self) -> Option<String> {
        let list: Vec<String> = self.cells.iter().filter(|c| c.connected).map(|c| format!("{} {}%", c.name, c.percent.round())).collect();
        Some(if list.is_empty() { "No batteries found".into() } else { list.join("\n") })
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        if !self.visible() {
            return 0.0;
        }
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        if self.visible() {
            row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
        }
    }
    fn click(&mut self, button: Button, _: (f32, f32)) -> Reply {
        if button == Button::Left { Reply::TogglePanel } else { Reply::None }
    }
}

// ----------------------------------------------------------------------

const ROW_H: f32 = 34.0;
const W: f32 = 320.0;

pub struct BatteriesPanel {
    opts: Opts,
    cells: Vec<Cell>,
}

impl BatteriesPanel {
    pub fn new(opts: &Opts) -> BatteriesPanel {
        BatteriesPanel { opts: opts.clone(), cells: Vec::new() }
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
        self.cells = cells(v, &self.opts);
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
        let small = cx.secondary(12.0);
        if self.cells.is_empty() {
            g.text_at("No batteries found (laptop or Bluetooth devices)", &small, x, y + line(g, &small) / 2.0, with_alpha(cx.look.dimmed(), alpha));
            return;
        }
        let body = cx.body();
        let fill = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
        let kinds = self.opts.str("device_icons") == Some("kind");
        for (i, c) in self.cells.iter().enumerate() {
            let top = y + i as f32 * ROW_H;
            // A disconnected device: greyed, with when it was last seen.
            let alpha = if c.connected { alpha } else { alpha * 0.45 };
            let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
            let low = c.connected && c.percent <= low_at(&self.opts) && !c.charging;
            let color = with_alpha(if low { cx.look.warn } else { cx.look.fg }, alpha);
            // Icon, name (and the laptop's note) above a meter, the percent at the end.
            let glyph = if c.laptop { battery_glyph(c.percent, c.charging) } else if kinds { kind_glyph(&c.name).to_string() } else { "\u{E702}".to_string() };
            let note = match c.seen.filter(|_| !c.connected) {
                Some(t) => format!("seen {} ago", ago(t)),
                None => c.note.clone(),
            };
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
            if !note.is_empty() && room - nw > 40.0 {
                g.text_at(&fit_text(g, &format!("· {note}"), &small, room - nw - 8.0), &small, tx + nw + 8.0, top + 9.0, dim);
            }
            let mw = W - 28.0;
            g.fill_round(rect(tx, top + 21.0, mw, 4.0), 2.0, white(0.15 * alpha));
            let k = (c.percent / 100.0).clamp(0.0, 1.0) as f32;
            if k > 0.0 {
                g.fill_round(rect(tx, top + 21.0, mw * k, 4.0), 2.0, if low { color } else if c.connected { with_alpha(fill, alpha) } else { fg });
            }
        }
    }
    fn tick(&mut self, _now: Instant) -> bool {
        false
    }
}
