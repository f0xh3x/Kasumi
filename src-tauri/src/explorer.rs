//! File Explorer styling (`[explorer]`, experimental).
//!
//! From outside: folder windows' and Explorer's dialogs' frames (backdrop, title bar,
//! corners, border: DWM window attributes), set as they show.
//!
//! From inside: parts of a folder window's header hidden (`hide`): Kasumi's helper
//! (`Kasumi.exe --explorer-inject <pid>`, `helper`) asks Windows' XAML Diagnostics to
//! load `kasumi_explorer.dll` (explorer-tap/) into the window's Explorer process; it
//! reads the rules file this module keeps (`rules_path`) and follows its changes. The
//! helper is its own short-lived process, so Kasumi never loads Explorer's XAML.

use crate::providers::SharedConfig;
use std::collections::HashSet;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};
use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWINDOWATTRIBUTE};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::*;

/// The library's class (explorer-tap/src/lib.rs CLSID_TAP).
const CLSID_TAP: GUID = GUID::from_u128(0x6c1f2b84_5a1e_4c62_9f5e_3d8a7b2c1e90);
const DLL: &str = "kasumi_explorer.dll";

/// Parts of the header, by name in Settings → the library's specs (`hide <spec>`), as
/// Explorer's tree has them (Windows 11 26100, mapped with the library's `dump`).
pub const PARTS: &[(&str, &[&str])] = &[
    // The command bar's whole row (New, Cut, Copy... Sort, View, Details).
    ("command_bar", &["FileExplorerExtensions.CommandBarControl_Wave1"]),
    ("details_button", &["CommandBar#FileExplorerSecondaryCommandBar"]),
    ("back", &["AppBarButton#backButton"]),
    ("forward", &["AppBarButton#forwardButton"]),
    ("up", &["AppBarButton#upButton"]),
    ("refresh", &["AppBarButton#refreshButton", "AppBarButton#stopButton"]),
    ("search", &["AutoSuggestBox#FileExplorerSearchBox"]),
    ("new_tab", &["Button#AddButton"]),
    // The tab strip (the window keeps its title row, with its buttons).
    ("tabs", &["TabViewListView#TabListView"]),
    // Under the tabs, between tabs, between the command bar's groups.
    ("lines", &["Border#LeftBottomBorderLine", "Border#RightBottomBorderLine", "Border#TabSeparator", "AppBarSeparator#"]),
];

/// Hiding is on: something to hide (or to map).
fn parts_on(cfg: &crate::config::ExplorerCfg) -> bool {
    cfg.enabled && (!cfg.hide.is_empty() || cfg.hide_custom.iter().any(|s| !s.trim().is_empty()) || cfg.map)
}

/// The rules file the library reads (%APPDATA%\Kasumi\explorer-rules.txt).
pub fn rules_path() -> std::path::PathBuf {
    crate::config::path().with_file_name("explorer-rules.txt")
}

fn rules_text(cfg: &crate::config::ExplorerCfg) -> String {
    if !parts_on(cfg) {
        return String::new();
    }
    let mut out = String::new();
    if cfg.map {
        out.push_str("dump\n");
    }
    for key in &cfg.hide {
        if let Some((_, specs)) = PARTS.iter().find(|(k, _)| k == key) {
            for s in *specs {
                out.push_str(&format!("hide {s}\n"));
            }
        }
    }
    for s in &cfg.hide_custom {
        if !s.trim().is_empty() {
            out.push_str(&format!("hide {}\n", s.trim()));
        }
    }
    out
}

fn note(line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(std::env::temp_dir().join("kasumi-explorer.log")) {
        let _ = writeln!(f, "[kasumi {}] {line}", std::process::id());
    }
}

// --- The helper process ----------------------------------------------------------------------------

