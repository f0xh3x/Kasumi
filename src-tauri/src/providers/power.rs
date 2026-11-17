//! Quick settings' power box ([widgets.quick] power, the Power mode tile): Windows'
//! power plans (powrprof), its power mode (Settings › Power › Power mode: the plan's
//! "overlay", only while Balanced is the plan) and Energy saver (Windows 11 24H2+).
//!
//! The power mode and Energy saver have no documented API: the power mode goes
//! through powrprof's overlay functions (what Settings and powercfg use), Energy
//! saver through the notification behind Settings' "Always use energy saver" (found
//! by G-Helper; no administrator needed), read back from the registry. Both are
//! looked up at run time, so a Windows without them only hides that part.

use serde_json::{json, Value};
use std::ffi::c_void;
use windows::core::GUID;
use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

#[link(name = "powrprof")]
extern "system" {
    fn PowerGetActiveScheme(root: *mut c_void, active: *mut *mut GUID) -> u32;
    fn PowerSetActiveScheme(root: *mut c_void, scheme: *const GUID) -> u32;
    fn PowerEnumerate(root: *mut c_void, scheme: *const GUID, sub: *const GUID, access: u32, index: u32, buf: *mut u8, size: *mut u32) -> u32;
    fn PowerReadFriendlyName(root: *mut c_void, scheme: *const GUID, sub: *const GUID, setting: *const GUID, buf: *mut u8, size: *mut u32) -> u32;
}

/// PowerEnumerate's ACCESS_SCHEME: the plans.
const ACCESS_SCHEME: u32 = 16;
/// The Balanced plan: the only one Windows offers the power mode for.
const BALANCED: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
/// The power modes, as Settings names them (Best power efficiency, Balanced, Best performance).
pub const MODES: [(&str, GUID); 3] = [
    ("efficiency", GUID::from_u128(0x961cc777_2547_4f9d_8174_7d86181b8a7a)),
    ("balanced", GUID::zeroed()),
    ("performance", GUID::from_u128(0xded574b5_45a0_4f42_8737_46345c09c238)),
];
/// The notification Settings' "Always use energy saver" publishes: 1 on, 2 off.
const ENERGY_SAVER_WNF: u64 = 0x41C6_013D_A3BC_3075;

/// A function of a system DLL, looked up by name (None where this Windows lacks it).
fn proc(dll: &str, name: &str) -> Option<unsafe extern "system" fn() -> isize> {
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    let dll = crate::win::wide(dll);
    let name = std::ffi::CString::new(name).ok()?;
    unsafe {
        let module = LoadLibraryW(windows::core::PCWSTR(dll.as_ptr())).ok()?;
        GetProcAddress(module, windows::core::PCSTR(name.as_ptr().cast()))
    }
}

fn active_plan() -> Option<GUID> {
    let mut p: *mut GUID = std::ptr::null_mut();
    unsafe {
        if PowerGetActiveScheme(std::ptr::null_mut(), &mut p) != 0 || p.is_null() {
            return None;
        }
        let g = *p;
        windows::Win32::Foundation::LocalFree(Some(windows::Win32::Foundation::HLOCAL(p.cast())));
        Some(g)
    }
}

fn plan_name(id: &GUID) -> String {
    let mut size = 0u32;
    let null = std::ptr::null();
    unsafe {
        PowerReadFriendlyName(std::ptr::null_mut(), id, null, null, std::ptr::null_mut(), &mut size);
        let mut buf = vec![0u8; size as usize];
        if size == 0 || PowerReadFriendlyName(std::ptr::null_mut(), id, null, null, buf.as_mut_ptr(), &mut size) != 0 {
            return String::new();
        }
        let wide: Vec<u16> = buf.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        crate::win::wide_to_string(&wide)
    }
}

