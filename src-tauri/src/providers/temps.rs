//! Hardware readings for the PC temps (performance) widget: load and memory from
//! Windows itself (perf.rs), temperatures, power and clocks from LibreHardwareMonitor's
//! remote web server when `lhm` is on (the default), its readings taking precedence.
//!
//! Config (`[widgets.temps]`): `url` (or `port`), `interval_ms`, and LHM
//! SensorIds for each reading: `cpu_sensor`, `gpu_sensor` (temperatures),
//! `cpu_load`, `cpu_power`, `cpu_clock`, `gpu_load`, `gpu_power`, `vram_used`,
//! `vram_total`, `ram_used`, `ram_available`. Defaults match an AMD CPU with
//! an NVIDIA GPU (find others in http://localhost:8085/data.json).

use super::{get_json, http_agent, opt_f64, opt_str, publish, widget_enabled, widget_opts};
use serde_json::{json, Map, Value};
use std::time::Duration;
use tauri::AppHandle;

/// (config key, JSON field, default SensorId)
const SENSORS: &[(&str, &str, &str)] = &[
    ("cpu_sensor", "cpu", "/amdcpu/0/temperature/2"),
    ("gpu_sensor", "gpu", "/gpu-nvidia/0/temperature/0"),
    ("cpu_load", "cpuLoad", "/amdcpu/0/load/0"),
    ("cpu_power", "cpuPower", "/amdcpu/0/power/0"),
    ("cpu_clock", "cpuClock", "/amdcpu/0/clock/1"),
    ("gpu_load", "gpuLoad", "/gpu-nvidia/0/load/0"),
    ("gpu_power", "gpuPower", "/gpu-nvidia/0/power/0"),
    ("vram_used", "vramUsed", "/gpu-nvidia/0/smalldata/1"),
    ("vram_total", "vramTotal", "/gpu-nvidia/0/smalldata/2"),
    ("ram_used", "ramUsed", "/ram/data/0"),
    ("ram_available", "ramAvailable", "/ram/data/1"),
];

/// Depth-first search for a sensor by its LHM SensorId. Also returns the name of
/// its hardware node (tree: hardware > sensor type > sensor).
fn find_sensor<'a>(node: &'a Value, id: &str, path: &mut Vec<&'a str>) -> Option<(&'a Value, String)> {
    if node.get("SensorId").and_then(Value::as_str) == Some(id) {
        let hardware = path.len().checked_sub(2).and_then(|i| path.get(i)).copied().unwrap_or_default();
        return Some((node, hardware.to_string()));
    }
    let children = node.get("Children")?.as_array()?;
    path.push(node.get("Text").and_then(Value::as_str).unwrap_or_default());
    let found = children.iter().find_map(|c| find_sensor(c, id, path));
    path.pop();
    found
}

/// LHM formats values for display, e.g. "90,6 °C" (decimal separator follows the locale).
fn parse_value(sensor: &Value) -> Option<f64> {
    let text = sensor.get("Value")?.as_str()?;
    text.split_whitespace().next()?.replace(',', ".").parse().ok()
}

pub fn run(app: AppHandle) {
    let agent = http_agent();
    let mut last_error = String::new();
    let mut perf = super::perf::Perf::default();
    loop {
        if !widget_enabled(&app, "temps") {
            publish(&app, "temps", Value::Null);
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        let opts = widget_opts(&app, "temps");
        let port = opt_f64(&opts, "port").unwrap_or(8085.0) as u16;
        let url = opt_str(&opts, "url", &format!("http://localhost:{port}/data.json"));
        let interval = opt_f64(&opts, "interval_ms").unwrap_or(1000.0).clamp(250.0, 60_000.0);

        let use_lhm = opts.get("lhm").and_then(|v| v.as_bool()) != Some(false);
        let mut out = Map::new();
        let reply = if use_lhm { Some(get_json(&agent, &url)) } else { None };
        out.insert("lhm".into(), json!(use_lhm));
        out.insert("lhmOk".into(), json!(matches!(reply, Some(Ok(_)))));
        match reply {
            Some(Ok(root)) => {
                last_error.clear();
                for &(key, field, default) in SENSORS {
                    let found = find_sensor(&root, &opt_str(&opts, key, default), &mut Vec::new());
                    out.insert(field.into(), json!(found.as_ref().and_then(|(s, _)| parse_value(s))));
                    // Hardware names for the panel headings.
                    if let (Some((_, hw)), "cpu" | "gpu") = (&found, field) {
                        out.insert(format!("{field}Name"), json!(hw));
                    }
                }
            }
            Some(Err(e)) => {
                if e != last_error {
                    eprintln!("[kasumi] temps: {url}: {e}");
                    last_error = e;
                }
            }
            None => {}
        }
        // Windows' own readings for what LibreHardwareMonitor didn't give.
        perf.fill(&mut out, opts.get("top_apps").and_then(|v| v.as_bool()) != Some(false));
        out.insert("ok".into(), json!(true));
        publish(&app, "temps", Value::Object(out));
        std::thread::sleep(Duration::from_millis(interval as u64));
    }
}
