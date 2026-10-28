//! The app launcher's backend: an index of installed apps (the Apps folder, which
//! has desktop and Store apps alike) plus Control Panel items such as Device
//! Manager and Windows Settings pages, their icons on demand, launching, and the global hotkey (`[launcher] hotkey`) that opens it on the monitor under the
//! cursor. The UI is a panel in a bar or dock (see engine::launcher_toggle).

use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::HSTRING;
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN,
};
use windows::Win32::UI::Shell::{
    BHID_EnumItems, IEnumShellItems, IShellItem, SHCreateItemFromParsingName, SIGDN, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
    SIGDN_PARENTRELATIVEPARSING,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, SetTimer, MSG, WM_HOTKEY, WM_TIMER,
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

fn display(item: &IShellItem, kind: SIGDN) -> Option<String> {
    unsafe {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Control Panel in its icon view: one entry per applet (the category view and
/// "All Control Panel items" also list every task inside each one).
const CONTROL_PANEL: &str = r"shell:::{26EE0668-A00A-44D7-9371-BEB064C98683}\0";

/// Programs on the PATH: `run:<path>` (searches match them only by their whole name).
const RUN: &str = "run:";

/// Windows Settings pages the launcher finds ([widgets.launcher] settings): the
/// `ms-settings:` link, the page's name, and words people search for. (Windows has no
/// API listing its pages; its own search index names them by internal ids.)
const SETTINGS_PAGES: &[(&str, &str, &str)] = &[
    ("display", "Display", "screen resolution scale scaling brightness monitor hdr orientation"),
    ("display-advanced", "Advanced display", "refresh rate hz monitor"),
    ("display-advancedgraphics", "Graphics", "gpu graphics card performance"),
    ("nightlight", "Night light", "blue light warm eyes"),
    ("sound", "Sound", "audio volume speakers headphones microphone output input"),
    ("sound-devices", "All sound devices", "audio speakers microphone"),
    ("apps-volume", "Volume mixer", "app volume audio mixer"),
    ("notifications", "Notifications", "do not disturb focus alerts banners"),
    ("powersleep", "Power & battery", "sleep battery power mode screen timeout energy saver hibernate"),
    ("storagesense", "Storage", "disk space cleanup storage sense drives temporary files"),
    ("multitasking", "Multitasking", "snap windows alt tab virtual desktops"),
    ("clipboard", "Clipboard", "clipboard history copy paste"),
    ("remotedesktop", "Remote Desktop", "rdp remote"),
    ("about", "About", "system info pc name specs rename computer windows version ram processor"),
    ("activation", "Activation", "product key license"),
    ("troubleshoot", "Troubleshoot", "fix problems troubleshooter"),
    ("recovery", "Recovery", "reset this pc restore advanced startup"),
    ("optionalfeatures", "Optional features", "add features"),
    ("developers", "For developers", "developer mode sudo"),
    ("bluetooth", "Bluetooth & devices", "bluetooth pair devices headphones wireless"),
    ("printers", "Printers & scanners", "printer scanner print"),
    ("mousetouchpad", "Mouse", "mouse pointer speed scroll wheel cursor buttons"),
    ("devices-touchpad", "Touchpad", "touchpad gestures trackpad"),
    ("typing", "Typing", "autocorrect spelling keyboard suggestions touch keyboard"),
    ("pen", "Pen & Windows Ink", "pen stylus ink"),
    ("autoplay", "AutoPlay", "usb removable drives media"),
    ("usb", "USB", "usb"),
    ("camera", "Cameras", "webcam camera"),
    ("mobile-devices", "Mobile devices", "phone link android iphone"),
    ("network-status", "Network & internet", "internet connection network"),
    ("network-wifi", "Wi-Fi", "wifi wireless wlan"),
    ("network-ethernet", "Ethernet", "cable lan wired"),
    ("network-vpn", "VPN", "vpn"),
    ("network-mobilehotspot", "Mobile hotspot", "hotspot tethering share internet"),
    ("network-airplanemode", "Airplane mode", "flight mode"),
    ("network-proxy", "Proxy", "proxy"),
    ("network-advancedsettings", "Advanced network settings", "adapters network reset dns"),
    ("personalization-background", "Background", "wallpaper desktop picture slideshow"),
    ("colors", "Colors", "dark mode light mode accent color theme transparency"),
    ("lockscreen", "Lock screen", "lock screen picture"),
    ("themes", "Themes", "theme desktop icons"),
    ("fonts", "Fonts", "fonts typeface"),
    ("personalization-start", "Start", "start menu"),
    ("taskbar", "Taskbar", "taskbar"),
    ("appsfeatures", "Installed apps", "uninstall programs remove apps"),
    ("defaultapps", "Default apps", "default browser open with file types"),
    ("startupapps", "Startup apps", "startup autostart boot"),
    ("appsforwebsites", "Apps for websites", "links"),
    ("videoplayback", "Video playback", "video hdr"),
    ("yourinfo", "Your info", "account profile picture name"),
    ("emailandaccounts", "Email & accounts", "email accounts"),
    ("signinoptions", "Sign-in options", "password pin windows hello fingerprint face login"),
    ("family-group", "Family", "family parental"),
    ("otherusers", "Other users", "add user accounts"),
    ("backup", "Windows Backup", "backup sync onedrive"),
    ("workplace", "Access work or school", "work school"),
    ("dateandtime", "Date & time", "clock time zone date"),
    ("regionformatting", "Language & region", "language region format country keyboard layout"),
    ("speech", "Speech", "voice speech recognition"),
    ("gaming-gamebar", "Game Bar", "xbox game bar"),
    ("gaming-gamedvr", "Captures", "screen recording clips"),
    ("gaming-gamemode", "Game Mode", "games performance"),
    ("easeofaccess-display", "Text size", "font size bigger text accessibility"),
    ("easeofaccess-visualeffects", "Visual effects", "animations transparency scrollbars"),
    ("easeofaccess-mousepointer", "Mouse pointer and touch", "cursor size pointer color"),
    ("easeofaccess-magnifier", "Magnifier", "zoom magnify"),
    ("easeofaccess-colorfilter", "Color filters", "color blind grayscale"),
    ("easeofaccess-highcontrast", "Contrast themes", "high contrast"),
    ("easeofaccess-narrator", "Narrator", "screen reader"),
    ("easeofaccess-audio", "Audio accessibility", "mono audio"),
    ("easeofaccess-closedcaptioning", "Captions", "subtitles captions"),
    ("easeofaccess-keyboard", "Keyboard accessibility", "sticky keys filter keys on-screen keyboard"),
    ("privacy", "Privacy & security", "privacy permissions"),
    ("privacy-location", "Location", "gps location"),
    ("privacy-webcam", "Camera permissions", "webcam privacy"),
    ("privacy-microphone", "Microphone permissions", "microphone privacy"),
    ("windowsdefender", "Windows Security", "antivirus defender virus firewall"),
    ("windowsupdate", "Windows Update", "updates check for updates"),
    ("windowsupdate-history", "Update history", "updates installed"),
    ("windowsupdate-options", "Advanced update options", "updates restart hours"),
    ("delivery-optimization", "Delivery Optimization", "download bandwidth"),
    ("windowsinsider", "Windows Insider Program", "insider preview builds"),
];

/// Settings pages' ids start with this (the link itself).
const SETTINGS: &str = "ms-settings:";
/// The Settings app, for those pages' icon.
const SETTINGS_APP: &str = "windows.immersivecontrolpanel_cw5n1h2txyewy!microsoft.windows.immersivecontrolpanel";

/// A Settings page's search words.
fn settings_keys(id: &str) -> Option<&'static str> {
    let page = id.strip_prefix(SETTINGS)?;
    SETTINGS_PAGES.iter().find(|p| p.0 == page).map(|p| p.2)
}

/// The shell path an id opens: Control Panel ids are absolute ("::{...}"), programs
/// on the PATH and Start's pinned shortcuts their file, Settings pages their link, app
/// ids are AppsFolder parsing names.
pub(crate) fn shell_path(id: &str) -> String {
    if id.starts_with("::") {
        format!("shell:{id}")
    } else if id.starts_with(SETTINGS) {
        // The link as it is (with "shell:" in front Explorer doesn't know it and opens
        // Documents).
        id.to_string()
    } else if let Some(path) = id.strip_prefix(RUN).or_else(|| id.strip_prefix(LNK)) {
        path.to_string()
    } else {
        format!("shell:AppsFolder\\{id}")
    }
}

/// Every program in the PATH's folders by its name (`notepad`, `regedit`, `cmd`),
/// the first of each name: Start finds them when typed in full, shortcut or not.
fn commands() -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for dir in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let Ok(files) = std::fs::read_dir(&dir) else { continue };
        for path in files.flatten().map(|f| f.path()) {
            if !path.extension().is_some_and(|x| x.eq_ignore_ascii_case("exe")) {
                continue;
            }
            let Some(name) = path.file_stem().and_then(|s| s.to_str()).map(str::to_lowercase) else { continue };
            if seen.insert(name.clone()) {
                out.push((format!("{RUN}{}", path.display()), name));
            }
        }
    }
    out
}

/// (id, name) of every item in a shell folder, the id read as `id_kind`.
fn items(folder: &str, id_kind: SIGDN) -> Vec<(String, String)> {
    let mut out = Vec::new();
    unsafe {
        let Ok(folder) = SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(folder), None) else {
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
                if let (Some(id), Some(name)) = (display(item, id_kind), display(item, SIGDN_NORMALDISPLAY)) {
                    if !name.is_empty() {
                        out.push((id, name));
                    }
                }
            }
        }
    }
    out.sort_by_key(|(_, name)| name.to_lowercase());
    out.dedup_by(|a, b| a.0 == b.0);
    out
}

