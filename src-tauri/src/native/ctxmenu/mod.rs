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
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::time::Duration;

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

const MSG_REQUEST: u32 = WM_APP + 1;
const MSG_OUTSIDE: u32 = WM_APP + 2;
const MSG_KEY: u32 = WM_APP + 3;
const MSG_FOREGROUND: u32 = WM_APP + 4;
const TIMER_SWITCH: usize = 1;
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_FADE: usize = 2;

struct Open {
    target: Target,
    built: Built,
    levels: Vec<Level>,
    style: Style,
    pt: POINT,
    mon: crate::win::Monitor,
    /// Hovered a moment: this level's row opens its submenu (or closes another).
    switch: Option<(usize, usize)>,
}

struct Ui {
    gfx: Gfx,
    open: Option<Open>,
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
            return;
        };
        HOST.store(h.0 as isize, Ordering::Relaxed);
        let gfx = match Gfx::new() {
            Ok(g) => g,
            Err(e) => {
                eprintln!("[kasumi] context menu: no graphics: {e}");
                return;
            }
        };
        UI.with(|u| *u.borrow_mut() = Some(Ui { gfx, open: None }));
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, None, Some(on_foreground), 0, 0, WINEVENT_OUTOFCONTEXT);
        std::thread::spawn(|| hooks());
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
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
                    if let Some(view) = file_view_at(info.pt) {
                        PENDING.store(view.0 as isize, Ordering::Relaxed);
                        return LRESULT(1);
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
    OPEN.store(false, Ordering::Relaxed);
    with_ui(|u| u.open = None);
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
    let entries = layout::arrange(&menu_cfg, built.entries.clone(), &built.ctx, &target);
    let mon = crate::win::monitors()
        .into_iter()
        .find(|m| pt.x >= m.left && pt.x < m.right && pt.y >= m.top && pt.y < m.bottom)
        .or_else(|| crate::win::monitors().into_iter().next());
    let Some(mon) = mon else { return };
    let shown = with_ui(|u| {
        let style = Style::new(&u.gfx, look, icons);
        let mut root = Level::new(&u.gfx, &style, entries, None, mon.scale as f32).ok()?;
        root.place_root(pt, &mon);
        root.draw(&u.gfx, &style).ok()?;
        u.open = Some(Open { target, built, levels: vec![root], style, pt, mon, switch: None });
        Some(())
    })
    .flatten();
    if shown.is_some() {
        OPEN.store(true, Ordering::Relaxed);
        unsafe {
            SetTimer(Some(host()), TIMER_FADE, 15, None);
        }
    }
}

/// A submenu out of `row` of level `li` (the levels after it go). `first`: its first
/// row chosen (opened with the keys).
fn open_sub(u: &mut Ui, li: usize, row: usize, first: bool) {
    let Some(o) = u.open.as_mut() else { return };
    o.levels.truncate(li + 1);
    let entry = &mut o.levels[li].entries[row];
    let items = match entry.sub.clone() {
        Some(Sub::Pending(h, pos)) => {
            let items = o.built.populate(h, pos);
            entry.sub = Some(Sub::Ready(items.clone()));
            items
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
    let Ok(mut lvl) = Level::new(&u.gfx, &o.style, items, Some(row), o.levels[li].scale) else { return };
    lvl.place_sub(&o.levels[li], row, &o.mon);
    if first {
        lvl.hover = lvl.next_usable(None, 1);
    }
    let _ = o.levels[li].draw(&u.gfx, &o.style);
    let _ = lvl.draw(&u.gfx, &o.style);
    o.levels.push(lvl);
    unsafe {
        SetTimer(Some(host()), TIMER_FADE, 15, None);
    }
}

/// What a chosen row does once the menu is gone.
enum Run {
    Shell(u32, String),
    View(shell::ViewCmd),
    Kasumi(layout::Cmd),
}

/// Row `row` of level `li` chosen: its submenu opens, or the menu closes and its
/// command runs (returned, to run after the menu's state is let go).
fn choose(u: &mut Ui, li: usize, row: usize) -> Option<(Open, Run)> {
    let o = u.open.as_mut()?;
    let e = o.levels.get(li)?.entries.get(row)?;
    if !e.usable() {
        return None;
    }
    if e.sub.is_some() {
        open_sub(u, li, row, false);
        return None;
    }
    let run = match &e.action {
        Action::Shell(off) => Run::Shell(*off, e.verb.clone()),
        Action::View(cmd) => Run::View(*cmd),
        Action::Kasumi(cmd) => Run::Kasumi(cmd.clone()),
        Action::None => return None,
    };
    OPEN.store(false, Ordering::Relaxed);
    let mut open = u.open.take()?;
    open.levels.clear();
    Some((open, run))
}

fn run(chosen: Option<(Open, Run)>) {
    let Some((open, run)) = chosen else { return };
    match run {
        Run::Shell(off, verb) => open.built.invoke(&open.target, off, &verb, open.pt),
        Run::View(cmd) => shell::run_view(&open.target, cmd),
        Run::Kasumi(cmd) => layout::run(&cmd, &open.built.ctx, &open.target),
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
            let _ = o.levels[li].draw(gfx, &o.style);
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
            let _ = o.levels[li].draw(gfx, &o.style);
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
                o.levels.truncate(li + 1);
                if o.levels[li].entries[r].sub.is_some() && o.levels[li].entries[r].usable() {
                    open_sub(u, li, r, false);
                }
            });
        }
        TIMER_FADE => {
            let more = with_ui(|u| u.open.as_mut().is_some_and(|o| o.levels.iter_mut().fold(false, |m, l| l.fade() || m))).unwrap_or(false);
            if more {
                unsafe {
                    SetTimer(Some(host()), TIMER_FADE, 15, None);
                }
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
    let ours = with_ui(|u| u.open.as_ref().is_some_and(|o| fg == o.target.root || o.levels.iter().any(|l| l.hwnd == fg))).unwrap_or(true);
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
                let _ = lvl.draw(gfx, &o.style);
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
                    o.levels.truncate(d);
                    let _ = o.levels[d - 1].draw(gfx, &o.style);
                } else if vk == VK_ESCAPE {
                    OPEN.store(false, Ordering::Relaxed);
                    u.open = None;
                }
                None
            }
            VK_APPS => {
                OPEN.store(false, Ordering::Relaxed);
                u.open = None;
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
                        let _ = lvl.draw(gfx, &o.style);
                        None
                    }
                }
            }
        }
    })
    .flatten();
    run(chosen);
}