/// The modules loaded in `pid` (name, full path).
fn modules(pid: u32) -> Vec<(String, String)> {
    use windows::Win32::System::Diagnostics::ToolHelp::*;
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPMODULE | TH32CS_SNAPMODULE32, pid) else { return out };
        let mut m = MODULEENTRY32W { dwSize: std::mem::size_of::<MODULEENTRY32W>() as u32, ..Default::default() };
        let mut ok = Module32FirstW(snap, &mut m).is_ok();
        while ok {
            let name = String::from_utf16_lossy(&m.szModule[..m.szModule.iter().position(|&c| c == 0).unwrap_or(0)]);
            let path = String::from_utf16_lossy(&m.szExePath[..m.szExePath.iter().position(|&c| c == 0).unwrap_or(0)]);
            out.push((name, path));
            ok = Module32NextW(snap, &mut m).is_ok();
        }
        let _ = windows::Win32::Foundation::CloseHandle(snap);
    }
    out
}

/// The library, copied under a name of its own per build (a loaded copy stays locked
/// in Explorer until its process ends; a new build must not need to overwrite it).
fn dll_copy() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let src = exe.with_file_name(DLL);
    let meta = std::fs::metadata(&src).ok()?;
    let stamp = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let dir = std::path::PathBuf::from(std::env::var("LOCALAPPDATA").ok()?).join("Kasumi").join("explorer");
    let _ = std::fs::create_dir_all(&dir);
    let dst = dir.join(format!("kasumi_explorer-{}-{stamp}.dll", meta.len()));
    if !dst.exists() {
        std::fs::copy(&src, &dst).ok()?;
    }
    Some(dst)
}

/// `--explorer-inject <pid> [--rules <file>]`. Exit codes: 0 loaded (or already there),
/// 2 no XAML in it yet (try later), 1 failed.
pub fn helper(args: &[String]) -> i32 {
    let Some(pid) = args.iter().position(|a| a == "--explorer-inject").and_then(|i| args.get(i + 1)).and_then(|p| p.parse::<u32>().ok()) else {
        return 1;
    };
    let rules = args.iter().position(|a| a == "--rules").and_then(|i| args.get(i + 1)).cloned().unwrap_or_else(|| rules_path().to_string_lossy().into_owned());
    let mods = modules(pid);
    if mods.iter().any(|(n, _)| n.to_ascii_lowercase().starts_with("kasumi_explorer")) {
        return 0;
    }
    // WinUI 3's (the Windows App Runtime's), not the system XAML's.
    let Some((_, xaml)) = mods.iter().find(|(n, p)| n.eq_ignore_ascii_case("Microsoft.UI.Xaml.dll") && p.contains("WindowsAppRuntime")) else {
        return 2;
    };
    let Some(dll) = dll_copy() else {
        note("no kasumi_explorer.dll next to Kasumi.exe");
        return 1;
    };
    type Init = unsafe extern "system" fn(PCWSTR, u32, PCWSTR, PCWSTR, GUID, PCWSTR) -> HRESULT;
    unsafe {
        use windows::Win32::System::LibraryLoader::*;
        let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
        // WinUI 3's diagnostics entry is in the framework's UDK, next to its XAML.
        let udk = std::path::Path::new(xaml).with_file_name("Microsoft.Internal.FrameworkUdk.dll");
        let path = wide(&udk.to_string_lossy());
        let Ok(module) = LoadLibraryExW(PCWSTR(path.as_ptr()), None, LOAD_WITH_ALTERED_SEARCH_PATH) else {
            note(&format!("couldn't load {}", udk.display()));
            return 1;
        };
        let Some(f) = GetProcAddress(module, windows::core::s!("InitializeXamlDiagnosticsEx")) else {
            note("no InitializeXamlDiagnosticsEx");
            return 1;
        };
        let init: Init = std::mem::transmute(f);
        let dll_w = wide(&dll.to_string_lossy());
        let rules_w = wide(&rules);
        let empty = wide("");
        let mut last = HRESULT(0);
        // WinUI 3's own connection names: "VisualDiagConnection" is the system XAML's, which
        // Explorer runs too (it took a request meant for WinUI 3 and gave up: a crash). Another
        // tool may hold the first ones.
        for i in 1..=4 {
            let end = wide(&format!("WinUIVisualDiagConnection{i}"));
            last = init(PCWSTR(end.as_ptr()), pid, PCWSTR(empty.as_ptr()), PCWSTR(dll_w.as_ptr()), CLSID_TAP, PCWSTR(rules_w.as_ptr()));
            if last.is_ok() {
                note(&format!("loaded into {pid} (connection {i})"));
                return 0;
            }
            note(&format!("connection {i}: {last:?}"));
        }
        note(&format!("couldn't load into {pid}: {last:?}"));
        1
    }
}

