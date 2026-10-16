//! Data providers: background threads that fetch system/external data and push
//! it to every overlay. Each value lives under a key ("temps", "media",
//! "weather"); the latest one is kept so a freshly loaded page can catch up.

pub mod audio;
pub mod claude;
pub mod clipboard;
pub mod extras;
pub mod komorebi;
pub mod media;
pub mod notifications;
pub mod systray;
pub mod system;
pub mod tasks;
mod temps;
mod weather;

use crate::config::Config;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, EventTarget, Manager, State};

/// The current config, shared with providers (the engine replaces it on reload).
pub type SharedConfig = Arc<RwLock<Config>>;

#[derive(Default)]
pub struct DataStore {
    values: Mutex<HashMap<String, Value>>,
}

/// Widget types whose pages read a data key. A key nobody lists here goes to
/// every overlay (the safe default for a new provider).
fn consumers(key: &str) -> Option<&'static [&'static str]> {
    Some(match key {
        "system" => &["quick"],
        "tasks" | "taskIcons" => &["tasks"],
        "temps" => &["temps"],
        "media" => &["media"],
        "weather" => &["weather"],
        "tray" => &["tray"],
        "keyboard" => &["language"],
        "focus" => &["focus"],
        "netspeed" => &["netspeed"],
        "disks" => &["disk"],
        "privacy" => &["privacy"],
        "claude" => &["claude"],
        "notifications" => &["notifications"],
        "clipboard" => &["clipboard"],
        "workspaces" => &["workspaces"],
        _ => return None,
    })
}

/// Sends one value to the overlays that show a widget using `key` (all of them
/// for a key without a listed consumer): a page that has no such widget would
/// only wake up to throw it away.
fn emit_data(app: &AppHandle, key: &str, value: &Value) {
    let labels = (!crate::ui::native()).then(|| consumers(key).map(|types| crate::engine::overlays_showing(app, types))).flatten();
    crate::ui::data(app, labels, key, value);
}

/// Stores `value` under `key` and sends it to the pages that use it, if it changed.
pub fn publish(app: &AppHandle, key: &str, value: Value) {
    {
        let store = app.state::<DataStore>();
        let mut values = store.values.lock().unwrap();
        if values.get(key) == Some(&value) {
            return;
        }
        values.insert(key.to_string(), value.clone());
    }
    emit_data(app, key, &value);
}

/// For a key holding an object that grows by entry (icons by app): the store
/// keeps the whole object, the pages are sent only the entries that changed and
/// merge them (`MERGED` in data.ts).
pub fn publish_entries(app: &AppHandle, key: &str, entries: serde_json::Map<String, Value>) {
    let changed = {
        let store = app.state::<DataStore>();
        let mut values = store.values.lock().unwrap();
        let held = values.entry(key.to_string()).or_insert_with(|| Value::Object(Default::default()));
        let Some(map) = held.as_object_mut() else { return };
        let mut changed = serde_json::Map::new();
        for (k, v) in entries {
            if map.get(&k) != Some(&v) {
                map.insert(k.clone(), v.clone());
                changed.insert(k, v);
            }
        }
        changed
    };
    if !changed.is_empty() {
        let labels = (!crate::ui::native()).then(|| consumers(key).map(|types| crate::engine::overlays_showing(app, types))).flatten();
        crate::ui::data_entries(app, labels, key, &changed);
    }
}

/// Sends one overlay the current values it uses. For after its layout changed:
/// a widget it didn't show before may have arrived, and what was published
/// meanwhile went only to the pages that had one.
pub fn resend(app: &AppHandle, label: &str) {
    // The native renderer keeps every value it was sent.
    if crate::ui::native() {
        return;
    }
    let keys: Vec<String> = app.state::<DataStore>().values.lock().unwrap().keys().cloned().collect();
    for key in keys {
        let Some(types) = consumers(&key) else { continue };
        if !crate::engine::overlay_shows(app, label, types) {
            continue;
        }
        // Read right before sending, so the value is as new as it can be (a
        // publish landing in the microseconds between this read and the send
        // could still be overtaken; the next change corrects it).
        let Some(value) = app.state::<DataStore>().values.lock().unwrap().get(&key).cloned() else { continue };
        let payload = json!({ "key": key, "value": value });
        let _ = app.emit_to(EventTarget::WebviewWindow { label: label.to_string() }, "notch://data", payload);
    }
}

#[tauri::command]
pub async fn get_data(store: State<'_, DataStore>) -> Result<HashMap<String, Value>, String> {
    Ok(store.values.lock().unwrap().clone())
}

/// Whether a widget is switched on (`[widgets.<name>] enabled`, default true).
fn widget_enabled(app: &AppHandle, name: &str) -> bool {
    app.state::<SharedConfig>().read().unwrap().widget_enabled(name)
}

/// Whether a widget of this type is on some bar (its provider only works then).
pub fn placed(app: &AppHandle, ty: &str) -> bool {
    app.state::<SharedConfig>().read().unwrap().widget_placed(ty)
}

/// An icon (width, height, straight RGBA) as the frontend takes it: `iconUrl()`.
pub fn rgba_json(w: u32, h: u32, rgba: &[u8]) -> Value {
    use base64::Engine;
    json!({ "w": w, "h": h, "rgba": base64::engine::general_purpose::STANDARD.encode(rgba) })
}

/// File bytes as a data URL the frontend can show directly (album art, pictures).
pub fn data_url(mime: &str, bytes: &[u8]) -> String {
    use base64::Engine;
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
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
            .name(format!("kasumi-{name}"))
            .spawn(move || f(app))
            .expect("spawn provider thread");
    };
    spawn("temps", temps::run);
    spawn("weather", weather::run);
    media::start(app);
    systray::start(app);
    tasks::start(app);
    system::start(app);
    extras::start(app);
    claude::start(app);
    clipboard::start(app);
    notifications::start(app);
    komorebi::start(app);
}
