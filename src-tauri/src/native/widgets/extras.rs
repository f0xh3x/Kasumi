//! Optional extras (src/widgets/extras.ts, claude.ts): the window in front,
//! network speed, privacy indicators, clipboard and notification counts, Claude
//! Code usage.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::{fit_text, row_draw, row_width, Act, Button, Ctx, Opts, Piece, Reply, Roll, Widget};
use crate::native::gfx::{hex, rgba, white, with_alpha};

/// The window in front: its title, its app's name, or both (`show`), up to `max_width`.
pub struct Focus {
    opts: Opts,
    title: String,
    app: String,
}

impl Focus {
    pub fn new(opts: Opts) -> Focus {
        Focus { opts, title: String::new(), app: String::new() }
    }
    fn text(&self, cx: &Ctx) -> String {
        let t = match self.opts.str("show") {
            Some("app") => self.app.clone(),
            Some("both") if !self.app.is_empty() => format!("{} — {}", self.app, self.title),
            _ => self.title.clone(),
        };
        fit_text(cx.gfx, &t, &cx.body(), self.opts.num("max_width").unwrap_or(300.0) as f32)
    }
}

impl Widget for Focus {
    fn keys(&self) -> &'static [&'static str] {
        &["focus"]
    }
    fn tooltip(&self) -> Option<String> {
        Some(self.title.clone()).filter(|t| !t.is_empty())
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let s = |k: &str| v.get(k).and_then(|t| t.as_str()).unwrap_or("").to_string();
        let (title, app) = (s("title"), s("app"));
        let changed = title != self.title || app != self.app;
        self.title = title;
        self.app = app;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        if self.title.is_empty() && self.app.is_empty() {
            return 0.0;
        }
        cx.gfx.measure(&self.text(cx), &cx.body())
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        cx.gfx.text_at(&self.text(cx), &cx.body(), x, cy, white(0.85 * alpha));
    }
    fn click(&mut self, _: Button, _: (f32, f32)) -> Reply {
        Reply::None
    }
}

// ----------------------------------------------------------------------

/// Bytes per second as "1.2 MB/s" (or bits: "9.6 Mb/s").
pub fn rate(bytes: f64, bits: bool) -> String {
    let mut v = if bits { bytes * 8.0 } else { bytes };
    let units = if bits { ["b/s", "Kb/s", "Mb/s", "Gb/s"] } else { ["B/s", "KB/s", "MB/s", "GB/s"] };
    let mut i = 0;
    while v >= 1000.0 && i < units.len() - 1 {
        v /= 1000.0;
        i += 1;
    }
    if v < 10.0 && i > 0 {
        format!("{v:.1} {}", units[i])
    } else {
        format!("{} {}", v.round(), units[i])
    }
}

/// Live download / upload rate (`show`: both | down | up, `units`: bytes | bits).
pub struct Netspeed {
    opts: Opts,
    down: Roll,
    up: Roll,
}

impl Netspeed {
    pub fn new(opts: Opts) -> Netspeed {
        Netspeed { opts, down: Roll::default(), up: Roll::default() }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let show = self.opts.str("show").unwrap_or("both");
        let part = |glyph: &str, r: &Roll| {
            // .netspeed-value: at least 58 px, so the row doesn't jump with every rate.
            let text = r.piece(cx.body(), cx.look.fg);
            let w = text.width(cx.gfx);
            vec![Piece::glyph(glyph, 11.0, cx.look.dimmed()), Piece::Gap(4.0), text, Piece::Gap((58.0 - w).max(0.0))]
        };
        let mut out = Vec::new();
        if show != "up" {
            out.extend(part("\u{E74B}", &self.down));
        }
        if show != "down" {
            if !out.is_empty() {
                out.push(Piece::Gap(10.0));
            }
            out.extend(part("\u{E74A}", &self.up));
        }
        out
    }
}

