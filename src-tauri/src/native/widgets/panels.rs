//! Panels that only show (or nearly): weather (weather.ts weatherPanel; or a look:
//! sky, curve, week), temps
//! (temps-panel.ts), privacy (extras.ts privacyPanel), tray (tray.ts trayPanel).

use std::collections::HashMap;

use serde_json::Value;

use super::icon::{decode, AppIcon, Style};
use super::weather::{deg, icon};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply};
use crate::native::geometry::{Cmd, Pt};
use crate::native::gfx::{arc_points, rect, rgba, white, with_alpha, Color, Font, Gfx};

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

/// The temps panel's look ([widgets.temps] style).
#[derive(Clone, Copy, PartialEq)]
enum TempsStyle {
    /// CPU and GPU cards side by side, memory under them.
    Cards,
    /// Each temperature on a dial from 30° to 100°, the warning end marked.
    Dials,
    /// Both temperatures over the last five minutes on one chart.
    Graph,
    /// One part per line: temperature, load, power.
    Table,
}

/// CPU / GPU temperatures (LibreHardwareMonitor) or load (Windows), power and memory,
/// in its look ([widgets.temps] style): cards (a 60 s sparkline each), dials, a graph or
/// a table; under it the apps using the most, each with End (a second click ends it).
pub struct TempsPanel {
    opts: Opts,
    style: TempsStyle,
    t: Value,
    /// The last five minutes of readings (CPU, GPU), oldest first.
    history: [Vec<f32>; 2],
    /// Where the apps' list starts (content y), as last measured.
    apps_y: f32,
    /// The End button under the cursor, and one armed (its program, until when).
    hover_end: Option<usize>,
    armed: Option<(String, std::time::Instant)>,
}

const APP_ROW: f32 = 26.0;

/// Readings the graph keeps (five minutes at the default once a second).
const GRAPH: usize = 300;
/// Dials: their radius, stroke and the scale they show.
const DIAL_R: f32 = 58.0;
const DIAL_W: f32 = 7.0;
const DIAL_LO: f64 = 30.0;
const DIAL_HI: f64 = 100.0;

