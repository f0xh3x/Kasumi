//! Quick settings' switches ([widgets.quick] switches, the Dark mode tile): the Wi-Fi
//! and Bluetooth radios turned on and off as Windows' own quick settings do
//! (Windows.Devices.Radios), and Windows' light / dark mode (its Personalize
//! registry values, then the change announced so apps follow).

use serde_json::{json, Value};
use windows::Devices::Radios::{Radio, RadioKind, RadioState};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
use windows::Win32::System::Registry::HKEY_CURRENT_USER;

const PERSONALIZE: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

fn radios() -> Vec<Radio> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    Radio::GetRadiosAsync().and_then(|op| op.get()).map(|v| v.into_iter().collect()).unwrap_or_default()
}

fn kind_of(name: &str) -> Option<RadioKind> {
    match name {
        "wifi" => Some(RadioKind::WiFi),
        "bluetooth" => Some(RadioKind::Bluetooth),
        _ => None,
    }
}

/// Whether each is on: `{"wifi": bool|null, "bluetooth": bool|null, "dark": bool}`
/// (null: this PC has no such radio).
pub fn state() -> Value {
    let all = radios();
    let on = |kind: RadioKind| all.iter().find(|r| r.Kind().ok() == Some(kind)).map(|r| r.State().ok() == Some(RadioState::On));
    let light = crate::win::reg_dword(HKEY_CURRENT_USER, PERSONALIZE, "AppsUseLightTheme").unwrap_or(1);
    json!({ "wifi": on(RadioKind::WiFi), "bluetooth": on(RadioKind::Bluetooth), "dark": light == 0 })
}

/// Turns a radio ("wifi", "bluetooth") on or off.
pub fn set_radio(name: &str, on: bool) -> Result<(), String> {
    let kind = kind_of(name).ok_or("unknown radio")?;
    let radio = radios().into_iter().find(|r| r.Kind().ok() == Some(kind)).ok_or("no such radio")?;
    let state = if on { RadioState::On } else { RadioState::Off };
    radio.SetStateAsync(state).and_then(|op| op.get()).map(|_| ()).map_err(|e| e.to_string())
}

/// Windows' apps and its own surfaces dark (or light), as Settings › Colors sets them.
pub fn set_dark(dark: bool) -> Result<(), String> {
    let value = if dark { 0u32 } else { 1u32 };
    for name in ["AppsUseLightTheme", "SystemUsesLightTheme"] {
        crate::win::reg_set_dword(PERSONALIZE, name, value);
    }
    crate::win::broadcast_setting("ImmersiveColorSet");
    Ok(())
}

#[cfg(test)]
mod tests {
    /// This PC's radios and mode, read only: `cargo test switches_read -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn switches_read() {
        println!("{}", super::state());
    }
}
