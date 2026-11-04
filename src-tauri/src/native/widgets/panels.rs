//! Panels that only show (or nearly): weather (weather.ts weatherPanel; or a look:
//! sky, curve, week), temps
//! (temps-panel.ts), privacy (extras.ts privacyPanel), tray (tray.ts trayPanel).

use std::collections::HashMap;

use serde_json::Value;

use super::icon::{decode, AppIcon, Style};
use super::weather::{deg, icon};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply};
use crate::native::geometry::{Cmd, Pt};
use crate::native::gfx::{rect, rgba, white, with_alpha, Color, Font, Gfx};

/// A line of text's height in a font.
fn line(g: &Gfx, f: &Font) -> f32 {
    g.text("Ag", f).map_or(f.size * 1.21, |t| t.height)
}

// ----------------------------------------------------------------------

/// The weather panel's look ([widgets.weather] style).
#[derive(Clone, Copy, PartialEq, Debug)]
enum WeatherStyle {
    /// The conditions and a strip of hours (or days).
    Classic,
    /// The top coloured like the sky outside, a line about what's coming, the strip.
    Sky,
    /// The next hours as a line.
    Curve,
    /// The days as low-to-high bars on one scale.
    Week,
}

/// Current conditions and the next hours (`hours`) or days (`forecast` = "daily"), in
/// its look ([widgets.weather] style).
pub struct WeatherPanel {
    opts: Opts,
    style: WeatherStyle,
    /// On a bottom surface (the panel's far edge is its top).
    bottom: bool,
    w: Value,
    width: f32,
}

const HOUR_W: f32 = 36.0;
const DAY_W: f32 = 40.0;
/// Sky: the margin inside the coloured band, the big temperature's size.
const SKY_PAD: f32 = 16.0;
const SKY_BIG: f32 = 48.0;
/// Curve and Week: their width; Curve: the graph's height (labels included), how many hours.
const PLOT_W: f32 = 270.0;
const CURVE_H: f32 = 92.0;
const CURVE_HOURS: usize = 12;
/// Week: a day's row and the gap between rows.
const DAY_ROW: f32 = 22.0;
const DAY_GAP: f32 = 4.0;

/// A WMO weather code that means rain or snow (or a storm) falling.
fn wet(code: i64) -> bool {
    matches!(code, 51..=67 | 71..=86 | 95..=99)
}

/// The sky's colour for a condition, by day or night.
fn sky(code: i64, day: bool) -> Color {
    let (r, g, b) = match (code, day) {
        (0 | 1, true) => (0.20, 0.42, 0.70),
        (2, true) => (0.27, 0.41, 0.58),
        (3 | 45 | 48, true) => (0.34, 0.39, 0.46),
        (71..=77 | 85 | 86, true) => (0.52, 0.58, 0.68),
        (95..=99, _) => (0.24, 0.19, 0.34),
        (_, true) => (0.22, 0.30, 0.40),
        (0 | 1, false) => (0.06, 0.10, 0.24),
        (_, false) => (0.11, 0.13, 0.19),
    };
    rgba(r, g, b, 1.0)
}

