//! The Claude usage panel (claude.ts claudePanel) and the media panel
//! (media-panel.ts): album art, title, a seek bar to drag, the keys.

use std::time::{Duration, Instant};

use serde_json::Value;

use super::extras::{day_start, duration, limits, now_secs, tokens, total_tokens};
use super::{fit_text, Act, Button, Ctx, Opts, Panel, Reply};
use crate::native::gfx::{rect, rgba, white, with_alpha, Font, Gfx};

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

/// The 5-hour window, today (per model) and the last 7 days, plus the plan's limits.
pub struct ClaudePanel {
    opts: Opts,
    u: Value,
}

enum Line {
    Head(String),
    Row(String, String, bool),
    Chart(Vec<(f64, &'static str)>),
}

impl ClaudePanel {
    pub fn new(opts: &Opts) -> ClaudePanel {
        ClaudePanel { opts: opts.clone(), u: Value::Null }
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
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
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

/// Now playing: album art, title and artist, a seek bar to drag, the keys.
pub struct MediaPanel {
    m: Value,
    art: Option<String>,
    picture: Option<windows::Win32::Graphics::Direct2D::ID2D1Bitmap>,
    picture_for: Option<String>,
    /// The seek bar's fraction while it's dragged.
    dragging: Option<f32>,
    hover_seek: bool,
    hovered_key: Option<usize>,
    /// The last layout (content px): rows' tops (title, artist, seek, keys), its
    /// height, and the seek track's left and width.
    rows: (f32, f32, f32, f32, f32),
    track: (f32, f32),
}

impl MediaPanel {
    pub fn new() -> MediaPanel {
        MediaPanel { m: Value::Null, art: None, picture: None, picture_for: None, dragging: None, hover_seek: false, hovered_key: None, rows: (0.0, 0.0, 0.0, 0.0, ART), track: (0.0, 0.0) }
    }
    fn f(&self, k: &str) -> f64 {
        self.m.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0)
    }
    fn b(&self, k: &str) -> bool {
        self.m.get(k).and_then(|v| v.as_bool()).unwrap_or(false)
    }
    /// Where playback is now (extrapolated between the player's updates).
    fn position(&self) -> f64 {
        let drift = if self.b("playing") { (now_secs() * 1000.0 - self.f("updatedAt")).max(0.0) } else { 0.0 };
        (self.f("positionMs") + drift).min(self.f("durationMs"))
    }
    fn on_seek(&self, p: (f32, f32)) -> bool {
        let (_, _, seek_y, _, _) = self.rows;
        p.1 >= seek_y && p.1 <= seek_y + 14.0 && p.0 >= self.track.0 && p.0 <= self.track.0 + self.track.1
    }
    fn key_at(&self, p: (f32, f32)) -> Option<usize> {
        let keys_y = self.rows.3;
        let left = INFO_X + (INFO_W - KEYS_W) / 2.0;
        if p.1 < keys_y || p.1 > keys_y + 28.0 {
            return None;
        }
        let i = ((p.0 - left) / 42.0).floor();
        (i >= 0.0 && i < 3.0 && p.0 - left - i * 42.0 < 32.0).then_some(i as usize)
    }
    fn media_call(action: &'static str, position: Option<f64>) -> Act {
        Act::call(move |app| {
            use tauri::Manager;
            let sender = app.state::<crate::providers::media::MediaSender>();
            let _ = tauri::async_runtime::block_on(crate::providers::media::media_control(sender, action.into(), position));
        })
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
        (12.0, 14.0, 14.0)
    }
    fn wake(&self) -> Option<Instant> {
        // The position moves on between the player's updates.
        self.b("playing").then(|| Instant::now() + Duration::from_millis(500))
    }
    fn tick(&mut self, _now: Instant) -> bool {
        true
    }
    fn tick_resizes(&self) -> bool {
        false
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        if self.picture_for != self.art {
            self.picture_for = self.art.clone();
            self.picture = self
                .art
                .as_deref()
                .and_then(|url| url.split_once(";base64,"))
                .and_then(|(_, b)| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.decode(b).ok()
                })
                .and_then(|bytes| g.picture(&bytes).ok());
        }
        let (th, ah) = (line(g, &cx.main(13.0)), line(g, &cx.secondary(12.0)));
        let seek = if self.f("durationMs") > 0.0 { 6.0 + 14.0f32.max(line(g, &cx.secondary(10.0))) + 3.0 } else { 0.0 };
        let total = th + 3.0 + ah + 3.0 + seek + 2.0 + 28.0;
        let top = (ART - total).max(0.0) / 2.0;
        self.rows = (top, top + th + 3.0, top + th + 3.0 + ah + 3.0 + 6.0, top + th + 3.0 + ah + 3.0 + seek + 2.0, total.max(ART));
        // Times (at least 28 px) either side of the track, 8 px from it.
        let f = cx.secondary(10.0);
        let lw = g.measure(&clock_time(self.f("durationMs")), &f).max(28.0);
        self.track = (INFO_X + lw + 8.0, INFO_W - 2.0 * lw - 16.0);
        (320.0, self.rows.4)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let (title_y, artist_y, seek_y, keys_y, h) = self.rows;
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
            g.text_at("Nothing playing", &tf, ix, y + title_y + line(g, &tf) / 2.0, fg);
            return;
        }
        let s = |k: &str| self.m.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        g.text_at(&fit_text(g, &s("title"), &tf, INFO_W), &tf, ix, y + title_y + line(g, &tf) / 2.0, fg);
        let af = cx.secondary(12.0);
        g.text_at(&fit_text(g, &s("artist"), &af, INFO_W), &af, ix, y + artist_y + line(g, &af) / 2.0, dim);
        let dur = self.f("durationMs");
        if dur > 0.0 {
            let frac = self.dragging.unwrap_or((self.position() / dur) as f32).clamp(0.0, 1.0);
            let f = cx.secondary(10.0);
            let cyy = y + seek_y + 7.0;
            g.text_at(&clock_time(frac as f64 * dur), &f, ix, cyy, dim);
            let total = clock_time(dur);
            let tw = g.measure(&total, &f);
            g.text_at(&total, &f, x + 320.0 - tw, cyy, dim);
            let (tx, tw) = self.track;
            let fill = if cx.opts.accent { with_alpha(cx.look.accent, alpha) } else { fg };
            g.fill_round(rect(x + tx, cyy - 2.0, tw, 4.0), 2.0, white(0.18 * alpha));
            g.fill_round(rect(x + tx, cyy - 2.0, tw * frac, 4.0), 2.0, fill);
            if self.b("canSeek") && (self.hover_seek || self.dragging.is_some()) {
                g.fill_round(rect(x + tx + tw * frac - 5.0, cyy - 5.0, 10.0, 10.0), 5.0, fill);
            }
        }
        let left = ix + (INFO_W - KEYS_W) / 2.0;
        let glyphs = ["\u{E892}", if self.b("playing") { "\u{E769}" } else { "\u{E768}" }, "\u{E893}"];
        let can = [self.b("canPrevious"), self.b("canPlayPause"), self.b("canNext")];
        let f = Font::icons(14.0);
        for (i, glyph) in glyphs.iter().enumerate() {
            let bx = left + i as f32 * 42.0;
            if self.hovered_key == Some(i) && can[i] {
                g.fill_round(rect(bx, y + keys_y, 32.0, 28.0), 8.0, with_alpha(cx.hover_bg(0.12), alpha));
            }
            let w = g.measure(glyph, &f);
            g.text_at(glyph, &f, bx + (32.0 - w) / 2.0, y + keys_y + 14.0, with_alpha(cx.look.fg, alpha * if can[i] { 1.0 } else { 0.35 }));
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
        let (tx, tw) = self.track;
        self.dragging = Some(((at.0 - tx) / tw.max(1.0)).clamp(0.0, 1.0));
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
}
