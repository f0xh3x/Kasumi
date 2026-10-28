//! Files dragged onto a native bar: an OLE drop target on each surface's window.
//! Where they're dragged goes to the surface as `notch://drag-over` (screen px,
//! null when they leave), the drop as `notch://drop` with its paths and point: on
//! an app in the tasks widget it opens them with that app, elsewhere it pins them
//!.

use std::cell::Cell;

use windows::core::{implement, Ref, Result};
use windows::Win32::Foundation::{HWND, POINTL};
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, CF_HDROP, DROPEFFECT, DROPEFFECT_LINK, DROPEFFECT_NONE};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

fn format() -> FORMATETC {
    FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 }
}

fn has_files(d: &IDataObject) -> bool {
    unsafe { d.QueryGetData(&format()) }.is_ok()
}

/// The dropped files' paths.
fn paths(d: &IDataObject) -> Vec<String> {
    let mut out = Vec::new();
    unsafe {
        let Ok(mut medium) = d.GetData(&format()) else { return out };
        let drop = HDROP(medium.u.hGlobal.0);
        let n = DragQueryFileW(drop, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(drop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            let got = DragQueryFileW(drop, i, Some(&mut buf)) as usize;
            out.push(String::from_utf16_lossy(&buf[..got]));
        }
        ReleaseStgMedium(&mut medium);
    }
    out
}

#[implement(IDropTarget)]
struct Target {
    label: String,
    files: Cell<bool>,
    /// The point last reported (OLE repeats DragOver while the cursor rests).
    last: Cell<Option<(i32, i32)>>,
}

impl Target {
    /// Tells the surface where files are being dragged (screen px), or that they left.
    fn over(&self, pt: Option<&POINTL>) {
        let at = pt.map(|p| (p.x, p.y));
        if at == self.last.get() {
            return;
        }
        self.last.set(at);
        let payload = at.map_or(serde_json::Value::Null, |(x, y)| serde_json::json!({ "x": x, "y": y }));
        super::post(super::Msg::Event(self.label.clone(), "notch://drag-over".into(), payload));
    }
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let ok = data.as_ref().is_some_and(has_files);
        self.files.set(ok);
        unsafe { *effect = if ok { DROPEFFECT_LINK } else { DROPEFFECT_NONE } };
        if ok {
            self.over(Some(pt));
        }
        Ok(())
    }
    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = if self.files.get() { DROPEFFECT_LINK } else { DROPEFFECT_NONE } };
        if self.files.get() {
            self.over(Some(pt));
        }
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        self.over(None);
        Ok(())
    }
    fn Drop(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let list = data.as_ref().map(paths).unwrap_or_default();
        unsafe { *effect = if list.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_LINK } };
        self.over(None);
        if !list.is_empty() {
            eprintln!("[kasumi] {}: dropped {list:?}", self.label);
            // Through the inbox: the native state may be in use right now.
            super::post(super::Msg::Event(self.label.clone(), "notch://drop".into(), serde_json::json!({ "paths": list, "x": pt.x, "y": pt.y })));
        }
        Ok(())
    }
}

/// Takes files dropped on a surface's window (the thread must have OLE initialized).
pub fn register(hwnd: HWND, label: &str) {
    let target: IDropTarget = Target { label: label.to_string(), files: Cell::new(false), last: Cell::new(None) }.into();
    if let Err(e) = unsafe { RegisterDragDrop(hwnd, &target) } {
        eprintln!("[kasumi] {label}: no file drops ({e})");
    }
}
