//! Current weather from Open-Meteo (no API key).
//!
//! Config (`[widgets.weather]`): `latitude`, `longitude`, `location` (label),
//! `units` ("metric" | "imperial"), `interval_min`.

use super::{get_json, http_agent, opt_f64, opt_str, publish, widget_enabled, widget_opts};
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::AppHandle;

const RETRY: Duration = Duration::from_secs(60);

#[derive(PartialEq, Clone)]
struct Query {
    lat: f64,
    lon: f64,
    imperial: bool,
}

fn query(opts: &toml::Table) -> Option<Query> {
    Some(Query {
        lat: opt_f64(opts, "latitude")?,
        lon: opt_f64(opts, "longitude")?,
        imperial: opt_str(opts, "units", "metric") == "imperial",
    })
}

fn fetch(agent: &ureq::Agent, q: &Query, location: &str) -> Result<Value, String> {
    let units = if q.imperial { "&temperature_unit=fahrenheit" } else { "" };
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}\
         &current=temperature_2m,apparent_temperature,weather_code,is_day\
         &hourly=temperature_2m,weather_code,is_day&forecast_hours=13\
         &daily=temperature_2m_max,temperature_2m_min&forecast_days=1&timezone=auto{units}",
        q.lat, q.lon
    );
    let r = get_json(agent, &url)?;
    let cur = &r["current"];
    let first = |v: &Value| v.get(0).cloned().unwrap_or(Value::Null);
    // Next hours, starting with the current one (local times like "2026-10-07T15:00").
    let h = &r["hourly"];
    let hourly: Vec<Value> = h["time"]
        .as_array()
        .map(|times| {
            times
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    json!({
                        "time": t,
                        "temp": h["temperature_2m"][i],
                        "code": h["weather_code"][i],
                        "isDay": h["is_day"][i].as_i64() == Some(1),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(json!({
        "temp": cur["temperature_2m"],
        "feels": cur["apparent_temperature"],
        "code": cur["weather_code"],
        "isDay": cur["is_day"].as_i64() == Some(1),
        "min": first(&r["daily"]["temperature_2m_min"]),
        "max": first(&r["daily"]["temperature_2m_max"]),
        "location": location,
        "hourly": hourly,
    }))
}

pub fn run(app: AppHandle) {
    let agent = http_agent();
    let mut fetched: Option<(Query, Instant)> = None;
    let mut next_try = Instant::now();
    loop {
        if !widget_enabled(&app, "weather") {
            std::thread::sleep(Duration::from_secs(5));
            continue;
        }
        let opts = widget_opts(&app, "weather");
        let interval = Duration::from_secs((opt_f64(&opts, "interval_min").unwrap_or(15.0).max(1.0) * 60.0) as u64);
        let Some(q) = query(&opts) else {
            publish(&app, "weather", Value::Null);
            std::thread::sleep(Duration::from_secs(5));
            continue;
        };

        // Refetch when the location/units change, the data is stale, or a retry is due.
        let stale = match &fetched {
            Some((last, at)) => *last != q || at.elapsed() >= interval,
            None => true,
        };
        if stale && Instant::now() >= next_try {
            match fetch(&agent, &q, &opt_str(&opts, "location", "")) {
                Ok(v) => {
                    publish(&app, "weather", v);
                    fetched = Some((q, Instant::now()));
                }
                Err(e) => {
                    eprintln!("[kasumi] weather: {e}");
                    next_try = Instant::now() + RETRY;
                }
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}