impl TempsPanel {
    pub fn new(opts: &Opts) -> TempsPanel {
        let style = match opts.str("style") {
            Some("dials") => TempsStyle::Dials,
            Some("graph") => TempsStyle::Graph,
            Some("table") => TempsStyle::Table,
            _ => TempsStyle::Cards,
        };
        TempsPanel { opts: opts.clone(), style, t: Value::Null, history: [Vec::new(), Vec::new()], apps_y: 0.0, hover_end: None, armed: None }
    }
    /// Temperatures known (LibreHardwareMonitor running): else the panel shows load.
    fn heat(&self) -> bool {
        self.get("cpu").is_some() || self.get("gpu").is_some()
    }
    /// A part's main reading ("cpu" / "gpu"): its temperature, else its load.
    fn reading(&self, k: &str) -> Option<f64> {
        if self.heat() { self.get(k) } else { self.get(&format!("{k}Load")) }
    }
    fn shown(&self, v: f64) -> String {
        if self.heat() { deg(v) } else { format!("{}%", v.round()) }
    }
    fn reading_color(&self, cx: &Ctx, v: Option<f64>) -> Color {
        if self.heat() {
            self.temp_color(cx, v)
        } else if v.is_some_and(|v| v >= 90.0) {
            cx.look.warn
        } else if cx.opts.accent {
            cx.look.accent
        } else {
            cx.look.fg
        }
    }
    /// The apps using the most, as the provider lists them.
    fn apps(&self) -> Vec<&Value> {
        self.t.get("top").and_then(|t| t.as_array()).map(|a| a.iter().collect()).unwrap_or_default()
    }
    /// LibreHardwareMonitor is to be used but isn't answering.
    fn lhm_missing(&self) -> bool {
        self.t.get("lhm").and_then(|v| v.as_bool()) == Some(true) && self.t.get("lhmOk").and_then(|v| v.as_bool()) == Some(false)
    }
    /// The look's own size (without the apps and the note).
    fn main_size(&self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        let small = line(g, &cx.secondary(11.0));
        match self.style {
            TempsStyle::Cards => (400.0, Self::card_h(g, cx) + 10.0 + small),
            TempsStyle::Dials => (4.0 * DIAL_R + 2.0 * DIAL_W + 22.0, DIAL_W / 2.0 + DIAL_R + 10.0 + line(g, &cx.secondary(12.0)) + 14.0 + small),
            TempsStyle::Graph => (400.0, line(g, &cx.secondary(12.0)) + 10.0 + 100.0 + 10.0 + small),
            TempsStyle::Table => (TABLE_W, small + 4.0 + self.table_rows().len() as f32 * TABLE_ROW),
        }
    }
    /// The apps' list and the note under the look: its height.
    fn extra_h(&self, cx: &Ctx) -> f32 {
        let small = line(cx.gfx, &cx.secondary(11.0));
        let n = self.apps().len();
        let mut h = 0.0;
        if n > 0 {
            h += 14.0 + small + 4.0 + n as f32 * APP_ROW;
        }
        if self.lhm_missing() {
            h += 10.0 + small;
        }
        h
    }
    fn draw_extra(&self, cx: &Ctx, x: f32, y: f32, w: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let (f, small) = (cx.secondary(12.0), cx.secondary(11.0));
        let mut yy = y;
        let apps = self.apps();
        if !apps.is_empty() {
            yy += 14.0;
            let mut hx = x;
            for ch in "USING THE MOST".chars() {
                hx += g.text_at(&ch.to_string(), &small, hx, yy + line(g, &small) / 2.0, dim) + 0.5;
            }
            yy += line(g, &small) + 4.0;
            for (i, a) in apps.iter().enumerate() {
                let cy = yy + APP_ROW / 2.0;
                let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let cpu = a.get("cpu").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let mem = a.get("mem").and_then(|v| v.as_f64()).unwrap_or(0.0);
                let exe = a.get("exe").and_then(|v| v.as_str()).unwrap_or("");
                let end = a.get("end").and_then(|v| v.as_bool()).unwrap_or(false);
                let bw = if end { 52.0 } else { 0.0 };
                if end {
                    let armed = self.armed.as_ref().is_some_and(|(e, t)| e == exe && std::time::Instant::now() < *t);
                    let (label, bg) = if armed {
                        ("Sure?", with_alpha(rgba(1.0, 80.0 / 255.0, 80.0 / 255.0, 0.35), alpha))
                    } else if self.hover_end == Some(i) {
                        ("End", with_alpha(cx.hover_bg(0.18), alpha))
                    } else {
                        ("End", white(0.1 * alpha))
                    };
                    g.fill_round(rect(x + w - bw, cy - 11.0, bw, 22.0), 7.0, bg);
                    g.text_at(label, &small, x + w - bw + (bw - g.measure(label, &small)) / 2.0, cy, fg);
                }
                let what = format!("{}% · {}", cpu.round(), super::basic::format_bytes(mem));
                let ww = g.measure(&what, &small);
                let right = x + w - bw - if end { 10.0 } else { 0.0 };
                g.text_at(&what, &small, right - ww, cy, dim);
                g.text_at(&fit_text(g, name, &f, right - ww - 12.0 - x), &f, x, cy, fg);
                yy += APP_ROW;
            }
        }
        if self.lhm_missing() {
            yy += 10.0;
            g.text_at(&fit_text(g, "Temperatures: LibreHardwareMonitor isn't running", &small, w), &small, x, yy + line(g, &small) / 2.0, dim);
        }
    }
    /// The End button (its row) under a point (content px).
    fn end_at(&self, w: f32, p: (f32, f32)) -> Option<usize> {
        let i = ((p.1 - self.apps_y) / APP_ROW).floor();
        let apps = self.apps();
        if i < 0.0 || i as usize >= apps.len() || p.0 < w - 52.0 || p.0 > w {
            return None;
        }
        apps[i as usize].get("end").and_then(|v| v.as_bool()).unwrap_or(false).then_some(i as usize)
    }
    fn get(&self, k: &str) -> Option<f64> {
        self.t.get(k).and_then(|v| v.as_f64())
    }
    fn warn(&self) -> f64 {
        self.opts.num("warn").unwrap_or(90.0)
    }
    fn card_h(g: &Gfx, cx: &Ctx) -> f32 {
        line(g, &cx.secondary(11.0)) + 5.0 + 24.0f32.max(line(g, &cx.main(22.0))) + 5.0 + line(g, &cx.secondary(11.0)) + 5.0 + line(g, &cx.secondary(11.0))
    }
    /// A temperature's colour: the warning one when hot, else the accent (with accent details) or the text.
    fn temp_color(&self, cx: &Ctx, t: Option<f64>) -> Color {
        if t.is_some_and(|t| t >= self.warn()) {
            cx.look.warn
        } else if cx.opts.accent {
            cx.look.accent
        } else {
            cx.look.fg
        }
    }
    /// Memory: used and total (GB).
    fn memory(&self) -> Option<(f64, f64)> {
        Some((self.get("ramUsed")?, self.get("ramUsed")? + self.get("ramAvailable")?))
    }
    /// A meter 4 px tall centred on `cy`, `v` percent full.
    fn meter(g: &Gfx, left: f32, w: f32, cy: f32, v: Option<f64>, fill: Color, alpha: f32) {
        g.fill_round(rect(left, cy - 2.0, w, 4.0), 2.0, white(0.15 * alpha));
        let v = (v.unwrap_or(0.0) / 100.0).clamp(0.0, 1.0) as f32;
        if v > 0.0 {
            g.fill_round(rect(left, cy - 2.0, w * v, 4.0), 2.0, fill);
        }
    }
    /// The memory line under the cards and dials: "Memory [meter] 14.2 / 31.8 GB", `w` wide.
    fn memory_row(&self, cx: &Ctx, x: f32, cy: f32, w: f32, alpha: f32) {
        let g = cx.gfx;
        let small = cx.secondary(11.0);
        let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { with_alpha(cx.look.fg, alpha) };
        let kw = g.text_at("Memory", &small, x, cy, with_alpha(cx.look.dimmed(), alpha));
        let mem = self.memory();
        let text = mem.map_or("–".into(), |(u, t)| format!("{u:.1} / {t:.1} GB"));
        let tw = g.measure(&text, &small).max(90.0);
        g.text_at(&text, &small, x + w - g.measure(&text, &small), cy, with_alpha(cx.look.fg, alpha));
        Self::meter(g, x + kw + 8.0, w - kw - 8.0 - tw - 8.0, cy, mem.map(|(u, t)| u / t * 100.0), fill, alpha);
    }

