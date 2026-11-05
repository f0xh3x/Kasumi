//! Watched folders, polled every two seconds while something wants them:
//! - "screenshots": the newest three in the Screenshots folder, with thumbnails, for
//!   the clipboard panel ([widgets.clipboard] screenshots); a click copies one
//!   (`copy_picture`);
//! - finished downloads: a pop-up saying so ([osd] downloads), once a new file in
//!   Downloads has stopped growing (browsers' partial files are left out).

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Manager};
use windows::core::{Interface, GUID, HSTRING};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HGDIOBJ};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::UI::Shell::{IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SHGetKnownFolderPath, KF_FLAG_DEFAULT, SIIGBF_BIGGERSIZEOK, SIIGBF_THUMBNAILONLY};

/// FOLDERID_Screenshots and FOLDERID_Downloads.
const SCREENSHOTS: GUID = GUID::from_u128(0xb7bede81_df94_4682_a7d8_57a52620b86f);
const DOWNLOADS: GUID = GUID::from_u128(0x374de290_123f_4565_9164_39c4925e467b);
/// Partial downloads (Chrome / Edge, Firefox, others) and temporary files.
const PARTIAL: &[&str] = &["crdownload", "part", "partial", "download", "tmp", "opdownload"];

fn known_folder(id: &GUID) -> Option<PathBuf> {
    unsafe {
        let p = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const _));
        s.map(PathBuf::from)
    }
}

/// Files in a folder (not sub-folders): path, size, modified.
fn files(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    rd.flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(UNIX_EPOCH)))
        })
        .collect()
}

/// A picture file's thumbnail as Explorer makes it (RGBA, up to `px` on a side).
fn thumbnail(path: &Path, px: i32) -> Option<(u32, u32, Vec<u8>)> {
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let bmp = factory.GetImage(SIZE { cx: px, cy: px }, SIIGBF_THUMBNAILONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let out = crate::win::bitmap_bgra(bmp);
        let _ = DeleteObject(HGDIOBJ(bmp.0));
        let (w, h, mut rgba) = out?;
        // Thumbnails of photos come without alpha: opaque.
        let has_alpha = rgba.as_chunks::<4>().0.iter().any(|p| p[3] != 0);
        for p in rgba.as_chunks_mut::<4>().0 {
            p.swap(0, 2);
            if !has_alpha {
                p[3] = 255;
            }
        }
        Some((w, h, rgba))
    }
}

/// The newest three screenshots, thumbnails kept per file and time.
fn screenshots(cache: &mut HashMap<PathBuf, (SystemTime, Value)>) -> Value {
    let Some(dir) = known_folder(&SCREENSHOTS) else { return json!([]) };
    let mut list: Vec<(PathBuf, u64, SystemTime)> = files(&dir)
        .into_iter()
        .filter(|(p, _, _)| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_lowercase().as_str(), "png" | "jpg" | "jpeg")))
        .collect();
    list.sort_by_key(|(_, _, m)| std::cmp::Reverse(*m));
    list.truncate(3);
    cache.retain(|p, _| list.iter().any(|(q, _, _)| q == p));
    Value::Array(
        list.into_iter()
            .map(|(path, _, modified)| {
                let thumb = match cache.get(&path) {
                    Some((m, t)) if *m == modified => t.clone(),
                    _ => {
                        let t = thumbnail(&path, 192).map_or(Value::Null, |(w, h, px)| super::rgba_json(w, h, &px));
                        cache.insert(path.clone(), (modified, t.clone()));
                        t
                    }
                };
                let time = modified.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
                json!({ "path": path.to_string_lossy(), "time": time, "thumb": thumb })
            })
            .collect(),
    )
}

/// Downloads: what's there (path → size and whether announced); new files are
/// announced once they've kept their size for a pass.
#[derive(Default)]
struct Downloads {
    seen: Option<HashMap<PathBuf, (u64, bool)>>,
}