/// What the launcher searches: apps first (they win a tie), then the Control
/// Panel items (Device Manager, Sound...), programs on the PATH no app has the name
/// of, and the Settings pages (the launcher shows those only with its option on).
fn enumerate() -> Vec<(String, String)> {
    sta();
    // Uninstallers and documents are in the Apps folder too; they aren't apps to launch.
    let mut out: Vec<(String, String)> = items("shell:AppsFolder", SIGDN_PARENTRELATIVEPARSING)
        .into_iter()
        .filter(|(id, name)| {
            let id = id.to_lowercase();
            let doc = [".txt", ".url", ".htm", ".html", ".chm", ".pdf", ".rtf", ".ini"].iter().any(|e| id.ends_with(e));
            !doc && !name.to_lowercase().starts_with("uninstall")
        })
        .collect();
    let names: HashSet<String> = out.iter().map(|(_, name)| name.to_lowercase()).collect();
    let panel = items(CONTROL_PANEL, SIGDN_DESKTOPABSOLUTEPARSING);
    out.extend(panel.into_iter().filter(|(_, name)| !names.contains(&name.to_lowercase())));
    out.extend(commands().into_iter().filter(|(_, name)| !names.contains(name)));
    out.extend(SETTINGS_PAGES.iter().map(|(page, name, _)| (format!("{SETTINGS}{page}"), name.to_string())));
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

/// What the dock's pin list ([widgets.tasks] pins) calls an app of the launcher's: its
/// AppUserModelID, or its program / shortcut (an Apps folder path under a known folder,
/// `{GUID}\app.exe`, made a real one). None for what the dock can't hold (Settings
/// pages, Control Panel items, sums).
pub fn dock_entry(id: &str) -> Option<String> {
    if id.starts_with("::") || id.starts_with(SETTINGS) || id.starts_with("calc:") {
        return None;
    }
    if let Some(path) = id.strip_prefix(RUN).or_else(|| id.strip_prefix(LNK)) {
        return Some(path.to_string());
    }
    if let Some(rest) = id.strip_prefix('{') {
        let (guid, tail) = rest.split_once("}\\")?;
        let guid = windows::core::GUID::try_from(guid).ok()?;
        let base = unsafe { windows::Win32::UI::Shell::SHGetKnownFolderPath(&guid, windows::Win32::UI::Shell::KF_FLAG_DEFAULT, None) }.ok()?;
        let base = unsafe {
            let s = base.to_string().ok();
            CoTaskMemFree(Some(base.0 as *const _));
            s?
        };
        return Some(format!("{base}\\{tail}"));
    }
    (!id.contains('\\')).then(|| id.to_string())
}

/// Start's pins changed (pinned or unpinned from the launcher's menu): read again, and
/// the launcher told.
pub fn pins_changed() {
    *PINS.lock().unwrap() = None;
    std::thread::spawn(|| {
        // Start writes its layout a moment after the command.
        std::thread::sleep(Duration::from_millis(700));
        sta();
        let pins = start_pins().into_iter().map(|(id, name)| json!({ "id": id, "name": name })).collect();
        crate::native::post(crate::native::Msg::Data("launcher:pinned".into(), Value::Array(pins)));
    });
}

/// Shortcuts pinned to Start that no app in the index has the name of: `lnk:<path>`.
const LNK: &str = "lnk:";

static PINS: Mutex<Option<(Instant, Vec<(String, String)>)>> = Mutex::new(None);

/// `%APPDATA%\x` -> its folder (the variables in Start's layout).
fn expand_vars(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        let Some(b) = rest[a + 1..].find('%') else { break };
        out.push_str(&rest[..a]);
        let name = &rest[a + 1..a + 1 + b];
        out.push_str(&std::env::var(name).unwrap_or_else(|_| format!("%{name}%")));
        rest = &rest[a + 2 + b..];
    }
    out + rest
}

