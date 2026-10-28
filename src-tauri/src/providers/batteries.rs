//! Batteries: the laptop's (system.rs) and each Bluetooth device's (mice, keyboards,
//! headphones) as Windows' Settings shows them: the level Windows keeps as the
//! device's Bluetooth battery property. Published as "batteries" while the
//! Batteries widget is on a bar or quick settings lists the devices.

use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tauri::AppHandle;
use windows::core::{w, GUID};
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW, SetupDiGetDeviceInstanceIdW, SetupDiGetDevicePropertyW, DIGCF_ALLCLASSES,
    DIGCF_PRESENT, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Properties::{DEVPKEY_Device_FriendlyName, DEVPKEY_NAME, DEVPROPTYPE};
use windows::Win32::Foundation::DEVPROPKEY;

/// DEVPKEY_Bluetooth_Battery: the level (a byte, 0..100).
const BATTERY: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x104ea319_6ee2_4701_bd47_8ddbf425bbe5), pid: 2 };
/// DEVPKEY_Device_IsConnected: whether it's connected now (a boolean, 0 or 0xFF).
const CONNECTED: DEVPROPKEY = DEVPROPKEY { fmtid: GUID::from_u128(0x83da6326_97a6_4088_9453_a1923f573b29), pid: 15 };

/// Connected Bluetooth devices that report a battery: (device id, name, percent), by
/// name.
pub fn devices() -> Vec<(String, String, u8)> {
    let mut out = Vec::new();
    // Bluetooth LE devices (mice, keyboards) and classic ones (headsets' hands-free).
    for enumerator in [w!("BTHLE"), w!("BTHENUM")] {
        unsafe {
            let Ok(set) = SetupDiGetClassDevsW(None, enumerator, None, DIGCF_PRESENT | DIGCF_ALLCLASSES) else { continue };
            let mut i = 0;
            loop {
                let mut data = SP_DEVINFO_DATA { cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32, ..Default::default() };
                if SetupDiEnumDeviceInfo(set, i, &mut data).is_err() {
                    break;
                }
                i += 1;
                let mut kind = DEVPROPTYPE::default();
                let mut level = [0u8; 4];
                if SetupDiGetDevicePropertyW(set, &data, &BATTERY, &mut kind, Some(&mut level), None, 0).is_err() || level[0] > 100 {
                    continue;
                }
                // Paired but not connected (another PC's, switched off): its level is old.
                let mut on = [0xFFu8; 1];
                if SetupDiGetDevicePropertyW(set, &data, &CONNECTED, &mut kind, Some(&mut on), None, 0).is_ok() && on[0] == 0 {
                    continue;
                }
                let text = |key: &DEVPROPKEY| {
                    let mut buf = [0u8; 512];
                    let mut kind = DEVPROPTYPE::default();
                    SetupDiGetDevicePropertyW(set, &data, key, &mut kind, Some(&mut buf), None, 0).ok()?;
                    let wide: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
                    Some(String::from_utf16_lossy(&wide)).filter(|s| !s.is_empty())
                };
                let name = text(&DEVPKEY_Device_FriendlyName).or_else(|| text(&DEVPKEY_NAME)).unwrap_or_else(|| "Bluetooth device".into());
                let mut id = [0u16; 400];
                let _ = SetupDiGetDeviceInstanceIdW(set, &data, Some(&mut id), None);
                let id = String::from_utf16_lossy(&id[..id.iter().position(|&c| c == 0).unwrap_or(0)]);
                out.push((id, name, level[0]));
            }
            let _ = SetupDiDestroyDeviceInfoList(set);
        }
    }
    // A headset can report on several of its parts: once per name and level.
    out.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()).then(a.0.cmp(&b.0)));
    out.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);
    out
}

fn wanted(app: &AppHandle) -> bool {
    super::placed(app, "batteries") || super::widget_opts(app, "quick").get("devices").and_then(|v| v.as_bool()) == Some(true)
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-batteries".into())
        .spawn(move || {
            // Devices every minute (their levels move slowly), the laptop every 5 s.
            let mut list: Vec<(String, String, u8)> = Vec::new();
            let mut read: Option<Instant> = None;
            loop {
                if !wanted(&app) {
                    read = None;
                    std::thread::sleep(Duration::from_secs(3));
                    continue;
                }
                if read.is_none_or(|t| t.elapsed() >= Duration::from_secs(60)) {
                    list = devices();
                    read = Some(Instant::now());
                }
                let devices: Vec<Value> = list.iter().map(|(id, name, p)| json!({ "id": id, "name": name, "percent": p })).collect();
                super::publish(&app, "batteries", json!({ "laptop": super::system::battery(), "devices": devices }));
                std::thread::sleep(Duration::from_secs(5));
            }
        })
        .expect("spawn batteries thread");
}

#[cfg(test)]
mod tests {
    /// This PC's Bluetooth batteries: `cargo test batteries_read -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn batteries_read() {
        let t = std::time::Instant::now();
        for d in super::devices() {
            println!("{d:?}");
        }
        println!("{} ms", t.elapsed().as_millis());
    }
}
