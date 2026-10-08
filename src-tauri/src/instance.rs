//! Single instance with a graceful handoff: a newly started StatusNotch asks the
//! running one to quit (so it removes its tray icon and windows cleanly) and
//! waits for it to exit before taking over.

use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::Threading::{CreateEventW, CreateMutexW, SetEvent, WaitForSingleObject};

struct Handles {
    quit: HANDLE,
    settings: HANDLE,
}

// Kernel handles are process-wide; they are only waited on / signalled.
unsafe impl Send for Handles {}
unsafe impl Sync for Handles {}

static HANDLES: std::sync::OnceLock<Handles> = std::sync::OnceLock::new();

/// Call first thing in main. Takes over from a running instance, if any.
/// With `--settings`, instead asks a running instance to open its settings
/// window and exits (or, if none is running, starts and opens it).
pub fn claim() {
    let want_settings = std::env::args().any(|a| a == "--settings");
    unsafe {
        let settings = CreateEventW(None, false, false, w!(r"Local\StatusNotch.Settings")).expect("create settings event");
        let already_running = GetLastError() == ERROR_ALREADY_EXISTS;
        if want_settings {
            let _ = SetEvent(settings);
            if already_running {
                std::process::exit(0);
            }
        }
        let quit = CreateEventW(None, false, false, w!("Local\\StatusNotch.Quit")).expect("create quit event");
        let mutex = CreateMutexW(None, false, w!("Local\\StatusNotch.Instance")).expect("create instance mutex");
        if GetLastError() == ERROR_ALREADY_EXISTS {
            eprintln!("[statusnotch] another instance is running; asking it to quit");
            let _ = SetEvent(quit);
        }
        // Owned (or abandoned) once the previous instance has exited.
        let r = WaitForSingleObject(mutex, Duration::from_secs(10).as_millis() as u32);
        if r != WAIT_OBJECT_0 && r.0 != 0x80 {
            eprintln!("[statusnotch] previous instance did not exit; continuing anyway");
        }
        let _ = HANDLES.set(Handles { quit, settings });
    }
}

/// True when something (`StatusNotch.exe --settings`) asked for the settings window.
pub fn settings_requested() -> bool {
    HANDLES.get().is_some_and(|h| unsafe { WaitForSingleObject(h.settings, 0) } == WAIT_OBJECT_0)
}

/// True once another instance has asked us to quit.
pub fn quit_requested() -> bool {
    HANDLES.get().is_some_and(|h| unsafe { WaitForSingleObject(h.quit, 0) } == WAIT_OBJECT_0)
}
