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
    /// A running timer's time beside the clock ([widgets.clock] timers), and when it's
    /// looked at next.
    timers: bool,
    timer_text: String,
    beat: Instant,
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
        let mut c = Clock { format, text: String::new(), next: Instant::now(), timers, timer_text: String::new(), beat: Instant::now(), pending: None };
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
            return Some(self.next.min(self.beat));
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
        self.beat = now + TIMER_TICK;
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

/// The calendar's look ([widgets.clock] style).
#[derive(Clone, Copy, PartialEq, Debug)]
enum CalStyle {
    /// The month (its parts as [`MonthLook`] says).
    Month,
    /// Today large, this week as a strip, how far through the day and the year.
    Week,
}

/// Where the month's name goes ([widgets.clock] title_align).
#[derive(Clone, Copy, PartialEq, Debug)]
enum TitleAlign {
    Left,
    Center,
    Right,
    /// Large, across the whole calendar.
    Big,
}

/// The month's parts ([widgets.clock]): week numbers, the weekday row, the days of the
/// months either side, past days faded, weekends dimmer, today filled or underlined,
/// arrows to other months, the name's place, size and year; this week's progress bars.
#[derive(Clone, Copy, Debug)]
struct MonthLook {
    weeks: bool,
    day_names: bool,
    other_days: bool,
    fade_past: bool,
    dim_weekends: bool,
    underline: bool,
    arrows: bool,
    title: TitleAlign,
    /// The name's size (not the big one's), and whether this year shows after it (another
    /// year's always does).
    title_size: f32,
    year: bool,
    progress: bool,
}

impl MonthLook {
    fn of(opts: &Opts) -> MonthLook {
        let on = |k: &str, default: bool| opts.flag(k).unwrap_or(default);
        MonthLook {
            weeks: on("week_numbers", true),
            day_names: on("day_names", true),
            other_days: on("other_days", true),
            fade_past: on("fade_past", false),
            dim_weekends: on("dim_weekends", false),
            underline: opts.str("today_mark") == Some("underline"),
            arrows: on("month_arrows", false),
            title: match opts.str("title_align") {
                Some("left") => TitleAlign::Left,
                Some("right") => TitleAlign::Right,
                Some("big") => TitleAlign::Big,
                _ => TitleAlign::Center,
            },
            title_size: opts.num("title_size").map_or(13.0, |v| v.clamp(11.0, 32.0) as f32),
            year: on("show_year", true),
            progress: on("progress", true),
        }
    }
}

/// The calendar in its look ([widgets.clock] style): the month or this week. With
/// [widgets.clock] timers, a row under it: a countdown's presets and a stopwatch, or the
/// one running with its buttons.
pub struct Calendar {
    style: CalStyle,
    look: MonthLook,
    /// The title's and the weekday row's heights as last measured, the big name's size
    /// and height, and the calendar's own height (above the timer row).
    heights: (f32, f32),
    big: (f32, f32),
    body_h: f32,
    timers: bool,
    /// What can be clicked as last laid out (content px), and the one hovered.
    boxes: std::cell::RefCell<Vec<(Hit, [f32; 4])>>,
    hovered: Option<Hit>,
    /// When it looks again (a running timer's time, today at midnight).
    beat: Instant,
    /// The month shown: months away from this one. Week: the month shown under today.
    shift: i32,
    full: bool,
}

/// What can be clicked: the timer row's buttons, the month's arrows, this week's strip
/// (the month) and the month under it (back to the week).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    /// A countdown of so many minutes.
    Preset(u64),
    Stopwatch,
    Pause,
    AddMinute,
    Reset,
    Stop,
    MonthBack,
    MonthOn,
    Strip,
    Month,
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
/// An arrow button beside the month's name.
const ARROW: f32 = 22.0;
/// This week: the big date's size, the gaps between its parts.
const BIG: f32 = 40.0;
const PART_GAP: f32 = 12.0;
const WEEKDAY_NAMES: [&str; 7] = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"];
const DOW: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

/// The month `shift` months from (y, m).
fn add_months(y: i32, m: u32, shift: i32) -> (i32, u32) {
    let k = y * 12 + m as i32 - 1 + shift;
    (k.div_euclid(12), k.rem_euclid(12) as u32 + 1)
}

