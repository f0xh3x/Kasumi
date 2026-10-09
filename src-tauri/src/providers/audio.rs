//! Audio for the quick settings panel: output devices (and switching the default
//! one), and per-app volume. Asked for by the panel while it's open (nothing polls
//! in the background).

use serde_json::{json, Value};
use std::ffi::c_void;
use windows::core::{Interface, GUID, HRESULT, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eCommunications, eConsole, eMultimedia, eRender, AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2,
    IMMDeviceEnumerator, ISimpleAudioVolume, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CLSCTX_ALL, COINIT_MULTITHREADED, STGM_READ};

fn enumerator() -> windows::core::Result<IMMDeviceEnumerator> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
    }
}

/// The active output devices: id, name, and which one is the default.
#[tauri::command]
pub async fn audio_outputs() -> Result<Value, String> {
    let e = enumerator().map_err(|e| e.to_string())?;
    unsafe {
        let default_id = e
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .and_then(|d| d.GetId())
            .map(|p| {
                let s = p.to_string().unwrap_or_default();
                CoTaskMemFree(Some(p.0 as *const c_void));
                s
            })
            .unwrap_or_default();
        let list = e.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        for i in 0..list.GetCount().unwrap_or(0) {
            let Ok(d) = list.Item(i) else { continue };
            let Ok(idp) = d.GetId() else { continue };
            let id = idp.to_string().unwrap_or_default();
            CoTaskMemFree(Some(idp.0 as *const c_void));
            let name = d
                .OpenPropertyStore(STGM_READ)
                .and_then(|s| s.GetValue(&PKEY_Device_FriendlyName))
                .map(|v| v.to_string())
                .unwrap_or_default();
            out.push(json!({ "id": id, "name": name, "default": id == default_id }));
        }
        Ok(json!(out))
    }
}

/// Makes `id` the default output for everything (as the Sound settings do), through
/// Windows' IPolicyConfig: undocumented, but what Windows itself and tools like
/// SoundSwitch / EarTrumpet use. Not in windows-rs, so called through its vtable.
#[tauri::command]
pub async fn audio_set_output(id: String) -> Result<(), String> {
    const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);
    const IID_POLICY_CONFIG: GUID = GUID::from_u128(0xf8679f50_850a_41cf_9c72_430f290290c8);
    /// IUnknown (3) + 10 methods before SetDefaultEndpoint(PCWSTR, ERole).
    const SET_DEFAULT_ENDPOINT: usize = 13;
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let unk: windows::core::IUnknown = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL).map_err(|e| e.to_string())?;
        let mut p: *mut c_void = std::ptr::null_mut();
        unk.query(&IID_POLICY_CONFIG, &mut p).ok().map_err(|e| e.to_string())?;
        let vtbl = *(p as *const *const usize);
        type SetDefault = unsafe extern "system" fn(*mut c_void, PCWSTR, i32) -> HRESULT;
        let set: SetDefault = std::mem::transmute(*vtbl.add(SET_DEFAULT_ENDPOINT));
        type Release = unsafe extern "system" fn(*mut c_void) -> u32;
        let release: Release = std::mem::transmute(*vtbl.add(2));
        let wide: Vec<u16> = id.encode_utf16().chain([0]).collect();
        let mut result = Ok(());
        for role in [eConsole, eMultimedia, eCommunications] {
            if let Err(e) = set(p, PCWSTR(wide.as_ptr()), role.0).ok() {
                result = Err(e.to_string());
            }
        }
        release(p);
        result
    }
}

/// Apps playing (or able to play) sound on the default output, one per process.
#[tauri::command]
pub async fn audio_sessions() -> Result<Value, String> {
    let mut out: Vec<Value> = Vec::new();
    for_sessions(|pid, vol| {
        if out.iter().any(|v| v["pid"] == pid) {
            return;
        }
        let path = crate::win::process_path(pid).unwrap_or_default();
        let name = std::path::Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| format!("App {pid}"));
        let icon = app_icon(&path);
        let level = unsafe { vol.GetMasterVolume() }.unwrap_or(1.0);
        let muted = unsafe { vol.GetMute() }.map(|m| m.as_bool()).unwrap_or(false);
        out.push(json!({ "pid": pid, "name": name, "icon": icon, "level": (level as f64 * 100.0).round() / 100.0, "muted": muted }));
    })?;
    out.sort_by(|a, b| a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase()));
    Ok(json!(out))
}

#[tauri::command]
pub async fn audio_session_set(pid: u32, level: Option<f32>, muted: Option<bool>) -> Result<(), String> {
    for_sessions(|p, vol| {
        if p != pid {
            return;
        }
        unsafe {
            if let Some(l) = level {
                let _ = vol.SetMasterVolume(l.clamp(0.0, 1.0), std::ptr::null());
            }
            if let Some(m) = muted {
                let _ = vol.SetMute(m, std::ptr::null());
            }
        }
    })
}

/// A program's icon for the per-app list, extracted once per program.
fn app_icon(path: &str) -> Value {
    use base64::Engine;
    static CACHE: std::sync::Mutex<Option<std::collections::HashMap<String, Value>>> = std::sync::Mutex::new(None);
    if path.is_empty() {
        return Value::Null;
    }
    let mut cache = CACHE.lock().unwrap();
    let map = cache.get_or_insert_with(Default::default);
    map.entry(path.to_lowercase())
        .or_insert_with(|| {
            super::tasks::exe_icon(path)
                .map(|(w, h, rgba)| json!({ "w": w, "h": h, "rgba": base64::engine::general_purpose::STANDARD.encode(rgba) }))
                .unwrap_or(Value::Null)
        })
        .clone()
}

/// Calls `f` with each live app session (not the system sounds) on the default output.
fn for_sessions(mut f: impl FnMut(u32, &ISimpleAudioVolume)) -> Result<(), String> {
    let e = enumerator().map_err(|e| e.to_string())?;
    unsafe {
        let device = e.GetDefaultAudioEndpoint(eRender, eConsole).map_err(|e| e.to_string())?;
        let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None).map_err(|e| e.to_string())?;
        let sessions = manager.GetSessionEnumerator().map_err(|e| e.to_string())?;
        for i in 0..sessions.GetCount().unwrap_or(0) {
            let Ok(ctrl) = sessions.GetSession(i) else { continue };
            let Ok(ctrl2) = ctrl.cast::<IAudioSessionControl2>() else { continue };
            if ctrl2.IsSystemSoundsSession().0 == 0 || ctrl.GetState().is_ok_and(|s| s == AudioSessionStateExpired) {
                continue;
            }
            let pid = ctrl2.GetProcessId().unwrap_or(0);
            if pid == 0 {
                continue;
            }
            if let Ok(vol) = ctrl.cast::<ISimpleAudioVolume>() {
                f(pid, &vol);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    /// Reads this PC's outputs and app sessions (hardware-dependent: run by hand
    /// with `cargo test audio -- --ignored`).
    #[test]
    #[ignore]
    fn lists_outputs_and_sessions() {
        let outputs = tauri::async_runtime::block_on(super::audio_outputs()).unwrap();
        println!("outputs: {outputs}");
        assert!(outputs.as_array().is_some_and(|a| a.iter().filter(|d| d["default"] == true).count() <= 1));
        let sessions = tauri::async_runtime::block_on(super::audio_sessions()).unwrap();
        println!("sessions: {sessions}");
    }
}