impl Widget for Netspeed {
    fn tooltip(&self) -> Option<String> {
        Some(format!("Download {}  ·  Upload {}", self.down.text, self.up.text))
    }
    fn keys(&self) -> &'static [&'static str] {
        &["netspeed"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let bits = self.opts.str("units") == Some("bits");
        let get = |k: &str| v.get(k).and_then(|n| n.as_f64()).unwrap_or(0.0);
        let a = self.down.set(rate(get("down"), bits), self.opts.roll);
        let b = self.up.set(rate(get("up"), bits), self.opts.roll);
        a || b
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
    fn animating(&self, now: Instant) -> bool {
        self.down.animating(now) || self.up.animating(now)
    }
    fn click(&mut self, _: Button, _: (f32, f32)) -> Reply {
        Reply::None
    }
}

// ----------------------------------------------------------------------

/// Microphone / camera / location in use: a warning-coloured icon each, only while some app uses it.
pub const DEVICES: [(&str, &str, &str); 3] = [("mic", "\u{E720}", "Microphone"), ("camera", "\u{E714}", "Camera"), ("location", "\u{E81D}", "Location")];

pub struct Privacy {
    opts: Opts,
    on: [bool; 3],
}

impl Privacy {
    pub fn new(opts: Opts) -> Privacy {
        Privacy { opts, on: [false; 3] }
    }
}

impl Widget for Privacy {
    fn keys(&self) -> &'static [&'static str] {
        &["privacy"]
    }
    fn tooltip(&self) -> Option<String> {
        let used: Vec<&str> = DEVICES.iter().enumerate().filter(|(i, _)| self.on[*i]).map(|(_, d)| d.2).collect();
        Some(if used.is_empty() { "Nothing is using the microphone, camera or location".into() } else { format!("In use: {}", used.join(", ")) })
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let mut on = [false; 3];
        for (i, (key, _, _)) in DEVICES.iter().enumerate() {
            on[i] = self.opts.flag(key) != Some(false) && v.get(*key).and_then(|a| a.as_array()).is_some_and(|a| !a.is_empty());
        }
        let changed = on != self.on;
        self.on = on;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
}

impl Privacy {
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let mut out = Vec::new();
        for (i, (_, glyph, _)) in DEVICES.iter().enumerate().filter(|(i, _)| self.on[*i]) {
            let _ = i;
            if !out.is_empty() {
                out.push(Piece::Gap(6.0));
            }
            out.push(Piece::glyph(*glyph, 14.0, cx.look.warn));
        }
        out
    }
}

// ----------------------------------------------------------------------

/// Clipboard history: the icon (+ how many with `count`); its panel lists them.
pub struct Clipboard {
    opts: Opts,
    count: usize,
}

impl Clipboard {
    pub fn new(opts: Opts) -> Clipboard {
        Clipboard { opts, count: 0 }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let mut out = vec![Piece::glyph("\u{E77F}", 14.0, cx.look.fg)];
        if self.opts.flag("count") == Some(true) && self.count > 0 {
            out.push(Piece::Gap(5.0));
            out.push(Piece::text(self.count.to_string(), cx.body(), cx.look.fg));
        }
        out
    }
}

impl Widget for Clipboard {
    fn keys(&self) -> &'static [&'static str] {
        &["clipboard"]
    }
    fn tooltip(&self) -> Option<String> {
        Some("Clipboard history".into())
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let n = v.as_array().map_or(0, |a| a.len());
        let changed = n != self.count;
        self.count = n;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
}

// ----------------------------------------------------------------------

/// The bell and how many notifications there are; new ones in `color` (#4cc2ff).
/// A click opens the panel and marks them seen. With the nudge effect the bell
/// shakes when one more arrives.
pub struct Notifications {
    opts: Opts,
    count: Roll,
    total: i64,
    unseen: bool,
    shake: Option<Instant>,
}

