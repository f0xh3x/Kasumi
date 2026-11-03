//! Kasumi's library inside File Explorer (experimental, [explorer] hide).
//!
//! Kasumi's helper (`Kasumi.exe --explorer-inject`) asks Windows' XAML Diagnostics to load
//! this into a folder window's Explorer process (InitializeXamlDiagnosticsEx: the hook
//! Visual Studio's Live Visual Tree uses). Explorer creates our class (`Tap`) and hands
//! it the diagnostics site; we watch its XAML tree being built (`Watcher`) and collapse
//! the parts the rules name, keeping each one's own value to give it back.
//!
//! Nothing of Explorer's code is patched: a part Windows renames is simply not found.
//! The rules live in a file (its path is the initialization data) that Kasumi rewrites
//! when the settings change; a thread notices and the UI thread applies them again.
//! Every entry point catches panics: nothing here may take Explorer down.
//!
//! Rules file, one per line: `dump` (write the tree to %TEMP%\kasumi-explorer-<pid>.txt),
//! `hide <spec>`; a spec is `Name`, `Type`, `Type#Name`, or `<parent spec> > <spec>`
//! (its direct parent), `*` matching any type.

#![allow(non_snake_case)]

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ffi::c_void;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use windows::core::{implement, interface, AsImpl, IUnknownImpl, Interface, BOOL, BSTR, GUID, HRESULT, IUnknown, IUnknown_Vtbl, PCWSTR, Ref, Result};
use windows::Win32::Foundation::{CLASS_E_CLASSNOTAVAILABLE, E_FAIL, E_POINTER, HWND, LPARAM, LRESULT, S_FALSE, S_OK, WPARAM};
use windows::Win32::System::Com::{IClassFactory, IClassFactory_Impl};
use windows::Win32::System::Ole::{IObjectWithSite, IObjectWithSite_Impl};
use windows::Win32::UI::WindowsAndMessaging::*;

/// Our class (Kasumi's helper passes the same id).
pub const CLSID_TAP: GUID = GUID::from_u128(0x6c1f2b84_5a1e_4c62_9f5e_3d8a7b2c1e90);

// --- XAML Diagnostics (xamlOM.h) ------------------------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ParentChildRelation {
    pub parent: u64,
    pub child: u64,
    pub child_index: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct SourceInfo {
    pub file_name: *const u16,
    pub line: u32,
    pub column: u32,
    pub char_position: u32,
    pub hash: *const u16,
}

/// Its strings belong to the caller (not freed here).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct VisualElement {
    pub handle: u64,
    pub src: SourceInfo,
    pub ty: *const u16,
    pub name: *const u16,
    pub num_children: u32,
}

