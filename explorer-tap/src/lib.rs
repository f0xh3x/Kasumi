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
//! `hide <spec>`; a spec is `Name`, a dotted `Type`, `Type#Name` or `Type#` (a type by its
//! full name or its last part: `AppBarSeparator#`), or a chain `<spec> > <spec> > ...` (each the direct
//! parent of the next), `*` matching any type.

#![allow(non_snake_case)]

use std::cell::RefCell;
use std::sync::Mutex;
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

/// One of an element's properties (GetPropertyValuesChain; its strings are the caller's).
#[repr(C)]
pub struct PropertyChainValue {
    pub index: u32,
    pub ty: *mut u16,
    pub declaring_type: *mut u16,
    pub value_type: *mut u16,
    pub item_type: *mut u16,
    pub value: *mut u16,
    pub overridden: i32,
    pub metadata_bits: i64,
    pub property_name: *mut u16,
    pub property_chain_index: u32,
}

#[repr(C)]
pub struct PropertyChainSource {
    pub handle: u64,
    pub target_type: *mut u16,
    pub name: *mut u16,
    pub source: i32,
    pub src: SourceInfo,
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

/// A failure logged once per process (not for every element).
fn note_once(line: &str) {
    use std::sync::Mutex;
    static SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen.iter().any(|s| s == line) {
        seen.push(line.to_string());
        log(line);
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
            // A dot: a type ("Microsoft.UI.Xaml.Controls.Button"); else a name. A short type
            // alone is written `AppBarSeparator#`.
            None if s.contains('.') || s == "*" => (Some(s.to_string()), None),
            None => (None, Some(s.to_string())),
        };
        One { ty: ty.filter(|t| t != "*" && !t.is_empty()), name: name.filter(|n| !n.is_empty()) }
    }
    fn matches(&self, e: &Elem) -> bool {
        // A type by its full name, or by its last part.
        let ty_ok = |t: &String| *t == e.ty || e.ty.rsplit('.').next() == Some(t.as_str());
        self.ty.as_ref().is_none_or(ty_ok) && self.name.as_ref().is_none_or(|n| *n == e.name)
    }
}

/// The last part is the element; the ones before, its parent, grandparent...
#[derive(Clone, Debug, PartialEq)]
struct Rule {
    chain: Vec<One>,
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
            r.hide.push(Rule { chain: spec.split(" > ").map(One::parse).collect() });
        }
    }
    r
}

// --- The watcher ------------------------------------------------------------------------------------

struct Elem {
    ty: String,
    name: String,
    parent: u64,
    /// The UI thread it lives on (each folder window has its own; a process can have
    /// several). It's changed only from there.
    thread: u32,
}

#[derive(Default)]
struct State {
    rules: Rules,
    elems: HashMap<u64, Elem>,
    /// Collapsed by us: the Visibility property's index and its own value (to give back).
    hidden: HashMap<u64, (u32, Option<u64>)>,
    /// The value "Collapsed" (made once).
    collapsed: Option<u64>,
}

const WM_RULES: u32 = WM_APP + 1;
const TIMER_BAND: usize = 1;

#[implement(IVisualTreeServiceCallback2, IVisualTreeServiceCallback)]
struct Watcher {
    tree: IVisualTreeService2,
    rules_path: String,
    /// Behind a lock (folder windows call in on their own threads); never held across a
    /// call into the tree, which can call back.
    state: Mutex<State>,
}

thread_local! {
    /// This UI thread's message window and the watcher, for its window procedure.
    static LOCAL: RefCell<Option<(HWND, IVisualTreeServiceCallback2)>> = const { RefCell::new(None) };
    /// This thread's folder windows' contents: where Explorer puts their top (under the
    /// header) and where they go instead (the command bar's row hidden).
    static BANDS: RefCell<HashMap<isize, (i32, i32)>> = RefCell::new(HashMap::new());
}

/// Every UI thread's message window (the rules thread pokes them all).
static WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static RULES_THREAD: std::sync::Once = std::sync::Once::new();

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn thread() -> u32 {
    unsafe { windows::Win32::System::Threading::GetCurrentThreadId() }
}

fn matches(st: &State, h: u64) -> bool {
    if !st.elems.contains_key(&h) {
        return false;
    }
    st.rules.hide.iter().any(|r| {
        // From the element up: each part of the chain, the next parent up.
        let mut at = Some(h);
        r.chain.iter().rev().all(|one| match at.and_then(|x| st.elems.get(&x)) {
            Some(e) if one.matches(e) => {
                at = Some(e.parent);
                true
            }
            _ => false,
        })
    })
}

