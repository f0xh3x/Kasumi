//! The keyboard hook, in a small helper process (`Kasumi.exe --keyhook <pid>`):
//! the volume keys (osd/volume.rs) and the Windows key opening the launcher
//! instead of Start ([launcher] start_menu).
//!
//! Why a separate process: while one of our windows had the focus, Windows didn't
//! run a low-level hook of ours that was first in line, nor the hooks after it.
//! Keys went straight to Windows: the Windows key opened Start instead of closing
//! the launcher, and other apps' shortcuts (PowerToys remaps) did nothing in our
//! windows. The helper never has a window, so it never has the focus.
//!
//! The two processes talk through named events: the app switches the helper's
//! behaviour on and off (`set_replace_volume`, `set_start_key`), the helper
//! signals volume key presses back.
//! A lone Windows key press comes back as a private hotkey the app registers
//! (`START_HOTKEY`): Windows lets the app that gets a hotkey take the focus.

use crate::win::{wide, KeyEvent};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HANDLE, LPARAM, LRESULT, WAIT_OBJECT_0, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    CreateEventW, OpenProcess, ResetEvent, SetEvent, WaitForMultipleObjects, WaitForSingleObject, INFINITE,
    PROCESS_SYNCHRONIZE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, VIRTUAL_KEY, VK_F24, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN,
    VK_RWIN, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, SetWindowsHookExW, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, MSG,
    WH_KEYBOARD_LL, WM_KEYDOWN, WM_SYSKEYDOWN,
};

/// The hotkey a lone Windows key press becomes (no keyboard has F24).
pub const START_HOTKEY: (HOT_KEY_MODIFIERS, VIRTUAL_KEY) = (HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0), VK_F24);

/// A volume key the helper took over.
#[derive(Clone, Copy, Debug)]
pub enum VolumeKey {
    Up,
    Down,
    Mute,
}

/// Named events shared by the app and the helper: two switches (manual reset)
/// and one signal per volume key (auto reset).
struct Events {
    replace_volume: HANDLE,
    start_key: HANDLE,
    volume: [HANDLE; 3],
}
unsafe impl Send for Events {}
unsafe impl Sync for Events {}

fn events() -> &'static Events {
    static EVENTS: OnceLock<Events> = OnceLock::new();
    EVENTS.get_or_init(|| {
        let make = |name: &str, manual: bool| unsafe {
            let n = wide(&format!("Local\\Kasumi.KeyHook.{name}"));
            CreateEventW(None, manual, false, PCWSTR(n.as_ptr())).unwrap_or_default()
        };
        Events {
            replace_volume: make("ReplaceVolume", true),
            start_key: make("StartKey", true),
            volume: [make("VolumeUp", false), make("VolumeDown", false), make("VolumeMute", false)],
        }
    })
}

fn is_set(h: HANDLE) -> bool {
    unsafe { WaitForSingleObject(h, 0) == WAIT_OBJECT_0 }
}

fn set(h: HANDLE, on: bool) {
    unsafe {
        let _ = if on { SetEvent(h) } else { ResetEvent(h) };
    }
}

// --- In the app ----------------------------------------------------------------

/// Starts the helper, and hands the volume key presses it signals to `on_volume`.
pub fn start(on_volume: fn(VolumeKey)) {
    let ev = events();
    let exe = std::env::current_exe();
    let spawn = |exe| {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new(exe).arg("--keyhook").arg(std::process::id().to_string()).creation_flags(CREATE_NO_WINDOW).spawn()
    };
    match exe.and_then(spawn) {
        Ok(_) => {}
        Err(e) => eprintln!("[kasumi] keyboard helper didn't start: {e}"),
    }
    let volume = ev.volume.map(|h| h.0 as isize);
    std::thread::Builder::new()
        .name("kasumi-volumekeys".into())
        .spawn(move || loop {
            let handles = volume.map(|h| HANDLE(h as *mut _));
            let r = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            match r.0.wrapping_sub(WAIT_OBJECT_0.0) {
                0 => on_volume(VolumeKey::Up),
                1 => on_volume(VolumeKey::Down),
                2 => on_volume(VolumeKey::Mute),
                _ => return,
            }
        })
        .expect("spawn volume key thread");
}

