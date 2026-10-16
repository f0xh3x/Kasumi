//! The seam between the backend and whatever draws the bars: WebView2 pages
//! (`[general] renderer = "web"`) or the native renderer (`"native"`, native/).
//! Everything the engine, providers and pop-ups send a bar goes through here.

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, EventTarget, Manager};

use crate::engine::{Layout, NotchLayout};
use crate::native::{self, Msg};
use crate::win::Monitor;

/// Whether the native renderer draws the bars.
pub fn native() -> bool {
    native::active()
}

fn emit<S: Serialize + Clone>(app: &AppHandle, label: &str, event: &str, payload: S) {
    let _ = app.emit_to(EventTarget::WebviewWindow { label: label.to_string() }, event, payload);
}

/// A bar's whole layout (`notch://layout`).
pub fn layout(app: &AppHandle, label: &str, layout: &Layout) {
    if native() {
        native::post(Msg::Layout(label.to_string(), Box::new(layout.clone())));
    } else {
        emit(app, label, "notch://layout", layout.clone());
    }
}

/// The bar opened or closed (`notch://state`).
pub fn state(app: &AppHandle, label: &str, expanded: bool) {
    if native() {
        native::post(Msg::State(label.to_string(), expanded));
    } else {
        emit(app, label, "notch://state", serde_json::json!({ "expanded": expanded }));
    }
}

/// Notches moved (`notch://positions`).
pub fn positions(app: &AppHandle, label: &str, notches: Vec<NotchLayout>) {
    if native() {
        native::post(Msg::Positions(label.to_string(), notches));
    } else {
        emit(app, label, "notch://positions", notches);
    }
}

/// The notch the cursor is coming near (`notch://nudge`).
pub fn nudge(app: &AppHandle, label: &str, notch: Option<String>) {
    if native() {
        native::post(Msg::Nudge(label.to_string(), notch));
    } else {
        emit(app, label, "notch://nudge", serde_json::json!({ "notch": notch }));
    }
}

/// Anything else for one bar: the launcher, pop-ups, dropped files.
pub fn event(app: &AppHandle, label: &str, event: &str, payload: Value) {
    if native() {
        native::post(Msg::Event(label.to_string(), event.to_string(), payload));
    } else {
        emit(app, label, event, payload);
    }
}

/// A data value (`notch://data`) for some bars (None: all of them). The native
/// renderer keeps every value, so it gets each once.
pub fn data(app: &AppHandle, labels: Option<Vec<String>>, key: &str, value: &Value) {
    if native() {
        native::post(Msg::Data(key.to_string(), value.clone()));
        return;
    }
    let payload = serde_json::json!({ "key": key, "value": value });
    match labels {
        None => {
            let _ = app.emit("notch://data", payload);
        }
        Some(labels) => {
            for label in labels {
                emit(app, &label, "notch://data", payload.clone());
            }
        }
    }
}

/// The changed entries of an object value (publish_entries; the pages merge them).
pub fn data_entries(app: &AppHandle, labels: Option<Vec<String>>, key: &str, entries: &serde_json::Map<String, Value>) {
    if native() {
        native::post(Msg::Entries(key.to_string(), entries.clone()));
    } else {
        data(app, labels, key, &Value::Object(entries.clone()));
    }
}

/// Creates a bar's native window; its handle. (Web bars are built by the engine.)
pub fn create_native(label: &str, monitor: &Monitor, layout: &Layout) -> Option<isize> {
    let h = native::create(label, monitor, layout);
    (h != 0).then_some(h)
}

pub fn destroy(app: &AppHandle, label: &str) {
    if native() {
        native::post(Msg::Destroy(label.to_string()));
    } else if let Some(w) = app.get_webview_window(label) {
        let _ = w.destroy();
    }
}

/// Shows or hides a bar (fullscreen apps), raising it when shown.
pub fn set_shown(app: &AppHandle, label: &str, hwnd: Option<isize>, shown: bool) {
    if native() {
        native::post(Msg::Shown(label.to_string(), shown));
        if let (true, Some(h)) = (shown, hwnd) {
            crate::win::raise_topmost(h);
        }
        return;
    }
    let app2 = app.clone();
    let label = label.to_string();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(&label) {
            let _ = if shown { w.show() } else { w.hide() };
        }
        if let (true, Some(h)) = (shown, hwnd) {
            crate::win::raise_topmost(h);
        }
    });
}

/// Lets clicks through a bar (closed) or not (open, over its shape).
pub fn click_through(app: &AppHandle, label: &str, hwnd: isize, ignore: bool) {
    if native() {
        native::post(Msg::ClickThrough(label.to_string(), ignore));
        return;
    }
    let app2 = app.clone();
    let label = label.to_string();
    let _ = app.run_on_main_thread(move || {
        if let Some(w) = app2.get_webview_window(&label) {
            let _ = w.set_ignore_cursor_events(ignore);
            // tao rewrites the extended style when toggling; restore ours.
            crate::win::apply_overlay_style(hwnd);
        }
    });
}

/// A closed web bar keeps WebView2's memory low (webmem.rs); native bars have none to trim.
pub fn low_memory(app: &AppHandle, label: &str, low: bool) {
    if native() {
        return;
    }
    if let Some(w) = app.get_webview_window(label) {
        crate::webmem::set_low(&w, low);
    }
}

/// The bar's monitor or window height changed. Native bars size their window to
/// what shows each frame, so they only need the monitor.
pub fn place(app: &AppHandle, label: &str, hwnd: isize, m: Monitor, top: i32, height_px: i32) {
    if native() {
        native::post(Msg::Place(label.to_string(), m));
        return;
    }
    let _ = app.run_on_main_thread(move || crate::win::place(hwnd, m.left, top, m.width(), height_px));
}