    fn draw_cards(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let small = cx.secondary(11.0);
        let lh = line(g, &small);
        let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
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
            let t = self.reading(temp);
            let tt = t.map_or("–".into(), |v| self.shown(v));
            let tw = g.text_at(&tt, &cx.main(22.0), left, yy + main_h / 2.0, with_alpha(self.reading_color(cx, t), alpha)).max(46.0);
            // The sparkline: the last minute, stretched to the card.
            let hist = &self.history[i][self.history[i].len().saturating_sub(HISTORY)..];
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
            Self::meter(g, left + 30.0 + 8.0, card_w - 30.0 - 8.0 - 30.0 - 8.0, cyy, self.get(load), fill, alpha);
            yy += lh + 5.0;
            g.text_at(&fit_text(g, detail, &small, card_w), &small, left, yy + lh / 2.0, dim);
        }
        self.memory_row(cx, x, y + Self::card_h(g, cx) + 10.0 + lh / 2.0, 400.0, alpha);
    }

    fn draw_dials(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        use std::f32::consts::PI;
        let g = cx.gfx;
        let small = cx.secondary(12.0);
        let (lo, hi) = if self.heat() { (DIAL_LO, DIAL_HI) } else { (0.0, 100.0) };
        let share = |t: f64| ((t - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
        let warn = share(if self.heat() { self.warn() } else { 90.0 });
        let dial_w = 2.0 * DIAL_R + DIAL_W;
        for (i, (label, temp, load)) in [("CPU", "cpu", "cpuLoad"), ("GPU", "gpu", "gpuLoad")].iter().enumerate() {
            let cx0 = x + dial_w / 2.0 + i as f32 * (dial_w + 22.0);
            let cy0 = y + DIAL_W / 2.0 + DIAL_R;
            // Left to right over the top: the track, the warning end, the reading.
            g.polyline(&arc_points(cx0, cy0, DIAL_R, PI, 0.0), DIAL_W, white(0.15 * alpha));
            g.polyline(&arc_points(cx0, cy0, DIAL_R, PI * (1.0 - warn), 0.0), DIAL_W, with_alpha(cx.look.warn, 0.5 * alpha));
            let t = self.reading(temp);
            let color = with_alpha(self.reading_color(cx, t), alpha);
            if let Some(v) = t.map(share).filter(|v| *v > 0.0) {
                g.polyline(&arc_points(cx0, cy0, DIAL_R, PI, PI * (1.0 - v)), DIAL_W, color);
            }
            let big = cx.main(26.0);
            let tt = t.map_or("–".into(), |v| self.shown(v));
            let tw = g.measure(&tt, &big);
            g.text_at(&tt, &big, cx0 - tw / 2.0, cy0 - 14.0, color);
            let lt = if self.heat() { format!("{label} · load {}%", fixed(self.get(load), 0)) } else { label.to_string() };
            let lw = g.measure(&lt, &small);
            let ly = cy0 + 10.0 + line(g, &small) / 2.0;
            let nw = g.text_at(label, &small, cx0 - lw / 2.0, ly, with_alpha(cx.look.fg, alpha));
            if lt.len() > label.len() {
                g.text_at(&lt[label.len()..], &small, cx0 - lw / 2.0 + nw, ly, with_alpha(cx.look.dimmed(), alpha));
            }
        }
        let my = y + DIAL_W / 2.0 + DIAL_R + 10.0 + line(g, &small) + 14.0 + line(g, &cx.secondary(11.0)) / 2.0;
        self.memory_row(cx, x, my, 2.0 * dial_w + 22.0, alpha);
    }

    fn draw_graph(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let body = cx.secondary(12.0);
        let small = cx.secondary(11.0);
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let colors = [if cx.opts.accent { cx.look.accent } else { cx.look.fg }, white(0.6)];
        // The key: a dot and the reading per part, the span on the right.
        let hh = line(g, &body);
        let mut kx = x;
        for (i, (label, k)) in [("CPU", "cpu"), ("GPU", "gpu")].iter().enumerate() {
            g.fill_round(rect(kx, y + hh / 2.0 - 3.0, 6.0, 6.0), 3.0, with_alpha(colors[i], alpha));
            kx += 12.0;
            kx += g.text_at(&format!("{label} {}", self.reading(k).map_or("–".into(), |v| self.shown(v))), &body, kx, y + hh / 2.0, fg) + 16.0;
        }
        let span = "last 5 minutes";
        g.text_at(span, &small, x + 400.0 - g.measure(span, &small), y + hh / 2.0, dim);
        // The chart: 30° (or lower) up to past the warning level.
        let top = y + hh + 10.0;
        let (ch, axis) = (100.0, 28.0);
        let all = self.history.iter().flatten().cloned();
        let (warn, unit) = if self.heat() { (self.warn() as f32, "°") } else { (90.0, "%") };
        let lo = if self.heat() { all.clone().fold(30.0f32, f32::min).floor() } else { 0.0 };
        let hi = if self.heat() { all.fold(warn + 5.0, f32::max).ceil() } else { 100.0 };
        let yv = |v: f32| top + ch - (v - lo) / (hi - lo) * ch;
        let warn_y = yv(warn);
        let mut dx = x + axis;
        while dx < x + 400.0 {
            g.polyline(&[(dx, warn_y), ((dx + 3.0).min(x + 400.0), warn_y)], 1.0, with_alpha(cx.look.warn, 0.6 * alpha));
            dx += 7.0;
        }
        g.fill_rect(rect(x + axis, yv(lo + (hi - lo) / 2.0).round(), 400.0 - axis, 1.0), white(0.08 * alpha));
        for v in [warn, lo] {
            g.text_at(&format!("{}{unit}", v.round()), &small, x, yv(v), dim);
        }
        let step = (400.0 - axis - 4.0) / (GRAPH - 1) as f32;
        for (i, h) in self.history.iter().enumerate().rev() {
            if h.len() < 2 {
                continue;
            }
            let x0 = x + 400.0 - 4.0 - step * (h.len() - 1) as f32;
            let pts: Vec<(f32, f32)> = h.iter().enumerate().map(|(k, v)| (x0 + k as f32 * step, yv(*v))).collect();
            g.polyline(&pts, if i == 0 { 1.8 } else { 1.5 }, with_alpha(colors[i], alpha));
            if let Some(&(lx, ly)) = pts.last() {
                g.fill_round(rect(lx - 3.0, ly - 3.0, 6.0, 6.0), 3.0, with_alpha(colors[i], alpha));
            }
        }
        // Under it: load, power, memory.
        let fy = top + ch + 10.0 + line(g, &small) / 2.0;
        let pct = |k: &str| fixed(self.get(k), 0);
        let left = if self.heat() {
            format!("Load {}% · {}%     Power {} W · {} W", pct("cpuLoad"), pct("gpuLoad"), pct("cpuPower"), pct("gpuPower"))
        } else {
            "CPU and graphics load".to_string()
        };
        g.text_at(&left, &small, x, fy, dim);
        if let Some((u, t)) = self.memory() {
            let m = format!("RAM {u:.1} / {t:.1} GB");
            g.text_at(&m, &small, x + 400.0 - g.measure(&m, &small), fy, dim);
        }
    }

    /// The table's rows: name, temperature, load %, the last column.
    fn table_rows(&self) -> Vec<(&'static str, Option<f64>, Option<f64>, String)> {
        let mut rows = vec![
            ("CPU", self.get("cpu"), self.get("cpuLoad"), format!("{} W", fixed(self.get("cpuPower"), 0))),
            ("GPU", self.get("gpu"), self.get("gpuLoad"), format!("{} W", fixed(self.get("gpuPower"), 0))),
        ];
        if let Some((u, t)) = self.memory() {
            rows.push(("Memory", None, Some(u / t * 100.0), format!("{u:.1} GB")));
        }
        if let (Some(u), Some(t)) = (self.get("vramUsed"), self.get("vramTotal")) {
            rows.push(("Video memory", None, Some(u / t * 100.0), format!("{:.1} GB", u / 1024.0)));
        }
        rows
    }

    fn draw_table(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let f = cx.secondary(12.5);
        let small = cx.secondary(11.0);
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
        // Right edges of the columns: temperature, load, the last.
        let cols = [TABLE_W - 130.0, TABLE_W - 58.0, TABLE_W];
        let right = |s: &str, f: &Font, edge: f32, cy: f32, c: Color| {
            g.text_at(s, f, edge - g.measure(s, f), cy, c);
        };
        let hh = line(g, &small);
        for (h, edge) in ["Temp", "Load", "Power"].iter().zip(cols) {
            right(h, &small, x + edge, y + hh / 2.0, dim);
        }
        let mut yy = y + hh + 4.0;
        for (name, t, load, last) in self.table_rows() {
            g.fill_rect(rect(x, yy.round(), TABLE_W, 1.0), white(0.08 * alpha));
            let cy = yy + TABLE_ROW / 2.0;
            g.text_at(name, &f, x, cy, fg);
            match t {
                Some(t) => right(&deg(t), &f, x + cols[0], cy, with_alpha(self.temp_color(cx, Some(t)), alpha)),
                None => right("–", &f, x + cols[0], cy, dim),
            }
            let lt = format!("{}%", fixed(load, 0));
            let lw = g.measure(&lt, &f);
            right(&lt, &f, x + cols[1], cy, fg);
            Self::meter(g, x + cols[1] - lw - 6.0 - 24.0, 24.0, cy, load, if load.is_some_and(|l| l >= 90.0) { with_alpha(cx.look.warn, alpha) } else { fill }, alpha);
            right(&last, &f, x + cols[2], cy, if t.is_some() { fg } else { dim });
            yy += TABLE_ROW;
        }
    }
}

const TABLE_W: f32 = 300.0;
const TABLE_ROW: f32 = 28.0;

impl Panel for TempsPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["temps"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.t = v.clone();
        for (i, k) in ["cpu", "gpu"].iter().enumerate() {
            if let Some(t) = self.reading(k) {
                self.history[i].push(t as f32);
                if self.history[i].len() > GRAPH {
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
            return (if self.style == TempsStyle::Table { TABLE_W } else { 400.0 }, line(g, &cx.secondary(11.0)));
        }
        let (w, h) = self.main_size(cx);
        self.apps_y = h + 14.0 + line(g, &cx.secondary(11.0)) + 4.0;
        (w, h + self.extra_h(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let small = cx.secondary(11.0);
        if !self.t.get("ok").and_then(|o| o.as_bool()).unwrap_or(false) {
            g.text_at("Waiting for the first readings", &small, x, y + line(g, &small) / 2.0, with_alpha(cx.look.dimmed(), alpha));
            return;
        }
        match self.style {
            TempsStyle::Cards => self.draw_cards(cx, x, y, alpha),
            TempsStyle::Dials => self.draw_dials(cx, x, y, alpha),
            TempsStyle::Graph => self.draw_graph(cx, x, y, alpha),
            TempsStyle::Table => self.draw_table(cx, x, y, alpha),
        }
        let (w, h) = self.main_size(cx);
        self.draw_extra(cx, x, y + h, w, alpha);
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let w = if self.style == TempsStyle::Table { TABLE_W } else if self.style == TempsStyle::Dials { 4.0 * DIAL_R + 2.0 * DIAL_W + 22.0 } else { 400.0 };
        let h = at.and_then(|p| self.end_at(w, p));
        let changed = h != self.hover_end;
        self.hover_end = h;
        changed
    }
    fn click(&mut self, button: Button, _at: (f32, f32)) -> Reply {
        let Some(i) = self.hover_end.filter(|_| button == Button::Left) else { return Reply::None };
        let Some(exe) = self.apps().get(i).and_then(|a| a.get("exe")).and_then(|e| e.as_str()).map(String::from) else { return Reply::None };
        // A second click within 3 s ends it.
        if self.armed.as_ref().is_some_and(|(e, t)| *e == exe && std::time::Instant::now() < *t) {
            self.armed = None;
            return Reply::Do(Act::call(move |_| crate::providers::perf::end_app(&exe)));
        }
        self.armed = Some((exe, std::time::Instant::now() + std::time::Duration::from_secs(3)));
        Reply::Redraw
    }
    fn tooltip(&self) -> Option<String> {
        let a = self.apps().into_iter().nth(self.hover_end?)?;
        Some(format!("End {} (click twice)", a.get("name").and_then(|n| n.as_str()).unwrap_or("it")))
    }
    fn closed(&mut self) {
        self.hover_end = None;
        self.armed = None;
    }
}

// ----------------------------------------------------------------------

/// The disk widget's panel, in its look ([widgets.disk] style): "drives" (each drive's
/// free space, the Recycle Bin with Empty, This PC and Disk Cleanup) or "rings" (a big
/// ring per drive). A drive opens it.
pub struct DiskPanel {
    opts: Opts,
    rings: bool,
    /// Drives: id ("C:"), name, total and free bytes.
    drives: Vec<(String, String, f64, f64)>,
    /// The Recycle Bin: bytes, items.
    bin: Option<(f64, f64)>,
    hovered: Option<DiskHit>,
    /// Drives: the buttons under the bin (what, label, x, width), as last measured.
    btns: Vec<(DiskHit, &'static str, f32, f32)>,
}

#[derive(Clone, Copy, PartialEq)]
enum DiskHit {
    Drive(usize),
    Empty,
    ThisPc,
    Cleanup,
}

const DISK_W: f32 = 330.0;
const DRIVE_H: f32 = 34.0;
const RING_D: f32 = 72.0;
const RING_GAP: f32 = 22.0;
const DISK_BTN_H: f32 = 26.0;

impl DiskPanel {
    pub fn new(opts: &Opts) -> DiskPanel {
        DiskPanel { opts: opts.clone(), rings: opts.str("style") == Some("rings"), drives: Vec::new(), bin: None, hovered: None, btns: Vec::new() }
    }
    fn warn(&self) -> f64 {
        self.opts.num("warn").unwrap_or(90.0) / 100.0
    }
    /// "System (C:)", or "C:" for a drive without a name.
    fn name(d: &(String, String, f64, f64)) -> String {
        if d.1.is_empty() { d.0.clone() } else { format!("{} ({})", d.1, d.0) }
    }
    fn small_name(d: &(String, String, f64, f64)) -> String {
        if d.1.is_empty() { d.0.clone() } else { d.1.clone() }
    }
    /// Drives: the buttons' row under the bin (its y in the content) and each button's x and width.
    fn buttons(&self, g: &Gfx, cx: &Ctx) -> Vec<(DiskHit, &'static str, f32, f32)> {
        let f = cx.secondary(12.0);
        let mut x = 0.0;
        [(DiskHit::ThisPc, "Open This PC"), (DiskHit::Cleanup, "Clean up")]
            .into_iter()
            .map(|(h, t)| {
                let w = g.measure(t, &f) + 20.0;
                let out = (h, t, x, w);
                x += w + 6.0;
                out
            })
            .collect()
    }
    /// The parts under a point, as last laid out.
    fn hit_at(&self, p: (f32, f32)) -> Option<DiskHit> {
        if self.rings {
            let i = (p.0 / (RING_D + RING_GAP)).floor();
            return (i >= 0.0 && (i as usize) < self.drives.len() && p.0 - i * (RING_D + RING_GAP) < RING_D).then_some(DiskHit::Drive(i as usize));
        }
        let n = self.drives.len() as f32;
        if p.1 < n * DRIVE_H {
            return Some(DiskHit::Drive((p.1 / DRIVE_H) as usize));
        }
        let bin_y = n * DRIVE_H + 9.0;
        if p.1 >= bin_y && p.1 < bin_y + DISK_BTN_H {
            return (p.0 >= DISK_W - 64.0 && self.bin.is_some_and(|b| b.1 > 0.0)).then_some(DiskHit::Empty);
        }
        let row = bin_y + DISK_BTN_H + 8.0;
        if p.1 >= row && p.1 < row + DISK_BTN_H {
            return self.btns.iter().find(|b| p.0 >= b.2 && p.0 < b.2 + b.3).map(|b| b.0);
        }
        None
    }
}

impl Panel for DiskPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["disks", "recycle"]
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        if key == "recycle" {
            self.bin = v.get("bytes").and_then(|b| b.as_f64()).zip(v.get("items").and_then(|i| i.as_f64()));
            return true;
        }
        let chosen: Vec<String> = self.opts.table.get("disks").and_then(|d| d.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
        self.drives = v
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|d| {
                let id = d.get("id")?.as_str()?.to_string();
                let label = d.get("label").and_then(|l| l.as_str()).unwrap_or("").to_string();
                (chosen.is_empty() || chosen.contains(&id)).then(|| (id, label, d.get("total").and_then(|t| t.as_f64()).unwrap_or(0.0), d.get("free").and_then(|f| f.as_f64()).unwrap_or(0.0)))
            })
            .collect();
        true
    }
    fn pad(&self) -> (f32, f32, f32) {
        (12.0, 12.0, 14.0)
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        self.btns = self.buttons(g, cx);
        if self.drives.is_empty() {
            let f = cx.secondary(12.0);
            return (g.measure("No drives", &f), line(g, &f));
        }
        if self.rings {
            let n = self.drives.len() as f32;
            return (n * RING_D + (n - 1.0) * RING_GAP, RING_D + 6.0 + line(g, &cx.secondary(12.0)) + 2.0 + line(g, &cx.secondary(11.0)));
        }
        (DISK_W, self.drives.len() as f32 * DRIVE_H + 9.0 + DISK_BTN_H + 8.0 + DISK_BTN_H)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let f = cx.secondary(12.0);
        let small = cx.secondary(11.0);
        let (fg, dim, warn) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha), with_alpha(cx.look.warn, alpha));
        let accent = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
        if self.drives.is_empty() {
            g.text_at("No drives", &f, x, y + line(g, &f) / 2.0, dim);
            return;
        }
        let fmt = super::basic::format_bytes;
        if self.rings {
            for (i, d) in self.drives.iter().enumerate() {
                let share = if d.2 > 0.0 { (d.2 - d.3) / d.2 } else { 0.0 };
                let full = share >= self.warn();
                let left = x + i as f32 * (RING_D + RING_GAP);
                let (rcx, rcy) = (left + RING_D / 2.0, y + RING_D / 2.0);
                if self.hovered == Some(DiskHit::Drive(i)) {
                    g.fill_round(rect(left - 4.0, y - 4.0, RING_D + 8.0, RING_D + 8.0), 12.0, with_alpha(cx.hover_bg(0.12), alpha));
                }
                g.ring(rcx, rcy, RING_D / 2.0 - 5.0, 7.0, share as f32, white(0.15 * alpha), if full { warn } else { accent });
                let pct = format!("{}%", (share * 100.0).round());
                let big = cx.main(15.0);
                g.text_at(&pct, &big, rcx - g.measure(&pct, &big) / 2.0, rcy, fg);
                let name = fit_text(g, &Self::small_name(d), &f, RING_D + RING_GAP - 4.0);
                let ny = y + RING_D + 6.0 + line(g, &f) / 2.0;
                g.text_at(&name, &f, rcx - g.measure(&name, &f) / 2.0, ny, fg);
                let free = format!("{} free", fmt(d.3));
                g.text_at(&free, &small, rcx - g.measure(&free, &small) / 2.0, ny + line(g, &f) / 2.0 + 2.0 + line(g, &small) / 2.0, if full { warn } else { dim });
            }
            return;
        }
        for (i, d) in self.drives.iter().enumerate() {
            let share = if d.2 > 0.0 { (d.2 - d.3) / d.2 } else { 0.0 };
            let full = share >= self.warn();
            let top = y + i as f32 * DRIVE_H;
            if self.hovered == Some(DiskHit::Drive(i)) {
                g.fill_round(rect(x - 6.0, top - 4.0, DISK_W + 12.0, DRIVE_H - 2.0), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            let cy = top + 8.0;
            let gw = g.text_at("\u{EDA2}", &Font::icons(14.0), x, cy + 1.0, fg);
            let free = format!("{} free of {}", fmt(d.3), fmt(d.2));
            let fw = g.measure(&free, &small);
            g.text_at(&free, &small, x + DISK_W - fw, cy, if full { warn } else { dim });
            g.text_at(&fit_text(g, &Self::name(d), &f, DISK_W - gw - 8.0 - fw - 10.0), &f, x + gw + 8.0, cy, fg);
            g.fill_round(rect(x, top + 19.0, DISK_W, 4.0), 2.0, white(0.15 * alpha));
            if share > 0.0 {
                g.fill_round(rect(x, top + 19.0, DISK_W * share as f32, 4.0), 2.0, if full { warn } else { accent });
            }
        }
        // The Recycle Bin, with Empty while it holds something.
        let bin_y = y + self.drives.len() as f32 * DRIVE_H;
        g.fill_rect(rect(x, bin_y.round(), DISK_W, 1.0), white(0.08 * alpha));
        let by = bin_y + 9.0;
        let cy = by + DISK_BTN_H / 2.0;
        let gw = g.text_at("\u{E74D}", &Font::icons(14.0), x, cy + 1.0, fg);
        let nw = g.text_at("Recycle Bin", &f, x + gw + 8.0, cy, fg);
        let (bytes, items) = self.bin.unwrap_or((0.0, 0.0));
        let what = if items > 0.0 { format!("{} · {} item{}", fmt(bytes), items, if items == 1.0 { "" } else { "s" }) } else { "Empty".into() };
        g.text_at(&what, &small, x + gw + 8.0 + nw + 8.0, cy, dim);
        let button = |hit: DiskHit, t: &str, bx: f32, bw: f32, y0: f32| {
            let on = self.hovered == Some(hit);
            g.fill_round(rect(bx, y0, bw, DISK_BTN_H), 8.0, if on { with_alpha(cx.hover_bg(0.18), alpha) } else { white(0.1 * alpha) });
            g.text_at(t, &f, bx + (bw - g.measure(t, &f)) / 2.0, y0 + DISK_BTN_H / 2.0, fg);
        };
        if items > 0.0 {
            button(DiskHit::Empty, "Empty", x + DISK_W - 64.0, 64.0, by);
        }
        let row = by + DISK_BTN_H + 8.0;
        for &(hit, t, bx, bw) in &self.btns {
            button(hit, t, x + bx, bw, row);
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.hit_at(p));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn tooltip(&self) -> Option<String> {
        match self.hovered? {
            DiskHit::Drive(i) => self.drives.get(i).map(|d| format!("Open {}", Self::name(d))),
            DiskHit::Empty => Some("Empty the Recycle Bin (Windows asks first)".into()),
            DiskHit::ThisPc => None,
            DiskHit::Cleanup => Some("Disk Cleanup: Windows' tool for freeing space".into()),
        }
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        match self.hit_at(at) {
            Some(DiskHit::Drive(i)) => self.drives.get(i).map_or(Reply::None, |d| Reply::CloseAnd(Act::open(&format!("{}\\", d.0)))),
            Some(DiskHit::Empty) => Reply::CloseAnd(Act::call(|app| crate::providers::extras::empty_recycle_bin(&app))),
            Some(DiskHit::ThisPc) => Reply::CloseAnd(Act::open("shell:MyComputerFolder")),
            Some(DiskHit::Cleanup) => Reply::CloseAnd(Act::open("cleanmgr.exe")),
            None => Reply::None,
        }
    }
    fn closed(&mut self) {
        self.hovered = None;
    }
}

// ----------------------------------------------------------------------

/// Which apps use the microphone, camera or location.
pub struct PrivacyPanel {
    rows: Vec<(&'static str, String, &'static str)>,
    width: f32,
    /// [widgets.privacy] switches: the three switches under the list.
    switches: bool,
    blocked: [bool; 3],
    /// Where the switches start (content y), the one under the cursor.
    sw_y: f32,
    hover_sw: Option<usize>,
}

const SW_ROW: f32 = 32.0;

impl PrivacyPanel {
    pub fn new(opts: &Opts) -> PrivacyPanel {
        PrivacyPanel { rows: Vec::new(), width: 240.0, switches: opts.flag("switches") == Some(true), blocked: [false; 3], sw_y: 0.0, hover_sw: None }
    }
    fn sw_at(&self, p: (f32, f32)) -> Option<usize> {
        if !self.switches || p.1 < self.sw_y || p.0 < 0.0 || p.0 > self.width {
            return None;
        }
        let i = ((p.1 - self.sw_y) / SW_ROW) as usize;
        (i < 3).then_some(i)
    }
}

impl PrivacyPanel {
    /// The switches: on (allowed, unmuted) or off, each a pill to click.
    fn draw_switches(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        if !self.switches {
            return;
        }
        let g = cx.gfx;
        let top = y + self.sw_y;
        g.fill_rect(rect(x, (top - 8.0).round(), self.width, 1.0), white(0.08 * alpha));
        let names = ["Microphone", "Camera", "Location"];
        let off_glyphs = ["\u{EC54}", "\u{E714}", "\u{E81D}"];
        for i in 0..3 {
            let cy = top + i as f32 * SW_ROW + SW_ROW / 2.0;
            let on = !self.blocked[i];
            if self.hover_sw == Some(i) {
                g.fill_round(rect(x - 6.0, cy - SW_ROW / 2.0 + 2.0, self.width + 12.0, SW_ROW - 4.0), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            let glyph = if on { super::extras::DEVICES[i].1 } else { off_glyphs[i] };
            let color = if on { cx.look.fg } else { rgba(1.0, 90.0 / 255.0, 90.0 / 255.0, 1.0) };
            g.text_at(glyph, &Font::icons(14.0), x, cy + 1.0, with_alpha(color, alpha));
            g.text_at(names[i], &cx.body(), x + 24.0, cy, with_alpha(cx.look.fg, alpha));
            let state = match (i, on) {
                (0, false) => "muted",
                (_, false) => "off for apps",
                _ => "",
            };
            let f = cx.secondary(12.0);
            // The switch: 30 × 18, the knob at its end.
            let (sw, sh) = (30.0, 18.0);
            let sx = x + self.width - sw;
            let fill = if on { if cx.opts.accent { cx.look.accent } else { cx.look.fg } } else { white(0.2) };
            g.fill_round(rect(sx, cy - sh / 2.0, sw, sh), sh / 2.0, with_alpha(fill, alpha));
            let kx = if on { sx + sw - 3.0 - 12.0 } else { sx + 3.0 };
            g.fill_round(rect(kx, cy - 6.0, 12.0, 12.0), 6.0, if on { with_alpha(cx.look.bg, alpha) } else { white(alpha) });
            if !state.is_empty() {
                g.text_at(state, &f, sx - 10.0 - g.measure(state, &f), cy, with_alpha(cx.look.dimmed(), alpha));
            }
        }
    }
}

impl Panel for PrivacyPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["privacy"]
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.sw_at(p));
        let changed = h != self.hover_sw;
        self.hover_sw = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let Some(i) = self.sw_at(at).filter(|_| button == Button::Left) else { return Reply::None };
        // Shown at once; the next reading confirms it.
        let on = self.blocked[i];
        self.blocked[i] = !on;
        let kind = ["mic", "camera", "location"][i];
        Reply::Do(Act::call(move |_| crate::providers::security::set(kind, on)))
    }
    fn closed(&mut self) {
        self.hover_sw = None;
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.blocked = super::extras::blocked(v);
        self.rows = super::extras::DEVICES
            .iter()
            .flat_map(|(key, glyph, label)| v.get(*key).and_then(|a| a.as_array()).cloned().unwrap_or_default().into_iter().filter_map(move |app| Some((*glyph, app.as_str()?.to_string(), *label))))
            .collect();
        true
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        let lh = line(g, &cx.body());
        let (w, h) = if self.rows.is_empty() {
            (g.measure("Nothing is using the microphone, camera or location.", &cx.secondary(cx.look.font_size)).max(260.0), lh)
        } else {
            let w = self.rows.iter().map(|(_, app, label)| 14.0 + 10.0 + g.measure(app, &cx.body()) + 10.0 + g.measure(label, &cx.secondary(cx.look.font_size))).fold(260.0, f32::max);
            (w, self.rows.len() as f32 * lh + (self.rows.len() as f32 - 1.0) * 8.0)
        };
        self.width = w;
        if !self.switches {
            return (w, h);
        }
        self.sw_y = h + 14.0;
        (w, h + 14.0 + 3.0 * SW_ROW)
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
        self.draw_switches(cx, x, y, alpha);
    }
}

// ----------------------------------------------------------------------

/// The hosted tray icons behind the chevron (not the `pinned` ones, which stay in the
/// bar): a grid (`icons_per_row`), or style "list", each with its app's name and hint.
/// A click goes to the app and closes the panel.
pub struct TrayPanel {
    opts: Opts,
    list: bool,
    icons: Vec<(String, String)>,
    /// Each icon's hint (its app's tooltip) and its app's name, by id.
    tips: HashMap<String, String>,
    names: HashMap<String, String>,
    pixels: HashMap<String, super::icon::Pixels>,
    bitmaps: HashMap<String, (String, AppIcon)>,
    style: Option<Style>,
    hovered: Option<usize>,
}

const CELL: f32 = 32.0;
const CELL_GAP: f32 = 4.0;
/// The list: its width, a row's height and the gap between rows.
const TRAY_LIST_W: f32 = 280.0;
const TRAY_ROW: f32 = 30.0;
const TRAY_ROW_GAP: f32 = 2.0;

impl TrayPanel {
    pub fn new(opts: &Opts) -> TrayPanel {
        let list = opts.str("style") == Some("list");
        TrayPanel { opts: opts.clone(), list, icons: Vec::new(), tips: HashMap::new(), names: HashMap::new(), pixels: HashMap::new(), bitmaps: HashMap::new(), style: None, hovered: None }
    }
    fn per_row(&self) -> usize {
        if self.list {
            return 1;
        }
        self.opts.num("icons_per_row").unwrap_or(5.0).max(1.0) as usize
    }
    /// The box of the `i`th icon's cell or row (content px).
    fn cell_box(&self, i: usize) -> (f32, f32, f32, f32) {
        if self.list {
            return (0.0, i as f32 * (TRAY_ROW + TRAY_ROW_GAP), TRAY_LIST_W, TRAY_ROW);
        }
        let (c, r) = ((i % self.per_row()) as f32, (i / self.per_row()) as f32);
        (c * (CELL + CELL_GAP), r * (CELL + CELL_GAP), CELL, CELL)
    }
    fn at(&self, p: (f32, f32)) -> Option<usize> {
        (0..self.icons.len()).find(|&i| {
            let (x, y, w, h) = self.cell_box(i);
            p.0 >= x && p.0 < x + w && p.1 >= y && p.1 < y + h
        })
    }
}

impl Panel for TrayPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["tray"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let pins = super::shell::tray_pins(&self.opts);
        self.icons.clear();
        for i in v.as_array().cloned().unwrap_or_default() {
            let (Some(id), Some(rgba)) = (i.get("id").and_then(|x| x.as_str()), i.get("rgba").and_then(|x| x.as_str())) else { continue };
            let s = |k: &str| i.get(k).and_then(|t| t.as_str()).unwrap_or("").to_string();
            if super::shell::tray_pinned(&pins, &s("exe")) {
                continue;
            }
            if !self.bitmaps.get(id).is_some_and(|(r, _)| r == rgba) {
                if let Some(px) = decode(&i) {
                    self.pixels.insert(id.to_string(), px);
                }
            }
            self.icons.push((id.to_string(), rgba.to_string()));
            // Its app's tooltip, else its program's name (as the tray widget's).
            self.tips.insert(id.to_string(), Some(s("tip")).filter(|t| !t.is_empty()).unwrap_or_else(|| s("exe")));
            let name = Some(s("name")).filter(|t| !t.is_empty()).unwrap_or_else(|| s("exe").trim_end_matches(".exe").to_string());
            self.names.insert(id.to_string(), name);
        }
        true
    }
    fn tooltip(&self) -> Option<String> {
        let (id, _) = self.icons.get(self.hovered?)?;
        self.tips.get(id).cloned().filter(|t| !t.is_empty())
    }
    fn pad(&self) -> (f32, f32, f32) {
        if self.list { (8.0, 8.0, 8.0) } else { (8.0, 12.0, 10.0) }
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
        if self.list {
            return (TRAY_LIST_W, self.icons.len() as f32 * (TRAY_ROW + TRAY_ROW_GAP) - TRAY_ROW_GAP);
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
        let body = cx.body();
        let small = cx.secondary(12.0);
        for (i, (id, _)) in self.icons.iter().enumerate() {
            let (bx, by, bw, bh) = self.cell_box(i);
            let (cx0, cy0) = (x + bx, y + by);
            if self.hovered == Some(i) {
                g.fill_round(rect(cx0, cy0, bw, bh), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            if !self.list {
                if let Some((_, b)) = self.bitmaps.get(id) {
                    b.draw(g, cx0 + 7.0, cy0 + 7.0, 18.0, alpha);
                }
                continue;
            }
            // A row: the icon, its app's name, and its hint when that says more.
            if let Some((_, b)) = self.bitmaps.get(id) {
                b.draw(g, g.snap(cx0 + 8.0), g.snap(cy0 + (TRAY_ROW - 17.0) / 2.0), 17.0, alpha);
            }
            let name = self.names.get(id).cloned().unwrap_or_default();
            let tx = cx0 + 8.0 + 17.0 + 10.0;
            let room = bx + bw - 8.0 - (tx - x);
            let nw = g.text_at(&fit_text(g, &name, &body, room), &body, tx, cy0 + TRAY_ROW / 2.0, with_alpha(cx.look.fg, alpha)).min(room);
            let tip = self.tips.get(id).cloned().unwrap_or_default();
            if !tip.is_empty() && tip != name && !tip.eq_ignore_ascii_case(&format!("{name}.exe")) && room - nw > 40.0 {
                let tip = tip.lines().next().unwrap_or("").to_string();
                g.text_at(&fit_text(g, &tip, &small, room - nw - 8.0), &small, tx + nw + 8.0, cy0 + TRAY_ROW / 2.0, with_alpha(cx.look.dimmed(), alpha));
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
        let (bx, by, bw, bh) = self.cell_box(i);
        // Apps put their menus at the point: the icon's middle.
        let centre = if self.list { (bx + 16.5, by + bh / 2.0) } else { (bx + bw / 2.0, by + bh / 2.0) };
        Reply::CloseAnd(Act::Tray { id: self.icons[i].0.clone(), button, at: centre })
    }
    fn closed(&mut self) {
        self.hovered = None;
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        (0..self.icons.len()).map(|i| self.cell_box(i)).collect()
    }
}