/// Whether the helper takes over the volume keys (the OSD handles them).
pub fn set_replace_volume(on: bool) {
    set(events().replace_volume, on);
}

/// Whether a lone Windows key press opens the launcher instead of Start.
pub fn set_start_key(on: bool) {
    set(events().start_key, on);
}

// --- In the helper process --------------------------------------------------------

/// A physical Windows key that's down but held back from Windows (its vk; 0 = none).
static WIN_HELD: AtomicU32 = AtomicU32::new(0);
/// Tags the keys we send, so the hook lets them through (low bits clear: see
/// win::send_keys).
const OURS: usize = 0x534E_0000;
/// Simulated keys tagged with this count as physical: tools/keys.ps1 -physical,
/// to test the Windows key without a hand on the keyboard.
const AS_PHYSICAL: usize = 0x534E_0200;

/// With the Start option on, a physical Windows key press is held back: if
/// another key follows (Win+E...), Win and that key are replayed in order and
/// the shortcut works as usual; if Win is released on its own, Windows never saw
/// it (no Start menu) and the launcher's private hotkey is pressed instead.
/// Simulated Win presses (PowerToys remaps, scripts) pass untouched: they come in
/// bursts, which replaying would reorder.
fn on_key(info: &KBDLLHOOKSTRUCT, down: bool) -> bool {
    if info.dwExtraInfo == OURS {
        return false;
    }
    let ev = events();
    let vk = info.vkCode as u16;
    let volume = match VIRTUAL_KEY(vk) {
        VK_VOLUME_UP => Some(0),
        VK_VOLUME_DOWN => Some(1),
        VK_VOLUME_MUTE => Some(2),
        _ => None,
    };
    if let Some(i) = volume {
        if !is_set(ev.replace_volume) {
            return false;
        }
        if down {
            set(ev.volume[i], true);
        }
        return true;
    }
    let held = WIN_HELD.load(Ordering::Relaxed) as u16;
    if vk == VK_LWIN.0 || vk == VK_RWIN.0 {
        if down {
            if held != 0 {
                return true; // auto-repeat while held back
            }
            let simulated = info.flags.0 & LLKHF_INJECTED.0 != 0 && info.dwExtraInfo != AS_PHYSICAL;
            if simulated || !is_set(ev.start_key) {
                return false;
            }
            WIN_HELD.store(vk as u32, Ordering::Relaxed);
            return true;
        }
        if held != vk {
            return false; // Windows has seen this one (a shortcut): its release too
        }
        WIN_HELD.store(0, Ordering::Relaxed);
        let k = KeyEvent::key;
        let (ctrl, alt, shift, key) = (VK_LCONTROL, VK_LMENU, VK_LSHIFT, START_HOTKEY.1);
        crate::win::send_keys(
            &[k(ctrl, false), k(alt, false), k(shift, false), k(key, false), k(key, true), k(shift, true), k(alt, true), k(ctrl, true)],
            OURS,
        );
        return true;
    }
    if held == 0 || !down {
        return false;
    }
    // A shortcut: Win first, then this key, as they were pressed.
    WIN_HELD.store(0, Ordering::Relaxed);
    let key = KeyEvent { vk, scan: info.scanCode as u16, up: false, extended: info.flags.0 & LLKHF_EXTENDED.0 != 0 };
    crate::win::send_keys(&[KeyEvent::key(VIRTUAL_KEY(held), false), key], OURS);
    true
}

unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
        let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        if on_key(info, down) {
            return LRESULT(1); // swallowed
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// `Kasumi.exe --keyhook <app pid>`: runs the hook until the app exits.
pub fn helper(app_pid: u32) {
    // Exit with the app (however it ends).
    std::thread::spawn(move || unsafe {
        if let Ok(app) = OpenProcess(PROCESS_SYNCHRONIZE, false, app_pid) {
            WaitForSingleObject(app, INFINITE);
        }
        std::process::exit(0);
    });
    let _ = events();
    unsafe {
        let module = GetModuleHandleW(None).ok();
        if let Err(e) = SetWindowsHookExW(WH_KEYBOARD_LL, Some(hook_proc), module.map(Into::into), 0) {
            eprintln!("[kasumi] keyboard hook failed: {e}");
            return;
        }
        // Low-level hooks are called through this thread's message loop.
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    }
}
