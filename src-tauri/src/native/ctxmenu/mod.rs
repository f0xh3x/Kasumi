//! Kasumi's own right-click menus for Explorer and the desktop ([context_menu],
//! experimental). A helper process (`kasumi.exe --menu`) takes right-clicks over
//! Explorer's file views (a low-level mouse hook), asks Explorer what was clicked,
//! asks Windows for the entries it would show (shell.rs) and draws them in Kasumi's
//! look (menu.rs). Nothing is loaded into Explorer. If the helper fails, right-clicks
//! go back to Explorer's own menu; Kasumi starts it again.

mod layout;
mod menu;
mod shell;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::{Duration, Instant};

use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{OpenProcess, WaitForSingleObject, INFINITE, PROCESS_SYNCHRONIZE};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::*;

use self::menu::{Level, Style};
use self::shell::{Action, Built, Hit, Sub, Target};
use super::gfx::Gfx;
use super::widgets::Look;

// --- In the app: the helper runs while it's on ---------------------------------------

/// Keeps the helper running while [context_menu] is on (native bars), and starts it
/// again if it ends.
pub fn watch(app: tauri::AppHandle) {
    let _ = std::thread::Builder::new().name("kasumi-ctxmenu".into()).spawn(move || {
        let mut child: Option<std::process::Child> = None;
        loop {
            let want = super::active()
                && tauri::Manager::state::<crate::providers::SharedConfig>(&app).read().map(|c| c.context_menu.enabled).unwrap_or(false);
            if let Some(c) = child.as_mut() {
                if let Ok(Some(status)) = c.try_wait() {
                    eprintln!("[kasumi] context menu helper ended ({status})");
                    child = None;
                } else if !want {
                    let _ = c.kill();
                    let _ = c.wait();
                    child = None;
                }
            }
            if want && child.is_none() {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                let spawn = std::env::current_exe().and_then(|exe| {
                    std::process::Command::new(exe).arg("--menu").arg(std::process::id().to_string()).creation_flags(CREATE_NO_WINDOW).spawn()
                });
                match spawn {
                    Ok(c) => child = Some(c),
                    Err(e) => eprintln!("[kasumi] context menu helper didn't start: {e}"),
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

// --- For the editor (settings) --------------------------------------------------------

/// Windows' entries seen in menus so far, per kind of click (the editor lists them).
fn seen_path() -> std::path::PathBuf {
    crate::config::dir().join("menu-seen.json")
}

/// Entries as the editor lists them: text, verb, a submenu or not, the icon (Kasumi's
/// glyph, or the app's own small bitmap).
fn entries_json(entries: &[shell::Entry]) -> Vec<serde_json::Value> {
    use serde_json::json;
    entries
        .iter()
        .filter(|e| !e.separator)
        .map(|e| {
            let mut v = json!({ "text": e.text, "verb": e.verb, "sub": e.sub.is_some() });
            match &e.icon {
                Some(shell::Icon::Glyph(g)) => v["glyph"] = json!(format!("{:X}", *g as u32)),
                Some(shell::Icon::Bitmap((w, h, px))) if *w <= 32 && *h <= 32 => {
                    use base64::Engine;
                    v["bitmap"] = json!({ "w": w, "h": h, "rgba": base64::engine::general_purpose::STANDARD.encode(px) });
                }
                _ => {}
            }
            v
        })
        .collect()
}

/// Remembers a menu's entries for its kind of click: this time's first, in Windows'
/// order, then ones seen before but not now (another file type's, say).
fn remember(name: &str, entries: &[shell::Entry]) {
    use serde_json::{Map, Value};
    let path = seen_path();
    let mut all: Map<String, Value> = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
    let key = |v: &Value| {
        let verb = v.get("verb").and_then(|x| x.as_str()).unwrap_or("");
        if verb.is_empty() {
            format!("text:{}", v.get("text").and_then(|x| x.as_str()).unwrap_or(""))
        } else {
            format!("verb:{verb}")
        }
    };
    let mut merged = entries_json(entries);
    if let Some(Value::Array(old)) = all.get(name) {
        for o in old {
            if !merged.iter().any(|m| key(m) == key(o)) {
                merged.push(o.clone());
            }
        }
    }
    merged.truncate(300);
    let changed = all.get(name) != Some(&Value::Array(merged.clone()));
    if changed {
        all.insert(name.into(), Value::Array(merged));
        let _ = std::fs::write(&path, Value::Object(all).to_string());
    }
}

/// For the settings' context menu editor: the entries seen per kind of click, and
/// Kasumi's default layout (what the editor starts from without one of your own).
#[tauri::command]
pub async fn context_menu_info() -> serde_json::Value {
    let mut seen: serde_json::Value = std::fs::read_to_string(seen_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_else(|| serde_json::json!({}));
    // Kasumi's bar menu: all its entries (Power too), and its folders' own.
    let bar = bar_entries(true, crate::presets::names_async().await);
    seen["bar"] = serde_json::Value::Array(entries_json(&bar));
    for e in &bar {
        if let Some(Sub::Ready(items)) = &e.sub {
            seen[format!("sub:{}", e.key)] = serde_json::Value::Array(entries_json(items));
        }
    }
    serde_json::json!({ "seen": seen, "default": layout::default_layout() })
}

// --- The helper ----------------------------------------------------------------------

/// The helper's message window, which the hooks post to.
static HOST: AtomicIsize = AtomicIsize::new(0);
/// A menu is showing (the hooks send keys and clicks elsewhere to it).
static OPEN: AtomicBool = AtomicBool::new(false);
/// A right button pressed over a file view (its SHELLDLL_DefView): the menu comes
/// when it's let go.
static PENDING: AtomicIsize = AtomicIsize::new(0);
/// Tags the input Kasumi sends (a right-click handed back), so the hook lets it by.
const TAG: usize = 0x4B4D_0000;
/// The helper: its hooks take Explorer's right-clicks (and run all the time). In
/// Kasumi itself (its bar menu) they only steer a menu while it shows.
static INTERCEPT: AtomicBool = AtomicBool::new(false);

const MSG_REQUEST: u32 = WM_APP + 1;
const MSG_OUTSIDE: u32 = WM_APP + 2;
const MSG_KEY: u32 = WM_APP + 3;
const MSG_FOREGROUND: u32 = WM_APP + 4;
const TIMER_SWITCH: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_ANIM: usize = 2;

struct Open {
    /// An Explorer menu: its view and Windows' menu (none: Kasumi's bar menu).
    shell: Option<(Target, Built)>,
    /// Kasumi's bar menu: the app its entries run in, and its click (for its commands).
    app: Option<tauri::AppHandle>,
    bar_ctx: Option<layout::Ctx>,
    /// An entry clicked once that asks for a second ("Confirm?").
    armed: Option<(usize, usize)>,
    levels: Vec<Level>,
    style: Rc<Style>,
    pt: POINT,
    mon: crate::win::Monitor,
    /// Hovered a moment: this level's row opens its submenu (or closes another).
    switch: Option<(usize, usize)>,
}

struct Ui {
    gfx: Gfx,
    open: Option<Open>,
    /// Levels on their way out (closing as they opened).
    closing: Vec<Level>,
    /// The last animation frame (none while nothing moves).
    last: Option<Instant>,
}

/// Draws frames while levels open or close.
fn animate() {
    unsafe {
        SetTimer(Some(host()), TIMER_ANIM, 15, None);
    }
}

/// Levels that go (in order, the menu first): they close as they opened, the last
/// one first and each before it a moment later, then they're gone.
fn retire(closing: &mut Vec<Level>, gone: Vec<Level>) {
    let n = gone.len();
    for (i, mut l) in gone.into_iter().enumerate() {
        l.retire(Duration::from_millis((n - 1 - i) as u64 * menu::CLOSE_STAGGER_MS));
        closing.push(l);
    }
    animate();
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
}

/// The helper's state, unless it's in use further up (a shell call pumping messages).
fn with_ui<R>(f: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    UI.with(|u| u.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn host() -> HWND {
    HWND(HOST.load(Ordering::Relaxed) as _)
}

fn pack(pt: POINT) -> LPARAM {
    LPARAM((((pt.y as i16 as u16 as u32) << 16) | (pt.x as i16 as u16 as u32)) as isize)
}

fn unpack(lp: LPARAM) -> POINT {
    POINT { x: (lp.0 & 0xffff) as u16 as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as u16 as i16 as i32 }
}

fn post(msg: u32, wp: usize, lp: LPARAM) {
    unsafe {
        let _ = PostMessageW(Some(host()), msg, WPARAM(wp), lp);
    }
}

/// `kasumi.exe --menu <app pid>`: runs until the app exits.
pub fn helper(app_pid: u32) {
    std::thread::spawn(move || unsafe {
        if let Ok(app) = OpenProcess(PROCESS_SYNCHRONIZE, false, app_pid) {
            WaitForSingleObject(app, INFINITE);
        }
        std::process::exit(0);
    });
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = windows::Win32::System::Ole::OleInitialize(None);
    }
    if !init_ui() {
        return;
    }
    INTERCEPT.store(true, Ordering::Relaxed);
    std::thread::spawn(|| hooks());
    unsafe {
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

/// The menus' windows and state on this thread (the helper's; in Kasumi, the native
/// thread's, for its bar menu), once.
fn init_ui() -> bool {
    if UI.with(|u| u.try_borrow().is_ok_and(|g| g.is_some())) {
        return true;
    }
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(host_proc), hInstance: instance.into(), lpszClassName: w!("KasumiMenuHost"), ..Default::default() });
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(menu_proc),
            hInstance: instance.into(),
            lpszClassName: menu::CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        });
        let Ok(h) = CreateWindowExW(Default::default(), w!("KasumiMenuHost"), w!(""), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, Some(instance.into()), None)
        else {
            eprintln!("[kasumi] context menu: no window");
            return false;
        };
        HOST.store(h.0 as isize, Ordering::Relaxed);
        let gfx = match Gfx::new() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[kasumi] context menu: no graphics: {e}");
                return false;
            }
        };
        UI.with(|u| *u.borrow_mut() = Some(Ui { gfx, open: None, closing: Vec::new(), last: None }));
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
    }
    true
}

thread_local! {
    /// In Kasumi: the hooks steering its bar menu, while it shows.
    static HOOKS: RefCell<Option<(HHOOK, HHOOK)>> = const { RefCell::new(None) };
}

/// A menu shows (or doesn't). In Kasumi, hooks for its keys and the clicks away from
/// it come and go with it (the helper's run all the time).
fn set_open(on: bool) {
    OPEN.store(on, Ordering::Relaxed);
    if INTERCEPT.load(Ordering::Relaxed) {
        return;
    }
    HOOKS.with(|h| {
        let mut h = h.borrow_mut();
        unsafe {
            if on && h.is_none() {
                let module = GetModuleHandleW(None).ok().map(Into::into);
                if let (Ok(m), Ok(k)) = (SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0), SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), module, 0)) {
                    *h = Some((m, k));
                }
            } else if !on {
                if let Some((m, k)) = h.take() {
                    let _ = UnhookWindowsHookEx(m);
                    let _ = UnhookWindowsHookEx(k);
                }
            }
        }
    });
}

