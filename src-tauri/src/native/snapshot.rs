//! `kasumi.exe --native-snapshot <dir> [--data <file.json>]`: draws every native
//! surface off screen in a few states and saves them as PNGs, then exits. For
//! checking the native bars without showing anything: it builds the layouts from
//! the live config and the real monitors, and never claims the single instance
//! (the running Kasumi is left alone). `--data` feeds widgets values as the
//! providers publish them ({ "key": value, ... }). A report of what was measured
//! goes to `<dir>/snapshot.txt`.

use std::collections::HashMap;
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use super::gfx::Gfx;
use super::surface::Surface;
use crate::engine::{layout_for, monitor_matches};

pub fn run(args: &[String]) {
    let value = |f: &str| args.iter().position(|a| a == f).and_then(|i| args.get(i + 1)).cloned();
    let Some(dir) = value("--native-snapshot") else { return };
    if let Some(secs) = value("--live").and_then(|s| s.parse::<u64>().ok()) {
        live(Path::new(&dir), secs);
        return;
    }
    let dir = Path::new(&dir).to_path_buf();
    let _ = std::fs::create_dir_all(&dir);
    let mut log = std::fs::File::create(dir.join("snapshot.txt")).ok();
    let mut say = |s: String| {
        if let Some(f) = log.as_mut() {
            let _ = writeln!(f, "{s}");
        }
    };
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED);
    }
    let cfg = match crate::config::load() {
        Ok(c) => c,
        Err(e) => {
            say(format!("config error: {e}"));
            return;
        }
    };
    let store: HashMap<String, Value> = value("--data")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<serde_json::Map<String, Value>>(&t).ok())
        .map(|m| m.into_iter().collect())
        .unwrap_or_default();
    let gfx = match Gfx::new() {
        Ok(g) => g,
        Err(e) => {
            say(format!("Direct2D unavailable: {e}"));
            return;
        }
    };
    let monitors = crate::win::monitors();
    for (i, m) in monitors.iter().enumerate() {
        let rule = cfg.monitors.iter().find(|r| monitor_matches(&r.matcher, m, i));
        if rule.is_some_and(|r| !r.enabled) {
            continue;
        }
        for (id, s) in cfg.surface_list() {
            if !cfg.surface_on(rule, &id, &s) {
                continue;
            }
            let label = format!("overlay-{}-{id}", m.device.trim_start_matches(['\\', '.']));
            let mut layout = layout_for(&cfg, &label, m, rule, &id, &[], &HashMap::new());
            let mut surface = match Surface::new(&gfx, &label, m.clone(), layout.clone(), &store, true) {
                Ok(s) => s,
                Err(e) => {
                    say(format!("{label}: {e}"));
                    continue;
                }
            };
            // As the engine would: the measured widths place the notches.
            let mut measured = HashMap::new();
            for _ in 0..3 {
                let Some(w) = surface.take_widths() else { break };
                measured.extend(w);
                layout = layout_for(&cfg, &label, m, rule, &id, &[], &measured);
                surface.update_positions(&gfx, layout.notches.clone());
            }
            say(format!("{label}: {} x {} @ {:.2}, widgets {:?}", layout.width, layout.height, m.scale, surface.widget_names()));
            for n in &layout.notches {
                say(format!("  notch {} x {:.1} w {:.1} h {:.1} open ({:.1}, {:.1}, {:.1}) widgets {:?}", n.id, n.x, n.width, n.height, n.open_x, n.open_width, n.open_height, n.widgets));
            }
            let mut shots: Vec<(String, bool, Option<String>, f64)> = vec![("closed".into(), false, None, 0.0), ("open".into(), true, None, 0.0)];
            for name in surface.widget_names() {
                if super::widgets::panel_for(&name).is_some() {
                    shots.push((format!("{name}-half"), true, Some(name.clone()), 0.5));
                    shots.push((name.clone(), true, Some(name), 1.0));
                }
            }
            for (state, open, panel, p) in shots {
                surface.pose(&gfx, open, panel.as_deref(), p);
                if let Err(e) = surface.render(&gfx) {
                    say(format!("  {state}: {e}"));
                    continue;
                }
                let (px, w, h) = surface.pixels();
                let file = dir.join(format!("{label}-{state}.png").replace('#', "_"));
                save_png(&file, px, w, h);
            }
        }
    }
}

