//! The keyboard hook, in a small helper process (`Kasumi.exe --keyhook <pid>`):
//! the volume keys (osd/volume.rs), the Windows key opening the launcher
//! instead of Start ([launcher] start_menu), Win + 1..9 opening the tasks
//! widget's apps ([widgets.tasks] win_keys), Alt + Tab showing Kasumi's
//! switcher ([alt_tab]), and the scroll wheel and paging keys for the scroll
//! position pop-up ([osd] scroll; a mouse hook only while that's on).
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
    VK_RWIN, VK_VOLUME_DOWN, VK_VOLUME_MUTE, VK_VOLUME_UP, VK_F23, VK_TAB, VK_ESCAPE, VK_LEFT, VK_RIGHT, VK_UP, VK_DOWN,
    VK_RMENU, VK_MENU, VK_SHIFT, GetAsyncKeyState,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, MsgWaitForMultipleObjects, PeekMessageW, SetWindowsHookExW, UnhookWindowsHookEx, HC_ACTION,
    KBDLLHOOKSTRUCT, LLKHF_EXTENDED, LLKHF_INJECTED, MSG, PM_REMOVE, QS_ALLINPUT, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN,
    WM_MOUSEWHEEL, WM_SYSKEYDOWN,
};

/// The hotkey a lone Windows key press becomes (no keyboard has F24).
pub const START_HOTKEY: (HOT_KEY_MODIFIERS, VIRTUAL_KEY) = (HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0), VK_F24);
/// The hotkey letting go of Alt becomes while Kasumi's switcher shows (it switches;
/// a hotkey lets the app bring the chosen window to the front).
pub const ALT_TAB_HOTKEY: (HOT_KEY_MODIFIERS, VIRTUAL_KEY) = (HOT_KEY_MODIFIERS(MOD_CONTROL.0 | MOD_ALT.0 | MOD_SHIFT.0), VK_F23);

/// Kasumi's Alt + Tab: what the keys asked for.
#[derive(Clone, Copy, Debug)]
pub enum AltTabKey {
    Next,
    Prev,
    Cancel,
    Commit,
}

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
    /// Win + 1..9 on (switch), and one signal per digit.
    win_keys: HANDLE,
    digits: [HANDLE; 9],
    /// Alt + Tab on (switch), and its signals: next, previous, cancel.
    alt_tab: HANDLE,
    alt_tab_keys: [HANDLE; 3],
    /// The language picker on (switch), and its signals: next, previous, cancel, commit.
    lang: HANDLE,
    lang_keys: [HANDLE; 4],
    /// The scroll pop-up on (switch), and its signals: the wheel turned, a paging key.
    scroll: HANDLE,
    scroll_wheel: HANDLE,
    scroll_key: HANDLE,
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
            win_keys: make("WinKeys", true),
            digits: std::array::from_fn(|i| make(&format!("WinDigit{}", i + 1), false)),
            alt_tab: make("AltTab", true),
            alt_tab_keys: [make("AltTabNext", false), make("AltTabPrev", false), make("AltTabCancel", false)],
            lang: make("Lang", true),
            lang_keys: [make("LangNext", false), make("LangPrev", false), make("LangCancel", false), make("LangCommit", false)],
            scroll: make("Scroll", true),
            scroll_wheel: make("ScrollWheel", false),
            scroll_key: make("ScrollKey", false),
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

/// Hands the Win + 1..9 presses the helper signals to `on_digit` (1..=9).
pub fn on_win_digit(on_digit: fn(u8)) {
    let digits = events().digits.map(|h| h.0 as isize);
    std::thread::Builder::new()
        .name("kasumi-winkeys".into())
        .spawn(move || loop {
            let handles = digits.map(|h| HANDLE(h as *mut _));
            let r = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            match r.0.wrapping_sub(WAIT_OBJECT_0.0) {
                i @ 0..=8 => on_digit(i as u8 + 1),
                _ => return,
            }
        })
        .expect("spawn win key thread");
}

/// Hands the language picker's next / previous / cancel / commit to `on_key`.
pub fn on_lang(on_key: fn(AltTabKey)) {
    let keys = events().lang_keys.map(|h| h.0 as isize);
    std::thread::Builder::new()
        .name("kasumi-lang".into())
        .spawn(move || loop {
            let handles = keys.map(|h| HANDLE(h as *mut _));
            let r = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            match r.0.wrapping_sub(WAIT_OBJECT_0.0) {
                0 => on_key(AltTabKey::Next),
                1 => on_key(AltTabKey::Prev),
                2 => on_key(AltTabKey::Cancel),
                3 => on_key(AltTabKey::Commit),
                _ => return,
            }
        })
        .expect("spawn language picker thread");
}