// --- The watcher ----------------------------------------------------------------------------------------

static LOADED: Mutex<Option<HashSet<u32>>> = Mutex::new(None);
static STYLE: Mutex<Option<crate::config::ExplorerCfg>> = Mutex::new(None);

/// Load the library into `pid` (once), from a thread: the window's XAML may not be up
/// yet, so it's tried a few times.
fn load_into(pid: u32) {
    {
        let mut l = LOADED.lock().unwrap();
        if !l.get_or_insert_with(HashSet::new).insert(pid) {
            return;
        }
    }
    std::thread::spawn(move || {
        use std::os::windows::process::CommandExt;
        let Ok(exe) = std::env::current_exe() else { return };
        for wait in [300u64, 1000, 2500, 5000] {
            std::thread::sleep(std::time::Duration::from_millis(wait));
            let code = std::process::Command::new(&exe).args(["--explorer-inject", &pid.to_string()]).creation_flags(0x0800_0000).status().ok().and_then(|s| s.code());
            match code {
                Some(2) => continue,
                Some(0) => return,
                _ => break,
            }
        }
        // Not loaded: tried again with its next window.
        if let Some(l) = LOADED.lock().unwrap().as_mut() {
            l.remove(&pid);
        }
    });
}

fn class_of(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

fn pid_of(h: HWND) -> u32 {
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
    pid
}

fn is_explorer(pid: u32) -> bool {
    use windows::Win32::System::Threading::*;
    unsafe {
        let Ok(p) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return false };
        let mut buf = [0u16; 260];
        let mut n = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(p, PROCESS_NAME_FORMAT(0), windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok();
        let _ = windows::Win32::Foundation::CloseHandle(p);
        ok && String::from_utf16_lossy(&buf[..n as usize]).to_ascii_lowercase().ends_with("\\explorer.exe")
    }
}

/// #rrggbb → COLORREF (0x00bbggrr).
fn colorref(s: &str) -> Option<u32> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some(((v & 0xff) << 16) | (v & 0xff00) | (v >> 16))
}

fn set_u32(h: HWND, attr: i32, v: u32) {
    if let Err(e) = unsafe { DwmSetWindowAttribute(h, DWMWINDOWATTRIBUTE(attr), &v as *const u32 as *const _, 4) } {
        note(&format!("frame attribute {attr} = {v:#x} refused: {e}"));
    }
}

/// A folder window's or an Explorer dialog's frame, as [explorer] says ("default": as
/// Windows has it). With `reset`, everything back to Windows' own.
fn style(h: HWND, cfg: &crate::config::ExplorerCfg, reset: bool) {
    const DARK: i32 = 20;
    const CORNERS: i32 = 33;
    const BORDER: i32 = 34;
    const CAPTION: i32 = 35;
    const TEXT: i32 = 36;
    const BACKDROP: i32 = 38;
    const DEFAULT_COLOR: u32 = 0xFFFF_FFFF;
    const NO_COLOR: u32 = 0xFFFF_FFFE;
    if reset {
        set_u32(h, CORNERS, 0);
        set_u32(h, BORDER, DEFAULT_COLOR);
        set_u32(h, CAPTION, DEFAULT_COLOR);
        set_u32(h, TEXT, DEFAULT_COLOR);
        set_u32(h, BACKDROP, 0);
        return;
    }
    match cfg.title_bar.as_str() {
        "dark" => set_u32(h, DARK, 1),
        "light" => set_u32(h, DARK, 0),
        _ => {}
    }
    match cfg.corners.as_str() {
        "round" => set_u32(h, CORNERS, 2),
        "small" => set_u32(h, CORNERS, 3),
        "square" => set_u32(h, CORNERS, 1),
        _ => {}
    }
    match cfg.border.as_str() {
        "none" => set_u32(h, BORDER, NO_COLOR),
        "accent" => {
            if let Some(c) = crate::win::system_accent().as_deref().and_then(colorref) {
                set_u32(h, BORDER, c);
            }
        }
        "color" => {
            if let Some(c) = colorref(&cfg.border_color) {
                set_u32(h, BORDER, c);
            }
        }
        _ => {}
    }
    if let Some(c) = colorref(&cfg.caption_color) {
        set_u32(h, CAPTION, c);
    }
    if let Some(c) = colorref(&cfg.caption_text) {
        set_u32(h, TEXT, c);
    }
    match cfg.backdrop.as_str() {
        "none" => set_u32(h, BACKDROP, 1),
        "mica" => set_u32(h, BACKDROP, 2),
        "acrylic" => set_u32(h, BACKDROP, 3),
        "mica_alt" => set_u32(h, BACKDROP, 4),
        _ => {}
    }
}

