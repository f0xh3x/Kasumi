//! The clock (src/widgets/clock.ts) and its calendar panel (calendar.ts).

use std::time::{Duration, Instant};

use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::GetLocalTime;

use super::{Ctx, Opts, Panel, Widget};
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

pub struct Clock {
    format: String,
    text: String,
    next: Instant,
}

impl Clock {
    pub fn new(opts: Opts) -> Clock {
        let format = match opts.str("format").unwrap_or("%H:%M") {
            // Windows' own short time format (Region settings).
            "system" => crate::win::short_time_pattern(),
            f => f.to_string(),
        };
        let mut c = Clock { format, text: String::new(), next: Instant::now() };
        c.tick(Instant::now());
        c
    }
}

impl Widget for Clock {
    fn wake(&self) -> Option<Instant> {
        Some(self.next)
    }
    fn tick(&mut self, now: Instant) -> bool {
        let t = self::now();
        // The next whole second (or minute, without seconds in the format).
        let ms = 1000 - t.wMilliseconds as u64 + 5;
        let wait = if self.format.contains("%S") { ms } else { (59 - t.wSecond as u64) * 1000 + ms };
        self.next = now + Duration::from_millis(wait);
        let text = strftime(&self.format, &t);
        let changed = text != self.text;
        self.text = text;
        changed
    }
    fn width(&mut self, cx: &Ctx) -> f32 {
        cx.gfx.measure(&self.text, &cx.main(14.0))
    }
    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        cx.gfx.text_at(&self.text, &cx.main(14.0), x, cy, with_alpha(cx.look.fg, alpha));
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

/// Month view, Monday first, with ISO week numbers; today in the accent.
pub struct Calendar;

// styles.css .panel-calendar: 228 px wide, a 22 px week column, 24 px rows 2 px apart.
const CAL_W: f32 = 228.0;
const WK_W: f32 = 22.0;
const ROW: f32 = 24.0;
const GAP: f32 = 2.0;

impl Calendar {
    fn heights(cx: &Ctx) -> (f32, f32) {
        let title = cx.gfx.text("Ag", &cx.main(13.0)).map_or(16.0, |t| t.height);
        let dow = cx.gfx.text("Mo", &cx.main(11.0)).map_or(13.0, |t| t.height);
        (title, dow)
    }
}

impl Panel for Calendar {
    fn wake(&self) -> Option<Instant> {
        // Today moves at midnight: look again every minute.
        Some(Instant::now() + Duration::from_secs(60))
    }
    fn tick(&mut self, _now: Instant) -> bool {
        true
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let (title, dow) = Self::heights(cx);
        (CAL_W, title + 8.0 + dow + 6.0 * (ROW + GAP))
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
