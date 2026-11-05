//! Windows' own performance readings for the PC temps widget (no LibreHardwareMonitor
//! needed): CPU load (GetSystemTimes), memory (GlobalMemoryStatusEx), graphics load
//! (the "GPU Engine" performance counters, 3D engines) and the apps using the most
//! CPU right now (each program's processes in this session, added up).

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, FILETIME};
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Performance::{
    PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW, PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::SystemInformation::{GetSystemInfo, GlobalMemoryStatusEx, MEMORYSTATUSEX, SYSTEM_INFO};
use windows::Win32::System::Threading::{GetCurrentProcessId, GetProcessTimes, GetSystemTimes, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE};

/// Programs that keep Windows (and Kasumi) running: listed, but never ended from here.
const PROTECTED: &[&str] = &[
    "explorer.exe", "dwm.exe", "csrss.exe", "winlogon.exe", "wininit.exe", "services.exe", "lsass.exe", "smss.exe", "svchost.exe", "sihost.exe", "ctfmon.exe",
    "fontdrvhost.exe", "taskhostw.exe", "runtimebroker.exe", "searchhost.exe", "startmenuexperiencehost.exe", "shellexperiencehost.exe", "textinputhost.exe",
    "audiodg.exe", "conhost.exe", "kasumi.exe", "system", "registry", "memory compression", "secure system",
];

/// How often the busiest apps are worked out (each time, every process is read).
const TOP_EVERY: Duration = Duration::from_secs(3);

fn ticks(t: FILETIME) -> u64 {
    ((t.dwHighDateTime as u64) << 32) | t.dwLowDateTime as u64
}

/// The graphics engines' load: a performance-counter query kept open.
struct Gpu {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

impl Gpu {
    fn open() -> Option<Gpu> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            if PdhOpenQueryW(PCWSTR::null(), 0, &mut query) != 0 {
                return None;
            }
            let path = crate::win::wide("\\GPU Engine(*engtype_3D)\\Utilization Percentage");
            let mut counter = PDH_HCOUNTER::default();
            if PdhAddEnglishCounterW(query, PCWSTR(path.as_ptr()), 0, &mut counter) != 0 {
                let _ = PdhCloseQuery(query);
                return None;
            }
            let _ = PdhCollectQueryData(query);
            Some(Gpu { query, counter })
        }
    }
    /// The busiest adapter's 3D load since the last reading (percent).
    fn load(&self) -> Option<f64> {
        unsafe {
            if PdhCollectQueryData(self.query) != 0 {
                return None;
            }
            let (mut size, mut count) = (0u32, 0u32);
            let _ = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
            if size == 0 {
                return None;
            }
            let mut buf = vec![0u8; size as usize];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items)) != 0 {
                return None;
            }
            // Instances are per process and engine ("pid_123_luid_..._engtype_3D"): add up per adapter (luid).
            let mut per: HashMap<String, f64> = HashMap::new();
            for i in 0..count as usize {
                let it = &*items.add(i);
                let name = it.szName.to_string().unwrap_or_default();
                let luid = name.split("_luid_").nth(1).and_then(|s| s.split("_phys").next()).unwrap_or("").to_string();
                *per.entry(luid).or_default() += it.FmtValue.Anonymous.doubleValue;
            }
            Some(per.values().cloned().fold(0.0, f64::max).clamp(0.0, 100.0))
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}

#[derive(Default)]
pub struct Perf {
    /// CPU: idle and busy ticks at the last reading.
    cpu: Option<(u64, u64)>,
    gpu: Option<Gpu>,
    gpu_tried: bool,
    /// Each process's CPU ticks at the last pass, and when that was.
    procs: HashMap<u32, u64>,
    last_top: Option<Instant>,
    top: Value,
    /// Programs' names, by path.
    names: HashMap<String, String>,
    cpu_name: Option<String>,
}

impl Perf {
    /// Fills in what LibreHardwareMonitor didn't give (or everything, without it):
    /// cpuLoad, gpuLoad, ramUsed / ramAvailable (GB), cpuName; and `top` (with `top_apps`).
    pub fn fill(&mut self, out: &mut Map<String, Value>, top_apps: bool) {
        let missing = |out: &Map<String, Value>, k: &str| out.get(k).is_none_or(|v| v.is_null());
        if missing(out, "cpuLoad") {
            out.insert("cpuLoad".into(), json!(self.cpu_load()));
        }
        if missing(out, "gpuLoad") {
            if !self.gpu_tried {
                self.gpu_tried = true;
                self.gpu = Gpu::open();
            }
            out.insert("gpuLoad".into(), json!(self.gpu.as_ref().and_then(Gpu::load)));
        }
        if missing(out, "ramUsed") || missing(out, "ramAvailable") {
            let mut m = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
            if unsafe { GlobalMemoryStatusEx(&mut m) }.is_ok() {
                const GB: f64 = 1024.0 * 1024.0 * 1024.0;
                out.insert("ramUsed".into(), json!((m.ullTotalPhys - m.ullAvailPhys) as f64 / GB));
                out.insert("ramAvailable".into(), json!(m.ullAvailPhys as f64 / GB));
            }
        }
        if missing(out, "cpuName") {
            let name = self.cpu_name.get_or_insert_with(|| {
                crate::win::reg_string(windows::Win32::System::Registry::HKEY_LOCAL_MACHINE, "HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0", "ProcessorNameString")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            });
            out.insert("cpuName".into(), json!(name));
        }
        if top_apps {
            if self.last_top.is_none_or(|t| t.elapsed() >= TOP_EVERY) {
                self.top = self.top_apps();
            }
            out.insert("top".into(), self.top.clone());
        } else {
            self.procs.clear();
            self.last_top = None;
        }
    }

