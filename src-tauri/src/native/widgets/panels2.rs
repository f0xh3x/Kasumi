//! The Claude usage panel (claude.ts claudePanel) and the media panel
//! (media-panel.ts): album art, title, a seek bar to drag, the keys; or one of its
//! looks ([widgets.media] style: record, glow, cover).

use std::time::{Duration, Instant};

use serde_json::Value;

use super::extras::{day_start, duration, elapsed, limits, now_secs, span, tokens, total_tokens, used};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply};
use crate::native::gfx::{rect, rgba, white, with_alpha, Color, Font, Gfx};

/// A line of text's height in a font.
fn line(g: &Gfx, f: &Font) -> f32 {
    g.text("Ag", f).map_or(f.size * 1.21, |t| t.height)
}

/// Local time parts of a unix time: (weekday Monday = 0, hour, minute).
fn local_parts(unix: f64) -> (i64, u32, u32) {
    // The offset from UTC now (good enough for a reset an hour or a week away).
    let (l, u) = unsafe { (windows::Win32::System::SystemInformation::GetLocalTime(), windows::Win32::System::SystemInformation::GetSystemTime()) };
    let secs = |t: &windows::Win32::Foundation::SYSTEMTIME| super::clock::days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32) * 86400 + t.wHour as i64 * 3600 + t.wMinute as i64 * 60 + t.wSecond as i64;
    let local = unix as i64 + (secs(&l) - secs(&u));
    let days = local.div_euclid(86400);
    let rem = local.rem_euclid(86400);
    (super::clock::weekday(days), (rem / 3600) as u32, ((rem % 3600) / 60) as u32)
}

const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// The Claude panel's look ([widgets.claude] style).
#[derive(Clone, Copy, PartialEq)]
enum ClaudeStyle {
    /// Labelled lines: the plan's limits, the window, today, the last 7 days.
    Report,
    /// The two limits as rings, when each resets, today's totals.
    Rings,
    /// Each limit as a timeline with a mark for now, and when you'd run out.
    Pace,
    /// The sessions of the last hour: working, waiting on you, done or idle.
    Sessions,
}

/// The 5-hour window, today (per model) and the last 7 days, plus the plan's limits;
/// or one of its looks.
pub struct ClaudePanel {
    opts: Opts,
    style: ClaudeStyle,
    u: Value,
    /// Sessions: the row under the cursor.
    hovered: Option<usize>,
}

const RINGS_W: f32 = 340.0;
const RING_BOX: f32 = 118.0;
const PACE_W: f32 = 330.0;
const SESS_W: f32 = 320.0;
const SESS_ROW: f32 = 34.0;

/// "14:02" for a unix time.
fn hm(t: f64) -> String {
    let (_, h, m) = local_parts(t);
    format!("{h:02}:{m:02}")
}

const DAYS: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];