/// `--live <seconds>`: the real native thread with every surface off screen,
/// opened and closed once a second for the first half, then left alone; reports
/// the frames drawn and what the process costs (private memory, CPU while idle).
fn live(dir: &Path, secs: u64) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX};
    use windows::Win32::System::Threading::GetCurrentProcess;
    use windows::Win32::System::WindowsProgramming::QueryProcessCycleTime;

    let _ = std::fs::create_dir_all(dir);
    let mut out = String::new();
    let private_mb = || {
        let mut pmc = PROCESS_MEMORY_COUNTERS_EX::default();
        unsafe {
            let _ = GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc as *mut _ as *mut PROCESS_MEMORY_COUNTERS, std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32);
        }
        pmc.PrivateUsage as f64 / (1024.0 * 1024.0)
    };
    let cycles = || {
        let mut c = 0u64;
        unsafe {
            let _ = QueryProcessCycleTime(GetCurrentProcess(), &mut c);
        }
        c
    };
    out += &format!("before start: {:.1} MB\n", private_mb());
    let cfg = match crate::config::load() {
        Ok(c) => c,
        Err(e) => {
            let _ = std::fs::write(dir.join("live.txt"), format!("config error: {e}"));
            return;
        }
    };
    super::start_headless();
    let mut labels = Vec::new();
    for (i, m) in crate::win::monitors().iter().enumerate() {
        let rule = cfg.monitors.iter().find(|r| monitor_matches(&r.matcher, m, i));
        if rule.is_some_and(|r| !r.enabled) {
            continue;
        }
        for (id, s) in cfg.surface_list() {
            if !cfg.surface_on(rule, &id, &s) {
                continue;
            }
            let label = format!("overlay-{}-{id}", m.device.trim_start_matches(['\\', '.']));
            let layout = layout_for(&cfg, &label, m, rule, &id, &[], &HashMap::new());
            let hwnd = super::create(&label, m, &layout);
            out += &format!("{label}: hwnd {hwnd:#x}\n");
            labels.push(label);
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    out += &format!("surfaces up: {:.1} MB\n", private_mb());
    let half = Duration::from_secs(secs / 2);
    let start = Instant::now();
    let mut open = false;
    while start.elapsed() < half {
        open = !open;
        for l in &labels {
            super::post(super::Msg::State(l.clone(), open));
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    for l in &labels {
        super::post(super::Msg::State(l.clone(), false));
    }
    std::thread::sleep(Duration::from_secs(1));
    let frames = super::FRAMES.load(Ordering::Relaxed);
    out += &format!("animated: {frames} surface frames in {:.1} s ({} opens and closes)\n", half.as_secs_f64(), half.as_secs());
    out += &format!("after animating: {:.1} MB\n", private_mb());
    let (c0, t0) = (cycles(), Instant::now());
    std::thread::sleep(Duration::from_secs(secs - secs / 2));
    let rate = (cycles() - c0) as f64 / t0.elapsed().as_secs_f64();
    out += &format!("idle: {:.1} M cycles/s, {} frames, {:.1} MB\n", rate / 1e6, super::FRAMES.load(Ordering::Relaxed) - frames, private_mb());
    let _ = std::fs::write(dir.join("live.txt"), out);
}

/// Premultiplied BGRA over mid grey, as an uncompressed PNG.
fn save_png(path: &Path, px: &[u8], w: i32, h: i32) {
    let (w, h) = (w as usize, h as usize);
    let mut raw = Vec::with_capacity((w * 3 + 1) * h);
    for row in 0..h {
        raw.push(0u8);
        for p in px[row * w * 4..(row + 1) * w * 4].chunks_exact(4) {
            let a = p[3] as u32;
            let over = |c: u8| (c as u32 + 128 * (255 - a) / 255).min(255) as u8;
            raw.extend_from_slice(&[over(p[2]), over(p[1]), over(p[0])]);
        }
    }
    // zlib with stored (uncompressed) deflate blocks.
    let mut z = vec![0x78, 0x01];
    for (i, chunk) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(last as u8);
        z.extend_from_slice(&(chunk.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(chunk.len() as u16)).to_le_bytes());
        z.extend_from_slice(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for &x in &raw {
        a = (a + x as u32) % 65521;
        b = (b + a) % 65521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut chunk = |kind: &[u8; 4], data: &[u8]| {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut c = kind.to_vec();
        c.extend_from_slice(data);
        out.extend_from_slice(&c);
        out.extend_from_slice(&crc32(&c).to_be_bytes());
    };
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(b"IHDR", &ihdr);
    chunk(b"IDAT", &z);
    chunk(b"IEND", &[]);
    let _ = std::fs::write(path, out);
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}