// --- The hooks (their own thread: they must answer at once) -------------------------

fn hooks() {
    unsafe {
        let module = GetModuleHandleW(None).ok().map(Into::into);
        if let Err(e) = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), module, 0) {
            eprintln!("[kasumi] context menu: mouse hook failed: {e}");
            return;
        }
        let _ = SetWindowsHookExW(WH_KEYBOARD_LL, Some(key_proc), module, 0);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    }
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) } as usize;
    String::from_utf16_lossy(&buf[..n])
}

/// The file view (its SHELLDLL_DefView) under `pt`: an Explorer folder window's items
/// or the desktop's, not its navigation pane, address bar or another app's dialog.
fn file_view_at(pt: POINT) -> Option<HWND> {
    unsafe {
        let h = WindowFromPoint(pt);
        if h.is_invalid() || !matches!(class_name(h).as_str(), "DirectUIHWND" | "SysListView32") {
            return None;
        }
        let parent = GetParent(h).ok()?;
        if class_name(parent) != "SHELLDLL_DefView" {
            return None;
        }
        let root = GetAncestor(h, GA_ROOT);
        matches!(class_name(root).as_str(), "CabinetWClass" | "Progman" | "WorkerW").then_some(parent)
    }
}

unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let info = &*(lp.0 as *const MSLLHOOKSTRUCT);
        if info.dwExtraInfo != TAG {
            match wp.0 as u32 {
                WM_RBUTTONDOWN => {
                    if INTERCEPT.load(Ordering::Relaxed) {
                        if let Some(view) = file_view_at(info.pt) {
                            PENDING.store(view.0 as isize, Ordering::Relaxed);
                            return LRESULT(1);
                        }
                    }
                    if OPEN.load(Ordering::Relaxed) {
                        post(MSG_OUTSIDE, 0, pack(info.pt));
                    }
                }
                WM_RBUTTONUP => {
                    let view = PENDING.swap(0, Ordering::Relaxed);
                    if view != 0 {
                        post(MSG_REQUEST, view as usize, pack(info.pt));
                        return LRESULT(1);
                    }
                }
                WM_LBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN => {
                    if OPEN.load(Ordering::Relaxed) {
                        post(MSG_OUTSIDE, 0, pack(info.pt));
                    }
                }
                _ => {}
            }
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

/// While a menu shows: its keys go to it (Explorer doesn't see them); any other key
/// (or Ctrl / Alt / Win with one) closes it and goes on.
unsafe extern "system" fn key_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && OPEN.load(Ordering::Relaxed) {
        let info = &*(lp.0 as *const KBDLLHOOKSTRUCT);
        let down = matches!(wp.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
        let vk = VIRTUAL_KEY(info.vkCode as u16);
        let held = |k: VIRTUAL_KEY| GetAsyncKeyState(k.0 as i32) < 0;
        let mods = held(VK_CONTROL) || held(VK_MENU) || held(VK_LWIN) || held(VK_RWIN);
        let nav = matches!(vk, VK_UP | VK_DOWN | VK_LEFT | VK_RIGHT | VK_RETURN | VK_ESCAPE | VK_HOME | VK_END | VK_SPACE | VK_APPS)
            || (0x30..=0x5A).contains(&vk.0);
        if nav && !mods {
            if down {
                post(MSG_KEY, vk.0 as usize, LPARAM(0));
            }
            return LRESULT(1);
        }
        if down && !matches!(vk, VK_SHIFT | VK_LSHIFT | VK_RSHIFT) {
            post(MSG_OUTSIDE, 1, LPARAM(0));
        }
    }
    CallNextHookEx(None, code, wp, lp)
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, hwnd: HWND, _: i32, _: i32, _: u32, _: u32) {
    if OPEN.load(Ordering::Relaxed) {
        post(MSG_FOREGROUND, hwnd.0 as usize, LPARAM(0));
    }
}

/// Gives the right-click back to Explorer (its own menu then shows): for what Kasumi
/// can't make a menu of.
fn hand_back() {
    let input = |flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 { mi: MOUSEINPUT { dwFlags: flags, dwExtraInfo: TAG, ..Default::default() } },
    };
    unsafe {
        SendInput(&[input(MOUSEEVENTF_RIGHTDOWN), input(MOUSEEVENTF_RIGHTUP)], std::mem::size_of::<INPUT>() as i32);
    }
}

