//! The clock (src/widgets/clock.ts) and its calendar panel (calendar.ts).

use std::time::{Duration, Instant};

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;

use super::{Act, Button, Ctx, Opts, Panel, Reply, Widget};
use crate::native::gfx::{rect, rgba, white, with_alpha, Font};

const DAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTH_NAMES: [&str; 12] = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

fn now() -> SYSTEMTIME {
    unsafe { GetLocalTime() }
}

/// strftime subset: %H %I %M %S %p %d %m %Y %y %a %b %%.
pub fn strftime(fmt: &str, t: &SYSTEMTIME) -> String {
    let mut out = String::new();
    let mut chars = fmt.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('H') => out.push_str(&format!("{:02}", t.wHour)),
            Some('I') => out.push_str(&format!("{:02}", if t.wHour % 12 == 0 { 12 } else { t.wHour % 12 })),
            Some('M') => out.push_str(&format!("{:02}", t.wMinute)),
            Some('S') => out.push_str(&format!("{:02}", t.wSecond)),
            Some('p') => out.push_str(if t.wHour < 12 { "AM" } else { "PM" }),
            Some('d') => out.push_str(&format!("{:02}", t.wDay)),
            Some('m') => out.push_str(&format!("{:02}", t.wMonth)),
            Some('Y') => out.push_str(&t.wYear.to_string()),
            Some('y') => out.push_str(&format!("{:02}", t.wYear % 100)),
            Some('a') => out.push_str(DAYS[t.wDayOfWeek as usize % 7]),
            Some('b') => out.push_str(MONTHS[(t.wMonth as usize).saturating_sub(1) % 12]),
            Some('%') => out.push('%'),
            Some(other) => {
                out.push('%');
                out.push(other);
            }
            None => out.push('%'),
        }
    }
    out
}

// --- Timers ([widgets.clock] timers): a countdown or a stopwatch, one for all screens ---

#[derive(Clone, Copy, PartialEq, Debug)]
enum Timer {
    /// Counting down: when it ends, or (paused) what's left.
    Countdown { end: Instant, left: Option<Duration> },
    /// Counting up: since when (running), and what was counted before.
    Stopwatch { since: Option<Instant>, acc: Duration },
}

thread_local! {
    static TIMER: std::cell::Cell<Option<Timer>> = const { std::cell::Cell::new(None) };
}

fn timer() -> Option<Timer> {
    TIMER.with(|t| t.get())
}

fn set_timer(t: Option<Timer>) {
    TIMER.with(|c| c.set(t));
}

/// What it shows now: time left (countdown) or counted (stopwatch).
fn shown(t: Timer, now: Instant) -> Duration {
    match t {
        Timer::Countdown { end, left } => left.unwrap_or_else(|| end.saturating_duration_since(now)),
        Timer::Stopwatch { since, acc } => acc + since.map_or(Duration::ZERO, |s| now.saturating_duration_since(s)),
    }
}

