//! The app launcher's backend: an index of installed apps (the Apps folder, which
//! has desktop and Store apps alike), their icons on demand, launching, and the
//! global hotkey (`[launcher] hotkey`) that opens it on the monitor under the
//! cursor. The UI is a panel in a bar or dock (see engine::launcher_toggle).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::HSTRING;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY, VK_LWIN, VK_RWIN,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName, SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
};
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, SetTimer, KBDLLHOOKSTRUCT, MSG, WM_APP, WM_HOTKEY, WM_TIMER,
};

struct Index {
    at: Instant,
    apps: Vec<(String, String)>,
}

static INDEX: Mutex<Option<Index>> = Mutex::new(None);
static ICONS: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);

/// Rebuilt when older than this (new installs show up the next time it opens).
const INDEX_AGE: Duration = Duration::from_secs(300);

fn sta() {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}

fn display(item: &IShellItem, kind: windows::Win32::UI::Shell::SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// (id, name) of every app in the Apps folder; `id` is its AppsFolder parsing name.
fn enumerate() -> Vec<(String, String)> {
    sta();
    let mut out = Vec::new();
    unsafe {
        let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from("shell:AppsFolder"), None) else {
            return out;
        };
        let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None, &BHID_EnumItems) else { return out };
        loop {
            let mut batch: [Option<IShellItem>; 32] = Default::default();
            let mut n = 0u32;
            if items.Next(&mut batch, Some(&mut n)).is_err() || n == 0 {
                break;
            }
            for item in batch.iter().take(n as usize).flatten() {
                let (Some(id), Some(name)) = (display(item, SIGDN_PARENTRELATIVEPARSING), display(item, SIGDN_NORMALDISPLAY)) else {
                    continue;
                };
                // Uninstallers and documents are in there too; they aren't apps to launch.
                let lower = name.to_lowercase();
                let doc = [".txt", ".url", ".htm", ".html", ".chm", ".pdf", ".rtf", ".ini"].iter().any(|e| id.to_lowercase().ends_with(e));
                if lower.starts_with("uninstall") || doc || name.is_empty() {
                    continue;
                }
                out.push((id, name));
            }
        }
    }
    out.sort_by_key(|(_, name)| name.to_lowercase());
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

fn apps() -> Vec<(String, String)> {
    if let Some(i) = INDEX.lock().unwrap().as_ref().filter(|i| i.at.elapsed() < INDEX_AGE) {
        return i.apps.clone();
    }
    let apps = enumerate();
    *INDEX.lock().unwrap() = Some(Index { at: Instant::now(), apps: apps.clone() });
    apps
}

#[tauri::command]
pub async fn launcher_apps() -> Result<Vec<Value>, String> {
    let list = tauri::async_runtime::spawn_blocking(apps).await.map_err(|e| e.to_string())?;
    Ok(list.into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect())
}

/// An app's icon as {w, h, rgba}.
#[tauri::command]
pub async fn launcher_icon(id: String) -> Result<Value, String> {
    if let Some(v) = ICONS.lock().unwrap().as_ref().and_then(|m| m.get(&id)) {
        return Ok(v.clone());
    }
    let key = id.clone();
    let icon = tauri::async_runtime::spawn_blocking(move || {
        sta();
        let item = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(format!("shell:AppsFolder\\{key}")), None) }.ok()?;
        crate::providers::tasks::item_icon(&item)
    })
    .await
    .map_err(|e| e.to_string())?
    .ok_or("no icon")?;
    let (w, h, rgba) = icon;
    let v = crate::providers::rgba_json(w, h, &rgba);
    ICONS.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, v.clone());
    Ok(v)
}

