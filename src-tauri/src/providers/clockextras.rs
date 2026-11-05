//! The clock panel's extras ([widgets.clock]):
//! - "screentime" (`screen_time`): how long each app was in front today, sampled every
//!   five seconds while you're at the PC (no input for five minutes: not counted), kept
//!   in screen-time.json beside the config (today per app, the last week's totals);
//! - "worldclocks" (`world_clocks`, `[{ name, zone }]`, zone a Windows time zone):
//!   each place's offset from UTC now, and this PC's.

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tauri::AppHandle;
use windows::Win32::Foundation::SYSTEMTIME;
use windows::Win32::System::SystemInformation::{GetLocalTime, GetSystemTime};
use windows::Win32::System::Time::{EnumDynamicTimeZoneInformation, SystemTimeToTzSpecificLocalTimeEx, DYNAMIC_TIME_ZONE_INFORMATION};

/// One sample's worth of time in front.
const STEP: Duration = Duration::from_secs(5);
/// No input for this long: away (not counted).
const AWAY_MS: u32 = 5 * 60 * 1000;

fn store_path() -> std::path::PathBuf {
    crate::config::dir().join("screen-time.json")
}

fn today() -> String {
    let t = unsafe { GetLocalTime() };
    format!("{:04}-{:02}-{:02}", t.wYear, t.wMonth, t.wDay)
}

/// A SYSTEMTIME in minutes since 1970 (for offsets between two of them).
fn minutes(t: &SYSTEMTIME) -> i64 {
    chrono::NaiveDate::from_ymd_opt(t.wYear as i32, t.wMonth as u32, t.wDay as u32)
        .and_then(|d| d.and_hms_opt(t.wHour as u32, t.wMinute as u32, 0))
        .map_or(0, |dt| dt.and_utc().timestamp() / 60)
}

/// Every time zone Windows knows: its key name and display name ("(UTC+09:00) Osaka, Sapporo, Tokyo").
fn zones() -> Vec<(String, String, DYNAMIC_TIME_ZONE_INFORMATION)> {
    let mut out = Vec::new();
    for i in 0.. {
        let mut z = DYNAMIC_TIME_ZONE_INFORMATION::default();
        if unsafe { EnumDynamicTimeZoneInformation(i, &mut z) } != 0 {
            break;
        }
        let key = crate::win::wide_to_string(&z.TimeZoneKeyName);
        let display = crate::win::reg_string(windows::Win32::System::Registry::HKEY_LOCAL_MACHINE, &format!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\Time Zones\{key}"), "Display").unwrap_or_else(|| key.clone());
        out.push((key, display, z));
    }
    out
}

/// For the settings page: the time zones to pick from, `[{id, name}]`, by UTC offset.
#[tauri::command]
pub async fn time_zones() -> Vec<Value> {
    zones().into_iter().map(|(id, name, _)| json!({ "id": id, "name": name })).collect()
}

fn world_clocks(opts: &toml::Table, all: &[(String, String, DYNAMIC_TIME_ZONE_INFORMATION)]) -> Value {
    let utc = unsafe { GetSystemTime() };
    let local = unsafe { GetLocalTime() };
    let list: Vec<Value> = opts
        .get("world_clocks")
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let name = c.get("name")?.as_str()?.to_string();
            let zone = c.get("zone")?.as_str()?;
            let (_, _, z) = all.iter().find(|(k, _, _)| k == zone)?;
            let mut there = SYSTEMTIME::default();
            unsafe { SystemTimeToTzSpecificLocalTimeEx(Some(z), &utc, &mut there) }.ok()?;
            Some(json!({ "name": name, "offset": minutes(&there) - minutes(&utc) }))
        })
        .collect();
    json!({ "local": minutes(&local) - minutes(&utc), "list": list })
}

/// Screen time: today's seconds per program, their names, the past days' totals.
#[derive(Default)]
struct Tracker {
    day: String,
    apps: HashMap<String, u64>,
    names: HashMap<String, String>,
    days: Map<String, Value>,
    loaded: bool,
    saved: Option<Instant>,
}

