//! Other apps' tooltips in Kasumi's look ([general] app_tooltips, experimental): when a
//! classic Windows tooltip (tooltips_class32: Explorer's, the tray's, older apps')
//! shows or changes its text, Kasumi's tooltip is drawn over it, at least as large;
//! when it hides, Kasumi's goes too. The app's own tooltip isn't touched (hiding
//! another app's window gets undone or leaves it never knowing it's gone), it's only
//! covered. Newer apps draw their tooltips inside their own windows: those stay.

use std::sync::atomic::{AtomicIsize, Ordering};

use tauri::{AppHandle, Manager};
use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
use windows::Win32::UI::Accessibility::{SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK};
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, GetWindowRect, IsWindowVisible, SendMessageTimeoutW, SetTimer, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_SHOW, MSG, OBJID_CLIENT, OBJID_WINDOW, SMTO_ABORTIFHUNG, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
    WM_GETTEXT, WM_TIMER,
};

use crate::providers::SharedConfig;

/// The tooltip Kasumi's covers now (0: none).
static COVERED: AtomicIsize = AtomicIsize::new(0);

fn class(h: HWND) -> String {
    crate::win::class_name(h)
}

thread_local! {
    /// UI Automation, made the first time a tooltip's window text is empty.
    static UIA: std::cell::OnceCell<Option<windows::Win32::UI::Accessibility::IUIAutomation>> = const { std::cell::OnceCell::new() };
}

/// Its text: its window's (WM_GETTEXT), else its accessible name (what screen readers
/// read out). Empty: it's left alone.
fn text(h: HWND) -> String {
    let mut buf = [0u16; 1024];
    let mut n = 0usize;
    unsafe {
        let _ = SendMessageTimeoutW(h, WM_GETTEXT, WPARAM(buf.len()), LPARAM(buf.as_mut_ptr() as isize), SMTO_ABORTIFHUNG, 60, Some(&mut n));
    }
    let own = String::from_utf16_lossy(&buf[..n.min(buf.len())]).trim().to_string();
    if !own.is_empty() {
        return own;
    }
    UIA.with(|u| {
        let uia = u.get_or_init(|| unsafe {
            use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            CoCreateInstance(&windows::Win32::UI::Accessibility::CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
        });
        uia.as_ref().and_then(|uia| unsafe { uia.ElementFromHandle(h).ok()?.CurrentName().ok() }).map(|s| s.to_string().trim().to_string()).unwrap_or_default()
    })
}

fn cover(h: HWND) {
    // The first of each in the log: whether tooltips' text can be read on this PC.
    static SAID: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
    let say = |bit: u8, what: &str| {
        if SAID.fetch_or(bit, Ordering::Relaxed) & bit == 0 {
            eprintln!("[kasumi] app tooltips: {what} ({})", crate::win::window_process(h));
        }
    };
    let t = text(h);
    let mut r = RECT::default();
    if t.is_empty() || unsafe { GetWindowRect(h, &mut r) }.is_err() || r.right <= r.left {
        if t.is_empty() {
            say(2, "a tooltip without readable text, left as it is");
        }
        uncover(h);
        return;
    }
    say(1, "covering a tooltip");
    COVERED.store(h.0 as isize, Ordering::Relaxed);
    crate::native::post(crate::native::Msg::AppTip(Some((t, [r.left, r.top, r.right, r.bottom]))));
}

fn uncover(h: HWND) {
    if COVERED.compare_exchange(h.0 as isize, 0, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
        crate::native::post(crate::native::Msg::AppTip(None));
    }
}

unsafe extern "system" fn on_event(_: HWINEVENTHOOK, event: u32, h: HWND, object: i32, child: i32, _: u32, _: u32) {
    if child != 0 || (object != OBJID_WINDOW.0 && object != OBJID_CLIENT.0) || h.is_invalid() {
        return;
    }
    // Hidden or gone: whatever it was, Kasumi's goes if it covered that one.
    if event == EVENT_OBJECT_HIDE || event == EVENT_OBJECT_DESTROY {
        uncover(h);
        return;
    }
    if class(h) != "tooltips_class32" {
        return;
    }
    if event == EVENT_OBJECT_SHOW || (event == EVENT_OBJECT_NAMECHANGE && unsafe { IsWindowVisible(h) }.as_bool()) {
        cover(h);
    }
}

fn wanted(app: &AppHandle) -> bool {
    let cfg = app.state::<SharedConfig>();
    let cfg = cfg.read().unwrap();
    cfg.general.app_tooltips && cfg.general.tooltips
}

/// Watches while the option is on (looked at every 2 s): the hooks come and go with it.
pub fn start(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-apptips".into())
        .spawn(move || unsafe {
            let mut hooks: Vec<HWINEVENTHOOK> = Vec::new();
            let mut on = false;
            SetTimer(None, 0, 2000, None);
            let mut msg = MSG::default();
            loop {
                let want = wanted(&app);
                if want != on {
                    on = want;
                    if on {
                        let flags = WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS;
                        for (lo, hi) in [(EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE), (EVENT_OBJECT_DESTROY, EVENT_OBJECT_DESTROY), (EVENT_OBJECT_NAMECHANGE, EVENT_OBJECT_NAMECHANGE)] {
                            hooks.push(SetWinEventHook(lo, hi, None, Some(on_event), 0, 0, flags));
                        }
                    } else {
                        for h in hooks.drain(..) {
                            let _ = UnhookWinEvent(h);
                        }
                        let h = COVERED.swap(0, Ordering::Relaxed);
                        if h != 0 {
                            crate::native::post(crate::native::Msg::AppTip(None));
                        }
                    }
                }
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    break;
                }
                if msg.message != WM_TIMER {
                    DispatchMessageW(&msg);
                }
            }
        })
        .expect("spawn app tooltips thread");
}

#[cfg(test)]
mod tests {
    /// The text of tooltips that exist now (shown or not), read only:
    /// `cargo test apptips_read -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn apptips_read() {
        use windows::core::BOOL;
        use windows::Win32::UI::WindowsAndMessaging::EnumWindows;
        unsafe extern "system" fn each(h: super::HWND, data: super::LPARAM) -> BOOL {
            let list = &mut *(data.0 as *mut Vec<super::HWND>);
            if super::class(h) == "tooltips_class32" {
                list.push(h);
            }
            true.into()
        }
        let mut list: Vec<super::HWND> = Vec::new();
        unsafe {
            let _ = EnumWindows(Some(each), super::LPARAM(&mut list as *mut _ as isize));
        }
        println!("{} tooltip windows", list.len());
        for h in list.iter().take(40) {
            let t = super::text(*h);
            if !t.is_empty() {
                println!("  {:?} -> {:?} ({})", h.0, t, crate::win::window_process(*h));
            }
        }
    }
}