impl Notifications {
    pub fn new(opts: Opts) -> Notifications {
        Notifications { opts, count: Roll::default(), total: -1, unseen: false, shake: None }
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let new = self.opts.str("color").and_then(hex).unwrap_or(rgba(0x4c as f32 / 255.0, 0xc2 as f32 / 255.0, 1.0, 1.0));
        let color = if self.unseen { new } else { cx.look.fg };
        let mut out = vec![Piece::glyph("\u{EA8F}", 14.0, color)];
        if self.opts.flag("count") != Some(false) && self.total > 0 {
            out.push(Piece::Gap(5.0));
            out.push(self.count.piece(cx.body(), color));
        }
        out
    }
    /// The bell's tilt now (styles.css bell-shake: 700 ms, easing out).
    fn tilt(&self, now: Instant) -> f32 {
        let Some(t0) = self.shake else { return 0.0 };
        let t = now.saturating_duration_since(t0).as_secs_f32() / 0.7;
        if t >= 1.0 {
            return 0.0;
        }
        let keys = [(0.0, 0.0), (0.15, 16.0), (0.3, -14.0), (0.45, 10.0), (0.6, -6.0), (0.75, 3.0), (1.0, 0.0)];
        let i = keys.iter().rposition(|k| k.0 <= t).unwrap_or(0).min(keys.len() - 2);
        let (a, b) = (keys[i], keys[i + 1]);
        a.1 + (b.1 - a.1) * super::ease((0.25, 0.1, 0.25, 1.0), (t - a.0) / (b.0 - a.0))
    }
}

impl Widget for Notifications {
    fn keys(&self) -> &'static [&'static str] {
        &["notifications"]
    }
    fn tooltip(&self) -> Option<String> {
        Some(match self.total.max(0) {
            0 => "No notifications".into(),
            n => format!("{n} notification{}", if n == 1 { "" } else { "s" }),
        })
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        let total = v.get("count").and_then(|c| c.as_i64()).unwrap_or(0);
        if self.opts.nudge && self.total >= 0 && total > self.total {
            self.shake = Some(Instant::now());
        }
        let unseen = v.get("unseen").and_then(|c| c.as_i64()).unwrap_or(0) > 0;
        let changed = self.count.set(if total > 0 { total.to_string() } else { String::new() }, self.opts.roll) || unseen != self.unseen || total != self.total;
        self.total = total;
        self.unseen = unseen;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let pieces = self.pieces(cx);
        let tilt = self.tilt(Instant::now());
        if tilt.abs() > 0.01 {
            // Rotated about its top middle (transform-origin 50% 15%).
            let w = pieces[0].width(cx.gfx);
            let (px, py) = (x + w / 2.0, cy - 7.0 + 14.0 * 0.15);
            unsafe {
                let mut was = windows_numerics::Matrix3x2::identity();
                cx.gfx.rt.GetTransform(&mut was);
                let rot = windows_numerics::Matrix3x2::rotation_around(tilt, windows_numerics::Vector2 { X: px, Y: py });
                cx.gfx.rt.SetTransform(&(rot * was));
                row_draw(cx.gfx, &pieces[..1], x, cy, alpha);
                cx.gfx.rt.SetTransform(&was);
            }
            row_draw(cx.gfx, &pieces[1..], x + w, cy, alpha);
        } else {
            row_draw(cx.gfx, &pieces, x, cy, alpha);
        }
    }
    fn animating(&self, now: Instant) -> bool {
        self.count.animating(now) || self.shake.is_some_and(|t| now.saturating_duration_since(t) < Duration::from_millis(720))
    }
    fn click(&mut self, button: Button, _: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        Reply::ToggleAnd(Act::call(|app| {
            tauri::async_runtime::block_on(async {
                let _ = crate::providers::notifications::notifications_seen(app).await;
            })
        }))
    }
}

// ----------------------------------------------------------------------
// Claude Code usage.

/// 1234 -> "1.2k", 3_400_000 -> "3.4M".
pub fn tokens(n: f64) -> String {
    if n >= 1e9 {
        format!("{:.1}B", n / 1e9)
    } else if n >= 1e6 {
        if n >= 1e7 { format!("{:.0}M", n / 1e6) } else { format!("{:.1}M", n / 1e6) }
    } else if n >= 1e3 {
        if n >= 1e4 { format!("{:.0}k", n / 1e3) } else { format!("{:.1}k", n / 1e3) }
    } else {
        format!("{}", n as i64)
    }
}

