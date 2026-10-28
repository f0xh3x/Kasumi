//! The seam between the backend and the native renderer (native/): everything the
//! engine, providers and pop-ups send a bar goes through here.

use serde_json::Value;

use crate::engine::{Layout, NotchLayout};
use crate::native::{self, Msg};
use crate::win::Monitor;

/// A bar's whole layout.
pub fn layout(label: &str, layout: &Layout) {
    native::post(Msg::Layout(label.to_string(), Box::new(layout.clone())));
}

/// The bar opened or closed.
pub fn state(label: &str, expanded: bool) {
    native::post(Msg::State(label.to_string(), expanded));
}

/// Notches moved.
pub fn positions(label: &str, notches: Vec<NotchLayout>) {
    native::post(Msg::Positions(label.to_string(), notches));
}

/// The notch the cursor is coming near.
pub fn nudge(label: &str, notch: Option<String>) {
    native::post(Msg::Nudge(label.to_string(), notch));
}

/// Anything else for one bar: the launcher, pop-ups, dropped files.
pub fn event(label: &str, event: &str, payload: Value) {
    native::post(Msg::Event(label.to_string(), event.to_string(), payload));
}

/// A data value, for every bar (the native renderer keeps each value it's sent).
pub fn data(key: &str, value: &Value) {
    native::post(Msg::Data(key.to_string(), value.clone()));
}

/// The changed entries of an object value (publish_entries).
pub fn data_entries(key: &str, entries: &serde_json::Map<String, Value>) {
    native::post(Msg::Entries(key.to_string(), entries.clone()));
}

/// Creates a bar's window; its handle.
pub fn create(label: &str, monitor: &Monitor, layout: &Layout) -> Option<isize> {
    let h = native::create(label, monitor, layout);
    (h != 0).then_some(h)
}

pub fn destroy(label: &str) {
    native::post(Msg::Destroy(label.to_string()));
}

/// Shows or hides a bar (fullscreen apps), raising it when shown.
pub fn set_shown(label: &str, hwnd: Option<isize>, shown: bool) {
    native::post(Msg::Shown(label.to_string(), shown));
    if let (true, Some(h)) = (shown, hwnd) {
        crate::win::raise_topmost(h);
    }
}

/// Lets clicks through a bar (closed) or not (open, over its shape).
pub fn click_through(label: &str, ignore: bool) {
    native::post(Msg::ClickThrough(label.to_string(), ignore));
}

/// The bar's monitor changed. Bars size their window to what shows each frame, so
/// they only need the monitor.
pub fn place(label: &str, m: Monitor) {
    native::post(Msg::Place(label.to_string(), m));
}
