//! The Security (privacy) widget's switches: every microphone muted (Core Audio, each
//! active capture device), and the camera and location allowed or not for apps (the
//! per-user privacy settings Windows' Settings › Privacy writes: ConsentStore "Value",
//! for Store apps and for desktop apps).

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eCapture, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};

const CONSENT: &str = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

/// Every active microphone's volume control.
fn microphones() -> Vec<IAudioEndpointVolume> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let Ok(e) = CoCreateInstance::<_, IMMDeviceEnumerator>(&MMDeviceEnumerator, None, CLSCTX_ALL) else { return Vec::new() };
        let Ok(list) = e.EnumAudioEndpoints(eCapture, DEVICE_STATE_ACTIVE) else { return Vec::new() };
        let n = list.GetCount().unwrap_or(0);
        (0..n).filter_map(|i| list.Item(i).ok()?.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok()).collect()
    }
}

/// All microphones muted (and there is one).
pub fn mic_muted() -> bool {
    let mics = microphones();
    !mics.is_empty() && mics.iter().all(|m| unsafe { m.GetMute() }.is_ok_and(|b| b.as_bool()))
}

/// A capability ("webcam", "location") denied to apps.
pub fn denied(cap: &str) -> bool {
    crate::win::reg_string(windows::Win32::System::Registry::HKEY_CURRENT_USER, &format!(r"{CONSENT}\{cap}"), "Value").is_some_and(|v| v.eq_ignore_ascii_case("Deny"))
}

fn set_string(path: &str, name: &str, value: &str) {
    use windows::Win32::System::Registry::{RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};
    let (path, name) = (crate::win::wide(path), crate::win::wide(name));
    let data = crate::win::wide(value);
    unsafe {
        let _ = RegSetKeyValueW(HKEY_CURRENT_USER, PCWSTR(path.as_ptr()), PCWSTR(name.as_ptr()), REG_SZ.0, Some(data.as_ptr() as *const _), (data.len() * 2) as u32);
    }
}

/// Turns one on (allowed / unmuted) or off: "mic", "camera" or "location".
pub fn set(kind: &str, on: bool) {
    match kind {
        "mic" => {
            for m in microphones() {
                let _ = unsafe { m.SetMute(!on, std::ptr::null()) };
            }
        }
        "camera" | "location" => {
            let cap = if kind == "camera" { "webcam" } else { "location" };
            let value = if on { "Allow" } else { "Deny" };
            set_string(&format!(r"{CONSENT}\{cap}"), "Value", value);
            set_string(&format!(r"{CONSENT}\{cap}\NonPackaged"), "Value", value);
        }
        _ => {}
    }
    eprintln!("[kasumi] security: {kind} {}", if on { "on" } else { "off" });
}