/// "2h 5m" / "40m".
pub fn duration(secs: f64) -> String {
    let m = (secs / 60.0).round().max(0.0) as i64;
    if m >= 60 { format!("{}h {}m", m / 60, m % 60) } else { format!("{m}m") }
}

pub fn now_secs() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |d| d.as_secs_f64())
}

/// Local midnight `days` days ago, in unix seconds.
pub fn day_start(days: i64) -> f64 {
    let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let since_midnight = t.wHour as f64 * 3600.0 + t.wMinute as f64 * 60.0 + t.wSecond as f64 + t.wMilliseconds as f64 / 1000.0;
    now_secs() - since_midnight - days as f64 * 86400.0
}

/// Tokens in hourly buckets from `from` up to `to` (unix seconds).
pub fn total_tokens(u: &Value, from: f64, to: f64) -> f64 {
    u.get("buckets")
        .and_then(|b| b.as_array())
        .map(|b| b.iter().filter(|b| b.get("h").and_then(|h| h.as_f64()).is_some_and(|h| h >= from && h < to)).filter_map(|b| b.get("tokens").and_then(|t| t.as_f64())).sum())
        .unwrap_or(0.0)
}

/// A plan limit: its percent and when it resets (unix seconds); one whose reset
/// has passed starts over.
pub fn limit(l: Option<&Value>) -> Option<(f64, Option<f64>)> {
    let l = l.filter(|l| !l.is_null())?;
    let percent = l.get("percent").and_then(|p| p.as_f64()).unwrap_or(0.0);
    let resets = l.get("resetsAt").and_then(|r| r.as_str()).and_then(|r| chrono::DateTime::parse_from_rfc3339(r).ok()).map(|d| d.timestamp() as f64);
    match resets {
        Some(t) if t <= now_secs() => Some((0.0, None)),
        r => Some((percent, r)),
    }
}

/// The account's limits (session, week) when `account` is on and it has them.
pub fn limits(u: &Value, opts: &Opts) -> (Option<(f64, Option<f64>)>, Option<(f64, Option<f64>)>) {
    let a = if opts.flag("account") != Some(false) { u.get("account").filter(|a| !a.is_null()) } else { None };
    (limit(a.and_then(|a| a.get("session"))), limit(a.and_then(|a| a.get("week"))))
}

/// Claude Code usage: a working dot, a meter and text, each optional.
pub struct Claude {
    opts: Opts,
    latest: Value,
    /// The pulsing dot's step while Claude works.
    pulse: usize,
    next: Instant,
}