enum Line {
    Head(String),
    Row(String, String, bool),
    Chart(Vec<(f64, &'static str)>),
}

impl ClaudePanel {
    pub fn new(opts: &Opts) -> ClaudePanel {
        let style = match opts.str("style") {
            Some("rings") => ClaudeStyle::Rings,
            Some("pace") => ClaudeStyle::Pace,
            Some("sessions") => ClaudeStyle::Sessions,
            _ => ClaudeStyle::Report,
        };
        ClaudePanel { opts: opts.clone(), style, u: Value::Null, hovered: None }
    }

    /// Today: tokens and messages.
    fn today(&self) -> (f64, f64) {
        let from = day_start(0);
        let sum = |k: &str| {
            self.u.get("buckets").and_then(|b| b.as_array()).into_iter().flatten().filter(|b| b.get("h").and_then(|h| h.as_f64()).is_some_and(|h| h >= from)).filter_map(|b| b.get(k).and_then(|v| v.as_f64())).sum::<f64>()
        };
        (sum("tokens"), sum("messages"))
    }
    /// The sessions the provider lists.
    fn sessions(&self) -> Vec<&Value> {
        self.u.get("sessions").and_then(|s| s.as_array()).map(|a| a.iter().collect()).unwrap_or_default()
    }
    /// When a limit resets, in words: "resets 14:02, in 3 h 10 min" / "resets Saturday 22:52".
    fn resets(&self, week: bool) -> Option<String> {
        let (_, end) = span(&self.u, &self.opts, week)?;
        Some(if week { format!("resets {} {}", DAYS[local_parts(end).0 as usize], hm(end)) } else { format!("resets {}, in {}", hm(end), duration(end - now_secs())) })
    }

    fn draw_rings(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let accent = with_alpha(cx.look.accent, alpha);
        let warn = with_alpha(cx.look.warn, alpha);
        let (s5, sw) = (used(&self.u, &self.opts, false), used(&self.u, &self.opts, true));
        let has_week = span(&self.u, &self.opts, true).is_some() || self.opts.num("weekly_limit").unwrap_or(0.0) > 0.0;
        let (rcx, rcy) = (x + RING_BOX / 2.0, y + RING_BOX / 2.0);
        if has_week {
            g.ring(rcx, rcy, 52.0, 9.0, sw as f32, white(0.15 * alpha), if sw >= 0.9 { warn } else { fg });
        }
        g.ring(rcx, rcy, if has_week { 38.0 } else { 48.0 }, 9.0, s5 as f32, white(0.15 * alpha), if s5 >= 0.9 { warn } else { accent });
        let pct = format!("{}%", (s5 * 100.0).round());
        let big = cx.main(18.0);
        g.text_at(&pct, &big, rcx - g.measure(&pct, &big) / 2.0, rcy, fg);
        // Beside them: each limit, when it resets; today.
        let f = cx.secondary(12.0);
        let small = cx.secondary(11.0);
        let lx = x + RING_BOX + 16.0;
        let lw = RINGS_W - RING_BOX - 16.0;
        let mut ly = y + (RING_BOX - self.legend_h(g, cx)) / 2.0;
        let mut entry = |name: &str, share: f64, color: crate::native::gfx::Color, sub: Option<String>| {
            let cy = ly + line(g, &f) / 2.0;
            g.fill_round(rect(lx, cy - 3.0, 6.0, 6.0), 3.0, color);
            g.text_at(name, &f, lx + 14.0, cy, fg);
            let p = format!("{}%", (share * 100.0).round());
            g.text_at(&p, &f, lx + lw - g.measure(&p, &f), cy, fg);
            ly += line(g, &f) + 2.0;
            if let Some(sub) = sub {
                g.text_at(&fit_text(g, &sub, &small, lw - 14.0), &small, lx + 14.0, ly + line(g, &small) / 2.0, dim);
                ly += line(g, &small);
            }
            ly += 9.0;
        };
        entry("5 hours", s5, accent, self.resets(false));
        if has_week {
            entry("Week", sw, fg, self.resets(true));
        }
        let (t, m) = self.today();
        g.text_at(&fit_text(g, &format!("Today {} tokens · {} messages", tokens(t), m), &small, lw), &small, lx, ly + line(g, &small) / 2.0, dim);
    }
    fn legend_h(&self, g: &Gfx, cx: &Ctx) -> f32 {
        let (f, small) = (line(g, &cx.secondary(12.0)), line(g, &cx.secondary(11.0)));
        let has_week = span(&self.u, &self.opts, true).is_some() || self.opts.num("weekly_limit").unwrap_or(0.0) > 0.0;
        (if has_week { 2.0 } else { 1.0 }) * (f + 2.0 + small + 9.0) + small
    }

    /// Pace: a limit's rows (its height).
    fn pace_rows(&self, week: bool) -> Option<(f64, f64, f64, f64)> {
        let (a, b) = span(&self.u, &self.opts, week)?;
        Some((a, b, used(&self.u, &self.opts, week), elapsed(&self.u, &self.opts, week)?))
    }
    fn pace_h(&self, g: &Gfx, cx: &Ctx) -> f32 {
        let (small, f) = (line(g, &cx.secondary(11.0)), line(g, &cx.secondary(12.0)));
        let block = small + 7.0 + 14.0 + 7.0 + f + 7.0 + small;
        let n = [false, true].iter().filter(|w| self.pace_rows(**w).is_some()).count() as f32;
        if n == 0.0 { f } else { n * block + (n - 1.0) * 14.0 }
    }
    fn draw_pace(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim, warn) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha), with_alpha(cx.look.warn, alpha));
        let accent = with_alpha(cx.look.accent, alpha);
        let (f, small) = (cx.secondary(12.0), cx.secondary(11.0));
        let mut yy = y;
        let mut any = false;
        for week in [false, true] {
            let Some((a, b, u, t)) = self.pace_rows(week) else { continue };
            any = true;
            let head = if week { "WEEK" } else { "5 HOURS" };
            let mut hx = x;
            for ch in head.chars() {
                hx += g.text_at(&ch.to_string(), &small, hx, yy + line(g, &small) / 2.0, dim) + 0.5;
            }
            let when = if week { format!("until {} {}", DAYS[local_parts(b).0 as usize], hm(b)) } else { format!("{} – {}", hm(a), hm(b)) };
            g.text_at(&when, &small, x + PACE_W - g.measure(&when, &small), yy + line(g, &small) / 2.0, dim);
            yy += line(g, &small) + 7.0;
            // The timeline: used so far, and a mark for now.
            let cy = yy + 7.0;
            g.fill_round(rect(x, cy - 3.0, PACE_W, 6.0), 3.0, white(0.15 * alpha));
            let fill = if u >= 0.9 { warn } else if week { white(0.85 * alpha) } else { accent };
            if u > 0.0 {
                g.fill_round(rect(x, cy - 3.0, PACE_W * u.min(1.0) as f32, 6.0), 3.0, fill);
            }
            let mx = g.snap(x + PACE_W * t as f32 - 1.0);
            g.fill_round(rect(mx, cy - 7.0, 2.0, 14.0), 1.0, if week { accent } else { fg });
            yy += 14.0 + 7.0;
            let used_w = g.text_at(&format!("{}% used", (u * 100.0).round()), &f, x, yy + line(g, &f) / 2.0, fg);
            let gone = format!(" · {}% of the {} gone", (t * 100.0).round(), if week { "week" } else { "time" });
            g.text_at(&gone, &f, x + used_w, yy + line(g, &f) / 2.0, dim);
            yy += line(g, &f) + 7.0;
            // Where this pace ends up: the limit reached before the reset, or how full by then.
            let (text, color) = if t > 0.03 && u / t >= 1.0 && u < 1.0 {
                let out = a + (now_secs() - a) / u;
                let early = b - out;
                let when = if week { format!("{} {}", DAYS[local_parts(out).0 as usize], hm(out)) } else { hm(out) };
                (format!("At this pace you reach the limit around {when}, {} early", duration(early).replace('h', " h").replace('m', " min")), warn)
            } else if u >= 1.0 {
                ("Limit reached".to_string(), warn)
            } else if t > 0.03 {
                (format!("On pace for {}% by the reset", ((u / t) * 100.0).round().min(100.0)), dim)
            } else {
                ("Just started".to_string(), dim)
            };
            g.text_at(&fit_text(g, &text, &small, PACE_W), &small, x, yy + line(g, &small) / 2.0, color);
            yy += line(g, &small) + 14.0;
        }
        if !any {
            g.text_at("No limits to pace yet", &f, x, y + line(g, &f) / 2.0, dim);
        }
    }

    fn draw_sessions(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let (f, small) = (cx.secondary(12.5), cx.secondary(11.5));
        let list = self.sessions();
        if list.is_empty() {
            g.text_at("No Claude Code sessions in the last hour", &f, x, y + line(g, &f) / 2.0, dim);
            return;
        }
        for (i, s) in list.iter().enumerate() {
            let top = y + i as f32 * (SESS_ROW + 2.0);
            let cy = top + SESS_ROW / 2.0;
            if self.hovered == Some(i) {
                g.fill_round(rect(x, top, SESS_W, SESS_ROW), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            let st = s.get("state").and_then(|v| v.as_str()).unwrap_or("idle");
            let quiet = s.get("quiet").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let (dot, text, tc) = match st {
                "working" => (with_alpha(rgba(0xd9 as f32 / 255.0, 0x77 as f32 / 255.0, 0x57 as f32 / 255.0, 1.0), alpha), "working".to_string(), dim),
                "asks" => (with_alpha(cx.look.accent, alpha), "asks you".to_string(), with_alpha(cx.look.accent, alpha)),
                "tool" => (with_alpha(cx.look.accent, alpha), format!("may need you · {}", duration(quiet).replace('h', " h").replace('m', " min")), with_alpha(cx.look.accent, alpha)),
                "done" => (white(0.6 * alpha), format!("your turn · {}", duration(quiet).replace('h', " h").replace('m', " min")), dim),
                _ => (white(0.25 * alpha), format!("idle {}", duration(quiet).replace('h', " h").replace('m', " min")), dim),
            };
            g.fill_round(rect(x + 8.0, cy - 3.0, 6.0, 6.0), 3.0, dot);
            let tw = g.measure(&text, &small);
            g.text_at(&text, &small, x + SESS_W - 8.0 - tw, cy, tc);
            let name = s.get("project").and_then(|v| v.as_str()).unwrap_or("");
            g.text_at(&fit_text(g, name, &f, SESS_W - 8.0 - 14.0 - tw - 16.0), &f, x + 8.0 + 14.0, cy, fg);
        }
        let fy = y + list.len() as f32 * (SESS_ROW + 2.0) + 4.0 + line(g, &small) / 2.0;
        g.text_at("Click one to go to its window", &small, x + 8.0, fy, dim);
    }

    fn lines(&self) -> Vec<Line> {
        let u = &self.u;
        let opts = &self.opts;
        let now = now_secs();
        let w = u.get("window").filter(|w| w.get("end").and_then(|e| e.as_f64()).is_some_and(|e| now < e));
        let wv = |k: &str| w.and_then(|w| w.get(k)).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let wl = opts.num("window_limit").unwrap_or(0.0);
        let kl = opts.num("weekly_limit").unwrap_or(0.0);
        let mut out = Vec::new();
        let row = |l: &str, v: String, dim: bool| Line::Row(l.to_string(), v, dim);
        let pct = |p: f64| format!("{}%", p.round());
        let hm = |t: f64| {
            let (_, h, m) = local_parts(t);
            format!("{h:02}:{m:02}")
        };
        let a = if opts.flag("account") != Some(false) { u.get("account").filter(|a| !a.is_null()) } else { None };
        let (session, week) = limits(u, opts);
        if a.is_some() && (session.is_some() || week.is_some()) {
            out.push(Line::Head("YOUR PLAN".into()));
            if let Some((p, r)) = session {
                out.push(row("5-hour limit", pct(p), false));
                out.push(row("Resets", r.map_or(String::new(), |r| format!("{} (in {})", hm(r), duration(r - now))), true));
            }
            if let Some((p, r)) = week {
                out.push(row("Weekly limit", pct(p), false));
                out.push(row("Resets", r.map_or(String::new(), |r| format!("{} {}", WEEKDAYS[local_parts(r).0 as usize], hm(r))), true));
            }
            for (key, label) in [("weekOpus", "Opus this week"), ("weekSonnet", "Sonnet this week")] {
                if let Some(p) = a.and_then(|a| a.get(key)).and_then(|l| l.get("percent")).and_then(|p| p.as_f64()) {
                    out.push(row(label, pct(p), true));
                }
            }
        } else if a.and_then(|a| a.get("error")).and_then(|e| e.as_str()) == Some("expired") {
            out.push(Line::Head("YOUR PLAN".into()));
            out.push(row("Sign-in expired", "open Claude Code to renew it".into(), true));
        }
        out.push(Line::Head("5-HOUR WINDOW".into()));
        if w.is_some() {
            out.push(row("Used", if wl > 0.0 { format!("{} of {}", tokens(wv("tokens")), tokens(wl)) } else { tokens(wv("tokens")) }, false));
            if wl > 0.0 {
                out.push(row("Left", tokens((wl - wv("tokens")).max(0.0)), false));
            }
            out.push(row("Resets", format!("{} (in {})", hm(wv("end")), duration(wv("end") - now)), false));
            out.push(row("Messages", format!("{}", wv("messages")), true));
        } else {
            out.push(row("Not running", "starts with your next message".into(), true));
        }
        // Today, from the hourly buckets.
        let from = day_start(0);
        let today: Vec<&Value> = u
            .get("buckets")
            .and_then(|b| b.as_array())
            .map(|b| b.iter().filter(|b| b.get("h").and_then(|h| h.as_f64()).is_some_and(|h| h >= from)).collect())
            .unwrap_or_default();
        let sum = |k: &str| today.iter().filter_map(|b| b.get(k).and_then(|v| v.as_f64())).sum::<f64>();
        let mut sessions: Vec<&str> = today.iter().flat_map(|b| b.get("sessions").and_then(|s| s.as_array()).into_iter().flatten().filter_map(|s| s.as_str())).collect();
        sessions.sort();
        sessions.dedup();
        out.push(Line::Head("TODAY".into()));
        out.push(row("Tokens", tokens(sum("tokens")), false));
        out.push(row("Messages", format!("{}", sum("messages")), true));
        out.push(row("Sessions", sessions.len().to_string(), true));
        let mut models: Vec<(String, f64)> = Vec::new();
        for b in &today {
            let m = b.get("model").and_then(|m| m.as_str()).unwrap_or("").trim_start_matches("claude-").to_string();
            let t = b.get("tokens").and_then(|t| t.as_f64()).unwrap_or(0.0);
            match models.iter_mut().find(|(k, _)| *k == m) {
                Some((_, v)) => *v += t,
                None => models.push((m, t)),
            }
        }
        models.sort_by(|a, b| b.1.total_cmp(&a.1));
        for (m, t) in models {
            out.push(row(&m, tokens(t), true));
        }
        if sum("cacheRead") > 0.0 {
            out.push(row("Cache reads", tokens(sum("cacheRead")), true));
        }
        if opts.flag("weekly") != Some(false) {
            let week = total_tokens(u, day_start(6), f64::INFINITY);
            out.push(Line::Head("LAST 7 DAYS".into()));
            out.push(row("Tokens", if kl > 0.0 { format!("{} of {}", tokens(week), tokens(kl)) } else { tokens(week) }, false));
            if kl > 0.0 {
                out.push(row("Left", tokens((kl - week).max(0.0)), false));
            }
            const NARROW: [&str; 7] = ["M", "T", "W", "T", "F", "S", "S"];
            out.push(Line::Chart(
                (0..7)
                    .map(|i| {
                        let (a, b) = (day_start(6 - i), day_start(5 - i));
                        (total_tokens(u, a, if i == 6 { f64::INFINITY } else { b }), NARROW[local_parts(a + 3600.0).0 as usize])
                    })
                    .collect(),
            ));
        }
        out
    }
}

impl Panel for ClaudePanel {
    fn keys(&self) -> &'static [&'static str] {
        &["claude"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.u = v.clone();
        true
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let n = if self.style == ClaudeStyle::Sessions { self.sessions().len() } else { 0 };
        let h = at.and_then(|(x, y)| {
            let i = (y / (SESS_ROW + 2.0)).floor();
            (x >= 0.0 && x < SESS_W && i >= 0.0 && (i as usize) < n && y - i * (SESS_ROW + 2.0) < SESS_ROW).then_some(i as usize)
        });
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn tooltip(&self) -> Option<String> {
        let s = self.sessions().into_iter().nth(self.hovered?)?;
        s.get("title").and_then(|t| t.as_str()).filter(|t| !t.is_empty()).map(String::from)
    }
    fn click(&mut self, button: Button, _at: (f32, f32)) -> Reply {
        let Some(s) = self.hovered.and_then(|i| self.sessions().into_iter().nth(i)).filter(|_| button == Button::Left) else { return Reply::None };
        let title = s.get("title").and_then(|t| t.as_str()).unwrap_or("").to_string();
        let project = s.get("project").and_then(|t| t.as_str()).unwrap_or("").to_string();
        Reply::CloseAnd(Act::call(move |_| crate::providers::claude::focus_session(&title, &project)))
    }
    fn closed(&mut self) {
        self.hovered = None;
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        match self.style {
            ClaudeStyle::Rings => return (RINGS_W, RING_BOX.max(self.legend_h(g, cx))),
            ClaudeStyle::Pace => return (PACE_W, self.pace_h(g, cx)),
            ClaudeStyle::Sessions => {
                let n = self.sessions().len() as f32;
                let small = line(g, &cx.secondary(11.5));
                return (SESS_W, if n == 0.0 { line(g, &cx.secondary(12.5)) } else { n * (SESS_ROW + 2.0) + 4.0 + small });
            }
            ClaudeStyle::Report => {}
        }
        let mut h = 0.0;
        for (i, l) in self.lines().iter().enumerate() {
            if i > 0 {
                h += 5.0;
            }
            h += match l {
                Line::Head(_) => line(g, &cx.secondary(11.0)) + if i > 0 { 6.0 } else { 0.0 },
                Line::Row(..) => line(g, &cx.body()),
                Line::Chart(_) => 66.0 + 4.0,
            };
        }
        (260.0, h)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        match self.style {
            ClaudeStyle::Rings => return self.draw_rings(cx, x, y, alpha),
            ClaudeStyle::Pace => return self.draw_pace(cx, x, y, alpha),
            ClaudeStyle::Sessions => return self.draw_sessions(cx, x, y, alpha),
            ClaudeStyle::Report => {}
        }
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let mut yy = y;
        for (i, l) in self.lines().iter().enumerate() {
            if i > 0 {
                yy += 5.0;
            }
            match l {
                Line::Head(t) => {
                    if i > 0 {
                        yy += 6.0;
                    }
                    let f = cx.secondary(11.0);
                    let h = line(g, &f);
                    // Uppercase with a little letter spacing (as the page's .claude-head).
                    let mut lx = x;
                    for ch in t.chars() {
                        lx += g.text_at(&ch.to_string(), &f, lx, yy + h / 2.0, dim) + 0.5;
                    }
                    yy += h;
                }
                Line::Row(label, value, d) => {
                    let f = cx.body();
                    let h = line(g, &f);
                    let c = if *d { dim } else { fg };
                    g.text_at(label, &f, x, yy + h / 2.0, c);
                    let vw = g.measure(value, &f);
                    g.text_at(value, &f, x + 260.0 - vw, yy + h / 2.0, c);
                    yy += h;
                }
                Line::Chart(days) => {
                    yy += 4.0;
                    let max = days.iter().map(|d| d.0).fold(1.0, f64::max);
                    let col = (260.0 - 6.0 * 6.0) / 7.0;
                    let df = cx.secondary(10.0);
                    let dh = line(g, &df);
                    let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { white(0.55 * alpha) };
                    for (k, (t, day)) in days.iter().enumerate() {
                        let left = x + k as f32 * (col + 6.0);
                        let bh = ((t / max) as f32 * 48.0).max(2.0);
                        let bottom = yy + 66.0 - dh - 4.0;
                        g.fill_round(rect(left, bottom - bh, col, bh), 3.0, fill);
                        let w = g.measure(day, &df);
                        g.text_at(day, &df, left + (col - w) / 2.0, yy + 66.0 - dh / 2.0, dim);
                    }
                    yy += 66.0;
                }
            }
        }
    }
}

// ----------------------------------------------------------------------

/// "1:05" / "1:02:03".
fn clock_time(ms: f64) -> String {
    let s = (ms / 1000.0).floor().max(0.0) as i64;
    let (h, m, sec) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{sec:02}")
    } else {
        format!("{m}:{sec:02}")
    }
}

// .media-panel: 320 wide, 84 px art 14 px from the info; keys 32 × 28, 10 apart.
const ART: f32 = 84.0;
const INFO_X: f32 = ART + 14.0;
const INFO_W: f32 = 320.0 - INFO_X;
const KEYS_W: f32 = 3.0 * 32.0 + 2.0 * 10.0;
// Record: its width, the seek ring's and the label's radius. Glow: its width. Cover: its
// width and the picture's height.
const RECORD_W: f32 = 220.0;
const RING_R: f32 = 62.0;
const DISC_R: f32 = 54.0;
const GLOW_W: f32 = 280.0;
const GLOW_PAD: f32 = 18.0;
const COVER_W: f32 = 250.0;
const COVER_H: f32 = 170.0;
/// A record turns once every six seconds (radians a second).
pub const RECORD_TURN: f32 = std::f32::consts::TAU / 6.0;

/// The media panel's look ([widgets.media] style).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MediaStyle {
    /// Cover, title, a seek bar, the keys.
    Classic,
    /// The cover as a turning record, its seek bar a ring around it.
    Record,
    /// The cover's colours glowing behind big type; progress along the panel's edge.
    Glow,
    /// The cover across the panel's top, fading into the bar and into the panel.
    Cover,
}