    fn cpu_load(&mut self) -> Option<f64> {
        let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
        unsafe { GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user)) }.ok()?;
        let (idle, total) = (ticks(idle), ticks(kernel) + ticks(user));
        let load = self.cpu.map(|(i0, t0)| {
            let (di, dt) = (idle.saturating_sub(i0) as f64, total.saturating_sub(t0) as f64);
            if dt > 0.0 { (100.0 * (1.0 - di / dt)).clamp(0.0, 100.0) } else { 0.0 }
        });
        self.cpu = Some((idle, total));
        load
    }

    /// The five programs using the most CPU since the last pass (then memory): name,
    /// program file, CPU percent of the whole machine, memory in bytes, whether it may be ended.
    fn top_apps(&mut self) -> Value {
        let now = Instant::now();
        let elapsed = self.last_top.map(|t| now.duration_since(t).as_secs_f64());
        self.last_top = Some(now);
        let cpus = unsafe {
            let mut si = SYSTEM_INFO::default();
            GetSystemInfo(&mut si);
            si.dwNumberOfProcessors.max(1) as f64
        };
        let session = |pid: u32| {
            let mut s = 0u32;
            unsafe { ProcessIdToSessionId(pid, &mut s) }.ok().map(|_| s)
        };
        let mine = session(unsafe { GetCurrentProcessId() });
        // exe → (cpu ticks used since last pass, memory, a path)
        let mut apps: HashMap<String, (u64, u64, Option<String>)> = HashMap::new();
        let mut seen: HashMap<u32, u64> = HashMap::new();
        unsafe {
            let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return json!([]) };
            let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
            let mut ok = Process32FirstW(snap, &mut e).is_ok();
            while ok {
                let pid = e.th32ProcessID;
                let exe = crate::win::wide_to_string(&e.szExeFile).to_lowercase();
                if pid != 0 && session(pid) == mine {
                    if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        let (mut c, mut x, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
                        let used = if GetProcessTimes(h, &mut c, &mut x, &mut k, &mut u).is_ok() { ticks(k) + ticks(u) } else { 0 };
                        let mut pmc = PROCESS_MEMORY_COUNTERS_EX::default();
                        let mem = if GetProcessMemoryInfo(h, &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32).is_ok() {
                            pmc.PrivateUsage as u64
                        } else {
                            0
                        };
                        let _ = CloseHandle(h);
                        let delta = self.procs.get(&pid).map_or(0, |was| used.saturating_sub(*was));
                        seen.insert(pid, used);
                        let a = apps.entry(exe).or_insert((0, 0, None));
                        a.0 += delta;
                        a.1 += mem;
                        if a.2.is_none() {
                            a.2 = crate::win::process_path(pid);
                        }
                    }
                }
                ok = Process32NextW(snap, &mut e).is_ok();
            }
            let _ = CloseHandle(snap);
        }
        self.procs = seen;
        let Some(elapsed) = elapsed.filter(|e| *e > 0.0) else { return json!([]) };
        let mut list: Vec<(String, u64, u64, Option<String>)> = apps.into_iter().map(|(exe, (c, m, p))| (exe, c, m, p)).collect();
        list.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)));
        list.into_iter()
            .take(5)
            .map(|(exe, c, mem, path)| {
                let name = match &path {
                    Some(p) => self.names.entry(p.clone()).or_insert_with(|| crate::win::file_description(p).unwrap_or_else(|| exe.trim_end_matches(".exe").to_string())).clone(),
                    None => exe.trim_end_matches(".exe").to_string(),
                };
                // Ticks are 100 ns.
                let cpu = (c as f64 / 1e7 / elapsed / cpus * 100.0).clamp(0.0, 100.0);
                json!({ "name": name, "exe": exe, "cpu": cpu, "mem": mem, "end": !PROTECTED.contains(&exe.as_str()) })
            })
            .collect()
    }
}

/// Ends every process of a program (`exe`, as `top` lists it) in this session, as
/// Task Manager's End task does. Programs that keep Windows running are left alone.
pub fn end_app(exe: &str) {
    let exe = exe.to_lowercase();
    if PROTECTED.contains(&exe.as_str()) {
        return;
    }
    let session = |pid: u32| {
        let mut s = 0u32;
        unsafe { ProcessIdToSessionId(pid, &mut s) }.ok().map(|_| s)
    };
    unsafe {
        let mine = session(GetCurrentProcessId());
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        let mut ended = 0;
        while ok {
            if crate::win::wide_to_string(&e.szExeFile).to_lowercase() == exe && session(e.th32ProcessID) == mine {
                if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, e.th32ProcessID) {
                    if TerminateProcess(h, 1).is_ok() {
                        ended += 1;
                    }
                    let _ = CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
        eprintln!("[kasumi] performance: ended {exe} ({ended} process(es))");
    }
}