impl WeatherPanel {
    pub fn new(opts: &Opts) -> WeatherPanel {
        let style = match opts.str("style") {
            Some("sky") => WeatherStyle::Sky,
            Some("curve") => WeatherStyle::Curve,
            Some("week") => WeatherStyle::Week,
            _ => WeatherStyle::Classic,
        };
        WeatherPanel { opts: opts.clone(), style, bottom: false, w: Value::Null, width: 0.0 }
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
    fn code(&self) -> i64 {
        self.w.get("code").and_then(|c| c.as_i64()).unwrap_or(3)
    }
    fn is_day(&self) -> bool {
        self.w.get("isDay").and_then(|d| d.as_bool()).unwrap_or(true)
    }
    /// The next hours: (hour "15", code, temperature), now first.
    fn hours(&self, n: usize) -> Vec<(String, i64, f64)> {
        self.w
            .get("hourly")
            .and_then(|h| h.as_array())
            .map(|h| {
                h.iter()
                    .take(n + 1)
                    .map(|h| {
                        let time = h.get("time").and_then(|t| t.as_str()).unwrap_or("");
                        (time.get(11..13).unwrap_or("").to_string(), h.get("code").and_then(|c| c.as_i64()).unwrap_or(3), h.get("temp").and_then(|t| t.as_f64()).unwrap_or(0.0))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    /// The days: (label "Today" / "Mon", code, low, high).
    fn days(&self) -> Vec<(String, i64, f64, f64)> {
        self.w
            .get("daily")
            .and_then(|d| d.as_array())
            .map(|days| {
                days.iter()
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
                        (label, d.get("code").and_then(|c| c.as_i64()).unwrap_or(3), g("min"), g("max"))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
    /// What's coming: rain (or snow) starting or stopping within the next hours.
    fn outlook(&self) -> String {
        let now = self.code();
        let what = Self::describe(now);
        let later = self.hours(12).into_iter().skip(1);
        if wet(now) {
            if let Some((h, _, _)) = later.clone().find(|(_, c, _)| !wet(*c)) {
                return format!("{what} until {h}:00");
            }
            return what.to_string();
        }
        match later.clone().find(|(_, c, _)| wet(*c)) {
            Some((h, c, _)) => {
                let kind = if matches!(c, 71..=77 | 85 | 86) { "snow" } else { "rain" };
                format!("{what}, {kind} from {h}:00")
            }
            None => what.to_string(),
        }
    }
    /// The strip: (time label, code, day, temp, min) per column.
    fn columns(&self) -> Vec<(String, i64, bool, String, Option<String>)> {
        let daily = self.opts.str("forecast") == Some("daily");
        let days = self.w.get("daily").and_then(|d| d.as_array()).filter(|d| !d.is_empty());
        if let (true, Some(_)) = (daily, days) {
            return self.days().into_iter().map(|(label, code, min, max)| (label, code, true, deg(max), Some(deg(min)))).collect();
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
    /// The strip's width and height.
    fn strip_size(&self, g: &Gfx, cx: &Ctx) -> (f32, f32) {
        let cols = self.columns();
        if cols.is_empty() {
            return (0.0, 0.0);
        }
        let daily = cols.first().is_some_and(|c| c.4.is_some());
        let w = cols.len() as f32 * if daily { DAY_W } else { HOUR_W } + (cols.len() as f32 - 1.0) * 2.0;
        let h = 6.0 + line(g, &cx.secondary(10.0)) + 5.0 + 16.0 + 5.0 + line(g, &cx.secondary(11.0)) + if daily { line(g, &cx.secondary(10.0)) - 3.0 } else { 0.0 } + 6.0;
        (w, h)
    }
    /// The strip from (x, top): the first column (now / today) highlighted.
    fn draw_strip(&self, cx: &Ctx, x: f32, top: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let cols = self.columns();
        let daily = cols.first().is_some_and(|c| c.4.is_some());
        let cw = if daily { DAY_W } else { HOUR_W };
        let (tf, vf, mf) = (cx.secondary(10.0), cx.secondary(11.0), cx.secondary(10.0));
        let h = self.strip_size(g, cx).1;
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
    fn classic_head_h(g: &Gfx, cx: &Ctx) -> f32 {
        (line(g, &cx.secondary(12.0)) + 2.0 * line(g, &cx.secondary(11.0)) + 2.0).max(line(g, &cx.main(24.0)))
    }
    /// Sky: the coloured band's height.
    fn sky_h(g: &Gfx, cx: &Ctx) -> f32 {
        SKY_PAD + line(g, &cx.secondary(11.0)) + 2.0 + line(g, &cx.main(SKY_BIG)) + 2.0 + line(g, &cx.main(13.0)) + SKY_PAD
    }
    /// Curve and Week: the header (temperature, condition, a note).
    fn small_head_h(g: &Gfx, cx: &Ctx) -> f32 {
        line(g, &cx.main(30.0)).max(line(g, &cx.main(13.0)) + 2.0 + line(g, &cx.secondary(12.0)))
    }
    fn draw_small_head(&self, cx: &Ctx, x: f32, y: f32, note: &str, alpha: f32) {
        let g = cx.gfx;
        let h = Self::small_head_h(g, cx);
        let temp_color = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { with_alpha(cx.look.fg, alpha) };
        let tw = g.text_at(&deg(self.f("temp")), &cx.main(30.0), x, y + h / 2.0, temp_color);
        let (cf, nf) = (cx.main(13.0), cx.secondary(12.0));
        let total = line(g, &cf) + 2.0 + line(g, &nf);
        let ty = y + (h - total) / 2.0;
        g.text_at(Self::describe(self.code()), &cf, x + tw + 12.0, ty + line(g, &cf) / 2.0, with_alpha(cx.look.fg, alpha));
        g.text_at(note, &nf, x + tw + 12.0, ty + line(g, &cf) + 2.0 + line(g, &nf) / 2.0, with_alpha(cx.look.dimmed(), alpha));
    }

    fn draw_classic(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let head_h = Self::classic_head_h(g, cx);
        let cy = y + head_h / 2.0;
        let code = self.code();
        let mut cx_ = x;
        super::row_draw(g, &[icon(code, self.is_day(), 26.0, cx.look.fg)], cx_, cy, alpha);
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
        self.draw_strip(cx, x, y + head_h + 12.0, alpha);
    }

    /// Sky: the band coloured like the sky (rising out of the bar's black, fading into
    /// the panel), the place, the temperature large, what's coming; the strip under it.
    fn draw_sky(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let w = self.width;
        let band_h = Self::sky_h(g, cx);
        let strip_h = self.strip_size(g, cx).1;
        // On a bottom dock the band is by the dock, the strip above it.
        let (band_y, strip_y) = if self.bottom { (y + strip_h + 8.0, y + 4.0) } else { (y, y + band_h + 4.0) };
        let band = rect(x, band_y, w, band_h);
        let (near, far) = (36.0, 44.0);
        if self.bottom {
            g.push_fade_v(band, far, near);
        } else {
            g.push_fade_v(band, near, far);
        }
        let (code, day) = (self.code(), self.is_day());
        g.fill_rect(band, with_alpha(sky(code, day), alpha));
        if matches!(code, 0..=2) {
            // The sun (or moon) as a soft light in a corner.
            let light = if day { rgba(1.0, 0.92, 0.65, 0.35 * alpha) } else { rgba(0.75, 0.82, 1.0, 0.18 * alpha) };
            g.glow(x + w - 40.0, band_y + band_h * 0.4, 110.0, 70.0, light);
        }
        g.pop_fade();
        let mut ty = band_y + SKY_PAD;
        let (pf, bf, cf) = (cx.secondary(11.0), cx.main(SKY_BIG), cx.main(13.0));
        let place = self.w.get("location").and_then(|l| l.as_str()).unwrap_or("");
        g.text_at(place, &pf, x + SKY_PAD, ty + line(g, &pf) / 2.0, white(0.72 * alpha));
        ty += line(g, &pf) + 2.0;
        g.text_at(&deg(self.f("temp")), &bf, x + SKY_PAD - 2.0, ty + line(g, &bf) / 2.0, white(alpha));
        ty += line(g, &bf) + 2.0;
        g.text_at(&fit_text(g, &self.outlook(), &cf, w - 2.0 * SKY_PAD), &cf, x + SKY_PAD, ty + line(g, &cf) / 2.0, white(alpha));
        let (sw, _) = self.strip_size(g, cx);
        self.draw_strip(cx, x + (w - sw) / 2.0, strip_y, alpha);
    }

    /// Curve: the next hours as a smooth line over a faint fill, now as a dot, every
    /// third hour's temperature over it and its time under it.
    fn draw_curve(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let head = Self::small_head_h(g, cx);
        self.draw_small_head(cx, x, y, &format!("Next {CURVE_HOURS} hours"), alpha);
        let hours = self.hours(CURVE_HOURS);
        if hours.len() < 2 {
            return;
        }
        let top = y + head + 10.0;
        let (lf, tf) = (cx.secondary(11.0), cx.secondary(10.0));
        let (label_h, time_h) = (line(g, &lf) + 4.0, line(g, &tf) + 4.0);
        let (plot_top, plot_bottom) = (top + label_h, top + CURVE_H - time_h);
        let lo = hours.iter().map(|h| h.2).fold(f64::MAX, f64::min);
        let hi = hours.iter().map(|h| h.2).fold(f64::MIN, f64::max);
        let span = (hi - lo).max(1.0);
        let (l, r) = (x + 10.0, x + PLOT_W - 10.0);
        let pts: Vec<(f32, f32)> = hours
            .iter()
            .enumerate()
            .map(|(i, h)| (l + (r - l) * i as f32 / (hours.len() - 1) as f32, plot_bottom - ((h.2 - lo) / span) as f32 * (plot_bottom - plot_top)))
            .collect();
        // Catmull-Rom through the points, as Béziers.
        let p = |i: usize| Pt(pts[i].0 as f64, pts[i].1 as f64);
        let mut curve = vec![Cmd::M(p(0))];
        for i in 0..pts.len() - 1 {
            let (p0, p1, p2, p3) = (p(i.saturating_sub(1)), p(i), p(i + 1), p((i + 2).min(pts.len() - 1)));
            curve.push(Cmd::C(Pt(p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0), Pt(p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0), p2));
        }
        let accent = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
        let mut area = curve.clone();
        area.extend([Cmd::L(Pt(r as f64, plot_bottom as f64 + 4.0)), Cmd::L(Pt(l as f64, plot_bottom as f64 + 4.0)), Cmd::Z]);
        g.fill_path_gradient(&area, (0.0, plot_top), (0.0, plot_bottom + 4.0), with_alpha(accent, 0.28 * alpha), with_alpha(accent, 0.0));
        g.stroke_path(&curve, 2.0, with_alpha(accent, alpha));
        for (i, (hour, _, t)) in hours.iter().enumerate() {
            if i % 3 != 0 && i != hours.len() - 1 {
                continue;
            }
            let (px, py) = pts[i];
            let centred = |s: &str, f: &Font, cy: f32, c| {
                let w = g.measure(s, f);
                g.text_at(s, f, (px - w / 2.0).clamp(x, x + PLOT_W - w), cy, c);
            };
            centred(&deg(*t), &lf, py - 6.0 - line(g, &lf) / 2.0, with_alpha(cx.look.fg, alpha));
            centred(if i == 0 { "Now" } else { hour }, &tf, plot_bottom + 4.0 + time_h / 2.0, with_alpha(cx.look.dimmed(), alpha));
        }
        // Now: a dot ringed in the panel's colour.
        let (nx, ny) = pts[0];
        g.fill_round(rect(nx - 5.5, ny - 5.5, 11.0, 11.0), 5.5, with_alpha(cx.look.bg, alpha));
        g.fill_round(rect(nx - 3.5, ny - 3.5, 7.0, 7.0), 3.5, with_alpha(cx.look.fg, alpha));
    }

    /// Week: each day's low to high as a bar on one scale, today's with a dot at now.
    fn draw_week(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let head = Self::small_head_h(g, cx);
        self.draw_small_head(cx, x, y, &format!("{} / {} today", deg(self.f("min")), deg(self.f("max"))), alpha);
        let days = self.days();
        if days.is_empty() {
            return;
        }
        let lo = days.iter().map(|d| d.2).fold(f64::MAX, f64::min);
        let hi = days.iter().map(|d| d.3).fold(f64::MIN, f64::max);
        let span = (hi - lo).max(1.0);
        let f = cx.secondary(12.0);
        let (label_w, num_w) = (44.0, 30.0);
        let bar_l = x + label_w + 16.0 + 8.0 + num_w + 8.0;
        let bar_w = x + PLOT_W - num_w - 8.0 - bar_l;
        let accent = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
        for (i, (label, code, min, max)) in days.iter().enumerate() {
            let top = y + head + 12.0 + i as f32 * (DAY_ROW + DAY_GAP);
            let cy = top + DAY_ROW / 2.0;
            let lf = if i == 0 { cx.main(12.0) } else { f.clone() };
            g.text_at(label, &lf, x, cy, if i == 0 { fg } else { dim });
            super::row_draw(g, &[icon(*code, true, 16.0, cx.look.fg)], x + label_w, cy, alpha);
            let lo_t = deg(*min);
            g.text_at(&lo_t, &f, bar_l - 8.0 - g.measure(&lo_t, &f), cy, dim);
            g.text_at(&deg(*max), &f, bar_l + bar_w + 8.0, cy, fg);
            g.fill_round(rect(bar_l, cy - 2.5, bar_w, 5.0), 2.5, white(0.12 * alpha));
            let a = bar_l + ((min - lo) / span) as f32 * bar_w;
            let b = bar_l + ((max - lo) / span) as f32 * bar_w;
            g.fill_round_gradient(rect(a, cy - 2.5, (b - a).max(5.0), 5.0), 2.5, rgba(0.5, 0.72, 1.0, alpha), with_alpha(accent, alpha));
            if i == 0 {
                let nx = bar_l + ((self.f("temp") - lo) / span).clamp(0.0, 1.0) as f32 * bar_w;
                g.fill_round(rect(nx - 5.5, cy - 5.5, 11.0, 11.0), 5.5, with_alpha(cx.look.bg, alpha));
                g.fill_round(rect(nx - 3.5, cy - 3.5, 7.0, 7.0), 3.5, fg);
            }
        }
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
        match self.style {
            // The band reaches the bar and both sides.
            WeatherStyle::Sky => (0.0, 10.0, 0.0),
            _ => (12.0, 14.0, 14.0),
        }
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        self.bottom = cx.look.bottom;
        if self.w.is_null() {
            let f = cx.secondary(12.0);
            return (g.measure("No weather data", &f) + if self.style == WeatherStyle::Sky { 28.0 } else { 0.0 }, line(g, &f));
        }
        let (strip_w, strip_h) = self.strip_size(g, cx);
        match self.style {
            WeatherStyle::Classic => {
                let temp_w = g.measure(&deg(self.f("temp")), &cx.main(24.0));
                let meta = [Self::describe(self.code()).to_string(), format!("Feels like {}", deg(self.f("feels"))), format!("{} / {}", deg(self.f("min")), deg(self.f("max")))];
                let meta_w = meta.iter().enumerate().map(|(i, m)| g.measure(m, &cx.secondary(if i == 0 { 12.0 } else { 11.0 }))).fold(0.0, f32::max);
                let place = self.w.get("location").and_then(|l| l.as_str()).unwrap_or("");
                let head = 26.0 + 10.0 + temp_w + 10.0 + 4.0 + meta_w + 10.0 + g.measure(place, &cx.secondary(11.0));
                self.width = head.max(strip_w).ceil();
                (self.width, Self::classic_head_h(g, cx) + 12.0 + strip_h)
            }
            WeatherStyle::Sky => {
                self.width = (strip_w + 2.0 * SKY_PAD).max(250.0).ceil();
                (self.width, Self::sky_h(g, cx) + 4.0 + strip_h + 4.0)
            }
            WeatherStyle::Curve => {
                self.width = PLOT_W;
                (PLOT_W, Self::small_head_h(g, cx) + 10.0 + CURVE_H)
            }
            WeatherStyle::Week => {
                self.width = PLOT_W;
                let n = self.days().len() as f32;
                (PLOT_W, Self::small_head_h(g, cx) + 12.0 + n * (DAY_ROW + DAY_GAP) - DAY_GAP)
            }
        }
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        if self.w.is_null() {
            let f = cx.secondary(12.0);
            let inset = if self.style == WeatherStyle::Sky { 14.0 } else { 0.0 };
            g.text_at("No weather data", &f, x + inset, y + line(g, &f) / 2.0, with_alpha(cx.look.fg, alpha));
            return;
        }
        match self.style {
            WeatherStyle::Classic => self.draw_classic(cx, x, y, alpha),
            WeatherStyle::Sky => self.draw_sky(cx, x, y, alpha),
            WeatherStyle::Curve => self.draw_curve(cx, x, y, alpha),
            WeatherStyle::Week => self.draw_week(cx, x, y, alpha),
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
    /// Each icon's hint (its app's tooltip), by id.
    tips: HashMap<String, String>,
    pixels: HashMap<String, super::icon::Pixels>,
    bitmaps: HashMap<String, (String, AppIcon)>,
    style: Option<Style>,
    hovered: Option<usize>,
}

const CELL: f32 = 32.0;
const CELL_GAP: f32 = 4.0;

impl TrayPanel {
    pub fn new(opts: &Opts) -> TrayPanel {
        TrayPanel { opts: opts.clone(), icons: Vec::new(), tips: HashMap::new(), pixels: HashMap::new(), bitmaps: HashMap::new(), style: None, hovered: None }
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
            // Its app's tooltip, else its program's name (as the tray widget's).
            let s = |k: &str| i.get(k).and_then(|t| t.as_str()).unwrap_or("").to_string();
            self.tips.insert(id.to_string(), Some(s("tip")).filter(|t| !t.is_empty()).unwrap_or_else(|| s("exe")));
        }
        true
    }
    fn tooltip(&self) -> Option<String> {
        let (id, _) = self.icons.get(self.hovered?)?;
        self.tips.get(id).cloned().filter(|t| !t.is_empty())
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