impl MediaStyle {
    pub fn of(opts: &Opts) -> MediaStyle {
        match opts.str("style") {
            Some("record") => MediaStyle::Record,
            Some("glow") => MediaStyle::Glow,
            Some("cover") => MediaStyle::Cover,
            _ => MediaStyle::Classic,
        }
    }
}

/// The bytes of the player's cover (a data URL).
pub fn art_bytes(url: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let (_, b) = url.split_once(";base64,")?;
    base64::engine::general_purpose::STANDARD.decode(b).ok()
}

/// The seek control as laid out (content px): a track (left, centre y, width), the
/// record's ring (centre x, centre y, radius), or none (nothing with a length plays).
#[derive(Clone, Copy, PartialEq)]
enum Seek {
    None,
    Line(f32, f32, f32),
    Ring(f32, f32, f32),
}

/// A cover's colour, brightened enough to glow (a dark cover still shows).
fn glowing(c: Color) -> Color {
    let m = c.r.max(c.g).max(c.b).max(0.05);
    let k = if m < 0.55 { 0.55 / m } else { 1.0 };
    rgba((c.r * k).min(1.0), (c.g * k).min(1.0), (c.b * k).min(1.0), 1.0)
}

/// Now playing, in its look: the cover, title and artist, a seek control to drag, the keys.
pub struct MediaPanel {
    style: MediaStyle,
    /// On a bottom surface (the panel's far edge is its top).
    bottom: bool,
    m: Value,
    /// When the position (or the record) moves on next.
    beat: Instant,
    art: Option<String>,
    picture: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap>,
    picture_for: Option<String>,
    /// The cover's colours (Glow).
    colors: Vec<Color>,
    /// The record's turn (radians) and when it last turned.
    angle: f32,
    turned: Instant,
    /// The seek control's fraction while it's dragged.
    dragging: Option<f32>,
    hover_seek: bool,
    hovered_key: Option<usize>,
    /// The last layout (content px): its size, the keys' boxes, the seek control, the
    /// title's and artist's rows' tops, and the cover's top (Cover).
    size: (f32, f32),
    keys: [(f32, f32, f32, f32); 3],
    seek: Seek,
    rows: (f32, f32),
    art_y: f32,
}