/// "4:59", "1:02:03" (a countdown rounds up: it shows 0:00 only as it ends).
fn clock_text(d: Duration, up: bool) -> String {
    let s = if up { d.as_secs_f64().ceil() as u64 } else { d.as_secs() };
    let (h, m, s) = (s / 3600, (s % 3600) / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{s:02}") } else { format!("{m}:{s:02}") }
}

fn running(t: Timer) -> bool {
    matches!(t, Timer::Countdown { left: None, .. } | Timer::Stopwatch { since: Some(_), .. })
}

/// A countdown that has run out: cleared (once, whichever clock notices first).
fn take_expired(now: Instant) -> bool {
    match timer() {
        Some(Timer::Countdown { end, left: None }) if now >= end => {
            set_timer(None);
            true
        }
        _ => false,
    }
}

/// Time's up: Windows' notification sound and a pop-up out of the notch.
fn times_up() -> Act {
    Act::call(|app| {
        unsafe {
            let _ = windows::Win32::System::Diagnostics::Debug::MessageBeep(windows::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK);
        }
        crate::osd::emit(&app, "timer", "notch://osd", serde_json::json!({ "kind": "timer", "level": 1.0, "muted": false, "text": "Time's up", "ms": 8000 }));
    })
}

/// While a timer runs, clocks and the calendar look again this often.
const TIMER_TICK: Duration = Duration::from_millis(250);

pub struct Clock {
    format: String,
    text: String,
    next: Instant,
    /// A running timer's time beside the clock ([widgets.clock] timers).
    timers: bool,
    timer_text: String,
    pending: Option<Act>,
}

impl Clock {
    pub fn new(opts: Opts) -> Clock {
        let format = match opts.str("format").unwrap_or("%H:%M") {
            // Windows' own short time format (Region settings).
            "system" => crate::win::short_time_pattern(),
            f => f.to_string(),
        };
        let timers = opts.flag("timers") == Some(true);
        let mut c = Clock { format, text: String::new(), next: Instant::now(), timers, timer_text: String::new(), pending: None };
        c.tick(Instant::now());
        c
    }
}

impl Clock {
    /// Between the time and a running timer's.
    const TIMER_GAP: f32 = 10.0;
}

impl Widget for Clock {
    fn wake(&self) -> Option<Instant> {
        if self.timers && timer().is_some_and(running) {
            return Some(self.next.min(Instant::now() + TIMER_TICK));
        }
        Some(self.next)
    }
    fn tick(&mut self, now: Instant) -> bool {
        let t = self::now();
        // The next whole second (or minute, without seconds in the format).
        let ms = 1000 - t.wMilliseconds as u64 + 5;
        let wait = if self.format.contains("%S") { ms } else { (59 - t.wSecond as u64) * 1000 + ms };
        self.next = now + Duration::from_millis(wait);
        let text = strftime(&self.format, &t);
        let mut changed = text != self.text;
        self.text = text;
        if self.timers {
            if take_expired(now) {
                self.pending = Some(times_up());
            }
            let tt = timer().map_or(String::new(), |t| clock_text(shown(t, now), matches!(t, Timer::Countdown { .. })));
            changed |= tt != self.timer_text;
            self.timer_text = tt;
        }
        changed
    }
    fn act(&mut self) -> Option<Act> {
        self.pending.take()
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        let w = cx.gfx.measure(&self.text, &cx.main(14.0));
        if self.timer_text.is_empty() {
            return w;
        }
        w + Self::TIMER_GAP + cx.gfx.measure("\u{E916}", &Font::icons(12.0)) + 4.0 + cx.gfx.measure(&self.timer_text, &cx.main(14.0))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let w = cx.gfx.text_at(&self.text, &cx.main(14.0), x, cy, with_alpha(cx.look.fg, alpha));
        if !self.timer_text.is_empty() {
            let dim = with_alpha(cx.look.dimmed(), alpha);
            let gx = x + w + Self::TIMER_GAP;
            let gw = cx.gfx.text_at("\u{E916}", &Font::icons(12.0), gx, cy + 1.0, dim);
            cx.gfx.text_at(&self.timer_text, &cx.main(14.0), gx + gw + 4.0, cy, dim);
        }
    }
    fn tooltip(&self) -> Option<String> {
        (!self.timer_text.is_empty()).then(|| match timer() {
            Some(Timer::Stopwatch { .. }) => "Stopwatch".to_string(),
            _ => "Timer".to_string(),
        })
    }
}

/// Days since 1970-01-01 (proleptic Gregorian).
pub(super) fn days_from_civil(y: i32, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y } as i64;
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = m as i64;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

/// A day number back to (year, month, day).
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    ((if m <= 2 { y + 1 } else { y }) as i32, m, d)
}

/// Monday = 0.
pub(super) fn weekday(days: i64) -> i64 {
    (days + 3).rem_euclid(7)
}

/// ISO-8601 week number of a day.
fn iso_week(days: i64) -> i64 {
    let thursday = days - weekday(days) + 3;
    let (y, _, _) = civil_from_days(thursday);
    (thursday - days_from_civil(y, 1, 1)) / 7 + 1
}

/// Month view, Monday first, with ISO week numbers; today in the accent. With
/// [widgets.clock] timers, a row under it: a countdown's presets and a stopwatch, or
/// the one running with its buttons.
#[derive(Default)]
pub struct Calendar {
    /// The title's and the weekday row's heights as last measured.
    heights: (f32, f32),
    timers: bool,
    /// The timer row's buttons as last laid out (content px), and the one hovered.
    boxes: std::cell::RefCell<Vec<(TimerHit, [f32; 4])>>,
    hovered: Option<TimerHit>,
}

