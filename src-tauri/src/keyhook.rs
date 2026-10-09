//! The one low-level keyboard hook, shared by the volume keys (osd/volume.rs) and
//! the Windows key opening the launcher instead of Start (launcher.rs). Windows
//! calls it for every key press system-wide and drops hooks that stall, so the
//! handlers are quick and lock-free (atomics) and hand real work to their threads.

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, HC_ACTION, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN,
    WM_SYSKEYDOWN,
};

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if crate::osd::volume::on_key(info.vkCode, down) || crate::launcher::on_key(info, down) {
            return LRESULT(1); // swallowed
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

pub fn start() {
    std::thread::Builder::new()
        .name("statusnotch-keyhook".into())
        .spawn(|| unsafe {
            let module = GetModuleHandleW(None).ok();
            if let Err(e) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module.map(Into::into), 0) {
                eprintln!("[statusnotch] keyboard hook failed: {e}");
                return;
            }
            // Low-level hooks are called through this thread's message loop.
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
        })
        .expect("spawn key hook");
}