// --- The menu ------------------------------------------------------------------------

unsafe extern "system" fn host_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        MSG_REQUEST => request(HWND(wp.0 as _), unpack(lp)),
        MSG_OUTSIDE => outside(unpack(lp), wp.0 == 1),
        MSG_KEY => key(VIRTUAL_KEY(wp.0 as u16)),
        MSG_FOREGROUND => foreground(HWND(wp.0 as _)),
        WM_TIMER => timer(wp.0),
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    LRESULT(0)
}

fn close() {
    set_open(false);
    with_ui(|u| {
        if let Some(mut o) = u.open.take() {
            let gone = std::mem::take(&mut o.levels);
            retire(&mut u.closing, gone);
        }
    });
}

/// The theme's look (with Windows' accent, if it follows it).
fn look(cfg: Option<&crate::config::Config>) -> Look {
    let mut theme = cfg.map(|c| c.theme.clone()).unwrap_or_default();
    if theme.accent_sync {
        if let Some(a) = crate::win::system_accent() {
            theme.accent = a;
        }
    }
    Look::from_theme(&theme, false)
}

fn request(defview: HWND, pt: POINT) {
    close();
    let Some(target) = shell::find_view(defview) else {
        eprintln!("[kasumi] context menu: view not found; Explorer's menu instead");
        hand_back();
        return;
    };
    let hit = shell::select_under(&target, pt);
    if matches!(hit, Hit::Other) {
        hand_back();
        return;
    }
    let shift = unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0;
    let Some(built) = shell::build(&target, &hit, shift) else {
        eprintln!("[kasumi] context menu: no entries; Explorer's menu instead");
        hand_back();
        return;
    };
    let cfg = crate::config::load().ok();
    let menu_cfg = cfg.as_ref().map(|c| c.context_menu.clone()).unwrap_or_default();
    let (look, icons) = (look(cfg.as_ref()), menu_cfg.icons);
    let anim = cfg.as_ref().map(|c| c.animation.clone()).unwrap_or_default();
    remember(built.ctx.kind.name(), &built.entries);
    let entries = layout::arrange(&menu_cfg, built.entries.clone(), &built.ctx, Some(&target));
    show(entries, pt, Some((target, built)), None, None, look, icons, anim);
}