/// The timer row's buttons.
#[derive(Clone, Copy, PartialEq, Debug)]
enum TimerHit {
    /// A countdown of so many minutes.
    Preset(u64),
    Stopwatch,
    Pause,
    AddMinute,
    Reset,
    Stop,
}

const PRESETS: [(u64, &str); 5] = [(1, "1m"), (5, "5m"), (10, "10m"), (25, "25m"), (60, "1h")];
/// The timer row: its gap to the calendar, height.
const TIMER_TOP: f32 = 12.0;
const TIMER_H: f32 = 28.0;

// styles.css .panel-calendar: 228 px wide, a 22 px week column, 24 px rows 2 px apart.
const CAL_W: f32 = 228.0;
const WK_W: f32 = 22.0;
const ROW: f32 = 24.0;
const GAP: f32 = 2.0;

impl Calendar {
    pub fn new(opts: &Opts) -> Calendar {
        Calendar { timers: opts.flag("timers") == Some(true), ..Default::default() }
    }

    fn grid_h(&self) -> f32 {
        let (title, dow) = self.heights;
        title + 8.0 + dow + 6.0 * (ROW + GAP)
    }

    /// The timer row's buttons: what, x (from the row's left), width, label or glyph,
    /// and whether that's a glyph. Idle: the presets and the stopwatch filling the row;
    /// running: its buttons at the row's end.
    fn timer_buttons(&self) -> Vec<(TimerHit, f32, f32, &'static str, bool)> {
        let b = |hit, glyph| (hit, 0.0, 30.0, glyph, true);
        let pause = |t: Timer| if running(t) { "\u{E769}" } else { "\u{E768}" };
        let mut out = match timer() {
            None => {
                let n = PRESETS.len() as f32 + 1.0;
                let w = (CAL_W - (n - 1.0) * 4.0) / n;
                let mut v: Vec<_> = PRESETS.iter().enumerate().map(|(i, (m, l))| (TimerHit::Preset(*m), i as f32 * (w + 4.0), w, *l, false)).collect();
                v.push((TimerHit::Stopwatch, (n - 1.0) * (w + 4.0), w, "\u{E916}", true));
                return v;
            }
            Some(t @ Timer::Countdown { .. }) => vec![b(TimerHit::Pause, pause(t)), (TimerHit::AddMinute, 0.0, 30.0, "+1", false), b(TimerHit::Stop, "\u{E71A}")],
            Some(t @ Timer::Stopwatch { .. }) => vec![b(TimerHit::Pause, pause(t)), b(TimerHit::Reset, "\u{E72C}"), b(TimerHit::Stop, "\u{E71A}")],
        };
        let mut x = CAL_W;
        for i in (0..out.len()).rev() {
            x -= out[i].2;
            out[i].1 = x;
            x -= 4.0;
        }
        out
    }

    fn timer_hit(&self, p: (f32, f32)) -> Option<TimerHit> {
        self.boxes.borrow().iter().find(|(_, b)| p.0 >= b[0] && p.0 < b[0] + b[2] && p.1 >= b[1] && p.1 < b[1] + b[3]).map(|(h, _)| *h)
    }

    fn heights(cx: &Ctx) -> (f32, f32) {
        let title = cx.gfx.text("Ag", &cx.main(13.0)).map_or(16.0, |t| t.height);
        let dow = cx.gfx.text("Mo", &cx.main(11.0)).map_or(13.0, |t| t.height);
        (title, dow)
    }
}