/// A window to style (and load into): a folder window (true), or a dialog of Explorer's.
fn ours(h: HWND, cfg: &crate::config::ExplorerCfg) -> Option<bool> {
    match class_of(h).as_str() {
        "CabinetWClass" => Some(true),
        "#32770" if cfg.dialogs && is_explorer(pid_of(h)) => Some(false),
        _ => None,
    }
}

unsafe extern "system" fn on_show(_hook: HWINEVENTHOOK, _event: u32, h: HWND, id_object: i32, id_child: i32, _thread: u32, _time: u32) {
    if id_object != 0 || id_child != 0 {
        return;
    }
    let Some(cfg) = STYLE.lock().unwrap().clone() else { return };
    let Some(folder) = ours(h, &cfg) else { return };
    if cfg.frames {
        style(h, &cfg, false);
    }
    if folder && parts_on(&cfg) {
        load_into(pid_of(h));
    }
}

/// Every folder window and Explorer dialog now.
fn windows_now(cfg: &crate::config::ExplorerCfg) -> Vec<(HWND, bool)> {
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> windows::core::BOOL {
        let v = unsafe { &mut *(lp.0 as *mut Vec<HWND>) };
        v.push(h);
        true.into()
    }
    let mut all: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut all as *mut _ as isize));
    }
    all.into_iter().filter_map(|h| ours(h, cfg).map(|f| (h, f))).collect()
}

/// Watches while something's on (the settings looked at every 2 s).
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-explorer".into())
        .spawn(move || unsafe {
            let mut hook: Option<HWINEVENTHOOK> = None;
            let mut last: Option<crate::config::ExplorerCfg> = None;
            let mut rules_written: Option<String> = None;
            SetTimer(None, 0, 2000, None);
            let mut msg = MSG::default();
            loop {
                let cfg = app.state::<SharedConfig>().read().unwrap().explorer.clone();
                if last.as_ref() != Some(&cfg) {
                    let frames = cfg.enabled && cfg.frames;
                    let parts = parts_on(&cfg);
                    // The rules (an empty file: the library gives everything back).
                    let rules = rules_text(&cfg);
                    if rules_written.as_ref() != Some(&rules) {
                        let _ = std::fs::write(rules_path(), &rules);
                        rules_written = Some(rules);
                    }
                    *STYLE.lock().unwrap() = (frames || parts).then(|| cfg.clone());
                    // Frames: every window now, set or given back to Windows.
                    let had_frames = last.as_ref().is_some_and(|l| l.enabled && l.frames);
                    if frames || had_frames {
                        for (h, _) in windows_now(&cfg) {
                            style(h, &cfg, !frames);
                        }
                    }
                    if parts {
                        for (h, folder) in windows_now(&cfg) {
                            if folder {
                                load_into(pid_of(h));
                            }
                        }
                    }
                    match (frames || parts, hook) {
                        (true, None) => {
                            let h = SetWinEventHook(EVENT_OBJECT_SHOW, EVENT_OBJECT_SHOW, None, Some(on_show), 0, 0, WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS);
                            hook = (!h.is_invalid()).then_some(h);
                        }
                        (false, Some(h)) => {
                            let _ = UnhookWinEvent(h);
                            hook = None;
                        }
                        _ => {}
                    }
                    last = Some(cfg);
                }
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    break;
                }
                if msg.message != WM_TIMER {
                    DispatchMessageW(&msg);
                }
            }
        })
        .expect("spawn explorer thread");
}