/// Hands Alt + Tab's next / previous / cancel to `on_key` (the commit comes as
/// ALT_TAB_HOTKEY, to the app's hotkey thread).
pub fn on_alt_tab(on_key: fn(AltTabKey)) {
    let keys = events().alt_tab_keys.map(|h| h.0 as isize);
    std::thread::Builder::new()
        .name("kasumi-alttab".into())
        .spawn(move || loop {
            let handles = keys.map(|h| HANDLE(h as *mut _));
            let r = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
            match r.0.wrapping_sub(WAIT_OBJECT_0.0) {
                0 => on_key(AltTabKey::Next),
                1 => on_key(AltTabKey::Prev),
                2 => on_key(AltTabKey::Cancel),
                _ => return,
            }
        })
        .expect("spawn alt tab thread");
}

/// Whether Alt + Tab shows Kasumi's switcher.
pub fn set_alt_tab(on: bool) {
    set(events().alt_tab, on);
}

/// Whether Win + Space opens Kasumi's language picker.
pub fn set_lang(on: bool) {
    set(events().lang, on);
}

/// Whether Win + 1..9 open the tasks widget's apps.
pub fn set_win_keys(on: bool) {
    set(events().win_keys, on);
}

/// Whether the helper takes over the volume keys (the OSD handles them).
pub fn set_replace_volume(on: bool) {
    set(events().replace_volume, on);
}

/// Whether a lone Windows key press opens the launcher instead of Start.
pub fn set_start_key(on: bool) {
    set(events().start_key, on);
}

/// Whether the helper reports the scroll wheel and paging keys (osd/scroll.rs).
pub fn set_scroll(on: bool) {
    set(events().scroll, on);
}

/// The scroll signals to wait on: the wheel turned, a paging key went down.
pub fn scroll_signals() -> [isize; 2] {
    let ev = events();
    [ev.scroll_wheel.0 as isize, ev.scroll_key.0 as isize]
}

// --- In the helper process --------------------------------------------------------

