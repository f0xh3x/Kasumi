//! Data providers: background threads that fetch system/external data and push
//! it to every overlay. Each value lives under a key ("temps", "media",
//! "weather"); the latest one is kept so a freshly loaded page can catch up.

pub mod media;
pub mod systray;
mod temps;
mod weather;

use crate::config::Config;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, State};

/// The current config, shared with providers (the engine replaces it on reload).
pub type SharedConfig = Arc<RwLock<Config>>;

#[derive(Default)]
pub struct DataStore {
    values: Mutex<HashMap<String, Value>>,
}

/// Stores `value` under `key` and broadcasts it if it changed.
pub fn publish(app: &AppHandle, key: &str, value: Value) {
    {
        let store = app.state::<DataStore>();
        let mut values = store.values.lock().unwrap();
        if values.get(key) == Some(&value) {
            return;
        }
        values.insert(key.to_string(), value.clone());
    }
    let _ = app.emit("notch://data", json!({ "key": key, "value": value }));
}

#[tauri::command]
pub async fn get_data(store: State<'_, DataStore>) -> Result<HashMap<String, Value>, String> {
    Ok(store.values.lock().unwrap().clone())
}

/// Whether a widget is switched on (`[widgets.<name>] enabled`, default true).
fn widget_enabled(app: &AppHandle, name: &str) -> bool {
    app.state::<SharedConfig>().read().unwrap().widget_enabled(name)
}

/// `[widgets.<name>]` from the current config.
fn widget_opts(app: &AppHandle, name: &str) -> toml::Table {
    let cfg = app.state::<SharedConfig>();
    let cfg = cfg.read().unwrap();
    cfg.widgets.get(name).and_then(|v| v.as_table()).cloned().unwrap_or_default()
}

fn opt_str(t: &toml::Table, key: &str, default: &str) -> String {
    t.get(key).and_then(|v| v.as_str()).unwrap_or(default).to_string()
}

fn opt_f64(t: &toml::Table, key: &str) -> Option<f64> {
    t.get(key).and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new().timeout(Duration::from_secs(5)).build()
}

fn get_json(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    let body = agent.get(url).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
    serde_json::from_str(&body).map_err(|e| e.to_string())
}

pub fn start(app: &AppHandle, cfg: SharedConfig) {
    app.manage(DataStore::default());
    app.manage(cfg);
    let spawn = |name: &str, f: fn(AppHandle)| {
        let app = app.clone();
        std::thread::Builder::new()
            .name(format!("statusnotch-{name}"))
            .spawn(move || f(app))
            .expect("spawn provider thread");
    };
    spawn("temps", temps::run);
    spawn("weather", weather::run);
    media::start(app);
    systray::start(app);
}