/// Kasumi's quitting: frames back to Windows' own, hidden parts given back.
pub fn restore() {
    if let Some(cfg) = STYLE.lock().unwrap().clone() {
        if cfg.frames {
            for (h, _) in windows_now(&cfg) {
                style(h, &cfg, true);
            }
        }
    }
    let _ = std::fs::write(rules_path(), "");
}

// --- Folder options -------------------------------------------------------------------------------------
// Windows' own (what Explorer's Folder Options set): the shell state ones through
// SHGetSetSettings, which open windows follow at once; the rest in the registry, for
// windows opened after.

const ADVANCED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced";
const CABINET: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\CabinetState";

/// Shell state flags: (name, SSF mask, in the second bit field, bit).
const SHELL_FLAGS: &[(&str, u32, bool, u32)] = &[
    ("hidden", 0x1, false, 0),
    ("extensions", 0x2, false, 1),
    ("system", 0x4_0000, false, 15),
    ("checkboxes", 0x80_0000, true, 3),
    ("status_bar", 0x400_0000, true, 6),
];

fn shell_state(mask: u32) -> windows::Win32::UI::Shell::SHELLSTATEA {
    let mut ss = windows::Win32::UI::Shell::SHELLSTATEA::default();
    unsafe { windows::Win32::UI::Shell::SHGetSetSettings(Some(&mut ss), windows::Win32::UI::Shell::SSF_MASK(mask), false) };
    ss
}

/// For the settings page: Explorer's folder options now.
#[tauri::command]
pub async fn explorer_options() -> serde_json::Value {
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    let mut out = serde_json::Map::new();
    for &(name, mask, second, bit) in SHELL_FLAGS {
        let ss = shell_state(mask);
        let field = if second { ss._bitfield2 } else { ss._bitfield1 };
        out.insert(name.into(), serde_json::Value::Bool(field >> bit & 1 == 1));
    }
    let dword = |path: &str, name: &str| crate::win::reg_dword(HKEY_CURRENT_USER, path, name);
    out.insert("compact".into(), serde_json::Value::Bool(dword(ADVANCED, "UseCompactMode") == Some(1)));
    out.insert("full_path".into(), serde_json::Value::Bool(dword(CABINET, "FullPath") == Some(1)));
    out.insert("launch_to".into(), serde_json::json!(dword(ADVANCED, "LaunchTo").unwrap_or(2)));
    serde_json::Value::Object(out)
}

/// For the settings page: one of Explorer's folder options set.
#[tauri::command]
pub async fn explorer_option_set(name: String, value: serde_json::Value) -> Result<(), String> {
    if let Some(&(_, mask, second, bit)) = SHELL_FLAGS.iter().find(|f| f.0 == name) {
        let on = value.as_bool().ok_or("not a switch")?;
        let mut ss = shell_state(mask);
        // A packed struct: its fields are copied out and back, not borrowed.
        let field = if second { ss._bitfield2 } else { ss._bitfield1 };
        let field = if on { field | (1 << bit) } else { field & !(1 << bit) };
        if second {
            ss._bitfield2 = field;
        } else {
            ss._bitfield1 = field;
        }
        unsafe { windows::Win32::UI::Shell::SHGetSetSettings(Some(&mut ss), windows::Win32::UI::Shell::SSF_MASK(mask), true) };
        return Ok(());
    }
    match name.as_str() {
        "compact" => crate::win::reg_set_dword(ADVANCED, "UseCompactMode", value.as_bool().ok_or("not a switch")? as u32),
        "full_path" => crate::win::reg_set_dword(CABINET, "FullPath", value.as_bool().ok_or("not a switch")? as u32),
        "launch_to" => crate::win::reg_set_dword(ADVANCED, "LaunchTo", value.as_u64().ok_or("not a number")? as u32),
        _ => return Err(format!("no option {name}")),
    }
    crate::win::broadcast_setting("ShellState");
    Ok(())
}