#[tauri::command]
pub async fn launcher_launch(id: String) -> Result<(), String> {
    crate::win::allow_foreground();
    std::process::Command::new("explorer.exe")
        .arg(format!("shell:AppsFolder\\{id}"))
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// "Win+Alt+Space" -> modifiers + virtual key.
pub fn parse_hotkey(s: &str) -> Option<(HOT_KEY_MODIFIERS, u32)> {
    let mut mods = HOT_KEY_MODIFIERS(0);
    let mut key = None;
    for part in s.split('+').map(|p| p.trim().to_lowercase()) {
        match part.as_str() {
            "win" | "super" | "meta" => mods |= MOD_WIN,
            "ctrl" | "control" => mods |= MOD_CONTROL,
            "alt" => mods |= MOD_ALT,
            "shift" => mods |= MOD_SHIFT,
            "space" => key = Some(0x20),
            "enter" | "return" => key = Some(0x0D),
            "tab" => key = Some(0x09),
            "esc" | "escape" => key = Some(0x1B),
            k if k.len() == 1 && k.chars().all(|c| c.is_ascii_alphanumeric()) => key = Some(k.to_ascii_uppercase().as_bytes()[0] as u32),
            k if k.starts_with('f') && k[1..].parse::<u32>().is_ok_and(|n| (1..=24).contains(&n)) => {
                key = Some(0x70 + k[1..].parse::<u32>().unwrap() - 1)
            }
            _ => return None,
        }
    }
    key.map(|k| (mods, k))
}

// --- [launcher] start_menu: the Windows key opens the launcher ---------------------

/// Read by the keyboard hook on every key: lock-free.
static START_KEY: AtomicBool = AtomicBool::new(false);
static WIN_DOWN: AtomicBool = AtomicBool::new(false);
/// Another key went down while Win was held: a Win+ shortcut, not Start.
static WIN_USED: AtomicBool = AtomicBool::new(false);
/// The launcher thread: it opens the launcher (the hook mustn't wait for that).
static THREAD: AtomicU32 = AtomicU32::new(0);
const WM_START_KEY: u32 = WM_APP + 1;
/// Tags the keys we send, so the hook lets them through.
const OURS: usize = 0x534E_5354;

/// From the keyboard hook (keyhook.rs). Win pressed and released on its own would
/// open Start: then a key that does nothing (0xE8 is unassigned) goes in between,
/// so Windows sees a shortcut and doesn't, and the launcher opens instead. Win+
/// shortcuts are untouched.
pub(crate) fn on_key(info: &KBDLLHOOKSTRUCT, down: bool) -> bool {
    if info.dwExtraInfo == OURS {
        return false;
    }
    let vk = info.vkCode as u16;
    if vk != VK_LWIN.0 && vk != VK_RWIN.0 {
        if down && WIN_DOWN.load(Ordering::Relaxed) {
            WIN_USED.store(true, Ordering::Relaxed);
        }
        return false;
    }
    if down {
        if !WIN_DOWN.swap(true, Ordering::Relaxed) {
            WIN_USED.store(false, Ordering::Relaxed);
        }
        return false;
    }
    let alone = WIN_DOWN.swap(false, Ordering::Relaxed) && !WIN_USED.load(Ordering::Relaxed);
    if !alone || !START_KEY.load(Ordering::Relaxed) {
        return false;
    }
    // Swallow this release and send it again after the masking key.
    let mask = VIRTUAL_KEY(0xE8);
    crate::win::send_keys(&[(mask, false), (mask, true), (VIRTUAL_KEY(vk), true)], OURS);
    unsafe {
        let _ = PostThreadMessageW(THREAD.load(Ordering::Relaxed), WM_START_KEY, WPARAM(0), LPARAM(0));
    }
    true
}

/// Keeps the configured hotkey registered and opens / closes the launcher on it
/// (and on the Windows key, with `start_menu`).
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("statusnotch-launcher".into())
        .spawn(move || unsafe {
            // Warm the app index so the first search is instant.
            let _ = std::thread::spawn(|| {
                let _ = apps();
            });
            THREAD.store(GetCurrentThreadId(), Ordering::Relaxed);
            SetTimer(None, 0, 500, None);
            let mut registered: Option<String> = None;
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY || msg.message == WM_START_KEY {
                    crate::engine::launcher_toggle(&app, None, None);
                    continue;
                }
                if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                    let want = {
                        let cfg = app.state::<crate::providers::SharedConfig>();
                        let cfg = cfg.read().unwrap();
                        START_KEY.store(cfg.launcher.enabled && cfg.launcher.start_menu, Ordering::Relaxed);
                        let hotkey = cfg.launcher.hotkey.trim();
                        (cfg.launcher.enabled && !hotkey.is_empty()).then(|| hotkey.to_string())
                    };
                    if want != registered {
                        let _ = UnregisterHotKey(None, 1);
                        registered = want.clone();
                        if let Some(hk) = want {
                            match parse_hotkey(&hk) {
                                Some((mods, vk)) => {
                                    if let Err(e) = RegisterHotKey(None, 1, mods | MOD_NOREPEAT, vk) {
                                        eprintln!("[statusnotch] launcher hotkey {hk} unavailable (in use?): {e}");
                                    } else {
                                        eprintln!("[statusnotch] launcher hotkey: {hk}");
                                    }
                                }
                                None => eprintln!("[statusnotch] launcher hotkey '{hk}' not understood"),
                            }
                        }
                    }
                    continue;
                }
                DispatchMessageW(&msg);
            }
        })
        .expect("spawn launcher thread");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkeys_parse() {
        assert_eq!(parse_hotkey("Win+Shift+Space"), Some((MOD_WIN | MOD_SHIFT, 0x20)));
        assert_eq!(parse_hotkey("ctrl + alt + K"), Some((MOD_CONTROL | MOD_ALT, b'K' as u32)));
        assert_eq!(parse_hotkey("Alt+F2"), Some((MOD_ALT, 0x71)));
        assert_eq!(parse_hotkey("Win+Banana"), None);
        assert_eq!(parse_hotkey("Win+Shift"), None);
    }
}