/// A menu at `pt` (the cursor): Explorer's (`shell`) or Kasumi's bar menu (`app`).
#[allow(clippy::too_many_arguments)]
fn show(
    entries: Vec<shell::Entry>,
    pt: POINT,
    shell: Option<(Target, Built)>,
    app: Option<tauri::AppHandle>,
    bar_ctx: Option<layout::Ctx>,
    look: Look,
    icons: bool,
    anim: crate::config::Animation,
) {
    let mon = crate::win::monitors()
        .into_iter()
        .find(|m| pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom)
        .or_else(|| crate::win::monitors().into_iter().next());
    let Some(mon) = mon else { return };
    let shown = with_ui(|u| {
        let style = Style::new(&u.gfx, look, icons, anim);
        let mut root = Level::new(&u.gfx, style.clone(), entries, None, mon.scale as f32).ok()?;
        root.place_root(pt, &mon);
        root.draw(&u.gfx).ok()?;
        u.open = Some(Open { shell, app, bar_ctx, armed: None, levels: vec![root], style, pt, mon, switch: None });
        Some(())
    })
    .flatten();
    if shown.is_some() {
        set_open(true);
        animate();
    }
}

/// A submenu out of `row` of level `li` (the levels after it go). `first`: its first
/// row chosen (opened with the keys).
fn open_sub(u: &mut Ui, li: usize, row: usize, first: bool) {
    let Some(o) = u.open.as_mut() else { return };
    if o.levels.len() > li + 1 {
        let gone = o.levels.split_off(li + 1);
        retire(&mut u.closing, gone);
    }
    let entry = &mut o.levels[li].entries[row];
    let items = match entry.sub.clone() {
        Some(Sub::Pending(h, pos)) => {
            let Some((target, built)) = &o.shell else { return };
            let items = built.populate(h, pos);
            // Remembered for the editor (by the submenu's own key), then arranged by its rules.
            if !entry.key.is_empty() {
                remember(&format!("sub:{}", entry.key), &items);
            }
            let items = if entry.rules.is_empty() { items } else { layout::arrange_sub(&entry.rules, items, &built.ctx, Some(target)) };
            entry.sub = Some(Sub::Ready(items.clone()));
            items
        }
        // One of Kasumi's (the bar menu's Presets...): arranged by its rules, once.
        Some(Sub::Ready(v)) if !entry.rules.is_empty() => {
            let rules = std::mem::take(&mut entry.rules);
            let ctx = o.shell.as_ref().map(|(_, b)| &b.ctx).or(o.bar_ctx.as_ref());
            let v = match ctx {
                Some(ctx) => layout::arrange_sub(&rules, v, ctx, o.shell.as_ref().map(|(t, _)| t)),
                None => v,
            };
            entry.sub = Some(Sub::Ready(v.clone()));
            v
        }
        Some(Sub::Ready(v)) => v,
        None => return,
    };
    let items = if items.is_empty() {
        let mut e = shell::Entry::separator();
        e.separator = false;
        e.text = "Empty".into();
        e.disabled = true;
        vec![e]
    } else {
        items
    };
    o.levels[li].hover = Some(row);
    let Ok(mut lvl) = Level::new(&u.gfx, o.style.clone(), items, Some(row), o.levels[li].scale) else { return };
    lvl.place_sub(&o.levels[li], row, &o.mon);
    menu::join(&mut o.levels[li], &mut lvl);
    if first {
        lvl.hover = lvl.next_usable(None, 1);
    }
    let _ = o.levels[li].draw(&u.gfx);
    let _ = lvl.draw(&u.gfx);
    o.levels.push(lvl);
    animate();
}

