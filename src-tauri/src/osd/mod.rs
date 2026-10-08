//! On-screen display for volume and brightness changes. The frontend grows the
//! OSD notch into a pill for a moment; the overlay stays click-through.

mod brightness;
mod volume;

use crate::providers::SharedConfig;
use serde_json::json;
use tauri::{AppHandle, Emitter, EventTarget, Manager};

/// Shows the OSD on the overlay under the cursor. `level` is 0..=1.
pub fn show(app: &AppHandle, kind: &str, level: f32, muted: bool) {
    if !app.state::<SharedConfig>().read().unwrap().osd.enabled {
        return;
    }
    let Some(label) = crate::engine::osd_target(app) else {
        eprintln!("[statusnotch] osd {kind}: no visible overlay under the cursor");
        return;
    };
    eprintln!("[statusnotch] osd {kind} {:.0}%{} -> {label}", level * 100.0, if muted { " muted" } else { "" });
    let _ = app.emit_to(
        EventTarget::WebviewWindow { label },
        "notch://osd",
        json!({ "kind": kind, "level": level.clamp(0.0, 1.0), "muted": muted }),
    );
}

/// Watches the foreground window's keyboard layout. Publishes it as data
/// ("keyboard") for the language widget, and pops the keyboard peek when the
/// layout changes while the same window stays in front (so merely switching to
/// an app with a different per-app layout doesn't trigger it).
#[derive(Default)]
pub struct KeyboardWatch {
    last: Option<(isize, u16)>,
}

impl KeyboardWatch {
    pub fn tick(&mut self, app: &AppHandle) {
        let Some((fg, lang)) = crate::win::foreground_layout() else { return };
        let prev = self.last.replace((fg, lang));
        if prev.is_some_and(|(_, l)| l == lang) {
            return;
        }
        let (code, name) = crate::win::language_names(lang);
        crate::providers::publish(app, "keyboard", json!({ "code": code, "name": name }));
        let same_window = prev.is_some_and(|(w, _)| w == fg);
        let enabled = {
            let cfg = app.state::<SharedConfig>();
            let cfg = cfg.read().unwrap();
            cfg.osd.enabled && cfg.osd.keyboard
        };
        if same_window && enabled {
            // The peek pops at the language widget, on whichever surface shows it.
            if let Some(label) = crate::engine::overlay_for(app, "language") {
                let _ = app.emit_to(EventTarget::WebviewWindow { label }, "notch://keyboard", json!({ "code": code, "name": name }));
            }
        }
    }
}

/// Language widget click: cycle the foreground window to the next layout.
#[tauri::command]
pub async fn switch_keyboard_layout() -> Result<(), String> {
    crate::win::next_keyboard_layout()
}

pub fn start(app: &AppHandle) {
    volume::start(app);
    brightness::start(app);
}
