//! Brightness: built-in panel changes (laptop Fn keys) arrive as WMI
//! `WmiMonitorBrightnessEvent`s. External monitors are left alone.

use crate::providers::SharedConfig;
use serde::Deserialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};
use wmi::WMIConnection;

#[derive(Deserialize)]
#[serde(rename = "WmiMonitorBrightnessEvent")]
#[allow(non_snake_case)]
struct BrightnessEvent {
    Brightness: u8,
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new().name("statusnotch-brightness".into()).spawn(move || run(app)).expect("spawn brightness");
}

fn listen(app: &AppHandle) -> wmi::WMIResult<()> {
    let con = WMIConnection::with_namespace_path("ROOT\\WMI")?;
    for event in con.notification::<BrightnessEvent>()? {
        let event = event?;
        if app.state::<SharedConfig>().read().unwrap().osd.brightness {
            super::show(app, "brightness", event.Brightness as f32 / 100.0, false);
        }
    }
    Ok(())
}

fn run(app: AppHandle) {
    loop {
        if let Err(e) = listen(&app) {
            eprintln!("[statusnotch] brightness events unavailable: {e}");
        }
        // The subscription ended or failed (e.g. no WMI brightness provider); retry later.
        std::thread::sleep(Duration::from_secs(60));
    }
}