/// What a chosen row does once the menu is gone.
enum Run {
    Shell(u32, String),
    View(shell::ViewCmd),
    Kasumi(layout::Cmd),
    App(String),
}

/// Row `row` of level `li` chosen: its submenu opens; one that asks for a second
/// click says "Confirm?" first; else the menu closes and its command runs (returned,
/// to run after the menu's state is let go).
fn choose(u: &mut Ui, li: usize, row: usize) -> Option<(Open, Run)> {
    let o = u.open.as_mut()?;
    let (usable, sub, confirm, action, verb) = {
        let e = o.levels.get(li)?.entries.get(row)?;
        (e.usable(), e.sub.is_some(), e.confirm, e.action.clone(), e.verb.clone())
    };
    if !usable {
        return None;
    }
    if sub {
        open_sub(u, li, row, false);
        return None;
    }
    if confirm && o.armed != Some((li, row)) {
        o.armed = Some((li, row));
        o.levels[li].entries[row].text = "Confirm?".into();
        let _ = o.levels[li].draw(&u.gfx);
        return None;
    }
    let run = match action {
        Action::Shell(off) => Run::Shell(off, verb),
        Action::View(cmd) => Run::View(cmd),
        Action::Kasumi(cmd) => Run::Kasumi(cmd),
        Action::App(id) => Run::App(id),
        Action::None => return None,
    };
    set_open(false);
    let mut open = u.open.take()?;
    let gone = std::mem::take(&mut open.levels);
    retire(&mut u.closing, gone);
    Some((open, run))
}

