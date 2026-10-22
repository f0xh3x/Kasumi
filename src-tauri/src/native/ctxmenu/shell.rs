//! The shell side of Kasumi's context menus: the Explorer view that was clicked
//! (and what's under the cursor there), Windows' own entries for it (IContextMenu,
//! as Explorer asks for them) and running the one chosen. In the `--menu` helper:
//! other apps' menu handlers load into it, not into Kasumi.

use windows::core::{Interface, GUID, PCSTR, PCWSTR, PSTR, PWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, PROPERTYKEY, WPARAM};
use windows::Win32::Graphics::Gdi::{GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, IDispatch, IServiceProvider, CLSCTX_ALL, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Variant::VARIANT;
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_SHIFT};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use super::layout::{Ctx, Kind};
use windows::Win32::System::SystemServices::{SFGAO_FOLDER, SFGAO_STREAM};

/// Straight RGBA pixels: width, height, data.
pub type Pixels = (u32, u32, Vec<u8>);

#[derive(Clone)]
pub enum Icon {
    Bitmap(Pixels),
    /// A Segoe Fluent Icons glyph.
    Glyph(char),
}

/// Kasumi's own entries for the empty space: View, Sort by, Refresh (Explorer's
/// view draws those; the menu Windows hands out without it has none).
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ViewCmd {
    /// FOLDERVIEWMODE and icon size.
    Mode(i32, i32),
    /// One of SORT_KEYS.
    Sort(usize),
    Ascending(bool),
    Refresh,
}

#[derive(Clone)]
pub enum Action {
    None,
    /// A command of Windows' menu (its offset from FIRST).
    Shell(u32),
    View(ViewCmd),
    /// One of Kasumi's own (layout.rs).
    Kasumi(super::layout::Cmd),
}

#[derive(Clone)]
pub enum Sub {
    /// Windows' submenu, filled in when it's opened (Send to, New, Open with...).
    Pending(isize, u32),
    Ready(Vec<Entry>),
}

#[derive(Clone)]
pub struct Entry {
    pub separator: bool,
    pub text: String,
    /// What follows a tab in its text (a shortcut), shown on the right.
    pub keys: String,
    /// Its letter (`&Open`).
    pub accel: Option<char>,
    /// Windows' name for the command ("open", "copy", "rename"...).
    pub verb: String,
    pub action: Action,
    pub disabled: bool,
    pub checked: bool,
    pub radio: bool,
    pub icon: Option<Icon>,
    pub sub: Option<Sub>,
    /// Windows' entries: `verb:<verb>`, else `text:<its text>` (as layouts match them,
    /// kept when the layout renames it).
    pub key: String,
    /// A Windows submenu's layout (its entries arranged when it opens).
    pub rules: Vec<crate::config::MenuNode>,
}

impl Entry {
    pub fn separator() -> Entry {
        Entry::item("", Action::None, None)
    }

    pub fn named(text: &str, action: Action, icon: Option<Icon>) -> Entry {
        let mut e = Entry::item(text, action, None);
        e.icon = icon;
        e
    }

    fn item(text: &str, action: Action, glyph: Option<char>) -> Entry {
        Entry {
            separator: text.is_empty(),
            text: text.into(),
            keys: String::new(),
            accel: text.chars().next().and_then(|c| c.to_lowercase().next()),
            verb: String::new(),
            action,
            disabled: false,
            checked: false,
            radio: false,
            icon: glyph.map(Icon::Glyph),
            sub: None,
            key: String::new(),
            rules: Vec::new(),
        }
    }

    /// Can be chosen (not a separator, not greyed out).
    pub fn usable(&self) -> bool {
        !self.separator && !self.disabled
    }
}

/// The Explorer view clicked (a folder window's tab, or the desktop).
pub struct Target {
    pub defview: HWND,
    pub root: HWND,
    pub desktop: bool,
    pub shell_view: IShellView,
    pub view: IFolderView2,
}

/// What's under the cursor in the view.
pub enum Hit {
    /// An item (now selected, if it wasn't).
    Item,
    /// Empty space (nothing selected now): the folder's own menu.
    Empty,
    /// Something Explorer handles itself (a column header): its menu, not ours.
    Other,
}

