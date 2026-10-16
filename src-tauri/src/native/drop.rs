//! Files dropped on a native bar (to pin them to the tasks widget): an OLE drop
//! target on each surface's window. What the pages did through WebView2
//! (dropzone.rs): the paths go to the surface as `notch://drop`.

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
}

impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let ok = data.as_ref().is_some_and(has_files);
        self.files.set(ok);
        unsafe { *effect = if ok { DROPEFFECT_LINK } else { DROPEFFECT_NONE } };
        Ok(())
    }
    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        unsafe { *effect = if self.files.get() { DROPEFFECT_LINK } else { DROPEFFECT_NONE } };
        Ok(())
    }
    fn DragLeave(&self) -> Result<()> {
        Ok(())
    }
    fn Drop(&self, data: Ref<'_, IDataObject>, _keys: MODIFIERKEYS_FLAGS, _pt: &POINTL, effect: *mut DROPEFFECT) -> Result<()> {
        let list = data.as_ref().map(paths).unwrap_or_default();
        unsafe { *effect = if list.is_empty() { DROPEFFECT_NONE } else { DROPEFFECT_LINK } };
        if !list.is_empty() {
            eprintln!("[kasumi] {}: dropped {list:?}", self.label);
            // Through the inbox: the native state may be in use right now.
            super::post(super::Msg::Event(self.label.clone(), "notch://drop".into(), serde_json::json!({ "paths": list })));
        }
        Ok(())
    }
}

/// Takes files dropped on a surface's window (the thread must have OLE initialized).
pub fn register(hwnd: HWND, label: &str) {
    let target: IDropTarget = Target { label: label.to_string(), files: Cell::new(false) }.into();
    if let Err(e) = unsafe { RegisterDragDrop(hwnd, &target) } {
        eprintln!("[kasumi] {label}: no file drops ({e})");
    }
}
