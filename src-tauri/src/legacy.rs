//! Moving over from StatusNotch (Kasumi's old name), on start: a StatusNotch
//! still running is asked to quit (it gives the Windows taskbar back as it
//! goes), its settings folder is copied to Kasumi's if Kasumi has none yet
//! (the old one stays, as a backup), and its autostart entry becomes Kasumi's.

use std::path::Path;
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
use windows::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
use windows::Win32::System::Threading::{OpenEventW, OpenMutexW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE, SYNCHRONIZATION_SYNCHRONIZE};

const RUN_KEY: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");

/// Asks a running StatusNotch to quit and waits (up to 10 s) until it has.
fn quit_old() {
    unsafe {
        let Ok(quit) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\StatusNotch.Quit")) else { return };
        let _ = SetEvent(quit);
        let _ = CloseHandle(quit);
        if let Ok(mutex) = OpenMutexW(SYNCHRONIZATION_SYNCHRONIZE, false, w!("Local\\StatusNotch.Instance")) {
            let r = WaitForSingleObject(mutex, Duration::from_secs(10).as_millis() as u32);
            if r != WAIT_OBJECT_0 && r.0 != 0x80 {
                eprintln!("[kasumi] StatusNotch didn't quit in time");
            }
            let _ = CloseHandle(mutex);
        }
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let name = entry.file_name();
        // Its log stays behind: Kasumi starts its own.
        if name.to_string_lossy().ends_with(".log") {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(&name))?;
        } else {
            std::fs::copy(&path, to.join(&name))?;
        }
    }
    Ok(())
}

static MOVED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Call first thing in main (before the log, which lives in the settings folder).
pub fn migrate() {
    quit_old();
    let new = crate::config::dir();
    let old = new.with_file_name("StatusNotch");
    let moved = !new.exists() && old.is_dir() && copy_dir(&old, &new).is_ok();
    // StatusNotch started with Windows: now Kasumi does.
    let autostart = unsafe { RegGetValueW(HKEY_CURRENT_USER, RUN_KEY, w!("StatusNotch"), RRF_RT_REG_SZ, None, None, None).is_ok() };
    if autostart {
        unsafe {
            let _ = RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, w!("StatusNotch"));
        }
        let _ = crate::autostart::set(true);
    }
    MOVED.store(moved || autostart, std::sync::atomic::Ordering::Relaxed);
}

/// For the log, once it's open.
pub fn report() {
    if MOVED.load(std::sync::atomic::Ordering::Relaxed) {
        eprintln!("[kasumi] moved over from StatusNotch (settings and/or autostart)");
    }
}