impl MediaPanel {
    pub fn new(opts: &Opts) -> MediaPanel {
        MediaPanel {
            style: MediaStyle::of(opts),
            bottom: false,
            m: Value::Null,
            beat: Instant::now(),
            art: None,
            picture: None,
            picture_for: None,
            colors: Vec::new(),
            angle: 0.0,
            turned: Instant::now(),
            dragging: None,
            hover_seek: false,
            hovered_key: None,
            size: (320.0, ART),
            keys: [(0.0, 0.0, 0.0, 0.0); 3],
            seek: Seek::None,
            rows: (0.0, 0.0),
            art_y: 0.0,
        }
    }
    fn f(&self, k: &str) -> f64 {
        self.m.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }
    fn b(&self, k: &str) -> bool {
        self.m.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
    }
    fn s(&self, k: &str) -> String {
        self.m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string()
    }
    /// Where playback is now (extrapolated between the player's updates).
    fn position(&self) -> f64 {
        let drift = if self.b("playing") { (now_secs() * 1000.0 - self.f("updatedAt")).max(0.0) } else { 0.0 };
        (self.f("positionMs") + drift).min(self.f("durationMs"))
    }
    /// How far along it is (0..1), the dragged spot while the seek control is held.
    fn frac(&self) -> f32 {
        let dur = self.f("durationMs");
        if dur <= 0.0 {
            return 0.0;
        }
        self.dragging.unwrap_or((self.position() / dur) as f32).clamp(0.0, 1.0)
    }
    fn on_seek(&self, p: (f32, f32)) -> bool {
        match self.seek {
            Seek::Line(x, cy, w) => p.1 >= cy - 7.0 && p.1 <= cy + 7.0 && p.0 >= x && p.0 <= x + w,
            Seek::Ring(cx, cy, r) => ((p.0 - cx).hypot(p.1 - cy) - r).abs() <= 10.0,
            Seek::None => false,
        }
    }
    /// The fraction a point on the seek control stands for (the ring: clockwise from the top).
    fn seek_frac(&self, p: (f32, f32)) -> f32 {
        match self.seek {
            Seek::Line(x, _, w) => ((p.0 - x) / w.max(1.0)).clamp(0.0, 1.0),
            Seek::Ring(cx, cy, _) => (p.0 - cx).atan2(cy - p.1).rem_euclid(std::f32::consts::TAU) / std::f32::consts::TAU,
            Seek::None => 0.0,
        }
    }
    fn key_at(&self, p: (f32, f32)) -> Option<usize> {
        self.keys.iter().position(|k| p.0 >= k.0 && p.0 < k.0 + k.2 && p.1 >= k.1 && p.1 < k.1 + k.3)
    }
    fn media_call(action: &'static str, position: Option<f64>) -> Act {
        Act::call(move |app| {
            use tauri::Manager;
            let sender = app.state::<crate::providers::media::MediaSender>();
            let _ = tauri::async_runtime::block_on(crate::providers::media::media_control(sender, action.into(), position));
        })
    }
    /// Three keys `w` × `h`, `gap` apart, from `left`.
    fn lay_keys(&mut self, left: f32, top: f32, w: f32, h: f32, gap: f32) {
        for (i, k) in self.keys.iter_mut().enumerate() {
            *k = (left + i as f32 * (w + gap), top, w, h);
        }
    }

