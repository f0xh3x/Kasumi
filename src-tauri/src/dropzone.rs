//! Files dropped onto a bar (to pin them to the tasks widget). WebView2 takes
//! the drop itself (Tauri's drop handler is off for the bars: Windows never
//! passed drops on the bars' WebView up to it), the page hands the dropped
//! files back with `chrome.webview.postMessageWithAdditionalObjects`, and this
//! reads their paths and sends them to the page as `notch://drop`.

use serde::Serialize;
use tauri::{Emitter, EventTarget, Manager, WebviewWindow};
use webview2_com::Microsoft::Web::WebView2::Win32::{ICoreWebView2File, ICoreWebView2WebMessageReceivedEventArgs2};
use webview2_com::{take_pwstr, WebMessageReceivedEventHandler};
use windows_core_062::{Interface, PWSTR};

/// What the page posts along with the dropped files.
const MESSAGE: &str = "statusnotch-drop";

#[derive(Clone, Serialize)]
struct DropEvent {
    kind: &'static str,
    paths: Vec<String>,
}

/// Listens for files dropped on this overlay's page.
pub fn attach(window: &WebviewWindow) {
    let app = window.app_handle().clone();
    let label = window.label().to_string();
    let result = window.with_webview(move |webview| unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else { return };
        let handler = WebMessageReceivedEventHandler::create(Box::new(move |_, args| {
            let Some(args) = args else { return Ok(()) };
            let mut text = PWSTR::null();
            if args.TryGetWebMessageAsString(&mut text).is_err() || take_pwstr(text) != MESSAGE {
                return Ok(());
            }
            let Ok(objects) = args.cast::<ICoreWebView2WebMessageReceivedEventArgs2>().and_then(|a| a.AdditionalObjects())
            else {
                return Ok(());
            };
            let mut count = 0u32;
            objects.Count(&mut count)?;
            let mut paths = Vec::new();
            for i in 0..count {
                if let Ok(file) = objects.GetValueAtIndex(i).and_then(|o| o.cast::<ICoreWebView2File>()) {
                    let mut path = PWSTR::null();
                    if file.Path(&mut path).is_ok() {
                        paths.push(take_pwstr(path));
                    }
                }
            }
            eprintln!("[statusnotch] {label}: dropped {paths:?}");
            let _ = app.emit_to(EventTarget::WebviewWindow { label: label.clone() }, "notch://drop", DropEvent { kind: "drop", paths });
            Ok(())
        }));
        let mut token = Default::default();
        if let Err(e) = core.add_WebMessageReceived(&handler, &mut token) {
            eprintln!("[statusnotch] couldn't accept dropped files: {e}");
        }
    });
    if let Err(e) = result {
        eprintln!("[statusnotch] couldn't accept dropped files: {e}");
    }
}