/// The apps pinned to Windows' Start menu, in its order ([widgets.launcher] pinned):
/// Windows has no API for them, but `Export-StartLayout` writes them as JSON (Windows
/// 11; elsewhere none). Store apps come as their app ids, desktop apps as their Start
/// menu shortcuts, matched to the index by name. Read again after a minute at most.
pub fn start_pins() -> Vec<(String, String)> {
    if let Some((at, pins)) = PINS.lock().unwrap().as_ref() {
        if at.elapsed() < Duration::from_secs(60) {
            return pins.clone();
        }
    }
    let file = std::env::temp_dir().join(format!("kasumi-start-{}.json", std::process::id()));
    let ok = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &format!("Export-StartLayout -Path '{}'", file.display())])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .is_ok_and(|s| s.success())
    };
    let text = if ok { std::fs::read_to_string(&file).unwrap_or_default() } else { String::new() };
    let _ = std::fs::remove_file(&file);
    let layout: Value = serde_json::from_str(text.trim_start_matches('\u{feff}')).unwrap_or(Value::Null);
    let index = apps();
    let by_id = |id: &str| index.iter().find(|(i, _)| i.eq_ignore_ascii_case(id)).cloned();
    let by_name = |name: &str| index.iter().find(|(_, n)| n.eq_ignore_ascii_case(name)).cloned();
    let mut pins = Vec::new();
    for p in layout.get("pinnedList").and_then(|l| l.as_array()).into_iter().flatten() {
        let s = |k: &str| p.get(k).and_then(|v| v.as_str());
        if let Some(found) = s("packagedAppId").and_then(by_id) {
            pins.push(found);
        } else if let Some(link) = s("desktopAppLink") {
            let path = expand_vars(link);
            let name = std::path::Path::new(&path).file_stem().and_then(|n| n.to_str()).unwrap_or("").to_string();
            pins.push(by_name(&name).unwrap_or_else(|| (format!("{LNK}{path}"), name)));
        }
    }
    pins.dedup();
    *PINS.lock().unwrap() = Some((Instant::now(), pins.clone()));
    pins
}