/// CMINVOKECOMMANDINFOEX's flags (shobjidl_core.h).
const CMIC_MASK_UNICODE: u32 = 0x4000;
const CMIC_MASK_SHIFT_DOWN: u32 = 0x1000_0000;
const CMIC_MASK_PTINVOKE: u32 = 0x2000_0000;
const CMIC_MASK_CONTROL_DOWN: u32 = 0x4000_0000;

const FIRST: u32 = 1;
const LAST: u32 = 0x7FFF;

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// The view whose SHELLDLL_DefView window is `defview`: among Explorer's windows
/// (each tab its own), or the desktop.
pub fn find_view(defview: HWND) -> Option<Target> {
    unsafe {
        let windows: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_ALL).ok()?;
        let view_of = |disp: IDispatch| -> Option<IShellView> {
            let sp: IServiceProvider = disp.cast().ok()?;
            let browser: IShellBrowser = sp.QueryService(&SID_STopLevelBrowser).ok()?;
            let sv = browser.QueryActiveShellView().ok()?;
            (sv.GetWindow().ok()? == defview).then_some(sv)
        };
        let mut found = None;
        for i in 0..windows.Count().unwrap_or(0) {
            if let Some(sv) = windows.Item(&VARIANT::from(i)).ok().and_then(view_of) {
                found = Some(sv);
                break;
            }
        }
        if found.is_none() {
            let (loc, none) = (VARIANT::from(CSIDL_DESKTOP as i32), VARIANT::default());
            let mut hwnd = 0i32;
            found = windows.FindWindowSW(&loc, &none, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH).ok().and_then(view_of);
        }
        let shell_view = found?;
        let view: IFolderView2 = shell_view.cast().ok()?;
        let root = GetAncestor(defview, GA_ROOT);
        let desktop = matches!(class_name(root).as_str(), "Progman" | "WorkerW");
        Some(Target { defview, root, desktop, shell_view, view })
    }
}

/// The view's items window (Explorer's DirectUIHWND, the desktop's SysListView32).
fn items_window(defview: HWND) -> Option<HWND> {
    unsafe {
        FindWindowExW(Some(defview), None, windows::core::w!("DirectUIHWND"), None)
            .or_else(|_| FindWindowExW(Some(defview), None, windows::core::w!("SysListView32"), None))
            .ok()
    }
}

/// The item whose box holds `pt`, looked for among the view's own items: for when
/// something else sits over the view (an invisible overlay like NVIDIA's, which the
/// cursor passes through but Windows' point lookup stops at).
unsafe fn item_in_view(uia: &IUIAutomation, t: &Target, pt: POINT) -> Option<IUIAutomationElement> {
    let list = uia.ElementFromHandle(items_window(t.defview)?).ok()?;
    let cond = uia
        .CreateOrCondition(
            &uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_ListItemControlTypeId.0)).ok()?,
            &uia.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_DataItemControlTypeId.0)).ok()?,
        )
        .ok()?;
    let all = list.FindAll(TreeScope_Descendants, &cond).ok()?;
    (0..all.Length().unwrap_or(0)).filter_map(|i| all.GetElement(i).ok()).find(|e| {
        e.CurrentBoundingRectangle().is_ok_and(|r| pt.x >= r.left && pt.x < r.right && pt.y >= r.top && pt.y < r.bottom)
    })
}