/// The numbers in a property's text ("0,41,0", "<0, 41, 0>", "41").
fn numbers(s: &str) -> Vec<f32> {
    s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter_map(|t| t.parse().ok()).collect()
}

impl Watcher {
    fn collapsed(&self) -> Option<u64> {
        if let Some(v) = lock(&self.state).collapsed {
            return Some(v);
        }
        let ty = BSTR::from("Microsoft.UI.Xaml.Visibility");
        let val = BSTR::from("Collapsed");
        let mut h = 0u64;
        let hr = unsafe { self.tree.CreateInstance(ty.as_ptr(), val.as_ptr(), &mut h) };
        if hr.is_err() {
            note_once(&format!("no Collapsed value: {hr:?}"));
            return None;
        }
        lock(&self.state).collapsed = Some(h);
        Some(h)
    }

    /// An element's properties: (name, index, value as text).
    fn props(&self, h: u64) -> Vec<(String, u32, String)> {
        let (mut sc, mut pc) = (0u32, 0u32);
        let (mut ps, mut pv): (*mut c_void, *mut c_void) = (std::ptr::null_mut(), std::ptr::null_mut());
        let hr = unsafe { self.tree.GetPropertyValuesChain(h, &mut sc, &mut ps, &mut pc, &mut pv) };
        if hr.is_err() {
            note_once(&format!("no property list: {hr:?}"));
            return Vec::new();
        }
        let mut out = Vec::new();
        unsafe {
            // Their strings and arrays are ours to free.
            let free = |p: *mut u16| {
                if !p.is_null() {
                    drop(BSTR::from_raw(p));
                }
            };
            if !pv.is_null() {
                for v in std::slice::from_raw_parts(pv as *const PropertyChainValue, pc as usize) {
                    out.push((wide(v.property_name), v.index, wide(v.value)));
                    for p in [v.ty, v.declaring_type, v.value_type, v.item_type, v.value, v.property_name] {
                        free(p);
                    }
                }
                windows::Win32::System::Com::CoTaskMemFree(Some(pv));
            }
            if !ps.is_null() {
                for src in std::slice::from_raw_parts(ps as *const PropertyChainSource, sc as usize) {
                    for p in [src.target_type, src.name, src.src.file_name as *mut u16, src.src.hash as *mut u16] {
                        free(p);
                    }
                }
                windows::Win32::System::Com::CoTaskMemFree(Some(ps));
            }
        }
        out
    }

    /// The element's Visibility property: by name, else from its property list.
    fn visibility_index(&self, h: u64) -> Option<u32> {
        let mut index = 0u32;
        let name: Vec<u16> = "Visibility\0".encode_utf16().collect();
        if unsafe { self.tree.GetPropertyIndex(h, PCWSTR(name.as_ptr()), &mut index) }.is_ok() {
            return Some(index);
        }
        let found = self.props(h).into_iter().find(|p| p.0 == "Visibility").map(|p| p.1);
        if found.is_none() {
            note_once("no Visibility property");
        }
        found
    }

    fn hide(&self, h: u64) {
        let what = {
            let st = lock(&self.state);
            if st.hidden.contains_key(&h) {
                return;
            }
            st.elems.get(&h).map(|e| format!("{}#{}", e.ty, e.name)).unwrap_or_default()
        };
        let Some(collapsed) = self.collapsed() else { return };
        let Some(index) = self.visibility_index(h) else { return };
        unsafe {
            let mut own = 0u64;
            let own = self.tree.GetProperty(h, index, &mut own).is_ok().then_some(own).filter(|&v| v != 0);
            let hr = self.tree.SetProperty(h, collapsed, index);
            if hr.is_ok() {
                lock(&self.state).hidden.insert(h, (index, own));
                note_once(&format!("hid {what}"));
            } else {
                note_once(&format!("couldn't hide {what}: {hr:?}"));
            }
        }
    }

    fn show(&self, h: u64) {
        let Some((index, own)) = lock(&self.state).hidden.remove(&h) else { return };
        unsafe {
            let _ = match own {
                Some(v) => self.tree.SetProperty(h, v, index),
                None => self.tree.ClearProperty(h, index),
            };
        }
    }

    /// The rules file changed: this thread's elements hidden as it says now, the rest
    /// given back.
    fn reapply(&self) {
        let rules = read_rules(&self.rules_path);
        let me = thread();
        let todo: Vec<(u64, bool)> = {
            let mut st = lock(&self.state);
            if rules != st.rules {
                st.rules = rules;
            }
            let mine: Vec<u64> = st.elems.iter().filter(|(_, e)| e.thread == me).map(|(&h, _)| h).collect();
            mine.into_iter().map(|h| (h, matches(&st, h))).collect()
        };
        for (h, hide) in todo {
            if hide {
                self.hide(h);
            } else {
                self.show(h);
            }
        }
        self.fix_bands();
    }

