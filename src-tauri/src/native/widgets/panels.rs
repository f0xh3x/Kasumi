//! Panels that only show (or nearly): weather (weather.ts weatherPanel), temps
//! (temps-panel.ts), privacy (extras.ts privacyPanel), tray (tray.ts trayPanel).

use std::collections::HashMap;

use serde_json::Value;

use super::icon::{decode, AppIcon, Style};
use super::weather::{deg, icon};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply};
use crate::native::gfx::{rect, white, with_alpha, Font, Gfx};

/// A line of text's height in a font.
fn line(g: &Gfx, f: &Font) -> f32 {
    g.text("Ag", f).map_or(f.size * 1.21, |t| t.height)
}

// ----------------------------------------------------------------------

/// Current conditions and the next hours (`hours`) or days (`forecast` = "daily").
pub struct WeatherPanel {
    opts: Opts,
    w: Value,
    width: f32,
}

const HOUR_W: f32 = 36.0;
const DAY_W: f32 = 40.0;

impl WeatherPanel {
    pub fn new(opts: &Opts) -> WeatherPanel {
        WeatherPanel { opts: opts.clone(), w: Value::Null, width: 0.0 }
    }
    fn describe(code: i64) -> &'static str {
        match code {
            0 => "Clear",
            1 => "Mostly clear",
            2 => "Partly cloudy",
            3 => "Overcast",
            45 | 48 => "Fog",
            51 | 53 | 55 | 56 | 57 => "Drizzle",
            61 | 63 | 65 => "Rain",
            66 | 67 => "Freezing rain",
            71 | 73 | 75 | 77 => "Snow",
            80..=82 => "Rain showers",
            85 | 86 => "Snow showers",
            95 | 96 | 99 => "Thunderstorm",
            _ => "",
        }
    }
    fn f(&self, k: &str) -> f64 {
        self.w.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }
    /// The strip: (time label, code, day, temp, min) per column.
    fn columns(&self) -> Vec<(String, i64, bool, String, Option<String>)> {
        let daily = self.opts.str("forecast") == Some("daily");
        let days = self.w.get("daily").and_then(|d| d.as_array()).filter(|d| !d.is_empty());
        if let (true, Some(days)) = (daily, days) {
            return days
                .iter()
                .enumerate()
                .map(|(i, d)| {
                    let date = d.get("date").and_then(|x| x.as_str()).unwrap_or("");
                    let label = if i == 0 {
                        "Today".to_string()
                    } else {
                        let p: Vec<i32> = date.split('-').filter_map(|x| x.parse().ok()).collect();
                        let wd = if p.len() == 3 { super::clock::weekday(super::clock::days_from_civil(p[0], p[1] as u32, p[2] as u32)) } else { 0 };
                        ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][wd as usize].to_string()
                    };
                    let g = |k: &str| d.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
                    (label, d.get("code").and_then(|c| c.as_i64()).unwrap_or(3), true, deg(g("max")), Some(deg(g("min"))))
                })
                .collect();
        }
        let hours = self.opts.num("hours").unwrap_or(8.0) as usize;
        self.w
            .get("hourly")
            .and_then(|h| h.as_array())
            .map(|h| {
                h.iter()
                    .take(hours + 1)
                    .enumerate()
                    .map(|(i, h)| {
                        let time = h.get("time").and_then(|t| t.as_str()).unwrap_or("");
                        let label = if i == 0 { "Now".into() } else { time.get(11..13).unwrap_or("").to_string() };
                        (label, h.get("code").and_then(|c| c.as_i64()).unwrap_or(3), h.get("isDay").and_then(|d| d.as_bool()).unwrap_or(true), deg(h.get("temp").and_then(|t| t.as_f64()).unwrap_or(0.0)), None)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Panel for WeatherPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["weather"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.w = v.clone();
        true
    }
    fn pad(&self) -> (f32, f32, f32) {
        (12.0, 14.0, 14.0)
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        if self.w.is_null() {
            let f = cx.secondary(12.0);
            return (g.measure("No weather data", &f), line(g, &f));
        }
        let temp_w = g.measure(&deg(self.f("temp")), &cx.main(24.0));
        let meta = [Self::describe(self.w.get("code").and_then(|c| c.as_i64()).unwrap_or(3)).to_string(), format!("Feels like {}", deg(self.f("feels"))), format!("{} / {}", deg(self.f("min")), deg(self.f("max")))];
        let meta_w = meta.iter().enumerate().map(|(i, m)| g.measure(m, &cx.secondary(if i == 0 { 12.0 } else { 11.0 }))).fold(0.0, f32::max);
        let place = self.w.get("location").and_then(|l| l.as_str()).unwrap_or("");
        let head = 26.0 + 10.0 + temp_w + 10.0 + 4.0 + meta_w + 10.0 + g.measure(place, &cx.secondary(11.0));
        let cols = self.columns();
        let daily = cols.first().is_some_and(|c| c.4.is_some());
        let strip = cols.len() as f32 * if daily { DAY_W } else { HOUR_W } + (cols.len().max(1) as f32 - 1.0) * 2.0;
        self.width = head.max(strip).ceil();
        let head_h = (line(g, &cx.secondary(12.0)) + 2.0 * line(g, &cx.secondary(11.0)) + 2.0).max(line(g, &cx.main(24.0)));
        let strip_h = 6.0 + line(g, &cx.secondary(10.0)) + 5.0 + 16.0 + 5.0 + line(g, &cx.secondary(11.0)) + if daily { line(g, &cx.secondary(10.0)) - 3.0 } else { 0.0 } + 6.0;
        (self.width, head_h + 12.0 + if cols.is_empty() { 0.0 } else { strip_h })
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        if self.w.is_null() {
            let f = cx.secondary(12.0);
            g.text_at("No weather data", &f, x, y + line(g, &f) / 2.0, fg);
            return;
        }
        let head_h = (line(g, &cx.secondary(12.0)) + 2.0 * line(g, &cx.secondary(11.0)) + 2.0).max(line(g, &cx.main(24.0)));
        let cy = y + head_h / 2.0;
        let code = self.w.get("code").and_then(|c| c.as_i64()).unwrap_or(3);
        let day = self.w.get("isDay").and_then(|d| d.as_bool()).unwrap_or(true);
        let mut cx_ = x;
        super::row_draw(g, &[icon(code, day, 26.0, cx.look.fg)], cx_, cy, alpha);
        cx_ += 26.0 + 10.0;
        let temp = deg(self.f("temp"));
        let temp_color = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
        cx_ += g.text_at(&temp, &cx.main(24.0), cx_, cy, temp_color) + 10.0 + 4.0;
        let meta = [
            (Self::describe(code).to_string(), cx.secondary(12.0), fg),
            (format!("Feels like {}", deg(self.f("feels"))), cx.secondary(11.0), dim),
            (format!("{} / {}", deg(self.f("min")), deg(self.f("max"))), cx.secondary(11.0), dim),
        ];
        let total: f32 = meta.iter().map(|(_, f, _)| line(g, f)).sum::<f32>() + 2.0;
        let mut my = cy - total / 2.0;
        for (t, f, c) in &meta {
            let h = line(g, f);
            g.text_at(t, f, cx_, my + h / 2.0, *c);
            my += h + 1.0;
        }
        let place = self.w.get("location").and_then(|l| l.as_str()).unwrap_or("");
        let pf = cx.secondary(11.0);
        let pw = g.measure(place, &pf);
        g.text_at(place, &pf, x + self.width - pw, y + line(g, &pf) / 2.0, dim);
        // The strip: the first column (now / today) highlighted.
        let cols = self.columns();
        let daily = cols.first().is_some_and(|c| c.4.is_some());
        let cw = if daily { DAY_W } else { HOUR_W };
        let top = y + head_h + 12.0;
        let (tf, vf, mf) = (cx.secondary(10.0), cx.secondary(11.0), cx.secondary(10.0));
        let h = 6.0 + line(g, &tf) + 5.0 + 16.0 + 5.0 + line(g, &vf) + if daily { line(g, &mf) - 3.0 } else { 0.0 } + 6.0;
        for (i, (label, code, isday, temp, min)) in cols.iter().enumerate() {
            let left = x + i as f32 * (cw + 2.0);
            if i == 0 {
                g.fill_round(rect(left, top, cw, h), 8.0, white(0.08 * alpha));
            }
            let centred = |s: &str, f: &Font, cy: f32, c| {
                let w = g.measure(s, f);
                g.text_at(s, f, left + (cw - w) / 2.0, cy, c);
            };
            let mut yy = top + 6.0;
            centred(label, &tf, yy + line(g, &tf) / 2.0, dim);
            yy += line(g, &tf) + 5.0;
            let ic = icon(*code, *isday, 16.0, cx.look.fg);
            let iw = ic.width(g);
            super::row_draw(g, &[ic], left + (cw - iw) / 2.0, yy + 8.0, alpha);
            yy += 16.0 + 5.0;
            centred(temp, &vf, yy + line(g, &vf) / 2.0, fg);
            yy += line(g, &vf) - 3.0;
            if let Some(m) = min {
                centred(m, &mf, yy + line(g, &mf) / 2.0, dim);
            }
        }
    }
}

// ----------------------------------------------------------------------

const HISTORY: usize = 60;

/// "AMD Ryzen 9 9955HX" → "Ryzen 9 9955HX", "NVIDIA GeForce RTX 5070 Laptop GPU" → "RTX 5070 Laptop".
fn short_name(name: &str) -> String {
    let mut n = name.trim();
    for p in ["AMD ", "Intel(R) ", "Intel ", "NVIDIA "] {
        if n.len() >= p.len() && n[..p.len()].eq_ignore_ascii_case(p) {
            n = &n[p.len()..];
        }
    }
    if n.len() >= 8 && n[..8].eq_ignore_ascii_case("GeForce ") {
        n = &n[8..];
    }
    n.strip_suffix(" GPU").unwrap_or(n).to_string()
}

fn fixed(v: Option<f64>, digits: usize) -> String {
    v.map_or("–".into(), |v| format!("{v:.digits$}"))
}

/// CPU / GPU cards (temperature, a 60 s sparkline, load, power) and memory, from LibreHardwareMonitor.
pub struct TempsPanel {
    opts: Opts,
    t: Value,
    history: [Vec<f32>; 2],
}

impl TempsPanel {
    pub fn new(opts: &Opts) -> TempsPanel {
        TempsPanel { opts: opts.clone(), t: Value::Null, history: [Vec::new(), Vec::new()] }
    }
    fn get(&self, k: &str) -> Option<f64> {
        self.t.get(k).and_then(|v| v.as_f64())
    }
    fn card_h(g: &Gfx, cx: &Ctx) -> f32 {
        line(g, &cx.secondary(11.0)) + 5.0 + 24.0f32.max(line(g, &cx.main(22.0))) + 5.0 + line(g, &cx.secondary(11.0)) + 5.0 + line(g, &cx.secondary(11.0))
    }
}

impl Panel for TempsPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["temps"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.t = v.clone();
        for (i, k) in ["cpu", "gpu"].iter().enumerate() {
            if let Some(t) = self.get(k) {
                self.history[i].push(t as f32);
                if self.history[i].len() > HISTORY {
                    self.history[i].remove(0);
                }
            }
        }
        true
    }
    fn pad(&self) -> (f32, f32, f32) {
        (12.0, 12.0, 14.0)
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        if !self.t.get("ok").and_then(|o| o.as_bool()).unwrap_or(false) {
            return (400.0, line(g, &cx.secondary(11.0)));
        }
        (400.0, Self::card_h(g, cx) + 10.0 + line(g, &cx.secondary(11.0)))
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let small = cx.secondary(11.0);
        let lh = line(g, &small);
        if !self.t.get("ok").and_then(|o| o.as_bool()).unwrap_or(false) {
            g.text_at("LibreHardwareMonitor web server not reachable", &small, x, y + lh / 2.0, dim);
            return;
        }
        let warn = self.opts.num("warn").unwrap_or(90.0);
        let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
        let bar = |left: f32, w: f32, cy: f32, v: Option<f64>| {
            g.fill_round(rect(left, cy - 2.0, w, 4.0), 2.0, white(0.15 * alpha));
            let v = (v.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
            if v > 0.0 {
                g.fill_round(rect(left, cy - 2.0, w * v, 4.0), 2.0, fill);
            }
        };
        let card_w = (400.0 - 18.0) / 2.0;
        let ghz = self.get("cpuClock").map_or(String::new(), |c| format!(" · {:.2} GHz", c / 1000.0));
        let vram = match (self.get("vramUsed"), self.get("vramTotal")) {
            (Some(u), Some(t)) => format!(" · VRAM {:.1} / {:.1} GB", u / 1024.0, t / 1024.0),
            _ => String::new(),
        };
        let cards = [
            ("CPU", "cpu", "cpuName", "cpuLoad", format!("{} W{ghz}", fixed(self.get("cpuPower"), 0))),
            ("GPU", "gpu", "gpuName", "gpuLoad", format!("{} W{vram}", fixed(self.get("gpuPower"), 0))),
        ];
        for (i, (label, temp, name, load, detail)) in cards.iter().enumerate() {
            let left = x + i as f32 * (card_w + 18.0);
            let mut yy = y;
            let lw = g.text_at(label, &small, left, yy + lh / 2.0, fg);
            let hw = self.t.get(*name).and_then(|n| n.as_str()).map(short_name).unwrap_or_default();
            g.text_at(&fit_text(g, &hw, &small, card_w - lw - 6.0), &small, left + lw + 6.0, yy + lh / 2.0, dim);
            yy += lh + 5.0;
            let main_h = 24.0f32.max(line(g, &cx.main(22.0)));
            let t = self.get(temp);
            let hot = t.is_some_and(|t| t >= warn);
            let tc = if hot { with_alpha(cx.look.warn, alpha) } else if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
            let tt = t.map_or("–".into(), deg);
            let tw = g.text_at(&tt, &cx.main(22.0), left, yy + main_h / 2.0, tc).max(46.0);
            // The sparkline: the last minute, stretched to the card.
            let hist = &self.history[i];
            if hist.len() >= 2 {
                let (sx, sw) = (left + tw + 8.0, card_w - tw - 8.0);
                let lo = hist.iter().cloned().fold(f32::INFINITY, f32::min) - 2.0;
                let hi = hist.iter().cloned().fold(f32::NEG_INFINITY, f32::max) + 2.0;
                let step = sw / (HISTORY - 1) as f32;
                let x0 = sx + sw - step * (hist.len() - 1) as f32;
                let top = yy + (main_h - 24.0) / 2.0;
                let pts: Vec<(f32, f32)> = hist.iter().enumerate().map(|(k, v)| (x0 + k as f32 * step, top + 22.0 - (v - lo) / (hi - lo) * 20.0)).collect();
                g.polyline(&pts, 1.5, white(0.55 * alpha));
            }
            yy += main_h + 5.0;
            let cyy = yy + lh / 2.0;
            g.text_at("Load", &small, left, cyy, dim);
            let lt = format!("{}%", fixed(self.get(load), 0));
            let ltw = g.measure(&lt, &small);
            g.text_at(&lt, &small, left + card_w - ltw, cyy, fg);
            bar(left + 30.0 + 8.0, card_w - 30.0 - 8.0 - 30.0 - 8.0, cyy, self.get(load));
            yy += lh + 5.0;
            g.text_at(&fit_text(g, detail, &small, card_w), &small, left, yy + lh / 2.0, dim);
        }
        // Memory: used of total.
        let cyy = y + Self::card_h(g, cx) + 10.0 + lh / 2.0;
        let kw = g.text_at("Memory", &small, x, cyy, dim);
        let (used, avail) = (self.get("ramUsed"), self.get("ramAvailable"));
        let total = match (used, avail) {
            (Some(u), Some(a)) => Some(u + a),
            _ => None,
        };
        let text = total.map_or("–".into(), |t| format!("{} / {t:.1} GB", fixed(used, 1)));
        let tw = g.measure(&text, &small).max(90.0);
        g.text_at(&text, &small, x + 400.0 - g.measure(&text, &small), cyy, fg);
        bar(x + kw + 8.0, 400.0 - kw - 8.0 - tw - 8.0, cyy, total.map(|t| used.unwrap_or(0.0) / t * 100.0));
    }
}

// ----------------------------------------------------------------------

/// Which apps use the microphone, camera or location.
pub struct PrivacyPanel {
    rows: Vec<(&'static str, String, &'static str)>,
    width: f32,
}

impl PrivacyPanel {
    pub fn new() -> PrivacyPanel {
        PrivacyPanel { rows: Vec::new(), width: 240.0 }
    }
}

impl Panel for PrivacyPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["privacy"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.rows = super::extras::DEVICES
            .iter()
            .flat_map(|(key, glyph, label)| v.get(*key).and_then(|a| a.as_array()).cloned().unwrap_or_default().into_iter().filter_map(move |app| Some((*glyph, app.as_str()?.to_string(), *label))))
            .collect();
        true
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        let lh = line(g, &cx.body());
        if self.rows.is_empty() {
            return (g.measure("Nothing is using the microphone, camera or location.", &cx.secondary(cx.look.font_size)).max(240.0), lh);
        }
        let w = self.rows.iter().map(|(_, app, label)| 14.0 + 10.0 + g.measure(app, &cx.body()) + 10.0 + g.measure(label, &cx.secondary(cx.look.font_size))).fold(240.0, f32::max);
        self.width = w;
        (w, self.rows.len() as f32 * lh + (self.rows.len() as f32 - 1.0) * 8.0)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let lh = line(g, &cx.body());
        let dim = with_alpha(cx.look.dimmed(), alpha);
        if self.rows.is_empty() {
            g.text_at("Nothing is using the microphone, camera or location.", &cx.secondary(cx.look.font_size), x, y + lh / 2.0, dim);
            return;
        }
        let w = self.width;
        for (i, (glyph, app, label)) in self.rows.iter().enumerate() {
            let cy = y + i as f32 * (lh + 8.0) + lh / 2.0;
            g.text_at(glyph, &Font::icons(14.0), x, cy + 1.0, with_alpha(cx.look.warn, alpha));
            g.text_at(app, &cx.body(), x + 24.0, cy, with_alpha(cx.look.fg, alpha));
            let f = cx.secondary(cx.look.font_size);
            let lw = g.measure(label, &f);
            g.text_at(label, &f, x + w - lw, cy, dim);
        }
    }
}

// ----------------------------------------------------------------------

/// The hosted tray icons in a grid (`icons_per_row`); a click goes to the app and closes the panel.
pub struct TrayPanel {
    opts: Opts,
    icons: Vec<(String, String)>,
    pixels: HashMap<String, super::icon::Pixels>,
    bitmaps: HashMap<String, (String, AppIcon)>,
    style: Option<Style>,
    hovered: Option<usize>,
}

const CELL: f32 = 32.0;
const CELL_GAP: f32 = 4.0;

impl TrayPanel {
    pub fn new(opts: &Opts) -> TrayPanel {
        TrayPanel { opts: opts.clone(), icons: Vec::new(), pixels: HashMap::new(), bitmaps: HashMap::new(), style: None, hovered: None }
    }
    fn per_row(&self) -> usize {
        self.opts.num("icons_per_row").unwrap_or(5.0).max(1.0) as usize
    }
    fn at(&self, p: (f32, f32)) -> Option<usize> {
        let (c, r) = ((p.0 / (CELL + CELL_GAP)).floor(), (p.1 / (CELL + CELL_GAP)).floor());
        if c < 0.0 || r < 0.0 || c as usize >= self.per_row() || p.0 - c * (CELL + CELL_GAP) >= CELL || p.1 - r * (CELL + CELL_GAP) >= CELL {
            return None;
        }
        let i = r as usize * self.per_row() + c as usize;
        (i < self.icons.len()).then_some(i)
    }
}

impl Panel for TrayPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["tray"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.icons.clear();
        for i in v.as_array().cloned().unwrap_or_default() {
            let (Some(id), Some(rgba)) = (i.get("id").and_then(|x| x.as_str()), i.get("rgba").and_then(|x| x.as_str())) else { continue };
            if !self.bitmaps.get(id).is_some_and(|(r, _)| r == rgba) {
                if let Some(px) = decode(&i) {
                    self.pixels.insert(id.to_string(), px);
                }
            }
            self.icons.push((id.to_string(), rgba.to_string()));
        }
        true
    }
    fn pad(&self) -> (f32, f32, f32) {
        (8.0, 12.0, 10.0)
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let style = Style::from(self.opts.str("icons"), cx.look.accent);
        if self.style != Some(style) {
            self.style = Some(style);
            self.bitmaps.clear();
        }
        // Bitmaps for icons that are new or changed (the pixels stay for a style change).
        for (id, rgba) in &self.icons {
            if self.bitmaps.get(id).is_some_and(|(r, _)| r == rgba) {
                continue;
            }
            if let Some(b) = self.pixels.get(id).and_then(|px| AppIcon::new(cx.gfx, px, style)) {
                self.bitmaps.insert(id.clone(), (rgba.clone(), b));
            }
        }
        if self.icons.is_empty() {
            let f = cx.secondary(12.0);
            return (cx.gfx.measure("No tray icons", &f) + 8.0, line(cx.gfx, &f) + 12.0);
        }
        let cols = self.per_row().min(self.icons.len()) as f32;
        let rows = self.icons.len().div_ceil(self.per_row()) as f32;
        (cols * CELL + (cols - 1.0) * CELL_GAP, rows * CELL + (rows - 1.0) * CELL_GAP)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        if self.icons.is_empty() {
            let f = cx.secondary(12.0);
            g.text_at("No tray icons", &f, x + 4.0, y + 6.0 + line(g, &f) / 2.0, with_alpha(cx.look.dimmed(), alpha));
            return;
        }
        for (i, (id, _)) in self.icons.iter().enumerate() {
            let (c, r) = ((i % self.per_row()) as f32, (i / self.per_row()) as f32);
            let (cx0, cy0) = (x + c * (CELL + CELL_GAP), y + r * (CELL + CELL_GAP));
            if self.hovered == Some(i) {
                g.fill_round(rect(cx0, cy0, CELL, CELL), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            if let Some((_, b)) = self.bitmaps.get(id) {
                b.draw(g, cx0 + 7.0, cy0 + 7.0, 18.0, alpha);
            }
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.at(p));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let Some(i) = self.at(at) else { return Reply::None };
        let button = match button {
            Button::Left => "left",
            Button::Right => "right",
            Button::Middle => return Reply::None,
        };
        let (c, r) = ((i % self.per_row()) as f32, (i / self.per_row()) as f32);
        let centre = (c * (CELL + CELL_GAP) + CELL / 2.0, r * (CELL + CELL_GAP) + CELL / 2.0);
        Reply::CloseAnd(Act::Tray { id: self.icons[i].0.clone(), button, at: centre })
    }
    fn closed(&mut self) {
        self.hovered = None;
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        (0..self.icons.len()).map(|i| ((i % self.per_row()) as f32 * (CELL + CELL_GAP), (i / self.per_row()) as f32 * (CELL + CELL_GAP), CELL, CELL)).collect()
    }
}