/// What's under `pt` in the view, selecting it as a right-click would: an item not
/// yet selected becomes the selection; empty space clears it.
pub fn select_under(t: &Target, pt: POINT) -> Hit {
    unsafe {
        let Ok(uia) = CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) else { return Hit::Other };
        let Ok(el) = uia.ElementFromPoint(pt) else { return Hit::Other };
        // Something not Explorer's is in the way: the view's items instead.
        let mut explorer = 0u32;
        GetWindowThreadProcessId(t.root, Some(&mut explorer));
        let el = if el.CurrentProcessId().is_ok_and(|p| p as u32 == explorer) {
            el
        } else {
            match item_in_view(&uia, t, pt) {
                Some(item) => item,
                None => {
                    let _ = t.shell_view.SelectItem(std::ptr::null(), SVSI_DESELECTOTHERS.0 as u32);
                    return Hit::Empty;
                }
            }
        };
        let walker = uia.ControlViewWalker().ok();
        let mut cur = Some(el);
        for _ in 0..8 {
            let Some(e) = cur.take() else { break };
            let ct = e.CurrentControlType().unwrap_or_default();
            if ct == UIA_ListItemControlTypeId || ct == UIA_DataItemControlTypeId {
                if let Ok(p) = e.GetCurrentPatternAs::<IUIAutomationSelectionItemPattern>(UIA_SelectionItemPatternId) {
                    if !p.CurrentIsSelected().is_ok_and(|b| b.as_bool()) {
                        let _ = p.Select();
                    }
                }
                return Hit::Item;
            }
            if ct == UIA_HeaderControlTypeId || ct == UIA_HeaderItemControlTypeId {
                return Hit::Other;
            }
            if e.CurrentNativeWindowHandle().is_ok_and(|h| h == t.defview) {
                break;
            }
            cur = walker.as_ref().and_then(|w| w.GetParentElement(&e).ok());
        }
        let _ = t.shell_view.SelectItem(std::ptr::null(), SVSI_DESELECTOTHERS.0 as u32);
        Hit::Empty
    }
}

/// Windows' menu for the selection (or the folder's empty space), read into entries,
/// and what was clicked.
pub struct Built {
    pub cm: IContextMenu,
    hmenu: HMENU,
    pub entries: Vec<Entry>,
    /// The folder shown (for commands that start something there).
    dir: Vec<u16>,
    pub ctx: Ctx,
}

impl Drop for Built {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyMenu(self.hmenu);
        }
    }
}

unsafe fn folder_pidl(t: &Target) -> Option<*mut ITEMIDLIST> {
    let pf: IPersistFolder2 = t.view.GetFolder().ok()?;
    pf.GetCurFolder().ok()
}

pub fn build(t: &Target, hit: &Hit, shift: bool) -> Option<Built> {
    unsafe {
        let folder = folder_pidl(t)?;
        let mut dir = vec![0u16; 1024];
        let _ = SHGetPathFromIDListEx(folder, &mut dir, GPFIDL_DEFAULT);
        let selected = t.view.ItemCount(SVGIO_SELECTION).unwrap_or(0);
        let background = matches!(hit, Hit::Empty) || selected == 0;
        let dir_s = String::from_utf16_lossy(&dir[..dir.iter().position(|&c| c == 0).unwrap_or(0)]);
        let mut ctx = Ctx { kind: if t.desktop { Kind::Desktop } else { Kind::Background }, paths: Vec::new(), dir: dir_s.clone(), here: dir_s, shift };
        let cm: Option<IContextMenu> = if background {
            (|| {
                let desk = SHGetDesktopFolder().ok()?;
                let sf: IShellFolder = if (*folder).mkid.cb == 0 { desk } else { desk.BindToObject(folder, None).ok()? };
                sf.CreateViewObject(t.root).ok()
            })()
        } else {
            (|| {
                let sel: IShellItemArray = t.view.GetSelection(false).ok()?;
                ctx.kind = selection_kind(&sel, &mut ctx.paths);
                if ctx.kind == Kind::Folder || ctx.kind == Kind::Drive {
                    ctx.here = ctx.paths[0].clone();
                }
                let n = sel.GetCount().ok()?;
                let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
                for i in 0..n {
                    if let Some(p) = sel.GetItemAt(i).ok().and_then(|it| SHGetIDListFromObject(&it).ok()) {
                        pidls.push(p);
                    }
                }
                let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const _).collect();
                let local = SHCreateShellItemArrayFromIDLists(&consts).ok();
                for p in pidls {
                    CoTaskMemFree(Some(p as *const _));
                }
                local?.BindToHandler(None, &BHID_SFUIObject).ok()
            })()
        };
        CoTaskMemFree(Some(folder as *const _));
        let cm = cm?;
        let hmenu = CreatePopupMenu().ok()?;
        let mut flags = CMF_NORMAL | CMF_CANRENAME;
        if !background {
            flags |= CMF_ITEMMENU;
        }
        if shift {
            flags |= CMF_EXTENDEDVERBS;
        }
        if cm.QueryContextMenu(hmenu, 0, FIRST, LAST, flags).is_err() {
            let _ = DestroyMenu(hmenu);
            return None;
        }
        let mut entries = read_menu(&cm, hmenu);
        tidy(&mut entries);
        Some(Built { cm, hmenu, entries, dir, ctx })
    }
}