/// How many days a month has.
fn month_len(y: i32, m: u32) -> i64 {
    let (ny, nm) = add_months(y, m, 1);
    days_from_civil(ny, nm, 1) - days_from_civil(y, m, 1)
}

impl Default for Calendar {
    fn default() -> Calendar {
        Calendar {
            style: CalStyle::Month,
            look: MonthLook::of(&Opts::default()),
            heights: (0.0, 0.0),
            big: (BIG, 0.0),
            body_h: 0.0,
            timers: false,
            boxes: Default::default(),
            hovered: None,
            beat: Instant::now(),
            shift: 0,
            full: false,
        }
    }
}

impl Calendar {
    pub fn new(opts: &Opts) -> Calendar {
        let style = if opts.str("style") == Some("week") { CalStyle::Week } else { CalStyle::Month };
        Calendar { style, look: MonthLook::of(opts), timers: opts.flag("timers") == Some(true), ..Default::default() }
    }

    /// The month shown (this one, or the arrows' pick).
    fn shown_month(&self) -> (i32, u32) {
        let t = now();
        add_months(t.wYear as i32, t.wMonth as u32, self.shift)
    }

    /// How many week rows the month takes: six with the months either side, else its own.
    fn weeks(&self) -> i64 {
        if self.look.other_days {
            return 6;
        }
        let (y, m) = self.shown_month();
        (weekday(days_from_civil(y, m, 1)) + month_len(y, m) + 6) / 7
    }

    /// The title row's height (the big name's is its own).
    fn title_h(&self) -> f32 {
        if self.look.title == TitleAlign::Big { self.big.1.max(self.heights.0) } else { self.heights.0 }
    }

    /// The month's height: its name (with `title`), the weekday row, the weeks.
    fn month_h(&self, title: bool) -> f32 {
        let dow = if self.look.day_names { self.heights.1 + GAP } else { 0.0 };
        (if title { self.title_h() + 8.0 } else { 0.0 }) + dow + self.weeks() as f32 * (ROW + GAP) - GAP
    }

    /// This week's parts: the big date's, the strip's and a progress row's heights.
    fn week_parts(cx: &Ctx) -> (f32, f32, f32) {
        let h = |s: &str, f: &Font| cx.gfx.text(s, f).map_or(f.size * 1.21, |t| t.height);
        let big = h("4", &cx.main(BIG));
        let strip = 6.0 + h("Mo", &cx.main(11.0)) + 2.0 + h("4", &cx.main(13.0)) + 6.0;
        let prog = h("Ag", &cx.secondary(11.0)) + 5.0 + 4.0;
        (big, strip, prog)
    }

    /// The timer row's buttons: what, x (from the row's left), width, label or glyph,
    /// and whether that's a glyph. Idle: the presets and the stopwatch filling the row;
    /// running: its buttons at the row's end.
    fn timer_buttons(&self) -> Vec<(Hit, f32, f32, &'static str, bool)> {
        let b = |hit, glyph| (hit, 0.0, 30.0, glyph, true);
        let pause = |t: Timer| if running(t) { "\u{E769}" } else { "\u{E768}" };
        let mut out = match timer() {
            None => {
                let n = PRESETS.len() as f32 + 1.0;
                let w = (CAL_W - (n - 1.0) * 4.0) / n;
                let mut v: Vec<_> = PRESETS.iter().enumerate().map(|(i, (m, l))| (Hit::Preset(*m), i as f32 * (w + 4.0), w, *l, false)).collect();
                v.push((Hit::Stopwatch, (n - 1.0) * (w + 4.0), w, "\u{E916}", true));
                return v;
            }
            Some(t @ Timer::Countdown { .. }) => vec![b(Hit::Pause, pause(t)), (Hit::AddMinute, 0.0, 30.0, "+1", false), b(Hit::Stop, "\u{E71A}")],
            Some(t @ Timer::Stopwatch { .. }) => vec![b(Hit::Pause, pause(t)), b(Hit::Reset, "\u{E72C}"), b(Hit::Stop, "\u{E71A}")],
        };
        let mut x = CAL_W;
        for i in (0..out.len()).rev() {
            x -= out[i].2;
            out[i].1 = x;
            x -= 4.0;
        }
        out
    }