fn run(chosen: Option<(Open, Run)>) {
    let Some((open, run)) = chosen else { return };
    match (run, &open.shell, &open.app) {
        (Run::Shell(off, verb), Some((t, b)), _) => b.invoke(t, off, &verb, open.pt),
        (Run::View(cmd), Some((t, _)), _) => shell::run_view(t, cmd),
        (Run::Kasumi(cmd), shell, _) => {
            let ctx = shell.as_ref().map(|(_, b)| &b.ctx).or(open.bar_ctx.as_ref());
            if let Some(ctx) = ctx {
                layout::run(&cmd, ctx, shell.as_ref().map(|(t, _)| t));
            }
        }
        (Run::App(id), _, Some(app)) => run_app(app, &id),
        _ => {}
    }
}

fn level_of(o: &Open, hwnd: HWND) -> Option<usize> {
    o.levels.iter().position(|l| l.hwnd == hwnd)
}

unsafe extern "system" fn menu_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
    match msg {
        WM_MOUSEACTIVATE => return LRESULT(MA_NOACTIVATE as isize),
        WM_MOUSEMOVE => {
            let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
            let _ = TrackMouseEvent(&mut tme);
            hover(hwnd, x, y);
        }
        WM_MOUSELEAVE => leave(hwnd),
        WM_LBUTTONUP | WM_RBUTTONUP => {
            let chosen = with_ui(|u| {
                let o = u.open.as_ref()?;
                let li = level_of(o, hwnd)?;
                let row = o.levels[li].row_at(x, y)?;
                choose(u, li, row)
            })
            .flatten();
            run(chosen);
        }
        _ => return DefWindowProcW(hwnd, msg, wp, lp),
    }
    LRESULT(0)
}

fn hover(hwnd: HWND, x: i32, y: i32) {
    with_ui(|u| {
        let gfx = &u.gfx;
        let Some(o) = u.open.as_mut() else { return };
        let Some(li) = level_of(o, hwnd) else { return };
        let row = o.levels[li].row_at(x, y);
        if o.levels[li].hover != row && row.is_some() {
            o.levels[li].hover = row;
            let _ = o.levels[li].draw(gfx);
        }
        let Some(r) = row else { return };
        let child = o.levels.get(li + 1).map(|c| c.parent_row);
        if child == Some(Some(r)) {
            o.switch = None;
        } else if child.is_some() || o.levels[li].entries[r].sub.is_some() {
            if o.switch != Some((li, r)) {
                o.switch = Some((li, r));
                unsafe {
                    SetTimer(Some(host()), TIMER_SWITCH, if child.is_some() { 300 } else { 250 }, None);
                }
            }
        }
    });
}

fn leave(hwnd: HWND) {
    with_ui(|u| {
        let gfx = &u.gfx;
        let Some(o) = u.open.as_mut() else { return };
        let Some(li) = level_of(o, hwnd) else { return };
        let keep = o.levels.get(li + 1).and_then(|c| c.parent_row);
        if o.levels[li].hover != keep {
            o.levels[li].hover = keep;
            let _ = o.levels[li].draw(gfx);
        }
    });
}

fn timer(id: usize) {
    unsafe {
        let _ = KillTimer(Some(host()), id);
    }
    match id {
        TIMER_SWITCH => {
            with_ui(|u| {
                let Some(o) = u.open.as_mut() else { return };
                let Some((li, r)) = o.switch.take() else { return };
                if o.levels.get(li).is_none_or(|l| l.hover != Some(r)) {
                    return;
                }
                if o.levels.len() > li + 1 {
                    let gone = o.levels.split_off(li + 1);
                    retire(&mut u.closing, gone);
                    let _ = o.levels[li].draw(&u.gfx);
                }
                if o.levels[li].entries[r].sub.is_some() && o.levels[li].entries[r].usable() {
                    open_sub(u, li, r, false);
                }
            });
        }
        TIMER_ANIM => {
            let more = with_ui(|u| {
                let now = Instant::now();
                let dt = u.last.map_or(1.0 / 60.0, |t| now.duration_since(t).as_secs_f64().min(0.05));
                u.last = Some(now);
                let gfx = &u.gfx;
                let mut more = false;
                let open = u.open.as_mut().map(|o| o.levels.iter_mut()).into_iter().flatten();
                for l in open.chain(u.closing.iter_mut()) {
                    if l.tick(dt) {
                        let _ = l.draw(gfx);
                        more = true;
                    }
                }
                // Gone: their menus' corners on their side come back (unless another
                // submenu has joined there since).
                let gone: Vec<isize> = u.closing.iter().filter(|l| l.gone()).map(|l| l.hwnd.0 as isize).collect();
                u.closing.retain(|l| !l.gone());
                if let Some(o) = u.open.as_mut() {
                    for l in o.levels.iter_mut() {
                        if gone.iter().any(|&g| l.unjoin(g)) {
                            let _ = l.draw(gfx);
                        }
                    }
                }
                if !more {
                    u.last = None;
                }
                more || !u.closing.is_empty()
            })
            .unwrap_or(false);
            if more {
                animate();
            }
        }
        _ => {}
    }
}