    fn dump(rel: &ParentChildRelation, e: &Elem, h: u64) {
        let path = std::env::temp_dir().join(format!("kasumi-explorer-{}.txt", std::process::id()));
        if let Ok(mut f) = std::fs::OpenOptions::new().append(true).create(true).open(path) {
            let _ = writeln!(f, "{h:x}\t{:x}\t{}\t{}\t{}", rel.parent, rel.child_index, e.ty, e.name);
        }
    }

    /// How much of the header this thread's folder windows need (logical px from its top)
    /// while their command bar's row is hidden: down to the address row's bottom, and a
    /// little room. None: the row isn't hidden (Explorer's own layout).
    fn header_needed(&self) -> Option<f32> {
        let me = thread();
        let nav = {
            let st = lock(&self.state);
            let bar_hidden = st.hidden.keys().any(|h| st.elems.get(h).is_some_and(|e| e.thread == me && e.ty.ends_with("CommandBarControl_Wave1")));
            if !bar_hidden {
                return None;
            }
            st.elems.iter().find(|(_, e)| e.thread == me && e.name == "NavigationBarControl").map(|(&h, _)| h)?
        };
        let props = self.props(nav);
        let get = |n: &str| props.iter().find(|p| p.0 == n).map(|p| p.2.clone()).unwrap_or_default();
        let (offset, height) = (get("ActualOffset"), get("ActualHeight"));
        note_once(&format!("address row: offset {offset:?}, height {height:?}"));
        let top = numbers(&offset).get(1).copied()?;
        let h = numbers(&height).first().copied()?;
        (h > 0.0).then_some(top + h + 8.0)
    }