/// A physical Windows key that's down but held back from Windows (its vk; 0 = none).
static WIN_HELD: AtomicU32 = AtomicU32::new(0);
/// A Windows key that's down and Windows has seen (Win + 1..9 then need care: see on_key).
static WIN_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// The held-back Windows key was used for Win + 1..9: its release does nothing.
static WIN_USED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Kasumi's switcher is showing (Alt + Tab, Alt still held).
static SWITCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// Kasumi's language picker is showing (Win + Space, Win still held).
static LANG_SWITCHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Win + Space with the language picker on: Space / Shift + Space go to it (Windows'
/// picker never sees them), Esc cancels, letting go of Win picks. True: swallowed.
fn lang_key(info: &KBDLLHOOKSTRUCT, down: bool) -> bool {
    let ev = events();
    let vk = VIRTUAL_KEY(info.vkCode as u16);
    let switching = LANG_SWITCHING.load(Ordering::Relaxed);
    let held = WIN_HELD.load(Ordering::Relaxed);
    let win = held != 0 || WIN_DOWN.load(Ordering::Relaxed);
    if vk == windows::Win32::UI::Input::KeyboardAndMouse::VK_SPACE && (switching || (win && is_set(ev.lang))) {
        if down {
            if !switching {
                LANG_SWITCHING.store(true, Ordering::Relaxed);
                if held == 0 {
                    // Windows saw Win go down: a key in between keeps its release from
                    // opening Start.
                    crate::win::send_keys(&[KeyEvent::key(VIRTUAL_KEY(0xE8), false), KeyEvent::key(VIRTUAL_KEY(0xE8), true)], OURS);
                } else {
                    WIN_USED.store(true, Ordering::Relaxed);
                }
            }
            let shift = unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0;
            set(ev.lang_keys[if shift { 1 } else { 0 }], true);
        }
        return true;
    }
    if !switching {
        return false;
    }
    match vk {
        VK_ESCAPE => {
            if down {
                LANG_SWITCHING.store(false, Ordering::Relaxed);
                set(ev.lang_keys[2], true);
            }
            true
        }
        // Win let go: pick (its release goes on as usual: no Start, see on_key).
        VK_LWIN | VK_RWIN if !down => {
            LANG_SWITCHING.store(false, Ordering::Relaxed);
            set(ev.lang_keys[3], true);
            false
        }
        _ => false,
    }
}

/// Set in a key event's flags while Alt is held (KBDLLHOOKSTRUCT).
const LLKHF_ALTDOWN: u32 = 0x20;

/// Alt + Tab while Kasumi's switcher is on: Tab / Shift + Tab, the arrows and Esc go
/// to it; letting go of Alt switches (through ALT_TAB_HOTKEY). True: swallowed.
fn alt_tab_key(info: &KBDLLHOOKSTRUCT, down: bool) -> bool {
    let ev = events();
    let vk = VIRTUAL_KEY(info.vkCode as u16);
    let switching = SWITCHING.load(Ordering::Relaxed);
    let alt_held = info.flags.0 & LLKHF_ALTDOWN != 0;
    if vk == VK_TAB && (switching || (alt_held && is_set(ev.alt_tab))) {
        if down {
            if !switching {
                SWITCHING.store(true, Ordering::Relaxed);
                // Alt was down on its own so far: a key in between keeps its release from
                // opening the window's menu bar.
                crate::win::send_keys(&[KeyEvent::key(VIRTUAL_KEY(0xE8), false), KeyEvent::key(VIRTUAL_KEY(0xE8), true)], OURS);
            }
            let shift = unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0;
            set(ev.alt_tab_keys[if shift { 1 } else { 0 }], true);
        }
        return true;
    }
    if !switching {
        return false;
    }
    match vk {
        VK_RIGHT | VK_DOWN => {
            if down {
                set(ev.alt_tab_keys[0], true);
            }
            true
        }
        VK_LEFT | VK_UP => {
            if down {
                set(ev.alt_tab_keys[1], true);
            }
            true
        }
        VK_ESCAPE => {
            if down {
                SWITCHING.store(false, Ordering::Relaxed);
                set(ev.alt_tab_keys[2], true);
            }
            true
        }
        // Alt let go: the private hotkey (Alt still down for it), then Alt's release.
        VK_LMENU | VK_RMENU | VK_MENU if !down => {
            SWITCHING.store(false, Ordering::Relaxed);
            let k = KeyEvent::key;
            let (ctrl, shift, key) = (VK_LCONTROL, VK_LSHIFT, ALT_TAB_HOTKEY.1);
            crate::win::send_keys(&[k(ctrl, false), k(shift, false), k(key, false), k(key, true), k(shift, true), k(ctrl, true), k(vk, true)], OURS);
            true
        }
        _ => false,
    }
}
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
    // Page Up / Down, Home, End: the scroll pop-up looks whether the view moved.
    if down && matches!(info.vkCode, 0x21..=0x24) && is_set(ev.scroll) {
        set(ev.scroll_key, true);
    }
    if alt_tab_key(info, down) || lang_key(info, down) {
        return true;
    }
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
    // Win + 1..9: the tasks widget's apps (the digit never reaches Windows).
    if (0x31..=0x39).contains(&vk) && is_set(ev.win_keys) && (held != 0 || WIN_DOWN.load(Ordering::Relaxed)) {
        if down {
            if held == 0 {
                // Windows saw Win go down: a key in between (one no keyboard has) keeps its
                // release from opening Start.
                crate::win::send_keys(&[KeyEvent::key(VIRTUAL_KEY(0xE8), false), KeyEvent::key(VIRTUAL_KEY(0xE8), true)], OURS);
            } else {
                WIN_USED.store(true, Ordering::Relaxed);
            }
            set(ev.digits[(vk - 0x31) as usize], true);
        }
        return true;
    }
    if vk == VK_LWIN.0 || vk == VK_RWIN.0 {
        if down {
            if held != 0 {
                return true; // auto-repeat while held back
            }
            let simulated = info.flags.0 & LLKHF_INJECTED.0 != 0 && info.dwExtraInfo != AS_PHYSICAL;
            if simulated || !is_set(ev.start_key) {
                WIN_DOWN.store(!simulated, Ordering::Relaxed);
                return false;
            }
            WIN_HELD.store(vk as u32, Ordering::Relaxed);
            return true;
        }
        WIN_DOWN.store(false, Ordering::Relaxed);
        if held != vk {
            return false; // Windows has seen this one (a shortcut): its release too
        }
        WIN_HELD.store(0, Ordering::Relaxed);
        if WIN_USED.swap(false, Ordering::Relaxed) {
            return true; // used for Win + 1..9: no Start, no launcher
        }
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
    // Windows sees Win down now (Win + Shift + Space then still reaches the picker).
    WIN_DOWN.store(true, Ordering::Relaxed);
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

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam.0 as u32 == WM_MOUSEWHEEL {
        set(events().scroll_wheel, true);
    }
    CallNextHookEx(None, code, wparam, lparam)
}

/// The mouse hook, on its own thread, only while the scroll pop-up is on: every mouse
/// move passes through a low-level hook, so none is kept otherwise.
fn mouse_hook() {
    let ev = events();
    let module = unsafe { GetModuleHandleW(None).ok() };
    loop {
        unsafe { WaitForSingleObject(ev.scroll, INFINITE) };
        let Ok(hook) = (unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module.map(Into::into), 0) }) else {
            eprintln!("[kasumi] mouse hook failed");
            return;
        };
        // Low-level hooks are called through this thread's message loop; once a second
        // it looks whether the switch went off.
        let mut msg = MSG::default();
        while is_set(ev.scroll) {
            unsafe {
                MsgWaitForMultipleObjects(None, false, 1000, QS_ALLINPUT);
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {}
            }
        }
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
    }
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
    std::thread::spawn(mouse_hook);
    unsafe {
        // Answer in time even while a game keeps the PC busy: a hook that answers too
        // late is skipped, and the Windows key then opens Start.
        let _ = windows::Win32::System::Threading::SetPriorityClass(
            windows::Win32::System::Threading::GetCurrentProcess(),
            windows::Win32::System::Threading::HIGH_PRIORITY_CLASS,
        );
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
