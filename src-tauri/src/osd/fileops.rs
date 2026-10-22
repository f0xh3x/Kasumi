//! File copies and moves ([osd] file_progress): Explorer's progress window
//! ("OperationStatusWindow") carries the overall percentage in its title ("42%
//! complete"); while it's open, that shows as a level pop-up. It only reads the
//! title: nothing of Explorer's is taken over (apps' own taskbar progress isn't
//! reachable without that).

use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowTextW, IsWindowVisible};

use crate::providers::SharedConfig;

/// The first percentage in a title ("42% complete", "%42 tamamlandı", "42 %").
fn percent(title: &str) -> Option<f32> {
    let chars: Vec<char> = title.chars().collect();
    let pct = chars.iter().position(|&c| c == '%')?;
    let digits = |range: &mut dyn Iterator<Item = usize>| -> String { range.map(|i| chars[i]).take_while(|c| c.is_ascii_digit()).collect() };
    // Before the sign (skipping a space), else after it.
    let start = if pct > 0 && chars[pct - 1] == ' ' { pct.saturating_sub(1) } else { pct };
    let before: String = digits(&mut (0..start).rev()).chars().rev().collect();
    let n = if before.is_empty() { digits(&mut (pct + 1..chars.len())) } else { before };
    n.parse::<f32>().ok().filter(|v| *v <= 100.0)
}

pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-fileops".into())
        .spawn(move || {
            // The copy window: since when it's open, the last percentage shown and when.
            let mut since: Option<Instant> = None;
            let mut shown: Option<(f32, Instant)> = None;
            loop {
                let on = {
                    let cfg = app.state::<SharedConfig>();
                    let cfg = cfg.read().unwrap();
                    cfg.osd.enabled && cfg.osd.file_progress
                };
                let window = if on { unsafe { FindWindowW(w!("OperationStatusWindow"), PCWSTR::null()) }.ok().filter(|h| unsafe { IsWindowVisible(*h) }.as_bool()) } else { None };
                match window {
                    Some(h) => {
                        let now = Instant::now();
                        let opened = *since.get_or_insert(now);
                        let mut buf = [0u16; 256];
                        let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
                        // Half a second in (not with the window opening, and no flash for tiny
                        // copies), then up for as long as it runs: on each change, and again
                        // every second while the number stands still.
                        if let Some(p) = percent(&String::from_utf16_lossy(&buf[..n])) {
                            let due = shown.is_none_or(|(last, at)| last != p || now.duration_since(at) >= Duration::from_secs(1));
                            if now.duration_since(opened) >= Duration::from_millis(500) && due {
                                shown = Some((p, now));
                                super::show_progress(&app, p / 100.0, 2500);
                            }
                        }
                        std::thread::sleep(Duration::from_millis(300));
                    }
                    None => {
                        // Done: a full bar for a moment, then it goes back (a calm moment after
                        // the window has closed).
                        if shown.is_some() {
                            super::show_progress(&app, 1.0, 1200);
                        }
                        since = None;
                        shown = None;
                        std::thread::sleep(Duration::from_millis(if on { 500 } else { 2000 }));
                    }
                }
            }
        })
        .expect("spawn file progress thread");
}

#[cfg(test)]
mod tests {
    use super::percent;

    #[test]
    fn reads_percentages() {
        assert_eq!(percent("42% complete"), Some(42.0));
        assert_eq!(percent("Baigta 7 %"), Some(7.0));
        assert_eq!(percent("%85 tamamlandı"), Some(85.0));
        assert_eq!(percent("Copying"), None);
    }
}