impl Panel for Calendar {
    fn wake(&self) -> Option<Instant> {
        // A running timer: its time. Else today moves at midnight: every minute.
        if self.timers && timer().is_some_and(running) {
            return Some(Instant::now() + TIMER_TICK);
        }
        Some(Instant::now() + Duration::from_secs(60))
    }
    fn tick(&mut self, _now: Instant) -> bool {
        true
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let (title, dow) = Self::heights(cx);
        self.heights = (title, dow);
        (CAL_W, self.grid_h() + if self.timers { TIMER_TOP + 1.0 + 10.0 + TIMER_H } else { 0.0 })
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        let (title, dow) = self.heights;
        let col = (CAL_W - WK_W) / 7.0;
        let grid = title + 8.0;
        let mut out = vec![(0.0, 0.0, CAL_W, title)];
        out.push((0.0, grid, WK_W, dow));
        for i in 0..7 {
            out.push((WK_W + col * i as f32, grid, col, dow));
        }
        for week in 0..6 {
            let top = grid + dow + GAP + week as f32 * (ROW + GAP);
            out.push((0.0, top, WK_W, ROW));
            for i in 0..7 {
                out.push((WK_W + col * i as f32, top, col, ROW));
            }
        }
        out
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let t = now();
        let (title_h, dow_h) = Self::heights(cx);
        let a = |c: f32| white(c * alpha);
        let centred = |s: &str, f: &Font, left: f32, w: f32, cy: f32, c| {
            let tw = g.measure(s, f);
            g.text_at(s, f, left + (w - tw) / 2.0, cy, c);
        };
        let title = format!("{} {}", MONTH_NAMES[(t.wMonth as usize).saturating_sub(1) % 12], t.wYear);
        centred(&title, &cx.main(13.0), x, CAL_W, y + title_h / 2.0, with_alpha(cx.look.fg, alpha));
        let col = (CAL_W - WK_W) / 7.0;
        let grid = y + title_h + 8.0;
        let dow_font = cx.main(11.0);
        for (i, d) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"].iter().enumerate() {
            centred(d, &dow_font, x + WK_W + col * i as f32, col, grid + dow_h / 2.0, with_alpha(cx.look.dimmed(), alpha));
        }
        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        let first = days_from_civil(t.wYear as i32, t.wMonth as u32, 1);
        let start = first - weekday(first);
        let wk_font = cx.main(10.0);
        let day_font = cx.main(12.0);
        for week in 0..6 {
            let top = grid + dow_h + GAP + week as f32 * (ROW + GAP);
            let row_start = start + week * 7;
            centred(&iso_week(row_start).to_string(), &wk_font, x, WK_W, top + ROW / 2.0, a(0.28));
            for i in 0..7 {
                let day = row_start + i;
                let (_, m, d) = civil_from_days(day);
                let left = x + WK_W + col * i as f32;
                let mut c = if m != t.wMonth as u32 { a(0.25) } else { with_alpha(cx.look.fg, alpha) };
                if day == today {
                    g.fill_round(rect(left, top, col, ROW), 12.0, with_alpha(cx.look.accent, alpha));
                    c = rgba(0.0, 0.0, 0.0, alpha);
                }
                centred(&d.to_string(), &day_font, left, col, top + ROW / 2.0, c);
            }
        }
        if !self.timers {
            return;
        }
        // The timer row, under a line.
        let line_y = y + self.grid_h() + TIMER_TOP;
        g.fill_rect(rect(x, line_y, CAL_W, 1.0), white(0.08 * alpha));
        let top = line_y + 1.0 + 10.0;
        let cy = top + TIMER_H / 2.0;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let mut boxes = Vec::new();
        if let Some(t) = timer() {
            // What's running: its glyph and time (dimmed while paused).
            let f = Font::icons(13.0);
            let gw = g.text_at(if matches!(t, Timer::Stopwatch { .. }) { "\u{E916}" } else { "\u{E823}" }, &f, x + 2.0, cy + 1.0, dim);
            let text = clock_text(shown(t, Instant::now()), matches!(t, Timer::Countdown { .. }));
            g.text_at(&text, &cx.main(16.0), x + 2.0 + gw + 8.0, cy, if running(t) { fg } else { dim });
        }
        for (hit, bx, bw, label, glyph) in self.timer_buttons() {
            let r = rect(x + bx, top, bw, TIMER_H);
            let on = self.hovered == Some(hit);
            g.fill_round(r, 8.0, white(if on { 0.14 } else { 0.06 } * alpha));
            let f = if glyph { Font::icons(12.0) } else { cx.main(11.0) };
            let w = g.measure(label, &f);
            g.text_at(label, &f, x + bx + (bw - w) / 2.0, cy + if glyph { 1.0 } else { 0.0 }, if on { fg } else { dim });
            boxes.push((hit, [bx, top - y, bw, TIMER_H]));
        }
        *self.boxes.borrow_mut() = boxes;
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.timer_hit(p));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        let Some(hit) = self.timer_hit(at) else { return Reply::None };
        let now = Instant::now();
        let next = match (hit, timer()) {
            (TimerHit::Preset(m), _) => Some(Timer::Countdown { end: now + Duration::from_secs(m * 60), left: None }),
            (TimerHit::Stopwatch, _) => Some(Timer::Stopwatch { since: Some(now), acc: Duration::ZERO }),
            (TimerHit::Pause, Some(t @ Timer::Countdown { end, left })) => Some(match left {
                None => Timer::Countdown { end, left: Some(shown(t, now)) },
                Some(l) => Timer::Countdown { end: now + l, left: None },
            }),
            (TimerHit::Pause, Some(Timer::Stopwatch { since, acc })) => Some(match since {
                Some(s) => Timer::Stopwatch { since: None, acc: acc + now.saturating_duration_since(s) },
                None => Timer::Stopwatch { since: Some(now), acc },
            }),
            (TimerHit::AddMinute, Some(Timer::Countdown { end, left })) => Some(Timer::Countdown { end: end + Duration::from_secs(60), left: left.map(|l| l + Duration::from_secs(60)) }),
            (TimerHit::Reset, Some(Timer::Stopwatch { since, .. })) => Some(Timer::Stopwatch { since: since.map(|_| now), acc: Duration::ZERO }),
            (TimerHit::Stop, _) => None,
            (_, t) => t,
        };
        set_timer(next);
        Reply::Redraw
    }
    fn tooltip(&self) -> Option<String> {
        Some(match (self.hovered?, timer()) {
            (TimerHit::Preset(m), _) if m >= 60 => format!("Start a {}-hour timer", m / 60),
            (TimerHit::Preset(m), _) => format!("Start a {m}-minute timer"),
            (TimerHit::Stopwatch, _) => "Start a stopwatch".into(),
            (TimerHit::Pause, Some(t)) if running(t) => "Pause".into(),
            (TimerHit::Pause, _) => "Resume".into(),
            (TimerHit::AddMinute, _) => "Add a minute".into(),
            (TimerHit::Reset, _) => "Back to 0:00".into(),
            (TimerHit::Stop, _) => "Stop".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_times() {
        // A countdown rounds up (4:59.4 left shows 5:00), a stopwatch down.
        assert_eq!(clock_text(Duration::from_millis(299_400), true), "5:00");
        assert_eq!(clock_text(Duration::from_millis(299_400), false), "4:59");
        assert_eq!(clock_text(Duration::from_secs(3723), false), "1:02:03");
        let now = Instant::now();
        // Paused: what was left stays; running: counts down to the end.
        let paused = Timer::Countdown { end: now, left: Some(Duration::from_secs(90)) };
        assert_eq!(shown(paused, now + Duration::from_secs(30)), Duration::from_secs(90));
        let on = Timer::Countdown { end: now + Duration::from_secs(60), left: None };
        assert_eq!(shown(on, now + Duration::from_secs(20)), Duration::from_secs(40));
        assert!(running(on) && !running(paused));
        let sw = Timer::Stopwatch { since: Some(now), acc: Duration::from_secs(5) };
        assert_eq!(shown(sw, now + Duration::from_secs(3)), Duration::from_secs(8));
        // Run out: cleared once.
        set_timer(Some(Timer::Countdown { end: now, left: None }));
        assert!(take_expired(now + Duration::from_millis(1)));
        assert!(!take_expired(now + Duration::from_millis(2)));
        assert_eq!(timer(), None);
    }

    #[test]
    fn strftime_formats() {
        let t = SYSTEMTIME { wYear: 2026, wMonth: 10, wDayOfWeek: 5, wDay: 16, wHour: 9, wMinute: 5, wSecond: 7, wMilliseconds: 0 };
        assert_eq!(strftime("%H:%M", &t), "09:05");
        assert_eq!(strftime("%I:%M %p %a %b %d %y %%", &t), "09:05 AM Fri Oct 16 26 %");
    }

    #[test]
    fn iso_weeks() {
        // 2026-10-16 is a Friday in week 42; 2021-01-03 (a Sunday) belongs to week 53 of 2020.
        assert_eq!(iso_week(days_from_civil(2026, 10, 16)), 42);
        assert_eq!(iso_week(days_from_civil(2021, 1, 3)), 53);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }
}