/// The power plans: (id, name), in Windows' order.
fn plans() -> Vec<(GUID, String)> {
    let mut out = Vec::new();
    for index in 0.. {
        let mut id = GUID::zeroed();
        let mut size = std::mem::size_of::<GUID>() as u32;
        let null = std::ptr::null();
        let r = unsafe { PowerEnumerate(std::ptr::null_mut(), null, null, ACCESS_SCHEME, index, (&mut id as *mut GUID).cast(), &mut size) };
        if r != 0 {
            break;
        }
        out.push((id, plan_name(&id)));
    }
    out
}

/// The power mode in effect (None where this Windows has none).
fn mode() -> Option<GUID> {
    let f = proc("powrprof.dll", "PowerGetEffectiveOverlayScheme")?;
    let get: unsafe extern "system" fn(*mut GUID) -> u32 = unsafe { std::mem::transmute(f) };
    let mut g = GUID::zeroed();
    (unsafe { get(&mut g) } == 0).then_some(g)
}

/// Energy saver on (None before Windows 11 24H2, which has no Energy saver to switch).
fn saver() -> Option<bool> {
    let build: u32 = crate::win::reg_string(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion", "CurrentBuildNumber")?.parse().ok()?;
    if build < 26100 {
        return None;
    }
    match crate::win::reg_dword(HKEY_LOCAL_MACHINE, r"SYSTEM\CurrentControlSet\Control\Power", "EnergySaverState") {
        Some(v) => Some(v == 1),
        // Not written yet: what the battery status says.
        None => Some(crate::providers::system::saver_on()),
    }
}

/// `{"saver": bool|null, "mode": "efficiency"|"balanced"|"performance"|""|null,
/// "plans": [{"id", "name", "active"}]}`. The mode is null while the plan isn't
/// Balanced (Windows offers none then), "" when it's one Settings doesn't list.
pub fn state() -> Value {
    let active = active_plan();
    let plans: Vec<Value> = plans()
        .into_iter()
        .map(|(id, name)| json!({ "id": format!("{id:?}"), "name": name, "active": Some(id) == active }))
        .collect();
    let mode = (active == Some(BALANCED)).then(mode).flatten().map(|g| MODES.iter().find(|m| m.1 == g).map_or("", |m| m.0));
    json!({ "saver": saver(), "mode": mode, "plans": plans })
}

/// Makes a plan (its id as `state` lists it) the active one.
pub fn set_plan(id: &str) -> Result<(), String> {
    let g = GUID::try_from(id).map_err(|_| format!("not a plan id: {id}"))?;
    match unsafe { PowerSetActiveScheme(std::ptr::null_mut(), &g) } {
        0 => Ok(()),
        e => Err(format!("power plan: error {e}")),
    }
}

/// Sets the power mode ("efficiency", "balanced", "performance").
pub fn set_mode(name: &str) -> Result<(), String> {
    let g = MODES.iter().find(|m| m.0 == name).ok_or("unknown power mode")?.1;
    let f = proc("powrprof.dll", "PowerSetActiveOverlayScheme").ok_or("no power mode on this Windows")?;
    let set: unsafe extern "system" fn(GUID) -> u32 = unsafe { std::mem::transmute(f) };
    match unsafe { set(g) } {
        0 => Ok(()),
        e => Err(format!("power mode: error {e}")),
    }
}

/// Turns Energy saver on or off (as Settings' "Always use energy saver").
pub fn set_saver(on: bool) -> Result<(), String> {
    let f = proc("ntdll.dll", "RtlPublishWnfStateData").ok_or("no Energy saver switch on this Windows")?;
    let publish: unsafe extern "system" fn(u64, *const c_void, *const c_void, u32, *const c_void) -> i32 = unsafe { std::mem::transmute(f) };
    let value: u32 = if on { 1 } else { 2 };
    match unsafe { publish(ENERGY_SAVER_WNF, std::ptr::null(), (&value as *const u32).cast(), 4, std::ptr::null()) } {
        0 => Ok(()),
        e => Err(format!("energy saver: status {e:#x}")),
    }
}

#[cfg(test)]
mod tests {
    /// This PC's plans, power mode and Energy saver, read only:
    /// `cargo test power_read -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn power_read() {
        println!("{:#}", super::state());
    }
}
