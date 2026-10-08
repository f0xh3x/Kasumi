//! Files dragged onto a bar (to pin them to the tasks widget). Our own OLE drop
//! target replaces wry's on each overlay: wry only ever answers "copy", which
//! sources offering just "link" (Start menu shortcuts, ...) refuse, so nothing
//! would be dropped. Events go to the overlay's frontend as `notch://drop`.

use serde::Serialize;
use std::cell::Cell;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use tauri::{AppHandle, Emitter, EventTarget};
use windows::core::{implement, BOOL, Ref};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, POINTL};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, TYMED_HGLOBAL};
use windows::Win32::System::Ole::{
    IDropTarget, IDropTarget_Impl, RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop, CF_HDROP, DROPEFFECT,
    DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_NONE,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, GetWindowThreadProcessId};

#[derive(Clone, Serialize)]
struct DropEvent {
    /// "enter", "over", "leave" or "drop".
    kind: &'static str,
    /// In the overlay's client area, physical pixels.
    x: i32,
    y: i32,
    paths: Vec<String>,
}

/// Takes over file drops on an overlay (call on the main thread, after the
/// webview exists): on the window and its webview host windows in this process.
pub fn register(app: &AppHandle, label: &str, hwnd: isize) {
    let top = HWND(hwnd as _);
    let mut targets = vec![top];
    unsafe extern "system" fn collect(h: HWND, data: LPARAM) -> BOOL {
        let list = &mut *(data.0 as *mut Vec<HWND>);
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        if pid == GetCurrentProcessId() {
            list.push(h);
        }
        true.into()
    }
    unsafe {
        let _ = EnumChildWindows(Some(top), Some(collect), LPARAM(&mut targets as *mut _ as isize));
    }
    let mut n = 0;
    for h in targets {
        let target: IDropTarget = Target {
            app: app.clone(),
            label: label.to_string(),
            top,
            effect: Cell::new(DROPEFFECT_NONE),
        }
        .into();
        unsafe {
            let revoked = RevokeDragDrop(h);
            let registered = RegisterDragDrop(h, &target);
            if registered.is_ok() {
                n += 1;
            }
            eprintln!("[statusnotch] {label}: drop target {:?}: revoke {:?}, register {:?}", h.0, revoked, registered);
        }
    }
    if n == 0 {
        eprintln!("[statusnotch] {label}: couldn't accept dropped files");
    }
}

#[implement(IDropTarget)]
struct Target {
    app: AppHandle,
    label: String,
    top: HWND,
    effect: Cell<DROPEFFECT>,
}

impl Target {
    fn send(&self, kind: &'static str, pt: &POINTL, paths: Vec<String>) {
        let mut p = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.top, &mut p);
        }
        let event = DropEvent { kind, x: p.x, y: p.y, paths };
        // Not from inside the drop call: answer Windows (and the drag source) first.
        let (app, label) = (self.app.clone(), self.label.clone());
        std::thread::spawn(move || {
            let _ = app.emit_to(EventTarget::WebviewWindow { label }, "notch://drop", event);
        });
    }
}

/// The paths of the files being dragged (none if it isn't files).
fn paths_of(data: Ref<'_, IDataObject>) -> Vec<String> {
    let Some(data) = data.as_ref() else { return Vec::new() };
    let format = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut paths = Vec::new();
    unsafe {
        let Ok(mut medium) = data.GetData(&format) else { return paths };
        let hdrop = HDROP(medium.u.hGlobal.0 as _);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        for i in 0..count {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            DragQueryFileW(hdrop, i, Some(&mut buf));
            paths.push(OsString::from_wide(&buf[..len]).to_string_lossy().into_owned());
        }
        ReleaseStgMedium(&mut medium);
    }
    paths
}

/// Link if the source allows it (a pin is a reference), else copy.
fn pick(allowed: DROPEFFECT) -> DROPEFFECT {
    if allowed.0 & DROPEFFECT_LINK.0 != 0 {
        DROPEFFECT_LINK
    } else if allowed.0 & DROPEFFECT_COPY.0 != 0 {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    }
}

#[allow(non_snake_case)]
impl IDropTarget_Impl for Target_Impl {
    fn DragEnter(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        eprintln!("[statusnotch] {}: drag enter...", self.label);
        let paths = paths_of(data);
        let chosen = if paths.is_empty() { DROPEFFECT_NONE } else { pick(unsafe { *effect }) };
        eprintln!("[statusnotch] {}: drag enter, allowed {:?}, {} file(s)", self.label, unsafe { *effect }, paths.len());
        self.effect.set(chosen);
        unsafe { *effect = chosen };
        if chosen != DROPEFFECT_NONE {
            self.send("enter", pt, paths);
        }
        Ok(())
    }

    fn DragOver(&self, _keys: MODIFIERKEYS_FLAGS, pt: &POINTL, effect: *mut DROPEFFECT) -> windows::core::Result<()> {
        let chosen = if self.effect.get() == DROPEFFECT_NONE { DROPEFFECT_NONE } else { pick(unsafe { *effect }) };
        unsafe { *effect = chosen };
        if chosen != DROPEFFECT_NONE {
            self.send("over", pt, Vec::new());
        }
        Ok(())
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.effect.replace(DROPEFFECT_NONE) != DROPEFFECT_NONE {
            self.send("leave", &POINTL::default(), Vec::new());
        }
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<'_, IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = paths_of(data);
        let chosen = if paths.is_empty() { DROPEFFECT_NONE } else { pick(unsafe { *effect }) };
        unsafe { *effect = chosen };
        self.effect.set(DROPEFFECT_NONE);
        eprintln!("[statusnotch] {}: dropped {:?}", self.label, paths);
        if chosen != DROPEFFECT_NONE {
            self.send("drop", pt, paths);
        }
        Ok(())
    }
}
