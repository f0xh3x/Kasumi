//! File copies and moves ([osd] file_progress): Explorer's progress window
//! ("OperationStatusWindow") carries the overall percentage in its title ("42%
//! complete"); while it's open, that shows as a level pop-up. It only reads the
//! title: nothing of Explorer's is taken over (apps' own taskbar progress isn't
//! reachable without that).

use std::time::Duration;

use tauri::{AppHandle, Manager};
use windows::core::{w, PCWSTR};
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, GetWindowTextW};

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
            let mut last: Option<f32> = None;
            loop {
                let on = {
                    let cfg = app.state::<SharedConfig>();
                    let cfg = cfg.read().unwrap();
                    cfg.osd.enabled && cfg.osd.file_progress
                };
                let window = if on { unsafe { FindWindowW(w!("OperationStatusWindow"), PCWSTR::null()) }.ok() } else { None };
                match window {
                    Some(h) => {
                        let mut buf = [0u16; 256];
                        let n = unsafe { GetWindowTextW(h, &mut buf) } as usize;
                        if let Some(p) = percent(&String::from_utf16_lossy(&buf[..n])) {
                            if last != Some(p) {
                                last = Some(p);
                                super::show_progress(&app, p / 100.0, 3000);
                            }
                        }
                        std::thread::sleep(Duration::from_millis(300));
                    }
                    None => {
                        // Finished (nearly there when it closed): a last full bar.
                        if last.is_some_and(|p| p >= 90.0) {
                            super::show_progress(&app, 1.0, 1200);
                        }
                        last = None;
                        std::thread::sleep(Duration::from_secs(1));
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