    fn hit(&self, p: (f32, f32)) -> Option<Hit> {
        self.boxes.borrow().iter().find(|(_, b)| p.0 >= b[0] && p.0 < b[0] + b[2] && p.1 >= b[1] && p.1 < b[1] + b[3]).map(|(h, _)| *h)
    }

    fn heights(&self, cx: &Ctx) -> (f32, f32) {
        let title = cx.gfx.text("Ag", &cx.main(self.look.title_size)).map_or(16.0, |t| t.height);
        let dow = cx.gfx.text("Mo", &cx.main(11.0)).map_or(13.0, |t| t.height);
        (title, dow)
    }

    /// The month's name and its year: another year's always, this one's as asked (never
    /// beside the big name).
    fn month_name(&self) -> String {
        let t = now();
        let (y, m) = self.shown_month();
        let name = MONTH_NAMES[(m as usize - 1) % 12];
        let this_year = y == t.wYear as i32;
        if this_year && (self.look.title == TitleAlign::Big || !self.look.year) { name.to_string() } else { format!("{name} {y}") }
    }

    /// The big name's font size: as large as fills the room beside the arrows.
    fn big_size(&self, cx: &Ctx) -> (f32, f32) {
        let room = CAL_W - if self.look.arrows { 2.0 * ARROW + 8.0 } else { 0.0 };
        let w = cx.gfx.measure(&self.month_name(), &cx.main(BIG)).max(1.0);
        let size = (BIG * room / w).clamp(18.0, 72.0).floor();
        (size, cx.gfx.text("Ag", &cx.main(size)).map_or(size * 1.21, |t| t.height))
    }

    /// The month from `y`: its name and arrows (with `title`), the weekday row, the weeks
    /// with their numbers, each part as [widgets.clock] says.
    fn draw_month(&self, cx: &Ctx, x: f32, y: f32, alpha: f32, title: bool, boxes: &mut Vec<(Hit, [f32; 4])>) {
        let g = cx.gfx;
        let t = now();
        let look = self.look;
        let (_, dow_h) = self.heights;
        let a = |c: f32| white(c * alpha);
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let centred = |s: &str, f: &Font, left: f32, w: f32, cy: f32, c| {
            let tw = g.measure(s, f);
            g.text_at(s, f, left + (w - tw) / 2.0, cy, c);
        };
        let mut grid = y;
        if title {
            let th = self.title_h();
            let cy = y + th / 2.0;
            let name = self.month_name();
            // The arrows: at the far side from a left or right name, either side of a
            // centred one, after a big one.
            let arrow_xs: [f32; 2] = match look.title {
                TitleAlign::Left | TitleAlign::Big => [CAL_W - 2.0 * ARROW - 2.0, CAL_W - ARROW],
                TitleAlign::Right => [0.0, ARROW + 2.0],
                TitleAlign::Center => [0.0, CAL_W - ARROW],
            };
            let (f, inset) = if look.title == TitleAlign::Big { (cx.main(self.big.0), 0.0) } else { (cx.main(look.title_size), 4.0) };
            let nw = g.measure(&name, &f);
            let nx = match look.title {
                TitleAlign::Left | TitleAlign::Big => x + inset,
                TitleAlign::Right => x + CAL_W - inset - nw,
                TitleAlign::Center => x + (CAL_W - nw) / 2.0,
            };
            g.text_at(&name, &f, nx, cy, fg);
            if look.arrows {
                let gf = Font::icons(11.0);
                for (k, (hit, glyph)) in [(Hit::MonthBack, "\u{E76B}"), (Hit::MonthOn, "\u{E76C}")].into_iter().enumerate() {
                    let bx = arrow_xs[k];
                    let on = self.hovered == Some(hit);
                    if on {
                        g.fill_round(rect(x + bx, cy - ARROW / 2.0, ARROW, ARROW), 7.0, white(0.1 * alpha));
                    }
                    let w = g.measure(glyph, &gf);
                    g.text_at(glyph, &gf, x + bx + (ARROW - w) / 2.0, cy + 1.0, if on { fg } else { dim });
                    boxes.push((hit, [bx, cy - y - ARROW / 2.0, ARROW, ARROW]));
                }
            }
            grid += th + 8.0;
        }
        let wk = if look.weeks { WK_W } else { 0.0 };
        let col = (CAL_W - wk) / 7.0;
        if look.day_names {
            for (i, d) in DOW.iter().enumerate() {
                centred(d, &cx.main(11.0), x + wk + col * i as f32, col, grid + dow_h / 2.0, dim);
            }
            grid += dow_h + GAP;
        }
        let (yy, mm) = self.shown_month();
        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        let first = days_from_civil(yy, mm, 1);
        let start = first - weekday(first);
        let last = first + month_len(yy, mm);
        let (wk_font, day_font) = (cx.main(10.0), cx.main(12.0));
        for week in 0..self.weeks() {
            let top = grid + week as f32 * (ROW + GAP);
            let row_start = start + week * 7;
            if look.weeks {
                centred(&iso_week(row_start).to_string(), &wk_font, x, WK_W, top + ROW / 2.0, a(0.28));
            }
            for i in 0..7 {
                let day = row_start + i;
                let ours = day >= first && day < last;
                if !ours && !look.other_days {
                    continue;
                }
                let left = x + wk + col * i as f32;
                let mut c = if !ours {
                    a(0.25)
                } else if look.fade_past && day < today {
                    a(0.32)
                } else if look.dim_weekends && i >= 5 {
                    a(0.62)
                } else {
                    fg
                };
                if day == today && !look.underline {
                    g.fill_round(rect(left, top, col, ROW), 12.0, with_alpha(cx.look.accent, alpha));
                    c = rgba(0.0, 0.0, 0.0, alpha);
                }
                let (_, _, d) = civil_from_days(day);
                centred(&d.to_string(), &day_font, left, col, top + ROW / 2.0, c);
                if day == today && look.underline {
                    g.fill_round(rect(left + col / 2.0 - 7.0, top + ROW - 3.0, 14.0, 2.0), 1.0, with_alpha(cx.look.accent, alpha));
                }
            }
        }
    }