impl Claude {
    pub fn new(opts: Opts) -> Claude {
        Claude { opts, latest: Value::Null, pulse: 0, next: Instant::now() + Duration::from_secs(30) }
    }
    fn busy(&self) -> bool {
        self.latest.get("busy").and_then(|b| b.as_bool()).unwrap_or(false)
    }
    /// The share the indicator shows (0..1): the account's limit, else of the set limit, else of the 5-hour window's time.
    fn share(&self) -> f64 {
        let u = &self.latest;
        let (session, week) = limits(u, &self.opts);
        let for_week = self.opts.str("indicator_for") == Some("week");
        if for_week {
            if let Some((p, _)) = week {
                return p / 100.0;
            }
            let l = self.opts.num("weekly_limit").unwrap_or(0.0);
            return if l > 0.0 { total_tokens(u, day_start(6), f64::INFINITY) / l } else { 0.0 };
        }
        if let Some((p, _)) = session {
            return p / 100.0;
        }
        let Some(w) = u.get("window").filter(|w| !w.is_null()) else { return 0.0 };
        let g = |k: &str| w.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let l = self.opts.num("window_limit").unwrap_or(0.0);
        if l > 0.0 { g("tokens") / l } else { (now_secs() - g("start")) / (g("end") - g("start")).max(1.0) }
    }
    fn text(&self) -> String {
        let u = &self.latest;
        let now = now_secs();
        let w = u.get("window").filter(|w| w.get("end").and_then(|e| e.as_f64()).is_some_and(|e| now < e));
        let wv = |k: &str| w.and_then(|w| w.get(k)).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let (session, week) = limits(u, &self.opts);
        let pct = |p: f64| format!("{}%", p.round());
        let mut bits = Vec::new();
        if self.opts.flag("show_window") != Some(false) {
            bits.push(match (session, w) {
                (Some((p, _)), _) => pct(p),
                (None, Some(_)) => tokens(wv("tokens")),
                _ => "0".into(),
            });
        }
        if self.opts.flag("show_reset") != Some(false) {
            match (session, w) {
                (Some((_, Some(r))), _) => bits.push(duration(r - now)),
                (_, Some(_)) => bits.push(duration(wv("end") - now)),
                _ => {}
            }
        }
        if self.opts.flag("show_today") == Some(true) {
            bits.push(format!("today {}", tokens(total_tokens(u, day_start(0), f64::INFINITY))));
        }
        if self.opts.flag("show_week_left") == Some(true) {
            match week {
                Some((p, _)) => bits.push(format!("week {}", pct(p))),
                None => {
                    let l = self.opts.num("weekly_limit").unwrap_or(0.0);
                    let used = total_tokens(u, day_start(6), f64::INFINITY);
                    bits.push(if l > 0.0 { format!("{} left", tokens((l - used).max(0.0))) } else { format!("week {}", tokens(used)) });
                }
            }
        }
        bits.join(" · ")
    }
    fn pieces(&self, cx: &Ctx) -> Vec<Piece> {
        let kind = self.opts.str("indicator").unwrap_or("bar");
        let v = self.share();
        let high = v >= 0.9;
        let fill = if high {
            cx.look.warn
        } else if cx.opts.accent {
            cx.look.accent
        } else {
            cx.look.fg
        };
        let meter = match kind {
            "none" => None,
            "circle" => Some(Piece::Ring { v: v as f32, fill }),
            _ => Some(Piece::Bar { len: self.opts.num("bar_length").unwrap_or(40.0) as f32, v: v as f32, fill }),
        };
        let right = self.opts.str("indicator_side") == Some("right");
        let text = self.text();
        let mut parts = Vec::new();
        if self.opts.flag("working") != Some(false) {
            // The dot pulses in six opacity steps while Claude works.
            const STEPS: [f32; 6] = [1.0, 0.8, 0.55, 0.35, 0.55, 0.8];
            let color = if self.busy() { with_alpha(rgba(0xd9 as f32 / 255.0, 0x77 as f32 / 255.0, 0x57 as f32 / 255.0, 1.0), STEPS[self.pulse % 6]) } else { white(0.25) };
            parts.push(Piece::Dot { d: 6.0, color });
        }
        let mut meter = meter;
        if !right {
            parts.extend(meter.take());
        }
        if self.opts.flag("text") != Some(false) && !text.is_empty() {
            parts.push(Piece::text(text, cx.body(), cx.look.fg));
        }
        parts.extend(meter);
        let mut out = Vec::new();
        for (i, p) in parts.into_iter().enumerate() {
            if i > 0 {
                out.push(Piece::Gap(7.0));
            }
            out.push(p);
        }
        out
    }
}

impl Widget for Claude {
    fn tooltip(&self) -> Option<String> {
        Some(if self.busy() { "Claude Code is working".into() } else { "Claude Code".into() })
    }
    fn keys(&self) -> &'static [&'static str] {
        &["claude"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.latest = v.clone();
        if self.busy() {
            self.next = self.next.min(Instant::now() + Duration::from_millis(200));
        }
        true
    }
    fn wake(&self) -> Option<Instant> {
        Some(self.next)
    }
    fn tick(&mut self, now: Instant) -> bool {
        // The pulse every 200 ms while busy; the countdown every 30 s.
        if self.busy() {
            self.pulse += 1;
            self.next = now + Duration::from_millis(200);
        } else {
            self.pulse = 0;
            self.next = now + Duration::from_secs(30);
        }
        true
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        row_width(cx.gfx, &self.pieces(cx))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        row_draw(cx.gfx, &self.pieces(cx), x, cy, alpha);
    }
}