impl Tracker {
    fn load(&mut self) {
        self.loaded = true;
        let Some(v) = std::fs::read_to_string(store_path()).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()) else { return };
        self.day = v["day"].as_str().unwrap_or("").to_string();
        self.apps = v["apps"].as_object().map(|m| m.iter().filter_map(|(k, s)| Some((k.clone(), s.as_u64()?))).collect()).unwrap_or_default();
        self.names = v["names"].as_object().map(|m| m.iter().filter_map(|(k, s)| Some((k.clone(), s.as_str()?.to_string()))).collect()).unwrap_or_default();
        self.days = v["days"].as_object().cloned().unwrap_or_default();
    }
    fn save(&mut self) {
        // The last week's totals only.
        let mut keys: Vec<String> = self.days.keys().cloned().collect();
        keys.sort();
        while keys.len() > 7 {
            self.days.remove(&keys.remove(0));
        }
        let v = json!({ "day": self.day, "apps": self.apps, "names": self.names, "days": self.days });
        let _ = std::fs::write(store_path(), v.to_string());
        self.saved = Some(Instant::now());
    }
    fn total(&self) -> u64 {
        self.apps.values().sum()
    }
    /// The app in front gets another step (unless you're away or it's the desktop).
    fn sample(&mut self) {
        if !self.loaded {
            self.load();
        }
        let day = today();
        if day != self.day {
            if !self.day.is_empty() {
                let total = self.total();
                self.days.insert(self.day.clone(), json!(total));
            }
            self.day = day;
            self.apps.clear();
        }
        use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
        let mut li = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
        let idle = unsafe { GetLastInputInfo(&mut li) }.as_bool().then(|| unsafe { windows::Win32::System::SystemInformation::GetTickCount() }.wrapping_sub(li.dwTime)).unwrap_or(0);
        if idle >= AWAY_MS {
            return;
        }
        let front = unsafe { windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow() };
        let pid = crate::win::window_pid(front);
        if pid == 0 || pid == std::process::id() {
            return;
        }
        let Some(path) = crate::win::process_path(pid) else { return };
        let exe = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_lowercase();
        if exe == "explorer.exe" && crate::win::window_title(front).is_empty() {
            return; // the desktop
        }
        if !self.names.contains_key(&exe) {
            let name = crate::win::file_description(&path).unwrap_or_else(|| exe.trim_end_matches(".exe").to_string());
            self.names.insert(exe.clone(), name);
        }
        *self.apps.entry(exe).or_default() += STEP.as_secs();
        if self.saved.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
            self.save();
        }
    }
    fn summary(&self) -> Value {
        let mut list: Vec<(&String, &u64)> = self.apps.iter().collect();
        list.sort_by(|a, b| b.1.cmp(a.1));
        let apps: Vec<Value> = list.into_iter().take(4).map(|(exe, s)| json!({ "name": self.names.get(exe).cloned().unwrap_or_else(|| exe.clone()), "secs": s })).collect();
        let week = self.total() + self.days.values().filter_map(|v| v.as_u64()).sum::<u64>();
        json!({ "total": self.total(), "week": week, "apps": apps })
    }
}

pub fn run(app: AppHandle) {
    let mut tracker = Tracker::default();
    let mut all_zones: Option<Vec<(String, String, DYNAMIC_TIME_ZONE_INFORMATION)>> = None;
    let mut last_clocks: Option<Instant> = None;
    loop {
        let placed = super::placed(&app, "clock");
        let opts = super::widget_opts(&app, "clock");
        if placed && opts.get("screen_time").and_then(|v| v.as_bool()) == Some(true) {
            tracker.sample();
            super::publish(&app, "screentime", tracker.summary());
        } else if tracker.loaded {
            tracker.save();
            tracker = Tracker::default();
        }
        let wants_clocks = placed && opts.get("world_clocks").and_then(|v| v.as_array()).is_some_and(|a| !a.is_empty());
        if wants_clocks && last_clocks.is_none_or(|t| t.elapsed() >= Duration::from_secs(30)) {
            let all = all_zones.get_or_insert_with(zones);
            super::publish(&app, "worldclocks", world_clocks(&opts, all));
            last_clocks = Some(Instant::now());
        }
        std::thread::sleep(STEP);
    }
}
