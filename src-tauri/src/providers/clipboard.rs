//! Clipboard history for the clipboard widget. Windows' own history (Win+V) isn't
//! readable by other apps, so this keeps one: a hidden message window listens for
//! clipboard changes while the widget is placed. Text and images (with small
//! thumbnails); in memory only, never saved; cleared when the widget is removed.
//! Copies marked "not for history" (password managers do this) are skipped.

use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::AppHandle;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    RegisterClipboardFormatW, RemoveClipboardFormatListener, SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW, SetTimer, HWND_MESSAGE, MSG,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_CLIPBOARDUPDATE, WM_TIMER, WNDCLASSW,
};

const CF_UNICODETEXT: u32 = 13;
const CF_DIB: u32 = 8;
/// Images kept (their full data stays in memory for pasting).
const MAX_IMAGES: usize = 8;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

enum Content {
    Text(String),
    Image { dib: Vec<u8>, thumb: Option<(u32, u32, Vec<u8>)> },
}

struct Item {
    id: u64,
    time: i64,
    content: Content,
}

#[derive(Default)]
struct History {
    items: Vec<Item>,
    next: u64,
}

static APP: OnceLock<AppHandle> = OnceLock::new();
static HISTORY: Mutex<History> = Mutex::new(History { items: Vec::new(), next: 1 });
static HWND_ID: AtomicIsize = AtomicIsize::new(0);
static LISTENING: AtomicBool = AtomicBool::new(false);

fn opts(app: &AppHandle) -> (usize, bool) {
    let t = super::widget_opts(app, "clipboard");
    let size = t.get("size").and_then(|v| v.as_integer()).unwrap_or(20).clamp(5, 100) as usize;
    let images = t.get("images").and_then(|v| v.as_bool()) != Some(false);
    (size, images)
}

pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    std::thread::Builder::new().name("kasumi-clipboard".into()).spawn(run).expect("spawn clipboard thread");
}