    /// The keys in their boxes: round buttons in the Cover look (play filled), else
    /// glyphs with a hover background.
    fn draw_keys(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let glyphs = ["\u{E892}", if self.b("playing") { "\u{E769}" } else { "\u{E768}" }, "\u{E893}"];
        let can = [self.b("canPrevious"), self.b("canPlayPause"), self.b("canNext")];
        let f = Font::icons(14.0);
        for (i, glyph) in glyphs.iter().enumerate() {
            let (bx, by, bw, bh) = self.keys[i];
            let (bx, by) = (x + bx, y + by);
            let hovered = self.hovered_key == Some(i) && can[i];
            let mut color = with_alpha(cx.look.fg, alpha * if can[i] { 1.0 } else { 0.35 });
            if self.style == MediaStyle::Cover {
                let main = i == 1;
                let bg = if main { with_alpha(cx.look.fg, if hovered { 0.85 } else { 1.0 }) } else { white(if hovered { 0.16 } else { 0.08 }) };
                g.fill_round(rect(bx, by, bw, bh), bw / 2.0, with_alpha(bg, alpha));
                if main {
                    color = with_alpha(cx.look.bg, alpha);
                }
            } else if hovered {
                g.fill_round(rect(bx, by, bw, bh), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            let w = g.measure(glyph, &f);
            g.text_at(glyph, &f, bx + (bw - w) / 2.0, by + bh / 2.0, color);
        }
    }

    /// A straight seek bar: its track, the part played, the knob while it's used.
    fn draw_track(&self, cx: &Ctx, x: f32, cy: f32, w: f32, alpha: f32) {
        let g = cx.gfx;
        let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { with_alpha(cx.look.fg, alpha) };
        let frac = self.frac();
        g.fill_round(rect(x, cy - 2.0, w, 4.0), 2.0, white(0.18 * alpha));
        g.fill_round(rect(x, cy - 2.0, w * frac, 4.0), 2.0, fill);
        if self.b("canSeek") && (self.hover_seek || self.dragging.is_some()) {
            g.fill_round(rect(x + w * frac - 5.0, cy - 5.0, 10.0, 10.0), 5.0, fill);
        }
    }

    fn draw_classic(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let h = self.size.1;
        let art_y = y + (h - ART) / 2.0;
        match &self.picture {
            Some(p) => g.fill_picture(p, rect(x, art_y, ART, ART), 14.0, alpha),
            None => {
                g.fill_round(rect(x, art_y, ART, ART), 14.0, rgba(0.102, 0.102, 0.102, alpha));
                let f = Font::icons(26.0);
                let w = g.measure("\u{E8D6}", &f);
                g.text_at("\u{E8D6}", &f, x + (ART - w) / 2.0, art_y + ART / 2.0, dim);
            }
        }
        let ix = x + INFO_X;
        let tf = cx.main(13.0);
        if self.m.is_null() {
            g.text_at("Nothing playing", &tf, ix, y + self.rows.0 + line(g, &tf) / 2.0, fg);
            return;
        }
        g.text_at(&fit_text(g, &self.s("title"), &tf, INFO_W), &tf, ix, y + self.rows.0 + line(g, &tf) / 2.0, fg);
        let af = cx.secondary(12.0);
        g.text_at(&fit_text(g, &self.s("artist"), &af, INFO_W), &af, ix, y + self.rows.1 + line(g, &af) / 2.0, dim);
        if let Seek::Line(tx, cy, tw) = self.seek {
            let dur = self.f("durationMs");
            let f = cx.secondary(10.0);
            g.text_at(&clock_time(self.frac() as f64 * dur), &f, ix, y + cy, dim);
            let total = clock_time(dur);
            let w = g.measure(&total, &f);
            g.text_at(&total, &f, x + 320.0 - w, y + cy, dim);
            self.draw_track(cx, x + tx, y + cy, tw, alpha);
        }
        self.draw_keys(cx, x, y, alpha);
    }

    fn draw_record(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let (rx, ry) = (x + RECORD_W / 2.0, y + RING_R + 2.0);
        // The label: the cover, turning; grooves over it, the hole in the middle.
        match &self.picture {
            Some(p) => g.fill_disc(p, rx, ry, DISC_R, self.angle, alpha),
            None => g.fill_round(rect(rx - DISC_R, ry - DISC_R, 2.0 * DISC_R, 2.0 * DISC_R), DISC_R, rgba(0.102, 0.102, 0.102, alpha)),
        }
        for r in [22.0, 30.0, 38.0, 46.0] {
            g.ring(rx, ry, r, 1.0, 0.0, rgba(0.0, 0.0, 0.0, 0.2 * alpha), rgba(0.0, 0.0, 0.0, 0.0));
        }
        g.fill_round(rect(rx - 6.0, ry - 6.0, 12.0, 12.0), 6.0, with_alpha(cx.look.bg, alpha));
        if let Seek::Ring(_, _, r) = self.seek {
            let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
            let frac = self.frac();
            g.ring(rx, ry, r, 3.0, frac, white(0.14 * alpha), fill);
            if self.b("canSeek") && (self.hover_seek || self.dragging.is_some()) {
                let a = frac * std::f32::consts::TAU;
                let (kx, ky) = (rx + r * a.sin(), ry - r * a.cos());
                g.fill_round(rect(kx - 5.0, ky - 5.0, 10.0, 10.0), 5.0, fill);
            }
        }
        let centred = |text: &str, f: &Font, top: f32, c: Color| {
            let t = fit_text(g, text, f, RECORD_W);
            let w = g.measure(&t, f);
            g.text_at(&t, f, x + (RECORD_W - w) / 2.0, y + top + line(g, f) / 2.0, c);
        };
        if self.m.is_null() {
            centred("Nothing playing", &cx.main(13.0), self.rows.0, fg);
            return;
        }
        centred(&self.s("title"), &cx.main(13.0), self.rows.0, fg);
        centred(&self.s("artist"), &cx.secondary(12.0), self.rows.1, dim);
        self.draw_keys(cx, x, y, alpha);
    }

    fn draw_glow(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let (w, h) = self.size;
        // The cover's colours glow from the panel's far corners (the bar's side stays black).
        let far = if self.bottom { y - GLOW_PAD } else { y + h + GLOW_PAD };
        let (left, right) = if self.bottom { (0, 1) } else { (2, 3) };
        if let (Some(a), Some(b)) = (self.colors.get(left), self.colors.get(right)) {
            g.glow(x - GLOW_PAD, far, 230.0, 130.0, with_alpha(glowing(*a), 0.42 * alpha));
            g.glow(x + w + GLOW_PAD, far, 190.0, 120.0, with_alpha(glowing(*b), 0.3 * alpha));
        }
        let tf = cx.main(20.0);
        if self.m.is_null() {
            g.text_at("Nothing playing", &tf, x, y + line(g, &tf) / 2.0, fg);
            return;
        }
        g.text_at(&fit_text(g, &self.s("title"), &tf, w), &tf, x, y + self.rows.0 + line(g, &tf) / 2.0, fg);
        let af = cx.secondary(12.0);
        g.text_at(&fit_text(g, &self.s("artist"), &af, w), &af, x, y + self.rows.1 + line(g, &af) / 2.0, dim);
        let key_cy = y + self.keys[0].1 + self.keys[0].3 / 2.0;
        if let Seek::Line(lx, cy, lw) = self.seek {
            let dur = self.f("durationMs");
            let f = cx.secondary(12.0);
            g.text_at(&format!("{} / {}", clock_time(self.frac() as f64 * dur), clock_time(dur)), &f, x, key_cy, dim);
            // Along the far edge: only the part played.
            let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
            let thick = if self.hover_seek || self.dragging.is_some() { 5.0 } else { 3.0 };
            g.fill_round(rect(x + lx, y + cy - thick / 2.0, (lw * self.frac()).max(thick), thick), thick / 2.0, fill);
        }
        self.draw_keys(cx, x, y, alpha);
    }

    fn draw_cover(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        // The cover fades into the bar where it meets it (no edge) and into the panel
        // under the title.
        let art = rect(x, y + self.art_y, COVER_W, COVER_H);
        let (near, far) = (44.0, 100.0);
        if self.bottom {
            g.push_fade_v(art, far, near);
        } else {
            g.push_fade_v(art, near, far);
        }
        match &self.picture {
            Some(p) => g.fill_picture(p, art, 0.0, alpha),
            None => g.fill_rect(art, rgba(0.102, 0.102, 0.102, alpha)),
        }
        g.pop_fade();
        let tf = cx.main(15.0);
        if self.m.is_null() {
            g.text_at("Nothing playing", &tf, x + 14.0, y + self.rows.0 + line(g, &tf) / 2.0, fg);
            return;
        }
        g.text_at(&fit_text(g, &self.s("title"), &tf, COVER_W - 28.0), &tf, x + 14.0, y + self.rows.0 + line(g, &tf) / 2.0, fg);
        let af = cx.secondary(12.0);
        g.text_at(&fit_text(g, &self.s("artist"), &af, COVER_W - 28.0), &af, x + 14.0, y + self.rows.1 + line(g, &af) / 2.0, dim);
        if let Seek::Line(tx, cy, tw) = self.seek {
            self.draw_track(cx, x + tx, y + cy, tw, alpha);
            let f = cx.secondary(12.0);
            g.text_at(&clock_time(self.frac() as f64 * self.f("durationMs")), &f, x + 14.0, y + self.keys[0].1 + self.keys[0].3 / 2.0, dim);
        }
        self.draw_keys(cx, x, y, alpha);
    }
}

impl Panel for MediaPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["media"]
    }
    fn data(&mut self, _key: &str, v: &Value) -> bool {
        self.m = v.clone();
        self.art = v.get("art").and_then(|a| a.as_str()).map(String::from);
        true
    }
    fn pad(&self) -> (f32, f32, f32) {
        match self.style {
            MediaStyle::Classic => (12.0, 14.0, 14.0),
            MediaStyle::Record => (14.0, 16.0, 16.0),
            MediaStyle::Glow => (16.0, GLOW_PAD, GLOW_PAD),
            // The cover reaches the bar and both sides; the text keeps its own margin.
            MediaStyle::Cover => (0.0, 14.0, 0.0),
        }
    }
    fn wake(&self) -> Option<Instant> {
        // The position moves on between the player's updates; a record turns.
        self.b("playing").then_some(self.beat)
    }
    fn tick(&mut self, now: Instant) -> bool {
        let turning = self.style == MediaStyle::Record && self.b("playing");
        if turning {
            self.angle = (self.angle + now.saturating_duration_since(self.turned).as_secs_f32().min(0.1) * RECORD_TURN) % std::f32::consts::TAU;
        }
        self.turned = now;
        self.beat = now + Duration::from_millis(if turning { 33 } else { 500 });
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        self.bottom = cx.look.bottom;
        if self.picture_for != self.art {
            self.picture_for = self.art.clone();
            let bytes = self.art.as_deref().and_then(art_bytes);
            self.picture = bytes.as_ref().and_then(|b| g.picture(b).ok());
            self.colors = match (&bytes, self.style) {
                (Some(b), MediaStyle::Glow) => g.picture_colors(b),
                _ => Vec::new(),
            };
        }
        let seeks = self.f("durationMs") > 0.0;
        let ah = line(g, &cx.secondary(12.0));
        match self.style {
            MediaStyle::Classic => {
                let th = line(g, &cx.main(13.0));
                let seek = if seeks { 6.0 + 14.0f32.max(line(g, &cx.secondary(10.0))) + 3.0 } else { 0.0 };
                let total = th + 3.0 + ah + 3.0 + seek + 2.0 + 28.0;
                let top = (ART - total).max(0.0) / 2.0;
                let seek_y = top + th + 3.0 + ah + 3.0 + 6.0;
                let keys_y = top + th + 3.0 + ah + 3.0 + seek + 2.0;
                self.rows = (top, top + th + 3.0);
                // Times (at least 28 px) either side of the track, 8 px from it.
                let lw = g.measure(&clock_time(self.f("durationMs")), &cx.secondary(10.0)).max(28.0);
                self.seek = if seeks { Seek::Line(INFO_X + lw + 8.0, seek_y + 7.0, INFO_W - 2.0 * lw - 16.0) } else { Seek::None };
                self.lay_keys(INFO_X + (INFO_W - KEYS_W) / 2.0, keys_y, 32.0, 28.0, 10.0);
                self.size = (320.0, total.max(ART));
            }
            MediaStyle::Record => {
                let th = line(g, &cx.main(13.0));
                let title = 2.0 * RING_R + 4.0 + 10.0;
                self.rows = (title, title + th + 2.0);
                let keys_y = title + th + 2.0 + ah + 10.0;
                self.seek = if seeks { Seek::Ring(RECORD_W / 2.0, RING_R + 2.0, RING_R) } else { Seek::None };
                self.lay_keys((RECORD_W - KEYS_W) / 2.0, keys_y, 32.0, 28.0, 10.0);
                self.size = (RECORD_W, keys_y + 28.0);
            }
            MediaStyle::Glow => {
                let th = line(g, &cx.main(20.0));
                self.rows = (0.0, th + 2.0);
                let keys_y = th + 2.0 + ah + 14.0;
                let h = keys_y + 28.0;
                // The progress runs along the far edge (inside the rounded corners).
                let edge = if self.bottom { -GLOW_PAD + 1.5 } else { h + GLOW_PAD - 1.5 };
                self.seek = if seeks { Seek::Line(-GLOW_PAD + 14.0, edge, GLOW_W + 2.0 * GLOW_PAD - 28.0) } else { Seek::None };
                self.lay_keys(GLOW_W - (3.0 * 32.0 + 2.0 * 4.0), keys_y, 32.0, 28.0, 4.0);
                self.size = (GLOW_W, h);
            }
            MediaStyle::Cover => {
                let th = line(g, &cx.main(15.0));
                // Title, artist, the seek bar, the keys: over the cover's faded end.
                let text = th + 2.0 + ah + 10.0 + 4.0 + 10.0 + 30.0;
                let (text_y, art_y, h) = if self.bottom { (0.0, text - 40.0, text - 40.0 + COVER_H) } else { (COVER_H - 40.0, 0.0, COVER_H - 40.0 + text) };
                self.art_y = art_y;
                self.rows = (text_y, text_y + th + 2.0);
                let seek_cy = text_y + th + 2.0 + ah + 10.0 + 2.0;
                self.seek = if seeks { Seek::Line(14.0, seek_cy, COVER_W - 28.0) } else { Seek::None };
                self.lay_keys(COVER_W - 14.0 - (3.0 * 30.0 + 2.0 * 6.0), seek_cy + 2.0 + 10.0, 30.0, 30.0, 6.0);
                self.size = (COVER_W, h);
            }
        }
        self.size
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        match self.style {
            MediaStyle::Classic => self.draw_classic(cx, x, y, alpha),
            MediaStyle::Record => self.draw_record(cx, x, y, alpha),
            MediaStyle::Glow => self.draw_glow(cx, x, y, alpha),
            MediaStyle::Cover => self.draw_cover(cx, x, y, alpha),
        }
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let seek = at.is_some_and(|p| self.on_seek(p));
        let key = at.and_then(|p| self.key_at(p));
        let changed = seek != self.hover_seek || key != self.hovered_key;
        self.hover_seek = seek;
        self.hovered_key = key;
        changed
    }
    fn press(&mut self, at: (f32, f32)) -> bool {
        if !self.b("canSeek") || !self.on_seek(at) {
            return false;
        }
        self.drag(at);
        true
    }
    fn drag(&mut self, at: (f32, f32)) -> Reply {
        self.dragging = Some(self.seek_frac(at));
        Reply::None
    }
    fn release(&mut self, at: (f32, f32)) -> Reply {
        self.drag(at);
        let Some(frac) = self.dragging.take() else { return Reply::None };
        let target = frac as f64 * self.f("durationMs");
        // Show the new position right away; the player confirms on its next update.
        if let Some(m) = self.m.as_object_mut() {
            m.insert("positionMs".into(), target.into());
            m.insert("updatedAt".into(), (now_secs() * 1000.0).into());
        }
        Reply::Do(Self::media_call("seek", Some(target)))
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let can = [self.b("canPrevious"), self.b("canPlayPause"), self.b("canNext")];
        match self.key_at(at) {
            Some(i) if button == Button::Left && can[i] => Reply::Do(Self::media_call(["previous", "play_pause", "next"][i], None)),
            _ => Reply::None,
        }
    }
    fn closed(&mut self) {
        self.hover_seek = false;
        self.hovered_key = None;
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        let (w, h) = self.size;
        match self.style {
            MediaStyle::Classic => vec![(0.0, 0.0, ART, h), (INFO_X, 0.0, INFO_W, h)],
            _ => vec![(0.0, 0.0, w, h)],
        }
    }
}