#[interface("18C9E2B6-3F43-4116-9F2B-FF935D7770D2")]
unsafe trait IXamlDiagnostics: IUnknown {
    fn GetDispatcher(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetUiLayer(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetApplication(&self, out: *mut *mut c_void) -> HRESULT;
    fn GetIInspectableFromHandle(&self, handle: u64, out: *mut *mut c_void) -> HRESULT;
    fn GetHandleFromIInspectable(&self, obj: *mut c_void, out: *mut u64) -> HRESULT;
    fn HitTest(&self, rect: windows::Win32::Foundation::RECT, count: *mut u32, out: *mut *mut u64) -> HRESULT;
    fn RegisterInstance(&self, obj: *mut c_void, out: *mut u64) -> HRESULT;
    fn GetInitializationData(&self, out: *mut BSTR) -> HRESULT;
}

#[interface("A593B11A-D17F-48BB-8F66-83910731C8A5")]
unsafe trait IVisualTreeService: IUnknown {
    fn AdviseVisualTreeChange(&self, callback: *mut c_void) -> HRESULT;
    fn UnadviseVisualTreeChange(&self, callback: *mut c_void) -> HRESULT;
    fn GetEnums(&self, count: *mut u32, out: *mut *mut c_void) -> HRESULT;
    fn CreateInstance(&self, type_name: *const u16, value: *const u16, out: *mut u64) -> HRESULT;
    fn GetPropertyValuesChain(&self, h: u64, sources: *mut u32, ps: *mut *mut c_void, count: *mut u32, pv: *mut *mut c_void) -> HRESULT;
    fn SetProperty(&self, h: u64, value: u64, index: u32) -> HRESULT;
    fn ClearProperty(&self, h: u64, index: u32) -> HRESULT;
    fn GetCollectionCount(&self, h: u64, out: *mut u32) -> HRESULT;
    fn GetCollectionElements(&self, h: u64, start: u32, count: *mut u32, out: *mut *mut c_void) -> HRESULT;
    fn AddChild(&self, parent: u64, child: u64, index: u32) -> HRESULT;
    fn RemoveChild(&self, parent: u64, index: u32) -> HRESULT;
    fn ClearChildren(&self, parent: u64) -> HRESULT;
}

#[interface("130F5136-EC43-4F61-89C7-9801A36D2E95")]
unsafe trait IVisualTreeService2: IVisualTreeService {
    fn GetPropertyIndex(&self, h: u64, name: PCWSTR, out: *mut u32) -> HRESULT;
    fn GetProperty(&self, h: u64, index: u32, out: *mut u64) -> HRESULT;
    fn ReplaceResource(&self, dictionary: u64, key: u64, value: u64) -> HRESULT;
    fn RenderTargetBitmap(&self, h: u64, options: i32, w: u32, h2: u32, out: *mut *mut c_void) -> HRESULT;
}

#[interface("AA7A8931-80E4-4FEC-8F3B-553F87B4966E")]
unsafe trait IVisualTreeServiceCallback: IUnknown {
    fn OnVisualTreeChange(&self, relation: ParentChildRelation, element: VisualElement, mutation: i32) -> HRESULT;
}

#[interface("BAD9EB88-AE77-4397-B948-5FA2DB0A19EA")]
unsafe trait IVisualTreeServiceCallback2: IVisualTreeServiceCallback {
    fn OnElementStateChanged(&self, element: u64, state: i32, context: PCWSTR) -> HRESULT;
}

// --- Logging ------------------------------------------------------------------------------------

fn log(line: &str) {
    let path = std::env::temp_dir().join("kasumi-explorer.log");
    if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
        let _ = writeln!(f, "[{}] {line}", std::process::id());
    }
}

fn wide(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    unsafe { PCWSTR(p).to_string().unwrap_or_default() }
}

// --- Rules ----------------------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
struct One {
    ty: Option<String>,
    name: Option<String>,
}

impl One {
    fn parse(s: &str) -> One {
        let s = s.trim();
        let (ty, name) = match s.split_once('#') {
            Some((t, n)) => (Some(t.trim().to_string()), Some(n.trim().to_string())),
            // A dot: a type ("Microsoft.UI.Xaml.Controls.Button"); else a name.
            None if s.contains('.') || s == "*" => (Some(s.to_string()), None),
            None => (None, Some(s.to_string())),
        };
        One { ty: ty.filter(|t| t != "*" && !t.is_empty()), name: name.filter(|n| !n.is_empty()) }
    }
    fn matches(&self, e: &Elem) -> bool {
        self.ty.as_ref().is_none_or(|t| *t == e.ty) && self.name.as_ref().is_none_or(|n| *n == e.name)
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Rule {
    parent: Option<One>,
    this: One,
}

#[derive(Default, PartialEq)]
struct Rules {
    dump: bool,
    hide: Vec<Rule>,
}

fn read_rules(path: &str) -> Rules {
    let mut r = Rules::default();
    let Ok(text) = std::fs::read_to_string(path) else { return r };
    for line in text.lines().map(str::trim) {
        if line == "dump" {
            r.dump = true;
        } else if let Some(spec) = line.strip_prefix("hide ") {
            let rule = match spec.split_once(" > ") {
                Some((p, c)) => Rule { parent: Some(One::parse(p)), this: One::parse(c) },
                None => Rule { parent: None, this: One::parse(spec) },
            };
            r.hide.push(rule);
        }
    }
    r
}

// --- The watcher ------------------------------------------------------------------------------------

struct Elem {
    ty: String,
    name: String,
    parent: u64,
}

const WM_RULES: u32 = WM_APP + 1;

#[implement(IVisualTreeServiceCallback2, IVisualTreeServiceCallback)]
struct Watcher {
    tree: IVisualTreeService2,
    rules_path: String,
    rules: RefCell<Rules>,
    elems: RefCell<HashMap<u64, Elem>>,
    /// Collapsed by us: the Visibility property's index and its own value (to give back).
    hidden: RefCell<HashMap<u64, (u32, Option<u64>)>>,
    /// The value "Collapsed" (made once).
    collapsed: Cell<Option<u64>>,
    /// A window on the UI thread: rules changed → applied there.
    window: Cell<Option<HWND>>,
}

thread_local! {
    /// The UI thread's watcher, for its window's procedure.
    static WATCHER: RefCell<Option<IVisualTreeServiceCallback2>> = const { RefCell::new(None) };
}

impl Watcher {
    fn matches(&self, h: u64) -> bool {
        let elems = self.elems.borrow();
        let Some(e) = elems.get(&h) else { return false };
        self.rules.borrow().hide.iter().any(|r| {
            r.this.matches(e)
                && match &r.parent {
                    None => true,
                    Some(p) => elems.get(&e.parent).is_some_and(|pe| p.matches(pe)),
                }
        })
    }

    fn collapsed(&self) -> Option<u64> {
        if let Some(v) = self.collapsed.get() {
            return Some(v);
        }
        let ty = BSTR::from("Microsoft.UI.Xaml.Visibility");
        let val = BSTR::from("Collapsed");
        let mut h = 0u64;
        let hr = unsafe { self.tree.CreateInstance(ty.as_ptr(), val.as_ptr(), &mut h) };
        if hr.is_err() {
            log(&format!("no Collapsed value: {hr:?}"));
            return None;
        }
        self.collapsed.set(Some(h));
        Some(h)
    }

    fn hide(&self, h: u64) {
        if self.hidden.borrow().contains_key(&h) {
            return;
        }
        let Some(collapsed) = self.collapsed() else { return };
        let mut index = 0u32;
        let name: Vec<u16> = "Visibility\0".encode_utf16().collect();
        unsafe {
            if self.tree.GetPropertyIndex(h, PCWSTR(name.as_ptr()), &mut index).is_err() {
                return;
            }
            let mut own = 0u64;
            let own = self.tree.GetProperty(h, index, &mut own).is_ok().then_some(own).filter(|&v| v != 0);
            let hr = self.tree.SetProperty(h, collapsed, index);
            if hr.is_ok() {
                self.hidden.borrow_mut().insert(h, (index, own));
            } else {
                log(&format!("couldn't hide {h:x}: {hr:?}"));
            }
        }
    }

    fn show(&self, h: u64) {
        let Some((index, own)) = self.hidden.borrow_mut().remove(&h) else { return };
        unsafe {
            let _ = match own {
                Some(v) => self.tree.SetProperty(h, v, index),
                None => self.tree.ClearProperty(h, index),
            };
        }
    }

    /// The rules file changed: hide what it names now, give back what it doesn't.
    fn reapply(&self) {
        let rules = read_rules(&self.rules_path);
        if rules == *self.rules.borrow() {
            return;
        }
        *self.rules.borrow_mut() = rules;
        let handles: Vec<u64> = self.elems.borrow().keys().copied().collect();
        for h in handles {
            if self.matches(h) {
                self.hide(h);
            } else {
                self.show(h);
            }
        }
    }

    fn dump(&self, rel: &ParentChildRelation, e: &Elem, h: u64) {
        let path = std::env::temp_dir().join(format!("kasumi-explorer-{}.txt", std::process::id()));
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
            let _ = writeln!(f, "{h:x}\t{:x}\t{}\t{}\t{}", rel.parent, rel.child_index, e.ty, e.name);
        }
    }

    /// The window on the UI thread the rules thread pokes.
    fn ensure_window(&self, me: IVisualTreeServiceCallback2) {
        if self.window.get().is_some() {
            return;
        }
        unsafe {
            let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            let class = windows::core::w!("KasumiExplorerRules");
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(rules_proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
            let Ok(hwnd) = CreateWindowExW(Default::default(), class, windows::core::w!("Kasumi"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None) else {
                return;
            };
            self.window.set(Some(hwnd));
            WATCHER.with(|w| *w.borrow_mut() = Some(me));
            // Poll the rules file's time (cheap) and poke the UI thread when it changes.
            let path = self.rules_path.clone();
            let target = hwnd.0 as isize;
            std::thread::spawn(move || {
                let stamp = |p: &str| std::fs::metadata(p).and_then(|m| m.modified()).ok();
                let mut last = stamp(&path);
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    let now = stamp(&path);
                    if now != last {
                        last = now;
                        if PostMessageW(Some(HWND(target as *mut _)), WM_RULES, WPARAM(0), LPARAM(0)).is_err() {
                            return;
                        }
                    }
                }
            });
        }
    }
}

unsafe extern "system" fn rules_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_RULES {
        let _ = catch_unwind(|| {
            WATCHER.with(|w| {
                if let Some(cb) = w.borrow().as_ref() {
                    // SAFETY: ours (made by `Watcher.into()`).
                    let me: &Watcher = unsafe { cb.as_impl() };
                    me.reapply();
                }
            })
        });
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

impl IVisualTreeServiceCallback_Impl for Watcher_Impl {
    unsafe fn OnVisualTreeChange(&self, relation: ParentChildRelation, element: VisualElement, mutation: i32) -> HRESULT {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let h = element.handle;
            if mutation == 1 {
                // Removed.
                self.elems.borrow_mut().remove(&h);
                self.hidden.borrow_mut().remove(&h);
                return;
            }
            let me: IVisualTreeServiceCallback2 = self.to_interface();
            self.ensure_window(me);
            let e = Elem { ty: wide(element.ty), name: wide(element.name), parent: relation.parent };
            if self.rules.borrow().dump {
                self.dump(&relation, &e, h);
            }
            self.elems.borrow_mut().insert(h, e);
            if self.matches(h) {
                self.hide(h);
            }
        }));
        if r.is_err() {
            log("panic in OnVisualTreeChange (caught)");
        }
        S_OK
    }
}

impl IVisualTreeServiceCallback2_Impl for Watcher_Impl {
    unsafe fn OnElementStateChanged(&self, _element: u64, _state: i32, _context: PCWSTR) -> HRESULT {
        S_OK
    }
}

// --- The class Explorer creates ----------------------------------------------------------------------

#[implement(IObjectWithSite)]
struct Tap {
    site: RefCell<Option<IUnknown>>,
}

/// Pointers handed to the thread that subscribes (COM objects aren't Send).
struct SendPtr(*mut c_void, *mut c_void);
unsafe impl Send for SendPtr {}

impl IObjectWithSite_Impl for Tap_Impl {
    fn SetSite(&self, site: Ref<'_, IUnknown>) -> Result<()> {
        let r = catch_unwind(AssertUnwindSafe(|| -> Result<()> {
            let Some(site) = site.as_ref() else {
                *self.site.borrow_mut() = None;
                return Ok(());
            };
            *self.site.borrow_mut() = Some(site.clone());
            let diag: IXamlDiagnostics = site.cast()?;
            let tree: IVisualTreeService2 = site.cast()?;
            let mut data = BSTR::new();
            unsafe {
                let _ = diag.GetInitializationData(&mut data);
            }
            let rules_path = data.to_string();
            log(&format!("loaded; rules {rules_path}"));
            let watcher = Watcher {
                tree: tree.clone(),
                rules: RefCell::new(read_rules(&rules_path)),
                rules_path,
                elems: RefCell::new(HashMap::new()),
                hidden: RefCell::new(HashMap::new()),
                collapsed: Cell::new(None),
                window: Cell::new(None),
            };
            let cb: IVisualTreeServiceCallback2 = watcher.into();
            // Subscribing from the thread that set the site deadlocks: another thread does it.
            let ptrs = SendPtr(tree.into_raw(), cb.into_raw());
            std::thread::spawn(move || {
                let ptrs = ptrs;
                unsafe {
                    let tree = IVisualTreeService2::from_raw(ptrs.0);
                    let cb = IVisualTreeServiceCallback2::from_raw(ptrs.1);
                    let hr = tree.AdviseVisualTreeChange(cb.as_raw());
                    if hr.is_err() {
                        log(&format!("couldn't watch the tree: {hr:?}"));
                    }
                    // Kept for the process's life (the subscription holds it too).
                    std::mem::forget(cb);
                    std::mem::forget(tree);
                }
            });
            Ok(())
        }));
        match r {
            Ok(r) => r,
            Err(_) => {
                log("panic in SetSite (caught)");
                Err(E_FAIL.into())
            }
        }
    }