/// What the selection is (and its paths): one file, folder or drive, or several.
unsafe fn selection_kind(sel: &IShellItemArray, paths: &mut Vec<String>) -> Kind {
    let n = sel.GetCount().unwrap_or(0);
    let mut kind = Kind::File;
    for i in 0..n {
        let Ok(item) = sel.GetItemAt(i) else { continue };
        if let Ok(p) = item.GetDisplayName(SIGDN_FILESYSPATH) {
            paths.push(p.to_string().unwrap_or_default());
            CoTaskMemFree(Some(p.0 as *const _));
        }
        if n == 1 {
            let a = item.GetAttributes(SFGAO_FOLDER | SFGAO_STREAM).unwrap_or_default();
            let path = paths.last().cloned().unwrap_or_default();
            kind = if path.len() == 3 && path.ends_with(":\\") {
                Kind::Drive
            } else if a.0 & SFGAO_FOLDER.0 != 0 && a.0 & SFGAO_STREAM.0 == 0 {
                Kind::Folder
            } else {
                Kind::File
            };
        }
    }
    if n > 1 {
        Kind::Multiple
    } else {
        kind
    }
}

/// Select all (0), invert the selection (1) or select none (2).
pub fn select(t: &Target, how: u8) {
    unsafe {
        if how == 2 {
            let _ = t.shell_view.SelectItem(std::ptr::null(), SVSI_DESELECTOTHERS.0 as u32);
            return;
        }
        let n = t.view.ItemCount(SVGIO_ALLVIEW).unwrap_or(0);
        for i in 0..n {
            let on = how == 0
                || t.view.Item(i).is_ok_and(|pidl| {
                    let state = t.view.GetSelectionState(pidl).unwrap_or(0);
                    CoTaskMemFree(Some(pidl as *const _));
                    state & SVSI_SELECT.0 as u32 == 0
                });
            let _ = t.view.SelectItem(i, if on { SVSI_SELECT.0 as u32 } else { SVSI_DESELECT.0 as u32 });
        }
    }
}

impl Built {
    /// A submenu of Windows', filled in by its handler as it would be on opening.
    pub fn populate(&self, hsub: isize, pos: u32) -> Vec<Entry> {
        unsafe {
            let (wp, lp) = (WPARAM(hsub as usize), LPARAM(pos as isize));
            if let Ok(cm3) = self.cm.cast::<IContextMenu3>() {
                let mut r = LRESULT(0);
                let _ = cm3.HandleMenuMsg2(WM_INITMENUPOPUP, wp, lp, Some(&mut r));
            } else if let Ok(cm2) = self.cm.cast::<IContextMenu2>() {
                let _ = cm2.HandleMenuMsg(WM_INITMENUPOPUP, wp, lp);
            }
            let mut e = read_menu(&self.cm, HMENU(hsub as _));
            tidy(&mut e);
            e
        }
    }

    /// Runs a command of Windows' menu.
    pub fn invoke(&self, t: &Target, off: u32, verb: &str, pt: POINT) {
        unsafe {
            // Renaming happens in the view (Explorer's edit box on the item).
            if verb.eq_ignore_ascii_case("rename") {
                if let Ok(i) = t.view.GetSelectedItem(-1) {
                    let flags = SVSI_EDIT.0 | SVSI_SELECT.0 | SVSI_DESELECTOTHERS.0 | SVSI_ENSUREVISIBLE.0 | SVSI_FOCUSED.0;
                    let _ = t.view.SelectItem(i, flags as u32);
                }
                return;
            }
            let mut mask = CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE;
            if GetAsyncKeyState(VK_SHIFT.0 as i32) < 0 {
                mask |= CMIC_MASK_SHIFT_DOWN;
            }
            if GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 {
                mask |= CMIC_MASK_CONTROL_DOWN;
            }
            let info = CMINVOKECOMMANDINFOEX {
                cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
                fMask: mask,
                hwnd: t.root,
                lpVerb: PCSTR(off as usize as *const u8),
                lpVerbW: PCWSTR(off as usize as *const u16),
                lpDirectoryW: PCWSTR(self.dir.as_ptr()),
                nShow: SW_SHOWNORMAL.0,
                ptInvoke: pt,
                ..Default::default()
            };
            if let Err(e) = self.cm.InvokeCommand(&info as *const _ as *const CMINVOKECOMMANDINFO) {
                eprintln!("[kasumi] context menu: {verb:?} didn't run: {e}");
            }
        }
    }
}

