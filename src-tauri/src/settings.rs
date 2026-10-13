//! The settings window: a regular, focusable app window (unlike the overlays),
//! opened from the tray or the menu. Only one exists; opening again focuses it.
//! The same page also makes the first-time setup and the update notes windows
//! (`settings.html?view=setup` / `?view=notes`), each one of its own.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

pub fn open(app: &AppHandle) {
    window(app, "settings", "settings.html", "Kasumi Settings", (940.0, 680.0), (760.0, 500.0));
}

/// The first-time setup (also from Settings > About).
pub fn open_setup(app: &AppHandle) {
    window(app, "setup", "settings.html?view=setup", "Kasumi setup", (820.0, 640.0), (700.0, 560.0));
}

/// What's new in `version`.
pub fn open_notes(app: &AppHandle, version: &str) {
    let url = format!("settings.html?view=notes&v={version}");
    window(app, "notes", &url, "What's new in Kasumi", (640.0, 620.0), (480.0, 400.0));
}

/// Opens (or focuses) one of the page's windows.
fn window(app: &AppHandle, label: &'static str, url: &str, title: &'static str, size: (f64, f64), min: (f64, f64)) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    // Build off the calling thread: window creation waits on the event loop,
    // which may be the thread we're called from (tray menu handlers).
    let app = app.clone();
    let url = url.to_string();
    std::thread::spawn(move || {
        // Frameless with our own title bar (settings.html); Windows still draws the
        // shadow, and rounded corners are requested below.
        let built = WebviewWindowBuilder::new(&app, label, WebviewUrl::App(url.into()))
            .title(title)
            .inner_size(size.0, size.1)
            .min_inner_size(min.0, min.1)
            .decorations(false)
            .shadow(true)
            // Tauri's file-drop handler blocks HTML drag-and-drop (the widget sorter).
            .disable_drag_drop_handler()
            .additional_browser_args(crate::win::BROWSER_ARGS)
            .theme(Some(tauri::Theme::Dark))
            .center()
            .build();
        match built {
            Ok(w) => {
                if let Ok(h) = w.hwnd() {
                    crate::win::round_corners(h.0 as isize);
                }
                let _ = w.set_focus();
            }
            Err(e) => eprintln!("[kasumi] {label} window: {e}"),
        }
    });
}

#[tauri::command]
pub async fn open_settings(app: AppHandle) {
    open(&app);
}

#[tauri::command]
pub async fn open_setup_window(app: AppHandle) {
    open_setup(&app);
}

/// The notes of this version (no `version`) or another one.
#[tauri::command]
pub async fn open_notes_window(app: AppHandle, version: Option<String>) {
    open_notes(&app, version.as_deref().unwrap_or(env!("CARGO_PKG_VERSION")));
}

/// At start: a first run (no config.toml before this start) opens the setup; a
/// new version since the last start shows what's new (`[general] update_notes`).
/// The version is remembered in `last-version` beside the config.
pub fn startup(app: &AppHandle, first_run: bool) {
    let marker = crate::config::dir().join("last-version");
    let current = env!("CARGO_PKG_VERSION");
    let last = std::fs::read_to_string(&marker).ok().map(|s| s.trim().to_string());
    let _ = std::fs::write(&marker, current);
    // Nothing remembered on an existing setup: updated from a version before this one.
    let updated = !first_run && last.as_deref() != Some(current);
    let notes = updated && app.state::<crate::providers::SharedConfig>().read().unwrap().general.update_notes;
    if !first_run && !notes {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        // Once the bars are up.
        std::thread::sleep(std::time::Duration::from_secs(3));
        if first_run {
            eprintln!("[kasumi] first run: opening the setup");
            open_setup(&app);
        } else {
            eprintln!("[kasumi] updated from {}: showing what's new", last.as_deref().unwrap_or("an older version"));
            open_notes(&app, current);
        }
    });
}

/// Installed font families, for the font picker.
#[tauri::command]
pub async fn fonts_list() -> Vec<String> {
    crate::win::font_families()
}

/// Detected monitors, for the per-monitor rules editor.
#[tauri::command]
pub async fn monitors_list() -> Vec<serde_json::Value> {
    crate::win::monitors()
        .iter()
        .enumerate()
        .map(|(i, m)| {
            serde_json::json!({
                "index": i + 1,
                "device": m.device,
                "name": if m.friendly.is_empty() { m.device.clone() } else { m.friendly.clone() },
                "primary": m.primary,
                "width": m.right - m.left,
                "height": m.bottom - m.top,
            })
        })
        .collect()
}

/// Every sensor LibreHardwareMonitor reports, for the sensor pickers.
#[tauri::command]
pub async fn lhm_sensors(url: String) -> Result<Vec<serde_json::Value>, String> {
    let agent = ureq::AgentBuilder::new().timeout(std::time::Duration::from_secs(3)).build();
    let body = agent.get(&url).call().map_err(|e| e.to_string())?.into_string().map_err(|e| e.to_string())?;
    let root: serde_json::Value = serde_json::from_str(&body).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    fn walk(n: &serde_json::Value, path: &mut Vec<String>, out: &mut Vec<serde_json::Value>) {
        if let Some(id) = n.get("SensorId").and_then(|v| v.as_str()) {
            out.push(serde_json::json!({
                "id": id,
                "name": n.get("Text"),
                "type": n.get("Type"),
                "hardware": path.len().checked_sub(2).and_then(|i| path.get(i)),
                "value": n.get("Value"),
            }));
        }
        if let Some(children) = n.get("Children").and_then(|c| c.as_array()) {
            path.push(n.get("Text").and_then(|t| t.as_str()).unwrap_or_default().to_string());
            for c in children {
                walk(c, path, out);
            }
            path.pop();
        }
    }
    walk(&root, &mut Vec::new(), &mut out);
    Ok(out)
}

/// The user's Windows short-time format as a strftime pattern (e.g. "%H:%M").
#[tauri::command]
pub async fn system_time_format() -> String {
    crate::win::short_time_pattern()
}

#[tauri::command]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}
