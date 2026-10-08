//! System status for the quick settings widget: battery, output volume, built-in
//! panel brightness and the network connection, published as "system" while a
//! quick settings widget is placed. Plus the commands its panel uses to change
//! volume and brightness.

use super::publish;
use serde::Deserialize;
use serde_json::{json, Value};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::GUID;
use windows::Networking::Connectivity::{NetworkConnectivityLevel, NetworkInformation};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use wmi::WMIConnection;

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new().name("statusnotch-system".into()).spawn(move || run(app)).expect("spawn system thread");
}

fn wanted(app: &AppHandle) -> bool {
    app.state::<super::SharedConfig>().read().unwrap().widget_placed("quick")
}

fn run(app: AppHandle) {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let mut brightness: Option<u8> = None;
    let mut last_brightness = Instant::now() - Duration::from_secs(60);
    loop {
        if !wanted(&app) {
            std::thread::sleep(Duration::from_secs(3));
            continue;
        }
        // WMI is slow; brightness only changes by hand, so read it less often.
        if last_brightness.elapsed() >= Duration::from_secs(5) {
            last_brightness = Instant::now();
            brightness = read_brightness();
        }
        let v = volume().and_then(|vol| unsafe { Ok((vol.GetMasterVolumeLevelScalar()?, vol.GetMute()?.as_bool())) }).ok();
        publish(
            &app,
            "system",
            json!({
                "battery": battery(),
                "volume": v.map(|(level, muted)| json!({ "level": level, "muted": muted })),
                "brightness": brightness,
                "network": network(),
            }),
        );
        std::thread::sleep(Duration::from_millis(1000));
    }
}

/// None on PCs without a battery.
fn battery() -> Value {
    let mut s = SYSTEM_POWER_STATUS::default();
    if unsafe { GetSystemPowerStatus(&mut s) }.is_err() || s.BatteryFlag == 128 || s.BatteryFlag == 255 {
        return Value::Null;
    }
    json!({
        "percent": if s.BatteryLifePercent <= 100 { Some(s.BatteryLifePercent) } else { None },
        "plugged": s.ACLineStatus == 1,
        "charging": s.BatteryFlag & 8 != 0,
        "secondsLeft": if s.BatteryLifeTime == u32::MAX { None } else { Some(s.BatteryLifeTime) },
        "saver": s.SystemStatusFlag == 1,
    })
}

fn network() -> Value {
    let Ok(profile) = NetworkInformation::GetInternetConnectionProfile() else {
        return json!({ "connected": false });
    };
    let level = profile.GetNetworkConnectivityLevel().unwrap_or(NetworkConnectivityLevel::None);
    json!({
        "connected": level != NetworkConnectivityLevel::None,
        "internet": level == NetworkConnectivityLevel::InternetAccess,
        "name": profile.ProfileName().map(|n| n.to_string()).unwrap_or_default(),
        "wifi": profile.IsWlanConnectionProfile().unwrap_or(false),
    })
}

fn volume() -> windows::core::Result<IAudioEndpointVolume> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?.Activate(CLSCTX_ALL, None)
    }
}

#[derive(Deserialize)]
#[serde(rename = "WmiMonitorBrightness")]
#[allow(non_snake_case)]
struct Brightness {
    CurrentBrightness: u8,
}

/// The built-in panel's brightness, if it can be controlled (laptops).
fn read_brightness() -> Option<u8> {
    let con = WMIConnection::with_namespace_path("ROOT\\WMI").ok()?;
    let rows: Vec<Brightness> = con.query().ok()?;
    rows.first().map(|b| b.CurrentBrightness)
}

/// Sets the output volume (0..=1) from the quick settings slider.
#[tauri::command]
pub async fn volume_set(level: f32) -> Result<(), String> {
    crate::osd::quiet_volume(Duration::from_millis(600));
    let vol = volume().map_err(|e| e.to_string())?;
    let ctx: *const GUID = std::ptr::null();
    unsafe {
        if vol.GetMute().map(|m| m.as_bool()).unwrap_or(false) {
            let _ = vol.SetMute(false, ctx);
        }
        vol.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), ctx).map_err(|e| e.to_string())
    }
}

#[tauri::command]
pub async fn volume_toggle_mute() -> Result<(), String> {
    let vol = volume().map_err(|e| e.to_string())?;
    unsafe {
        let muted = vol.GetMute().map_err(|e| e.to_string())?.as_bool();
        vol.SetMute(!muted, std::ptr::null()).map_err(|e| e.to_string())
    }
}

#[derive(Deserialize)]
#[serde(rename = "WmiMonitorBrightnessMethods")]
struct BrightnessMethods {
    #[serde(rename = "__Path")]
    path: String,
}

/// Sets the built-in panel's brightness (0..=100) through WMI.
#[tauri::command]
pub async fn brightness_set(level: u8) -> Result<(), String> {
    let con = WMIConnection::with_namespace_path("ROOT\\WMI").map_err(|e| e.to_string())?;
    let target: Vec<BrightnessMethods> = con.query().map_err(|e| e.to_string())?;
    let target = target.first().ok_or("no controllable display")?;
    let class = con.get_object("WmiMonitorBrightnessMethods").map_err(|e| e.to_string())?;
    let method = class.get_method("WmiSetBrightness").map_err(|e| e.to_string())?.ok_or("no WmiSetBrightness")?;
    let params = method.spawn_instance().map_err(|e| e.to_string())?;
    params.put_property("Timeout", 0u32).map_err(|e| e.to_string())?;
    params.put_property("Brightness", level.min(100)).map_err(|e| e.to_string())?;
    con.exec_method(&target.path, "WmiSetBrightness", Some(&params)).map_err(|e| e.to_string())?;
    Ok(())
}