/// A menu's items as entries (its submenus left to fill in when opened).
unsafe fn read_menu(cm: &IContextMenu, hmenu: HMENU) -> Vec<Entry> {
    let mut out = Vec::new();
    let n = GetMenuItemCount(Some(hmenu)).max(0) as u32;
    for i in 0..n {
        let mut buf = [0u16; 512];
        let mut mii = MENUITEMINFOW {
            cbSize: std::mem::size_of::<MENUITEMINFOW>() as u32,
            fMask: MIIM_FTYPE | MIIM_STATE | MIIM_ID | MIIM_SUBMENU | MIIM_STRING | MIIM_BITMAP,
            dwTypeData: PWSTR(buf.as_mut_ptr()),
            cch: buf.len() as u32 - 1,
            ..Default::default()
        };
        if GetMenuItemInfoW(hmenu, i, true, &mut mii).is_err() {
            continue;
        }
        if mii.fType.0 & MFT_SEPARATOR.0 != 0 {
            out.push(Entry::separator());
            continue;
        }
        let raw = String::from_utf16_lossy(&buf[..(mii.cch as usize).min(buf.len())]);
        let (text, keys, accel) = parse(&raw);
        if text.is_empty() {
            continue; // drawn by its handler, with no text we could show
        }
        let ours = mii.wID >= FIRST && mii.wID <= LAST;
        let sub = (!mii.hSubMenu.is_invalid()).then(|| Sub::Pending(mii.hSubMenu.0 as isize, i));
        let verb = if sub.is_none() && ours { verb_of(cm, mii.wID - FIRST) } else { String::new() };
        let icon = glyph_for(&verb).map(Icon::Glyph).or_else(|| pixels(mii.hbmpItem).map(Icon::Bitmap));
        let key = if verb.is_empty() { format!("text:{text}") } else { format!("verb:{verb}") };
        out.push(Entry {
            separator: false,
            key,
            rules: Vec::new(),
            text,
            keys,
            accel,
            action: if sub.is_none() && ours { Action::Shell(mii.wID - FIRST) } else { Action::None },
            verb,
            disabled: mii.fState.0 & (MFS_DISABLED.0 | MFS_GRAYED.0) != 0,
            checked: mii.fState.0 & MFS_CHECKED.0 != 0,
            radio: mii.fType.0 & MFT_RADIOCHECK.0 != 0,
            icon,
            sub,
        });
    }
    out
}

/// `&Open\tCtrl+O`: the text, what follows the tab, and the letter.
fn parse(raw: &str) -> (String, String, Option<char>) {
    let (label, keys) = raw.split_once('\t').unwrap_or((raw, ""));
    let mut text = String::new();
    let mut accel = None;
    let mut it = label.chars();
    while let Some(c) = it.next() {
        if c != '&' {
            text.push(c);
            continue;
        }
        match it.next() {
            Some('&') => text.push('&'),
            Some(n) => {
                if accel.is_none() {
                    accel = n.to_lowercase().next();
                }
                text.push(n);
            }
            None => {}
        }
    }
    let text = text.trim().to_string();
    let accel = accel.or_else(|| text.chars().next().and_then(|c| c.to_lowercase().next()));
    (text, keys.trim().to_string(), accel)
}

