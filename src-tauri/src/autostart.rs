//! Start with Windows via the per-user Run key.

use windows::core::{w, PCWSTR};
use windows::Win32::System::Registry::{
    RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
};

const RUN_KEY: PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const VALUE: PCWSTR = w!("Kasumi");

fn command() -> Vec<u16> {
    let exe = std::env::current_exe().map(|p| p.display().to_string()).unwrap_or_default();
    crate::win::wide(&format!("\"{exe}\""))
}

pub fn is_enabled() -> bool {
    unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE, RRF_RT_REG_SZ, None, None, None).is_ok() }
}

pub fn set(enabled: bool) -> Result<(), String> {
    let result = unsafe {
        if enabled {
            let cmd = command();
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                VALUE,
                REG_SZ.0,
                Some(cmd.as_ptr().cast()),
                (cmd.len() * 2) as u32,
            )
        } else {
            RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, VALUE)
        }
    };
    result.ok().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn autostart_get() -> bool {
    is_enabled()
}

#[tauri::command]
pub async fn autostart_set(enabled: bool) -> Result<(), String> {
    set(enabled)
}

/// If autostart is on, point it at the running executable (e.g. after moving
/// from a debug build to a release build).
pub fn refresh() {
    if is_enabled() {
        let _ = set(true);
    }
}