    /// This week: the date large with its weekday, the week as a strip (today in the
    /// accent), how far through the day and the year; the month under it once asked for.
    fn draw_week(&self, cx: &Ctx, x: f32, y: f32, alpha: f32, boxes: &mut Vec<(Hit, [f32; 4])>) {
        let g = cx.gfx;
        let t = now();
        let (fg, dim) = (with_alpha(cx.look.fg, alpha), with_alpha(cx.look.dimmed(), alpha));
        let (big_h, strip_h, prog_h) = Self::week_parts(cx);
        let today = days_from_civil(t.wYear as i32, t.wMonth as u32, t.wDay as u32);
        let nw = g.text_at(&t.wDay.to_string(), &cx.main(BIG), x, y + big_h / 2.0, fg);
        let wd = weekday(today) as usize;
        g.text_at(WEEKDAY_NAMES[wd], &cx.main(13.0), x + nw + 12.0, y + big_h / 2.0 - 9.0, fg);
        let sub = format!("{} {} · week {}", MONTH_NAMES[(t.wMonth as usize).saturating_sub(1) % 12], t.wYear, iso_week(today));
        g.text_at(&sub, &cx.secondary(12.0), x + nw + 12.0, y + big_h / 2.0 + 9.0, dim);
        // The strip: Monday to Sunday.
        let top = y + big_h + PART_GAP;
        let cw = CAL_W / 7.0;
        let (lf, nf) = (cx.main(11.0), cx.main(13.0));
        let dow_h = self.heights.1;
        let monday = today - wd as i64;
        for i in 0..7 {
            let day = monday + i as i64;
            let left = x + cw * i as f32;
            let (label_c, num_c) = if day == today {
                g.fill_round(rect(left + 1.0, top, cw - 2.0, strip_h), 9.0, with_alpha(cx.look.accent, alpha));
                (rgba(0.0, 0.0, 0.0, alpha), rgba(0.0, 0.0, 0.0, alpha))
            } else if day < today {
                (dim, dim)
            } else {
                (fg, fg)
            };
            let centred = |s: &str, f: &Font, cy: f32, c| {
                let w = g.measure(s, f);
                g.text_at(s, f, left + (cw - w) / 2.0, cy, c);
            };
            centred(DOW[i], &lf, top + 6.0 + dow_h / 2.0, label_c);
            let (_, _, d) = civil_from_days(day);
            let nh = strip_h - 6.0 - dow_h - 2.0 - 6.0;
            centred(&d.to_string(), &nf, top + 6.0 + dow_h + 2.0 + nh / 2.0, num_c);
        }
        boxes.push((Hit::Strip, [0.0, top - y, CAL_W, strip_h]));
        let mut py = top + strip_h + PART_GAP;
        // How far through the day and the year ([widgets.clock] progress).
        if self.look.progress {
            let day_frac = (t.wHour as f32 * 3600.0 + t.wMinute as f32 * 60.0 + t.wSecond as f32) / 86400.0;
            let jan1 = days_from_civil(t.wYear as i32, 1, 1);
            let year_len = days_from_civil(t.wYear as i32 + 1, 1, 1) - jan1;
            let year_frac = ((today - jan1) as f32 + day_frac) / year_len as f32;
            let lf = cx.secondary(11.0);
            let lh = prog_h - 9.0;
            for (label, frac) in [("Today", day_frac), ("This year", year_frac)] {
                g.text_at(label, &lf, x, py + lh / 2.0, dim);
                let pct = format!("{}%", (frac * 100.0).floor());
                let pw = g.measure(&pct, &lf);
                g.text_at(&pct, &lf, x + CAL_W - pw, py + lh / 2.0, dim);
                let by = py + lh + 5.0;
                g.fill_round(rect(x, by, CAL_W, 4.0), 2.0, white(0.14 * alpha));
                g.fill_round(rect(x, by, (CAL_W * frac).max(4.0), 4.0), 2.0, fg);
                py += prog_h + 8.0;
            }
            py -= 8.0;
        } else {
            py -= PART_GAP;
        }
        if self.full {
            let my = py + PART_GAP;
            g.fill_rect(rect(x, my - PART_GAP / 2.0, CAL_W, 1.0), white(0.08 * alpha));
            self.draw_month(cx, x, my, alpha, false, boxes);
            boxes.push((Hit::Month, [0.0, my - y, CAL_W, self.month_h(false)]));
        }
    }
}