fn outside(pt: POINT, force: bool) {
    let inside = with_ui(|u| u.open.as_ref().is_some_and(|o| o.levels.iter().any(|l| l.contains(pt)))).unwrap_or(false);
    if force || !inside {
        close();
    }
}

fn foreground(fg: HWND) {
    let ours = with_ui(|u| u.open.as_ref().is_some_and(|o| o.shell.as_ref().is_some_and(|(t, _)| fg == t.root) || o.levels.iter().any(|l| l.hwnd == fg))).unwrap_or(true);
    if !ours {
        close();
    }
}

fn key(vk: VIRTUAL_KEY) {
    let chosen = with_ui(|u| {
        let gfx = &u.gfx;
        let o = u.open.as_mut()?;
        let d = o.levels.len() - 1;
        let lvl = &mut o.levels[d];
        let cur = lvl.hover;
        match vk {
            VK_DOWN | VK_UP | VK_HOME | VK_END => {
                lvl.hover = match vk {
                    VK_DOWN => lvl.next_usable(cur, 1),
                    VK_UP => lvl.next_usable(cur, -1),
                    VK_HOME => lvl.next_usable(None, 1),
                    _ => lvl.next_usable(None, -1),
                };
                let _ = lvl.draw(gfx);
                None
            }
            VK_RIGHT => {
                if let Some(r) = cur.filter(|&r| lvl.entries[r].sub.is_some() && lvl.entries[r].usable()) {
                    open_sub(u, d, r, true);
                }
                None
            }
            VK_LEFT | VK_ESCAPE => {
                if d > 0 {
                    let gone = o.levels.split_off(d);
                    retire(&mut u.closing, gone);
                    let _ = o.levels[d - 1].draw(gfx);
                } else if vk == VK_ESCAPE {
                    set_open(false);
                    if let Some(mut o) = u.open.take() {
                        let gone = std::mem::take(&mut o.levels);
                        retire(&mut u.closing, gone);
                    }
                }
                None
            }
            VK_APPS => {
                set_open(false);
                if let Some(mut o) = u.open.take() {
                    let gone = std::mem::take(&mut o.levels);
                    retire(&mut u.closing, gone);
                }
                None
            }
            VK_RETURN | VK_SPACE => {
                let r = cur?;
                if lvl.entries[r].sub.is_some() {
                    open_sub(u, d, r, true);
                    None
                } else {
                    choose(u, d, r)
                }
            }
            _ => {
                // A letter: the row it marks (chosen if it's the only one), else the next.
                let ch = (vk.0 as u8 as char).to_ascii_lowercase();
                let hits: Vec<usize> = (0..lvl.entries.len()).filter(|&i| lvl.entries[i].usable() && lvl.entries[i].accel == Some(ch)).collect();
                match hits.as_slice() {
                    [] => None,
                    [only] => {
                        let r = *only;
                        if lvl.entries[r].sub.is_some() {
                            open_sub(u, d, r, true);
                            None
                        } else {
                            choose(u, d, r)
                        }
                    }
                    many => {
                        lvl.hover = Some(many.iter().copied().find(|&i| cur.is_none_or(|c| i > c)).unwrap_or(many[0]));
                        let _ = lvl.draw(gfx);
                        None
                    }
                }
            }
        }
    })
    .flatten();
    run(chosen);
}

// --- Kasumi's bar menu (right-click on a bar's empty space; on the native thread) ------