impl Downloads {
    fn scan(&mut self, app: &AppHandle) {
        let Some(dir) = known_folder(&DOWNLOADS) else { return };
        let now: Vec<(PathBuf, u64, SystemTime)> = files(&dir);
        let Some(seen) = self.seen.as_mut() else {
            // The first look: what's already there isn't new.
            self.seen = Some(now.into_iter().map(|(p, s, _)| (p, (s, true))).collect());
            return;
        };
        let mut next = HashMap::new();
        for (path, size, _) in now {
            let partial = path.extension().and_then(|e| e.to_str()).is_some_and(|e| PARTIAL.contains(&e.to_lowercase().as_str()));
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if partial || name.starts_with('~') || name.starts_with('.') {
                continue;
            }
            let done = match seen.get(&path) {
                Some(&(was, announced)) => {
                    if !announced && was == size && size > 0 {
                        crate::osd::emit(app, "download", "notch://osd", json!({ "kind": "download", "level": 1.0, "muted": false, "text": format!("Downloaded {name}"), "ms": 3500 }));
                        true
                    } else {
                        announced
                    }
                }
                None => false,
            };
            next.insert(path, (size, done));
        }
        *seen = next;
    }
}

pub fn run(app: AppHandle) {
    let mut cache = HashMap::new();
    let mut downloads = Downloads::default();
    let mut com = false;
    loop {
        let shots = super::placed(&app, "clipboard") && super::widget_opts(&app, "clipboard").get("screenshots").and_then(|v| v.as_bool()) == Some(true);
        let dl = {
            let cfg = app.state::<super::SharedConfig>();
            let cfg = cfg.read().unwrap();
            cfg.osd.enabled && cfg.osd.downloads
        };
        if (shots || dl) && !com {
            // Shell thumbnails need COM on this thread.
            let _ = unsafe { windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_APARTMENTTHREADED) };
            com = true;
        }
        if shots {
            super::publish(&app, "screenshots", screenshots(&mut cache));
        }
        if dl {
            downloads.scan(&app);
        } else {
            downloads = Downloads::default();
        }
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Puts a picture file on the clipboard as a picture (CF_DIB), decoded by Windows
/// Imaging Component: ready to paste into a chat or a document.
pub fn copy_picture(path: &str) {
    use windows::Win32::Graphics::Imaging::*;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
    let pixels = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        (|| -> windows::core::Result<(u32, u32, Vec<u8>)> {
            let wic: IWICImagingFactory = CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)?;
            let decoder = wic.CreateDecoderFromFilename(&HSTRING::from(path), None, windows::Win32::Foundation::GENERIC_READ, WICDecodeMetadataCacheOnDemand)?;
            let frame = decoder.GetFrame(0)?;
            let conv = wic.CreateFormatConverter()?;
            conv.Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
            let (mut w, mut h) = (0u32, 0u32);
            conv.GetSize(&mut w, &mut h)?;
            let mut px = vec![0u8; (w * h * 4) as usize];
            conv.CopyPixels(std::ptr::null(), w * 4, &mut px)?;
            Ok((w, h, px))
        })()
    };
    let Ok((w, h, px)) = pixels else {
        eprintln!("[kasumi] screenshots: couldn't read {path}");
        return;
    };
    // BITMAPINFOHEADER (40 bytes) + bottom-up BGRA rows.
    let mut dib = Vec::with_capacity(40 + px.len());
    for v in [40u32, w, h] {
        dib.extend(v.to_le_bytes());
    }
    dib.extend(1u16.to_le_bytes());
    dib.extend(32u16.to_le_bytes());
    dib.extend(0u32.to_le_bytes()); // BI_RGB
    dib.extend((w * h * 4).to_le_bytes());
    for _ in 0..4 {
        dib.extend(0u32.to_le_bytes());
    }
    for row in (0..h as usize).rev() {
        dib.extend_from_slice(&px[row * w as usize * 4..(row + 1) * w as usize * 4]);
    }
    crate::win::set_clipboard_bytes(8, &dib);
}