    fn GetSite(&self, riid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        match self.site.borrow().as_ref() {
            Some(s) => unsafe { s.query(riid, out).ok() },
            None => Err(E_FAIL.into()),
        }
    }
}

#[implement(IClassFactory)]
struct Factory;

impl IClassFactory_Impl for Factory_Impl {
    fn CreateInstance(&self, outer: Ref<'_, IUnknown>, riid: *const GUID, out: *mut *mut c_void) -> Result<()> {
        if outer.is_some() {
            return Err(windows::Win32::Foundation::CLASS_E_NOAGGREGATION.into());
        }
        let tap: IObjectWithSite = Tap { site: RefCell::new(None) }.into();
        unsafe { tap.query(riid, out).ok() }
    }
    fn LockServer(&self, _lock: BOOL) -> Result<()> {
        Ok(())
    }
}

#[no_mangle]
pub unsafe extern "system" fn DllGetClassObject(clsid: *const GUID, riid: *const GUID, out: *mut *mut c_void) -> HRESULT {
    if clsid.is_null() || riid.is_null() || out.is_null() {
        return E_POINTER;
    }
    let r = catch_unwind(|| unsafe {
        *out = std::ptr::null_mut();
        if *clsid != CLSID_TAP {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let f: IClassFactory = Factory.into();
        f.query(riid, out)
    });
    r.unwrap_or(E_FAIL)
}

/// Never unloaded (its watcher lives as long as Explorer's process).
#[no_mangle]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    S_FALSE
}