unsafe fn verb_of(cm: &IContextMenu, off: u32) -> String {
    let mut buf = [0u16; 256];
    if cm.GetCommandString(off as usize, GCS_VERBW, None, PSTR(buf.as_mut_ptr() as *mut u8), buf.len() as u32).is_err() {
        return String::new();
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(0);
    String::from_utf16_lossy(&buf[..n])
}

/// Kasumi's icon for one of Windows' own commands (Windows' classic menu has none).
fn glyph_for(verb: &str) -> Option<char> {
    let g = match verb.to_ascii_lowercase().as_str() {
        "open" => '\u{E8E5}',
        "opennewwindow" | "opennewprocess" => '\u{E8A7}',
        "opennewtab" => '\u{EC6C}',
        "cut" => '\u{E8C6}',
        "copy" => '\u{E8C8}',
        "paste" => '\u{E77F}',
        "pastelink" | "link" => '\u{E71B}',
        "delete" => '\u{E74D}',
        "rename" => '\u{E8AC}',
        "properties" => '\u{E946}',
        "runas" => '\u{E7EF}',
        "edit" => '\u{E70F}',
        "print" => '\u{E749}',
        "openas" => '\u{E7AC}',
        "pintohome" | "pintostartscreen" => '\u{E718}',
        "unpinfromhome" => '\u{E77A}',
        "windows.share" | "share" => '\u{E72D}',
        "copyaspath" => '\u{E8C8}',
        "find" => '\u{E721}',
        "undo" => '\u{E7A7}',
        "redo" => '\u{E7A6}',
        "personalize" => '\u{E771}',
        "display" => '\u{E7F4}',
        "openinterminal" | "windows.terminal.open" => '\u{E756}',
        "eject" => '\u{E7E7}',
        "format" => '\u{EDA2}',
        "empty" => '\u{E74D}',
        _ => return None,
    };
    Some(g)
}

/// A menu item's bitmap (32-bit, as handlers give them) as straight RGBA.
unsafe fn pixels(hbmp: HBITMAP) -> Option<Pixels> {
    let v = hbmp.0 as isize;
    // None, or HBMMENU_* (system glyphs: -1 is drawn by its handler, 1..11 Windows').
    if v == 0 || (-1..=11).contains(&v) {
        return None;
    }
    let mut bm = BITMAP::default();
    if GetObjectW(HGDIOBJ(hbmp.0), std::mem::size_of::<BITMAP>() as i32, Some(&mut bm as *mut _ as *mut _)) == 0 {
        return None;
    }
    let (w, h) = (bm.bmWidth, bm.bmHeight.abs());
    if w <= 0 || h <= 0 || w > 256 || h > 256 {
        return None;
    }
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut px = vec![0u8; (w * h * 4) as usize];
    let dc = GetDC(None);
    let got = GetDIBits(dc, hbmp, 0, h as u32, Some(px.as_mut_ptr() as *mut _), &mut bmi, DIB_RGB_COLORS);
    ReleaseDC(None, dc);
    if got == 0 {
        return None;
    }
    let alpha = px.chunks(4).any(|p| p[3] != 0);
    for p in px.chunks_mut(4) {
        let (b, g, r) = (p[0], p[1], p[2]);
        let a = if alpha { p[3] } else { 255 };
        // Premultiplied (as menu bitmaps are) to straight.
        let un = |c: u8| if a == 0 || a == 255 { c } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
        p[0] = un(r);
        p[1] = un(g);
        p[2] = un(b);
        p[3] = a;
    }
    Some((w as u32, h as u32, px))
}

/// No separators at the ends or twice in a row.
pub fn tidy(entries: &mut Vec<Entry>) {
    let mut out: Vec<Entry> = Vec::with_capacity(entries.len());
    for e in entries.drain(..) {
        if e.separator && out.last().is_none_or(|l| l.separator) {
            continue;
        }
        out.push(e);
    }
    while out.last().is_some_and(|l| l.separator) {
        out.pop();
    }
    *entries = out;
}

// --- View, Sort by, Refresh ---------------------------------------------------------

const PROPS: GUID = GUID::from_u128(0xB725F130_47EF_101A_A5F1_02608C9EEBAC);
const SORT_KEYS: [(&str, u32); 4] = [("Name", 10), ("Date modified", 14), ("Type", 4), ("Size", 12)];

pub fn view_entries(t: &Target) -> Vec<Entry> {
    let (mut mode, mut size) = (FOLDERVIEWMODE(0), 0i32);
    unsafe {
        let _ = t.view.GetViewModeAndIconSize(&mut mode, &mut size);
    }
    let modes: Vec<(&str, FOLDERVIEWMODE, i32)> = if t.desktop {
        vec![("Large icons", FVM_ICON, 96), ("Medium icons", FVM_ICON, 48), ("Small icons", FVM_ICON, 32)]
    } else {
        vec![
            ("Extra large icons", FVM_ICON, 256),
            ("Large icons", FVM_ICON, 96),
            ("Medium icons", FVM_ICON, 48),
            ("Small icons", FVM_SMALLICON, 16),
            ("List", FVM_LIST, 16),
            ("Details", FVM_DETAILS, 16),
            ("Tiles", FVM_TILE, 48),
            ("Content", FVM_CONTENT, 32),
        ]
    };
    // The current one: its mode, and for icons the size nearest.
    let current = modes
        .iter()
        .enumerate()
        .filter(|(_, m)| m.1 == mode)
        .min_by_key(|(_, m)| if m.1 == FVM_ICON { (m.2 - size).abs() } else { 0 })
        .map(|(i, _)| i);
    let view = modes
        .iter()
        .enumerate()
        .map(|(i, (name, m, s))| {
            let mut e = Entry::item(name, Action::View(ViewCmd::Mode(m.0, *s)), None);
            e.radio = true;
            e.checked = current == Some(i);
            e
        })
        .collect();

    let mut col = [SORTCOLUMN::default()];
    unsafe {
        let _ = t.view.GetSortColumns(&mut col);
    }
    let ascending = col[0].direction.0 >= 0;
    let mut sort: Vec<Entry> = SORT_KEYS
        .iter()
        .enumerate()
        .map(|(i, (name, pid))| {
            let mut e = Entry::item(name, Action::View(ViewCmd::Sort(i)), None);
            e.radio = true;
            e.checked = col[0].propkey.fmtid == PROPS && col[0].propkey.pid == *pid;
            e
        })
        .collect();
    sort.push(Entry::separator());
    for (name, up) in [("Ascending", true), ("Descending", false)] {
        let mut e = Entry::item(name, Action::View(ViewCmd::Ascending(up)), None);
        e.radio = true;
        e.checked = ascending == up;
        sort.push(e);
    }

    let mut view_e = Entry::item("View", Action::None, Some('\u{E8A9}'));
    view_e.sub = Some(Sub::Ready(view));
    let mut sort_e = Entry::item("Sort by", Action::None, Some('\u{E8CB}'));
    sort_e.sub = Some(Sub::Ready(sort));
    vec![view_e, sort_e, Entry::item("Refresh", Action::View(ViewCmd::Refresh), Some('\u{E72C}'))]
}

pub fn run_view(t: &Target, cmd: ViewCmd) {
    unsafe {
        match cmd {
            ViewCmd::Mode(m, s) => {
                let _ = t.view.SetViewModeAndIconSize(FOLDERVIEWMODE(m), s);
            }
            ViewCmd::Sort(i) => {
                let mut col = [SORTCOLUMN::default()];
                let _ = t.view.GetSortColumns(&mut col);
                let dir = if col[0].direction.0 == 0 { SORT_ASCENDING } else { col[0].direction };
                let key = PROPERTYKEY { fmtid: PROPS, pid: SORT_KEYS[i].1 };
                let _ = t.view.SetSortColumns(&[SORTCOLUMN { propkey: key, direction: dir }]);
            }
            ViewCmd::Ascending(up) => {
                let mut col = [SORTCOLUMN::default()];
                if t.view.GetSortColumns(&mut col).is_ok() {
                    col[0].direction = if up { SORT_ASCENDING } else { SORT_DESCENDING };
                    let _ = t.view.SetSortColumns(&col);
                }
            }
            ViewCmd::Refresh => {
                let _ = t.shell_view.Refresh();
            }
        }
    }
}