#[tauri::command]
pub async fn launcher_apps() -> Result<Vec<Value>, String> {
    let list = tauri::async_runtime::spawn_blocking(apps).await.map_err(|e| e.to_string())?;
    Ok(list
        .into_iter()
        .map(|(id, name)| match settings_keys(&id) {
            Some(keys) => json!({ "id": id, "name": name, "keys": keys }),
            None => json!({ "id": id, "name": name }),
        })
        .collect())
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
        // Settings pages: the Settings app's icon.
        let path = if key.starts_with(SETTINGS) { format!("shell:AppsFolder\\{SETTINGS_APP}") } else { shell_path(&key) };
        let item = unsafe { SHCreateItemFromParsingName::<_, _, IShellItem>(&HSTRING::from(path), None) }.ok()?;
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
        .arg(shell_path(&id))
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

/// The signed-in user for the launcher's user button: name and account picture
/// (Settings > Accounts > Your info), as a data URL (None without one).
#[tauri::command]
pub async fn launcher_user() -> Value {
    let picture = crate::win::user_sid()
        .and_then(|sid| {
            let key = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\AccountPicture\Users\{sid}");
            crate::win::reg_string(windows::Win32::System::Registry::HKEY_LOCAL_MACHINE, &key, "Image96")
        })
        .and_then(|path| std::fs::read(path).ok())
        .map(|bytes| crate::providers::data_url("image/jpeg", &bytes));
    json!({ "name": std::env::var("USERNAME").unwrap_or_default(), "picture": picture })
}

/// [launcher] start_menu: the keyboard helper (keyhook.rs) presses this hotkey on
/// a lone Windows key press.
const START_HOTKEY_ID: i32 = 2;
/// [alt_tab]: the keyboard helper presses this hotkey when Alt is let go of while
/// Kasumi's switcher shows (getting a hotkey lets us bring the window up).
const ALT_TAB_HOTKEY_ID: i32 = 3;

/// Keeps the configured hotkey registered and opens / closes the launcher on it
/// (and on the Windows key, with `start_menu`).
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-launcher".into())
        .spawn(move || unsafe {
            // Warm the app index so the first search is instant.
            let _ = std::thread::spawn(|| {
                let _ = apps();
            });
            SetTimer(None, 0, 500, None);
            let mut registered: Option<String> = None;
            let mut start_registered = false;
            let mut alt_tab_registered = false;
            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                if msg.message == WM_HOTKEY && msg.wParam.0 as i32 == ALT_TAB_HOTKEY_ID {
                    crate::native::alt_tab(crate::keyhook::AltTabKey::Commit);
                    continue;
                }
                if msg.message == WM_HOTKEY {
                    crate::engine::launcher_toggle(&app, None, None, true);
                    continue;
                }
                if msg.message == WM_TIMER && msg.hwnd.is_invalid() {
                    let want = {
                        let cfg = app.state::<crate::providers::SharedConfig>();
                        let cfg = cfg.read().unwrap();
                        let start = cfg.launcher.enabled && cfg.launcher.start_menu;
                        if start != start_registered {
                            start_registered = start;
                            if start {
                                let (mods, vk) = crate::keyhook::START_HOTKEY;
                                if let Err(e) = RegisterHotKey(None, START_HOTKEY_ID, mods | MOD_NOREPEAT, vk.0 as u32) {
                                    eprintln!("[kasumi] Windows key for the launcher unavailable: {e}");
                                }
                            } else {
                                let _ = UnregisterHotKey(None, START_HOTKEY_ID);
                            }
                        }
                        crate::keyhook::set_start_key(start);
                        // Win + 1..9: the tasks widget's apps (on unless switched off).
                        let win_keys = cfg.widgets.get("tasks").and_then(|t| t.get("win_keys")).and_then(|v| v.as_bool()) != Some(false);
                        crate::keyhook::set_win_keys(win_keys);
                        // Kasumi's Alt + Tab.
                        let alt_tab = cfg.alt_tab.enabled;
                        if alt_tab != alt_tab_registered {
                            alt_tab_registered = alt_tab;
                            if alt_tab {
                                let (mods, vk) = crate::keyhook::ALT_TAB_HOTKEY;
                                if let Err(e) = RegisterHotKey(None, ALT_TAB_HOTKEY_ID, mods | MOD_NOREPEAT, vk.0 as u32) {
                                    eprintln!("[kasumi] Alt + Tab switcher unavailable: {e}");
                                }
                            } else {
                                let _ = UnregisterHotKey(None, ALT_TAB_HOTKEY_ID);
                            }
                        }
                        crate::keyhook::set_alt_tab(alt_tab);
                        // Kasumi's language picker.
                        crate::keyhook::set_lang(cfg.lang_switch.enabled);
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
                                        eprintln!("[kasumi] launcher hotkey {hk} unavailable (in use?): {e}");
                                    } else {
                                        eprintln!("[kasumi] launcher hotkey: {hk}");
                                    }
                                }
                                None => eprintln!("[kasumi] launcher hotkey '{hk}' not understood"),
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
    fn expands_variables() {
        std::env::set_var("KASUMI_TEST_DIR", r"C:\Users\x");
        assert_eq!(expand_vars(r"%KASUMI_TEST_DIR%\Start Menu\a.lnk"), r"C:\Users\x\Start Menu\a.lnk");
        assert_eq!(expand_vars(r"%NO_SUCH_VAR_K%\a"), r"%NO_SUCH_VAR_K%\a");
        assert_eq!(expand_vars("100% sure"), "100% sure");
    }

    /// The Start menu's pins on this PC: `cargo test start_pins_read -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn start_pins_read() {
        let t = Instant::now();
        let pins = start_pins();
        println!("{} pins in {} ms", pins.len(), t.elapsed().as_millis());
        for p in pins {
            println!("  {p:?}");
        }
    }

    #[test]
    fn dock_entries() {
        assert_eq!(dock_entry("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App").as_deref(), Some("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"));
        assert_eq!(dock_entry("run:C:\\Windows\\regedit.exe").as_deref(), Some("C:\\Windows\\regedit.exe"));
        assert_eq!(dock_entry("lnk:C:\\x\\Zen.lnk").as_deref(), Some("C:\\x\\Zen.lnk"));
        // System32 (FOLDERID_System) made a real folder.
        let notepad = dock_entry("{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\\notepad.exe").unwrap_or_default().to_lowercase();
        assert!(notepad.ends_with("\\system32\\notepad.exe"), "{notepad}");
        for none in ["ms-settings:display", "::{26EE0668-A00A-44D7-9371-BEB064C98683}\\0\\x", "calc:4", "{not-a-guid}\\a.exe"] {
            assert_eq!(dock_entry(none), None, "{none}");
        }
    }

    #[test]
    fn opens_what_each_id_is() {
        assert_eq!(shell_path("ms-settings:personalization-background"), "ms-settings:personalization-background");
        assert_eq!(shell_path("::{26EE0668-A00A-44D7-9371-BEB064C98683}\\0\\::{74246BFC}"), "shell:::{26EE0668-A00A-44D7-9371-BEB064C98683}\\0\\::{74246BFC}");
        assert_eq!(shell_path("run:C:\\Windows\\notepad.exe"), "C:\\Windows\\notepad.exe");
        assert_eq!(shell_path("lnk:C:\\Start Menu\\Zen.lnk"), "C:\\Start Menu\\Zen.lnk");
        assert_eq!(shell_path("Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"), "shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App");
        // Every Settings page in the table is a link of its own.
        for (page, ..) in SETTINGS_PAGES {
            assert!(!page.contains(':') && !page.contains(' '), "{page}");
        }
    }

    #[test]
    fn hotkeys_parse() {
        assert_eq!(parse_hotkey("Win+Shift+Space"), Some((MOD_WIN | MOD_SHIFT, 0x20)));
        assert_eq!(parse_hotkey("ctrl + alt + K"), Some((MOD_CONTROL | MOD_ALT, b'K' as u32)));
        assert_eq!(parse_hotkey("Alt+F2"), Some((MOD_ALT, 0x71)));
        assert_eq!(parse_hotkey("Win+Banana"), None);
        assert_eq!(parse_hotkey("Win+Shift"), None);
    }
}