    /// This thread's folder windows: their contents under the address row while the
    /// command bar's row is hidden (Explorer keeps the header's height), back where
    /// Explorer puts them when it isn't.
    fn fix_bands(&self) {
        let need = self.header_needed();
        for cab in thread_cabinets() {
            unsafe {
                let Ok(content) = FindWindowExW(Some(cab), None, windows::core::w!("ShellTabWindowClass"), None) else { continue };
                let Ok(bridge) = FindWindowExW(Some(cab), None, windows::core::w!("Microsoft.UI.Content.DesktopChildSiteBridge"), None) else { continue };
                let b = client_rect(cab, bridge);
                let scale = windows::Win32::UI::HiDpi::GetDpiForWindow(cab) as f32 / 96.0;
                let target = match need {
                    Some(n) => (b.top + (n * scale).round() as i32).min(b.bottom),
                    None => b.bottom,
                };
                BANDS.with(|m| m.borrow_mut().insert(content.0 as isize, (b.bottom, target)));
                let _ = windows::Win32::UI::Shell::SetWindowSubclass(content, Some(content_proc), 0x4b41, 0);
                let r = client_rect(cab, content);
                if r.top != target {
                    let _ = SetWindowPos(content, None, r.left, target, r.right - r.left, r.bottom - target, SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }
    }
}

/// A child's rectangle in its parent's client coordinates.
fn client_rect(parent: HWND, child: HWND) -> windows::Win32::Foundation::RECT {
    let mut r = windows::Win32::Foundation::RECT::default();
    unsafe {
        let _ = GetWindowRect(child, &mut r);
        let mut pts = [windows::Win32::Foundation::POINT { x: r.left, y: r.top }, windows::Win32::Foundation::POINT { x: r.right, y: r.bottom }];
        windows::Win32::Graphics::Gdi::MapWindowPoints(None, Some(parent), &mut pts);
        windows::Win32::Foundation::RECT { left: pts[0].x, top: pts[0].y, right: pts[1].x, bottom: pts[1].y }
    }
}

/// This thread's folder windows.
fn thread_cabinets() -> Vec<HWND> {
    unsafe extern "system" fn each(h: HWND, lp: LPARAM) -> BOOL {
        let mut buf = [0u16; 32];
        let n = unsafe { GetClassNameW(h, &mut buf) };
        if String::from_utf16_lossy(&buf[..n.max(0) as usize]) == "CabinetWClass" {
            unsafe { (*(lp.0 as *mut Vec<HWND>)).push(h) };
        }
        true.into()
    }
    let mut out: Vec<HWND> = Vec::new();
    unsafe {
        let _ = EnumThreadWindows(thread(), Some(each), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

/// A folder window's contents: Explorer placing them right under the header goes under
/// the address row instead (BANDS), before it's drawn.
unsafe extern "system" fn content_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if msg == WM_WINDOWPOSCHANGING {
        let _ = catch_unwind(|| {
            let Some((b_bottom, target)) = BANDS.with(|b| b.borrow().get(&(hwnd.0 as isize)).copied()) else { return };
            if target == b_bottom {
                return;
            }
            let pos = unsafe { &mut *(lp.0 as *mut WINDOWPOS) };
            if !pos.flags.contains(SWP_NOMOVE) {
                // Explorer's own placement (right under the header): moved up, as much taller.
                if (pos.y - b_bottom).abs() <= 2 {
                    let d = pos.y - target;
                    pos.y = target;
                    if !pos.flags.contains(SWP_NOSIZE) {
                        pos.cy += d;
                    }
                }
            } else if !pos.flags.contains(SWP_NOSIZE) {
                // Resized where it is (already moved up): as much taller as it's higher.
                if let Ok(parent) = unsafe { GetParent(hwnd) } {
                    let r = client_rect(parent, hwnd);
                    if r.top < b_bottom - 2 {
                        pos.cy += b_bottom - r.top;
                    }
                }
            }
        });
    }
    unsafe { windows::Win32::UI::Shell::DefSubclassProc(hwnd, msg, wp, lp) }
}

unsafe extern "system" fn rules_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if msg == WM_RULES || (msg == WM_TIMER && wp.0 == TIMER_BAND) {
        let _ = catch_unwind(|| {
            LOCAL.with(|l| {
                if let Some((_, cb)) = l.borrow().as_ref() {
                    // SAFETY: ours (made by `Watcher.into()`).
                    let me: &Watcher = unsafe { cb.as_impl() };
                    if msg == WM_RULES {
                        me.reapply();
                    } else {
                        me.fix_bands();
                    }
                }
            })
        });
        return LRESULT(0);
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

impl Watcher_Impl {
    /// This UI thread's message window (rules changes and the band's check land there),
    /// and the thread that watches the rules file (once per process).
    fn ensure_window(&self) {
        if LOCAL.with(|l| l.borrow().is_some()) {
            return;
        }
        let me: IVisualTreeServiceCallback2 = self.to_interface();
        unsafe {
            let inst = windows::Win32::System::LibraryLoader::GetModuleHandleW(None).unwrap_or_default();
            let class = windows::core::w!("KasumiExplorerRules");
            RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(rules_proc), hInstance: inst.into(), lpszClassName: class, ..Default::default() });
            let Ok(hwnd) = CreateWindowExW(Default::default(), class, windows::core::w!("Kasumi"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(inst.into()), None) else {
                return;
            };
            LOCAL.with(|l| *l.borrow_mut() = Some((hwnd, me)));
            lock(&WINDOWS).push(hwnd.0 as isize);
            SetTimer(Some(hwnd), TIMER_BAND, 1000, None);
        }
        let path = self.rules_path.clone();
        RULES_THREAD.call_once(move || {
            // Poll the rules file's time (cheap) and poke every UI thread when it changes.
            std::thread::spawn(move || {
                let stamp = |p: &str| std::fs::metadata(p).and_then(|m| m.modified()).ok();
                let mut last = stamp(&path);
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(700));
                    let now = stamp(&path);
                    if now != last {
                        last = now;
                        lock(&WINDOWS).retain(|&h| unsafe { PostMessageW(Some(HWND(h as *mut _)), WM_RULES, WPARAM(0), LPARAM(0)) }.is_ok());
                    }
                }
            });
        });
    }
}

impl IVisualTreeServiceCallback_Impl for Watcher_Impl {
    unsafe fn OnVisualTreeChange(&self, relation: ParentChildRelation, element: VisualElement, mutation: i32) -> HRESULT {
        let r = catch_unwind(AssertUnwindSafe(|| {
            let h = element.handle;
            if mutation == 1 {
                // Removed.
                let mut st = lock(&self.state);
                st.elems.remove(&h);
                st.hidden.remove(&h);
                return;
            }
            self.ensure_window();
            let e = Elem { ty: wide(element.ty), name: wide(element.name), parent: relation.parent, thread: thread() };
            let hide = {
                let mut st = lock(&self.state);
                if st.rules.dump {
                    Watcher::dump(&relation, &e, h);
                }
                st.elems.insert(h, e);
                matches(&st, h)
            };
            if hide {
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
            log(&format!("loaded; rules {rules_path}: {} to hide", read_rules(&rules_path).hide.len()));
            let watcher = Watcher { tree: tree.clone(), state: Mutex::new(State { rules: read_rules(&rules_path), ..Default::default() }), rules_path };
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
