//! Release builds have no console; send stderr (all `eprintln!` logging) to
//! %APPDATA%\StatusNotch\statusnotch.log so problems can be diagnosed after the fact.

use std::os::windows::io::IntoRawHandle;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_ERROR_HANDLE};

const MAX_BYTES: u64 = 1024 * 1024;

pub fn init() {
    // Already redirected (e.g. launched from a shell with `2> file`): leave it.
    if unsafe { GetStdHandle(STD_ERROR_HANDLE) }.is_ok_and(|h| !h.is_invalid() && !h.0.is_null()) {
        return;
    }
    let dir = crate::config::dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("statusnotch.log");
    // Start fresh once the log grows past 1 MB.
    let append = std::fs::metadata(&path).is_ok_and(|m| m.len() < MAX_BYTES);
    let file = std::fs::OpenOptions::new().create(true).write(true).append(append).truncate(!append).open(&path);
    if let Ok(file) = file {
        let handle = HANDLE(file.into_raw_handle());
        unsafe {
            let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
        eprintln!("[statusnotch] --- started (unix {now}) ---");
    }
}
