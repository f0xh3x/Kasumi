//! The settings window: a regular, focusable app window (unlike the overlays),
//! opened from the tray or the menu. Only one exists; opening again focuses it.

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "settings";

pub fn open(app: &AppHandle) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        return;
    }
    // Build off the calling thread: window creation waits on the event loop,
    // which may be the thread we're called from (tray menu handlers).
    let app = app.clone();
    std::thread::spawn(move || {
        // Frameless with our own title bar (settings.html); Windows still draws the
        // shadow, and rounded corners are requested below.
        let built = WebviewWindowBuilder::new(&app, LABEL, WebviewUrl::App("settings.html".into()))
            .title("StatusNotch Settings")
            .inner_size(940.0, 680.0)
            .min_inner_size(760.0, 500.0)
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
            Err(e) => eprintln!("[statusnotch] settings window: {e}"),
        }
    });
}

#[tauri::command]
pub async fn open_settings(app: AppHandle) {
    open(&app);
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