impl Panel for Calendar {
    fn wake(&self) -> Option<Instant> {
        // A running timer: its time; else today moves at midnight: every minute.
        Some(self.beat)
    }
    fn tick(&mut self, now: Instant) -> bool {
        let running = self.timers && timer().is_some_and(running);
        self.beat = now + if running { TIMER_TICK } else { Duration::from_secs(60) };
        true
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        self.heights = self.heights(cx);
        if self.look.title == TitleAlign::Big {
            self.big = self.big_size(cx);
        }
        self.body_h = match self.style {
            CalStyle::Month => self.month_h(true),
            CalStyle::Week => {
                let (big, strip, prog) = Self::week_parts(cx);
                let progress = if self.look.progress { PART_GAP + prog + 8.0 + prog } else { 0.0 };
                big + PART_GAP + strip + progress + if self.full { PART_GAP + self.month_h(false) } else { 0.0 }
            }
        };
        (CAL_W, self.body_h + if self.timers { TIMER_TOP + 1.0 + 10.0 + TIMER_H } else { 0.0 })
    }
    fn pieces(&self) -> Vec<(f32, f32, f32, f32)> {
        if self.style != CalStyle::Month {
            return vec![(0.0, 0.0, CAL_W, self.body_h)];
        }
        // The cascade: the name, the weekday row, each week.
        let mut out = vec![(0.0, 0.0, CAL_W, self.title_h())];
        let mut top = self.title_h() + 8.0;
        if self.look.day_names {
            out.push((0.0, top, CAL_W, self.heights.1));
            top += self.heights.1 + GAP;
        }
        for week in 0..self.weeks() {
            out.push((0.0, top + week as f32 * (ROW + GAP), CAL_W, ROW));
        }
        out
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let mut boxes = Vec::new();
        match self.style {
            CalStyle::Month => self.draw_month(cx, x, y, alpha, true, &mut boxes),
            CalStyle::Week => self.draw_week(cx, x, y, alpha, &mut boxes),
        }
        if self.timers {
            // The timer row, under a line.
            let line_y = y + self.body_h + TIMER_TOP;
            g.fill_rect(rect(x, line_y, CAL_W, 1.0), white(0.08 * alpha));
            let top = line_y + 1.0 + 10.0;
            let cy = top + TIMER_H / 2.0;
            let fg = with_alpha(cx.look.fg, alpha);
            let dim = with_alpha(cx.look.dimmed(), alpha);
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
        }
        *self.boxes.borrow_mut() = boxes;
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.hit(p));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        let Some(hit) = self.hit(at) else { return Reply::None };
        let now = Instant::now();
        let next = match (hit, timer()) {
            (Hit::MonthBack | Hit::MonthOn, t) => {
                self.shift += if hit == Hit::MonthOn { 1 } else { -1 };
                t
            }
            (Hit::Strip | Hit::Month, t) => {
                self.full = !self.full;
                t
            }
            (Hit::Preset(m), _) => Some(Timer::Countdown { end: now + Duration::from_secs(m * 60), left: None }),
            (Hit::Stopwatch, _) => Some(Timer::Stopwatch { since: Some(now), acc: Duration::ZERO }),
            (Hit::Pause, Some(t @ Timer::Countdown { end, left })) => Some(match left {
                None => Timer::Countdown { end, left: Some(shown(t, now)) },
                Some(l) => Timer::Countdown { end: now + l, left: None },
            }),
            (Hit::Pause, Some(Timer::Stopwatch { since, acc })) => Some(match since {
                Some(s) => Timer::Stopwatch { since: None, acc: acc + now.saturating_duration_since(s) },
                None => Timer::Stopwatch { since: Some(now), acc },
            }),
            (Hit::AddMinute, Some(Timer::Countdown { end, left })) => Some(Timer::Countdown { end: end + Duration::from_secs(60), left: left.map(|l| l + Duration::from_secs(60)) }),
            (Hit::Reset, Some(Timer::Stopwatch { since, .. })) => Some(Timer::Stopwatch { since: since.map(|_| now), acc: Duration::ZERO }),
            (Hit::Stop, _) => None,
            (_, t) => t,
        };
        set_timer(next);
        self.beat = now;
        Reply::Redraw
    }
    fn tooltip(&self) -> Option<String> {
        Some(match (self.hovered?, timer()) {
            (Hit::Preset(m), _) if m >= 60 => format!("Start a {}-hour timer", m / 60),
            (Hit::Preset(m), _) => format!("Start a {m}-minute timer"),
            (Hit::Stopwatch, _) => "Start a stopwatch".into(),
            (Hit::Pause, Some(t)) if running(t) => "Pause".into(),
            (Hit::Pause, _) => "Resume".into(),
            (Hit::AddMinute, _) => "Add a minute".into(),
            (Hit::Reset, _) => "Back to 0:00".into(),
            (Hit::Stop, _) => "Stop".into(),
            (Hit::MonthBack, _) => "Previous month".into(),
            (Hit::MonthOn, _) => "Next month".into(),
            (Hit::Strip, _) => "Show the month".into(),
            (Hit::Month, _) => "Back to this week".into(),
        })
    }
    fn closed(&mut self) {
        // Opened again: this month, this week.
        self.shift = 0;
        self.full = false;
        self.hovered = None;
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
    fn months() {
        assert_eq!(add_months(2026, 11, 2), (2027, 1));
        assert_eq!(add_months(2026, 1, -1), (2025, 12));
        assert_eq!(month_len(2024, 2), 29);
        assert_eq!(month_len(2026, 11), 30);
        assert_eq!(month_len(2026, 12), 31);
    }

    #[test]
    fn iso_weeks() {
        // 2026-10-16 is a Friday in week 42; 2021-01-03 (a Sunday) belongs to week 53 of 2020.
        assert_eq!(iso_week(days_from_civil(2026, 10, 16)), 42);
        assert_eq!(iso_week(days_from_civil(2021, 1, 3)), 53);
        assert_eq!(civil_from_days(days_from_civil(2024, 2, 29)), (2024, 2, 29));
    }
}
