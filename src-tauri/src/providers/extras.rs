//! Small pollers sharing one thread, each only while its widget is placed:
//! - "focus": the window in front (title, app),
//! - "netspeed": bytes per second down / up over the hardware network adapters,
//! - "recycle": items and size in the recycle bin,
//! - "privacy": apps using the microphone, camera or location right now.

use serde_json::json;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};
use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
use windows::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ,
};
use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId};

fn placed(app: &AppHandle, ty: &str) -> bool {
    let cfg = app.state::<super::SharedConfig>();
    let cfg = cfg.read().unwrap();
    cfg.widget_placed(ty)
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("statusnotch-extras".into())
        .spawn(move || run(app))
        .expect("spawn extras thread");
}

fn run(app: AppHandle) {
    let mut net = NetSpeed::default();
    let mut last_net = Instant::now() - Duration::from_secs(10);
    let mut last_bin = last_net;
    let mut last_privacy = last_net;
    loop {
        let now = Instant::now();
        if placed(&app, "focus") {
            super::publish(&app, "focus", focus());
        }
        if placed(&app, "netspeed") && now - last_net >= Duration::from_secs(1) {
            last_net = now;
            if let Some((down, up)) = net.sample() {
                super::publish(&app, "netspeed", json!({ "down": down, "up": up }));
            }
        } else if !placed(&app, "netspeed") {
            net = NetSpeed::default();
        }
        if placed(&app, "recycle") && now - last_bin >= Duration::from_secs(3) {
            last_bin = now;
            super::publish(&app, "recycle", recycle_bin());
        }
        if placed(&app, "privacy") && now - last_privacy >= Duration::from_secs(1) {
            last_privacy = now;
            super::publish(&app, "privacy", privacy());
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

/// The window in front: its title and its program's name (none for our own
/// windows or the desktop).
fn focus() -> serde_json::Value {
    unsafe {
        let h: HWND = GetForegroundWindow();
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if h.is_invalid() || pid == std::process::id() {
            return json!(null);
        }
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(h, &mut buf) as usize;
        let title = String::from_utf16_lossy(&buf[..n]);
        let app = crate::win::process_name(pid).unwrap_or_default();
        if title.is_empty() || matches!(app.to_lowercase().as_str(), "explorer" if title == "Program Manager") {
            return json!(null);
        }
        json!({ "title": title, "app": app })
    }
}

/// Totals of the hardware network adapters; rates from the change since the last sample.
#[derive(Default)]
struct NetSpeed {
    last: Option<(Instant, u64, u64)>,
}

impl NetSpeed {
    fn sample(&mut self) -> Option<(f64, f64)> {
        let (inb, outb) = net_totals()?;
        let now = Instant::now();
        let prev = self.last.replace((now, inb, outb));
        let (t, i, o) = prev?;
        let secs = (now - t).as_secs_f64().max(0.001);
        Some((inb.saturating_sub(i) as f64 / secs, outb.saturating_sub(o) as f64 / secs))
    }
}

fn net_totals() -> Option<(u64, u64)> {
    unsafe {
        let mut table: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut table).is_err() || table.is_null() {
            return None;
        }
        let rows = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        let (mut i, mut o) = (0u64, 0u64);
        for r in rows {
            // Hardware adapters only (virtual switches would count traffic twice), and up.
            let hardware = r.InterfaceAndOperStatusFlags._bitfield & 1 != 0;
            if hardware && r.OperStatus.0 == 1 {
                i += r.InOctets;
                o += r.OutOctets;
            }
        }
        FreeMibTable(table as *const _);
        Some((i, o))
    }
}

fn recycle_bin() -> serde_json::Value {
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe {
        if SHQueryRecycleBinW(PCWSTR::null(), &mut info).is_err() {
            return json!(null);
        }
    }
    json!({ "items": info.i64NumItems, "bytes": info.i64Size })
}

/// Empties the recycle bin (Windows asks for confirmation itself).
#[tauri::command]
pub async fn recycle_empty() -> Result<(), String> {
    use windows::Win32::UI::Shell::SHEmptyRecycleBinW;
    unsafe { SHEmptyRecycleBinW(None, PCWSTR::null(), 0) }.map_err(|e| e.to_string())
}

/// Apps using each device now: Windows' privacy records (ConsentStore) keep, per
/// app, when it last started / stopped using it; a stop of 0 means "still using".
fn privacy() -> serde_json::Value {
    // A record can say "in use" long after the app quit or crashed: only running
    // programs count (Store apps are taken at their word).
    let running = running_exes();
    let base = r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";
    let mut out = serde_json::Map::new();
    for (key, dev) in [("microphone", "mic"), ("webcam", "camera"), ("location", "location")] {
        let mut apps: Vec<String> = Vec::new();
        let path = format!(r"{base}\{key}");
        for sub in subkeys(HKEY_CURRENT_USER, &path) {
            if sub == "NonPackaged" {
                for exe in subkeys(HKEY_CURRENT_USER, &format!(r"{path}\NonPackaged")) {
                    // "C:#Program Files#App#App.exe"
                    let name = exe.rsplit('#').next().unwrap_or(&exe);
                    if running.contains(&name.to_lowercase()) && in_use(&format!(r"{path}\NonPackaged\{exe}")) {
                        apps.push(name.trim_end_matches(".exe").trim_end_matches(".EXE").to_string());
                    }
                }
            } else if in_use(&format!(r"{path}\{sub}")) {
                // "Microsoft.WindowsCamera_8wekyb3d8bbwe" -> "WindowsCamera"
                let name = sub.split('_').next().unwrap_or(&sub);
                apps.push(name.rsplit('.').next().unwrap_or(name).to_string());
            }
        }
        apps.sort();
        apps.dedup();
        out.insert(dev.into(), json!(apps));
    }
    serde_json::Value::Object(out)
}

/// File names (lowercase) of the running programs.
fn running_exes() -> std::collections::HashSet<String> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    let mut out = std::collections::HashSet::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let n = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
            out.insert(String::from_utf16_lossy(&e.szExeFile[..n]).to_lowercase());
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn subkeys(root: HKEY, path: &str) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let mut key = HKEY::default();
        let p = wide(path);
        if RegOpenKeyExW(root, PCWSTR(p.as_ptr()), None, KEY_READ, &mut key).is_err() {
            return out;
        }
        let mut i = 0;
        loop {
            let mut buf = [0u16; 512];
            let mut len = buf.len() as u32;
            if RegEnumKeyExW(key, i, Some(windows::core::PWSTR(buf.as_mut_ptr())), &mut len, None, None, None, None).is_err() {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
            i += 1;
        }
        let _ = RegCloseKey(key);
    }
    out
}

fn in_use(path: &str) -> bool {
    let read = |name: &str| -> Option<u64> {
        unsafe {
            let mut key = HKEY::default();
            let p = wide(path);
            if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(p.as_ptr()), None, KEY_READ, &mut key).is_err() {
                return None;
            }
            let n = wide(name);
            let mut v = 0u64;
            let mut size = 8u32;
            let ok = RegQueryValueExW(key, PCWSTR(n.as_ptr()), None, None, Some(&mut v as *mut u64 as *mut u8), Some(&mut size)).is_ok();
            let _ = RegCloseKey(key);
            ok.then_some(v)
        }
    };
    matches!((read("LastUsedTimeStart"), read("LastUsedTimeStop")), (Some(s), Some(0)) if s != 0)
}