/// The bar menu at the cursor.
pub fn bar_menu(app: tauri::AppHandle) {
    if !init_ui() {
        return;
    }
    close();
    let Some((x, y)) = crate::win::cursor_pos() else { return };
    let cfg = crate::config::load().ok();
    let power = cfg.as_ref().is_some_and(|c| c.bar_menu.power);
    let anim = cfg.as_ref().map(|c| c.animation.clone()).unwrap_or_default();
    // Arranged by its layout ([bar_menu] layout; none: as Kasumi lists them).
    let home = std::env::var("USERPROFILE").unwrap_or_default();
    let shift = unsafe { GetAsyncKeyState(VK_SHIFT.0 as i32) } < 0;
    let ctx = layout::Ctx { kind: layout::Kind::Bar, paths: Vec::new(), dir: home.clone(), here: home, shift };
    let mut rules = cfg.as_ref().map(|c| c.bar_menu.layout.clone()).unwrap_or_default();
    if rules.is_empty() {
        rules.push(crate::config::MenuNode { rest: true, ..Default::default() });
    }
    let entries = layout::arrange_with(&rules, &[], bar_entries(power, crate::presets::names()), &ctx, None);
    show(entries, POINT { x, y }, None, Some(app), Some(ctx), look(cfg.as_ref()), true, anim);
}

/// An entry's name in layouts: `verb:kasumi.<id>` (the editor places it by that).
fn named_as(mut e: shell::Entry, id: &str) -> shell::Entry {
    e.verb = format!("kasumi.{}", id.replace(':', "."));
    e.key = format!("verb:{}", e.verb);
    e
}

fn app_item(text: &str, id: &str, g: char) -> shell::Entry {
    named_as(shell::Entry::named(text, Action::App(id.into()), Some(shell::Icon::Glyph(g))), id)
}

fn app_folder(text: &str, id: &str, g: char, items: Vec<shell::Entry>) -> shell::Entry {
    let mut e = shell::Entry::named(text, Action::None, Some(shell::Icon::Glyph(g)));
    e.sub = Some(Sub::Ready(items));
    named_as(e, id)
}

fn bar_entries(power: bool, presets: Vec<(String, bool)>) -> Vec<shell::Entry> {
    let mut presets: Vec<shell::Entry> = presets
        .into_iter()
        .map(|(name, current)| {
            let id = format!("preset:{name}");
            let mut e = named_as(shell::Entry::named(&name, Action::App(id.clone()), None), &id);
            e.radio = true;
            e.checked = current;
            e
        })
        .collect();
    if !presets.is_empty() {
        presets.push(shell::Entry::separator());
    }
    presets.push(app_item("Manage presets…", "page:presets", '\u{E713}'));
    let mut v = vec![
        app_item("Kasumi settings", "settings", '\u{E713}'),
        app_item("Edit this bar…", "page:widgets", '\u{E70F}'),
        app_folder("Presets", "presets", '\u{E790}', presets),
        shell::Entry::separator(),
        app_item("Task Manager", "taskmgr", '\u{E9D9}'),
        app_item("Show desktop", "desktop", '\u{E7F4}'),
        app_folder(
            "Hide the bars",
            "hide",
            '\u{ED1A}',
            vec![
                app_item("For 10 minutes", "hide:600", '\u{E916}'),
                app_item("For an hour", "hide:3600", '\u{E916}'),
                app_item("Until I bring them back (tray icon)", "hide:0", '\u{ED1A}'),
            ],
        ),
        shell::Entry::separator(),
    ];
    if power {
        let confirm = |mut e: shell::Entry| {
            e.confirm = true;
            e
        };
        v.push(app_folder(
            "Power",
            "power",
            '\u{E7E8}',
            vec![
                app_item("Lock", "power:lock", '\u{E72E}'),
                confirm(app_item("Sleep", "power:sleep", '\u{E708}')),
                confirm(app_item("Restart", "power:restart", '\u{E777}')),
                confirm(app_item("Shut down", "power:shutdown", '\u{E7E8}')),
            ],
        ));
        v.push(shell::Entry::separator());
    }
    v.push(app_item("Restart Kasumi", "restart", '\u{E72C}'));
    v.push(app_item("Quit Kasumi", "quit", '\u{E711}'));
    v
}

/// One of the bar menu's entries.
fn run_app(app: &tauri::AppHandle, id: &str) {
    let (kind, arg) = id.split_once(':').unwrap_or((id, ""));
    match kind {
        "settings" => crate::settings::open(app),
        "page" => crate::settings::open_page(app, arg),
        "preset" => {
            let name = arg.to_string();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::presets::preset_apply(name).await {
                    eprintln!("[kasumi] preset: {e}");
                }
            });
        }
        "taskmgr" => {
            let _ = std::process::Command::new("taskmgr.exe").spawn();
        }
        "desktop" => crate::win::toggle_desktop(),
        "hide" => crate::engine::snooze(arg.parse().unwrap_or(0)),
        "power" => {
            let (app, action) = (app.clone(), arg.to_string());
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::actions::run_action(app, action.clone(), None).await {
                    eprintln!("[kasumi] {action}: {e}");
                }
            });
        }
        "restart" => app.restart(),
        "quit" => app.exit(0),
        _ => {}
    }
}