fn run() {
    unsafe {
        let class = w!("KasumiClipboard");
        let wc = WNDCLASSW { lpfnWndProc: Some(wndproc), lpszClassName: class, ..Default::default() };
        RegisterClassW(&wc);
        let Ok(hwnd) =
            CreateWindowExW(WINDOW_EX_STYLE(0), class, PCWSTR::null(), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None)
        else {
            eprintln!("[kasumi] clipboard: could not create its window");
            return;
        };
        HWND_ID.store(hwnd.0 as isize, Ordering::Relaxed);
        // Listen only while the widget is placed (checked every 2 s).
        SetTimer(Some(hwnd), 1, 2000, None);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            DispatchMessageW(&msg);
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    let Some(app) = APP.get() else { return DefWindowProcW(hwnd, msg, wparam, lparam) };
    match msg {
        WM_TIMER => {
            let want = super::placed(app, "clipboard");
            if want != LISTENING.load(Ordering::Relaxed) {
                LISTENING.store(want, Ordering::Relaxed);
                if want {
                    let _ = AddClipboardFormatListener(hwnd);
                } else {
                    let _ = RemoveClipboardFormatListener(hwnd);
                    *HISTORY.lock().unwrap() = History { items: Vec::new(), next: 1 };
                    publish(app);
                }
            }
            LRESULT(0)
        }
        WM_CLIPBOARDUPDATE => {
            capture(app, hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Opens the clipboard, retrying a moment while another app has it open.
fn open(hwnd: HWND) -> bool {
    for _ in 0..10 {
        if unsafe { OpenClipboard(Some(hwnd)) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    false
}

/// A copy that asks not to be kept (password managers set these formats).
fn excluded() -> bool {
    unsafe {
        let exclude = RegisterClipboardFormatW(w!("ExcludeClipboardContentFromMonitorProcessing"));
        if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
            return true;
        }
        let can = RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory"));
        if can != 0 && IsClipboardFormatAvailable(can).is_ok() {
            if let Ok(h) = GetClipboardData(can) {
                let p = GlobalLock(HGLOBAL(h.0)) as *const u32;
                let allowed = p.is_null() || *p != 0;
                let _ = GlobalUnlock(HGLOBAL(h.0));
                return !allowed;
            }
        }
        false
    }
}

fn capture(app: &AppHandle, hwnd: HWND) {
    let (size, images) = opts(app);
    if !open(hwnd) {
        return;
    }
    let content = unsafe {
        if excluded() {
            None
        } else if IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() {
            read_global(CF_UNICODETEXT).and_then(|bytes| {
                let wide: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let end = wide.iter().position(|&c| c == 0).unwrap_or(wide.len());
                let text = String::from_utf16_lossy(&wide[..end]);
                (!text.trim().is_empty()).then_some(Content::Text(text))
            })
        } else if images && IsClipboardFormatAvailable(CF_DIB).is_ok() {
            read_global(CF_DIB).map(|dib| {
                let thumb = thumbnail(&dib);
                Content::Image { dib, thumb }
            })
        } else {
            None
        }
    };
    unsafe {
        let _ = CloseClipboard();
    }
    let Some(content) = content else { return };
    {
        let mut h = HISTORY.lock().unwrap();
        // The same thing copied again (or pasted from here) moves to the top.
        h.items.retain(|i| match (&i.content, &content) {
            (Content::Text(a), Content::Text(b)) => a != b,
            (Content::Image { dib: a, .. }, Content::Image { dib: b, .. }) => a != b,
            _ => true,
        });
        let id = h.next;
        h.next += 1;
        h.items.insert(0, Item { id, time: chrono::Utc::now().timestamp(), content });
        h.items.truncate(size);
        // Cap the images kept (count and bytes), oldest first.
        let mut count = 0;
        let mut bytes = 0;
        h.items.retain(|i| match &i.content {
            Content::Image { dib, .. } => {
                count += 1;
                bytes += dib.len();
                count <= MAX_IMAGES && bytes <= MAX_IMAGE_BYTES
            }
            _ => true,
        });
    }
    publish(app);
}

unsafe fn read_global(format: u32) -> Option<Vec<u8>> {
    let h = GetClipboardData(format).ok()?;
    let g = HGLOBAL(h.0);
    let p = GlobalLock(g) as *const u8;
    if p.is_null() {
        return None;
    }
    let data = std::slice::from_raw_parts(p, GlobalSize(g)).to_vec();
    let _ = GlobalUnlock(g);
    Some(data)
}

/// A small RGBA copy of a 24 / 32-bit DIB (at most 160 x 90).
fn thumbnail(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let i32_at = |o: usize| dib.get(o..o + 4).map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = i32_at(0)? as usize;
    let width = i32_at(4)?;
    let height = i32_at(8)?;
    let bpp = u16::from_le_bytes([*dib.get(14)?, *dib.get(15)?]) as usize;
    let compression = i32_at(16)?;
    if width <= 0 || height == 0 || !(bpp == 24 || bpp == 32) || !(compression == 0 || compression == 3) {
        return None;
    }
    let (w, h_abs) = (width as usize, height.unsigned_abs() as usize);
    let masks = if compression == 3 && header == 40 { 12 } else { 0 };
    let offset = header + masks;
    let stride = (w * bpp).div_ceil(32) * 4;
    let scale = (w as f64 / 160.0).max(h_abs as f64 / 90.0).max(1.0);
    let (tw, th) = (((w as f64 / scale) as usize).max(1), ((h_abs as f64 / scale) as usize).max(1));
    let mut out = Vec::with_capacity(tw * th * 4);
    for y in 0..th {
        let sy = ((y as f64 * scale) as usize).min(h_abs - 1);
        // Positive height: rows are stored bottom-up.
        let row = if height > 0 { h_abs - 1 - sy } else { sy };
        for x in 0..tw {
            let sx = ((x as f64 * scale) as usize).min(w - 1);
            let p = offset + row * stride + sx * (bpp / 8);
            let px = dib.get(p..p + 3)?;
            out.extend_from_slice(&[px[2], px[1], px[0], 255]);
        }
    }
    Some((tw as u32, th as u32, out))
}

fn publish(app: &AppHandle) {
    let h = HISTORY.lock().unwrap();
    let items: Vec<serde_json::Value> = h
        .items
        .iter()
        .map(|i| match &i.content {
            Content::Text(t) => json!({ "id": i.id, "time": i.time, "kind": "text", "text": t.chars().take(600).collect::<String>() }),
            Content::Image { thumb, dib } => json!({
                "id": i.id, "time": i.time, "kind": "image", "bytes": dib.len(),
                "thumb": thumb.as_ref().map(|(w, h, rgba)| super::rgba_json(*w, *h, rgba)),
            }),
        })
        .collect();
    super::publish(app, "clipboard", json!(items));
}

/// Puts an item back on the clipboard and pastes it into the app in front.
#[tauri::command]
pub async fn clipboard_paste(id: u64) -> Result<(), String> {
    let hwnd = HWND(HWND_ID.load(Ordering::Relaxed) as *mut _);
    {
        let h = HISTORY.lock().unwrap();
        let item = h.items.iter().find(|i| i.id == id).ok_or("gone")?;
        let (format, bytes) = match &item.content {
            Content::Text(t) => (CF_UNICODETEXT, t.encode_utf16().chain([0]).flat_map(|c| c.to_le_bytes()).collect::<Vec<u8>>()),
            Content::Image { dib, .. } => (CF_DIB, dib.clone()),
        };
        if !open(hwnd) {
            return Err("the clipboard is busy".into());
        }
        unsafe {
            let _ = EmptyClipboard();
            if let Ok(g) = GlobalAlloc(GMEM_MOVEABLE, bytes.len()) {
                let p = GlobalLock(g) as *mut u8;
                if !p.is_null() {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
                    let _ = GlobalUnlock(g);
                    let _ = SetClipboardData(format, Some(HANDLE(g.0)));
                }
            }
            let _ = CloseClipboard();
        }
    }
    // The bars never take the focus: the app you were in is still in front.
    std::thread::sleep(std::time::Duration::from_millis(60));
    unsafe {
        use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP, VK_CONTROL};
        keybd_event(VK_CONTROL.0 as u8, 0, Default::default(), 0);
        keybd_event(b'V', 0, Default::default(), 0);
        keybd_event(b'V', 0, KEYEVENTF_KEYUP, 0);
        keybd_event(VK_CONTROL.0 as u8, 0, KEYEVENTF_KEYUP, 0);
    }
    Ok(())
}

#[tauri::command]
pub async fn clipboard_remove(id: u64) -> Result<(), String> {
    HISTORY.lock().unwrap().items.retain(|i| i.id != id);
    if let Some(app) = APP.get() {
        publish(app);
    }
    Ok(())
}

#[tauri::command]
pub async fn clipboard_clear() -> Result<(), String> {
    HISTORY.lock().unwrap().items.clear();
    if let Some(app) = APP.get() {
        publish(app);
    }
    Ok(())
}
