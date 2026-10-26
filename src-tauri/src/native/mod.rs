//! The native renderer ([general] renderer = "native"): the bars and docks drawn
//! with Direct2D / DirectWrite into layered windows instead of WebView2 pages.
//! One thread owns every surface; the engine and providers talk to it through
//! `ui.rs`, which posts `Msg`s here. A surface reports what a page reported
//! (notch widths, interactive regions) straight to the engine.

pub mod ctxmenu;
pub mod geometry;
mod drop;
mod floating;
mod gfx;
mod icons;
mod popup;
mod scrollbar;
pub mod snapshot;
mod surface;
mod tooltip;
mod widgets;

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use serde_json::Value;
use tauri::AppHandle;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{SetThreadDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows::Win32::UI::WindowsAndMessaging::*;

use crate::engine::{Layout, NotchLayout};
use crate::win::Monitor;
use widgets::{Act, Button};

pub use surface::Surface;

/// Damped spring (src/spring.ts), integrated in fixed sub-steps.
#[derive(Clone, Debug)]
pub struct Spring {
    pub value: f64,
    pub velocity: f64,
    pub target: f64,
    pub stiffness: f64,
    pub damping: f64,
    pub mass: f64,
    /// Speed while heading to 1 (opening) and to 0 (closing).
    pub open: f64,
    pub close: f64,
    /// [animation] bounce: may overshoot; off: at least critically damped.
    pub bounce: bool,
}

impl Spring {
    pub fn new(value: f64, stiffness: f64, damping: f64) -> Spring {
        Spring { value, velocity: 0.0, target: value, stiffness, damping, mass: 1.0, open: 1.0, close: 1.0, bounce: false }
    }

    pub fn step(&mut self, dt: f64) {
        let f = if self.target >= 0.5 { self.open } else { self.close };
        let k = self.stiffness * f * f;
        let c = if self.bounce { self.damping * f } else { (self.damping * f).max(2.0 * (k * self.mass).sqrt()) };
        let mut left = dt;
        while left > 0.0 {
            let s = left.min(1.0 / 240.0);
            let a = (-k * (self.value - self.target) - c * self.velocity) / self.mass;
            self.velocity += a * s;
            self.value += self.velocity * s;
            left -= s;
        }
    }

    pub fn settled(&self) -> bool {
        (self.value - self.target).abs() < 5e-4 && self.velocity.abs() < 5e-3
    }

    pub fn snap(&mut self) {
        self.value = self.target;
        self.velocity = 0.0;
    }

    /// Stiffness, damping and mass ([animation]), the open / close speeds, bounce.
    pub fn configure(&mut self, stiffness: f64, damping: f64, a: &crate::config::Animation) {
        self.stiffness = stiffness;
        self.damping = damping;
        self.mass = a.mass.max(0.01);
        let ok = |v: f64| if v.is_finite() && v > 0.0 { v.clamp(0.25, 4.0) } else { 1.0 };
        self.open = ok(a.open_speed);
        self.close = ok(a.close_speed);
        self.bounce = a.bounce;
    }
}

/// What the engine and providers tell the native thread.
pub enum Msg {
    Create { label: String, monitor: Monitor, layout: Box<Layout>, reply: Sender<isize> },
    Destroy(String),
    Layout(String, Box<Layout>),
    /// The monitor or the window's height changed (engine `win::place`).
    Place(String, Monitor),
    State(String, bool),
    Positions(String, Vec<NotchLayout>),
    Nudge(String, Option<String>),
    /// Pop-ups and the rest: event name and payload, as the pages got them.
    Event(String, String, Value),
    Data(String, Value),
    /// Entries of an object key that changed (publish_entries).
    Entries(String, serde_json::Map<String, Value>),
    Shown(String, bool),
    ClickThrough(String, bool),
    /// Win + n (1..9): the tasks widget's nth app, on the monitor under the cursor.
    WinKey(u8),
    /// Alt + Tab's keys (Kasumi's switcher).
    AltTab(crate::keyhook::AltTabKey),
    Lang(crate::keyhook::AltTabKey),
    /// Win + Space still held a moment after: the picker shows (that press's number).
    LangReveal(u64),
    /// Alt + Tab's windows changed while it shows (one closed from it): laid out again.
    AltTabChanged,
    /// The scroll position pop-up (osd/scroll.rs).
    Scroll(Box<ScrollShow>),
}

/// Where a view has scrolled to (shares of the track), on which monitor, for how long,
/// how long the bar is, and the springs it moves with.
pub struct ScrollShow {
    pub monitor: Monitor,
    pub top: f32,
    pub size: f32,
    pub ms: u64,
    pub length: f64,
    pub width: f64,
    pub anim: crate::config::Animation,
    pub theme: crate::config::Theme,
    pub geometry: crate::config::Geometry,
}

/// Alt + Tab's keys from the keyboard helper (and the commit hotkey).
pub fn alt_tab(key: crate::keyhook::AltTabKey) {
    post(Msg::AltTab(key));
}

/// The language picker's keys from the keyboard helper (Win + Space).
pub fn lang_switch(key: crate::keyhook::AltTabKey) {
    post(Msg::Lang(key));
}

/// Where each app's icon is in the native bars and docks (screen px): the surface, its
/// monitor, the app key and box. A window minimizing animates there (providers/tasks.rs
/// answers the shell's HSHELL_GETMINRECT with it).
static ICONS: Mutex<Vec<(String, String, String, [i32; 4])>> = Mutex::new(Vec::new());

pub fn icon_rect(device: &str, app: &str) -> Option<[i32; 4]> {
    ICONS.lock().unwrap().iter().find(|(_, d, a, _)| d == device && a == app).map(|e| e.3)
}

/// A surface's icons (its earlier ones replaced; another surface's on the monitor kept).
pub(crate) fn set_icon_rects(surface: &str, device: &str, rects: Vec<(String, [i32; 4])>) {
    let mut icons = ICONS.lock().unwrap();
    icons.retain(|(s, _, _, _)| s != surface);
    icons.extend(rects.into_iter().map(|(a, r)| (surface.to_string(), device.to_string(), a, r)));
}

/// Win + n from the keyboard helper.
pub fn win_key(n: u8) {
    post(Msg::WinKey(n));
}

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// Surfaces drawn off screen (the headless live test in snapshot.rs).
static OFFSCREEN: AtomicBool = AtomicBool::new(false);
/// Frames drawn while animating (the live test counts them).
pub static FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static SENDER: OnceLock<Mutex<Sender<Msg>>> = OnceLock::new();
/// The thread's message-only window: posting to it wakes the thread.
static WAKE: AtomicIsize = AtomicIsize::new(0);
const WM_WAKE: u32 = WM_APP + 1;
/// The cursor left a window (TrackMouseEvent).
const WM_MOUSELEAVE: u32 = 0x02A3;
const TIMER_WIDGETS: usize = 1;
const TIMER_SCROLL: usize = 2;

/// Whether the native renderer draws the bars (chosen at start).
pub fn active() -> bool {
    ACTIVE.load(Ordering::Relaxed)
}

/// Hands a message to the native thread.
pub fn post(msg: Msg) {
    let Some(tx) = SENDER.get() else { return };
    if tx.lock().unwrap().send(msg).is_ok() {
        let h = WAKE.load(Ordering::Relaxed);
        if h != 0 {
            unsafe {
                let _ = PostMessageW(Some(HWND(h as *mut _)), WM_WAKE, WPARAM(0), LPARAM(0));
            }
        }
    }
}

/// Creates a surface's window on the native thread; its handle (0 on failure).
pub fn create(label: &str, monitor: &Monitor, layout: &Layout) -> isize {
    let (reply, rx) = mpsc::channel();
    post(Msg::Create { label: label.to_string(), monitor: monitor.clone(), layout: Box::new(layout.clone()), reply });
    rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap_or(0)
}

/// Starts the native thread (the renderer is native from here on).
pub fn start(app: &AppHandle) {
    spawn(Some(app.clone()));
}

/// The native thread without the app, its surfaces off screen (snapshot.rs's live test).
pub fn start_headless() {
    OFFSCREEN.store(true, Ordering::Relaxed);
    spawn(None);
}

fn spawn(app: Option<AppHandle>) {
    let (tx, rx) = mpsc::channel();
    if SENDER.set(Mutex::new(tx)).is_err() {
        return;
    }
    ACTIVE.store(true, Ordering::Relaxed);
    let (ready_tx, ready_rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("kasumi-native".into())
        .spawn(move || run(app, rx, ready_tx))
        .expect("spawn native thread");
    // The engine creates surfaces right away: wait for the wake window.
    let _ = ready_rx.recv_timeout(std::time::Duration::from_secs(5));
}

/// Everything the native thread owns.
pub(crate) struct Native {
    gfx: gfx::Gfx,
    surfaces: Vec<Surface>,
    /// Every published value (all keys: a surface built later catches up from here).
    store: HashMap<String, Value>,
    app: Option<AppHandle>,
    /// Alt + Tab in the middle of the screen (made the first time).
    alt_float: Option<floating::Floating>,
    /// The scroll position bar (made the first time).
    scroll: Option<scrollbar::ScrollBar>,
    /// Win + Space pressed, the picker not shown yet (a quick press just switches):
    /// which press (its number) and its direction.
    lang_pending: Option<(u64, bool)>,
    lang_presses: u64,
}

thread_local! {
    static NATIVE: RefCell<Option<Native>> = const { RefCell::new(None) };
}

pub(crate) const SURFACE_CLASS: windows::core::PCWSTR = w!("KasumiNativeSurface");

/// Registers the surface window class (once per process).
pub(crate) fn register_class() {
    static DONE: AtomicBool = AtomicBool::new(false);
    if DONE.swap(true, Ordering::Relaxed) {
        return;
    }
    unsafe {
        let instance = GetModuleHandleW(None).unwrap_or_default();
        RegisterClassW(&WNDCLASSW {
            lpfnWndProc: Some(surface_proc),
            hInstance: instance.into(),
            lpszClassName: SURFACE_CLASS,
            hCursor: LoadCursorW(None, IDC_ARROW).unwrap_or_default(),
            ..Default::default()
        });
        RegisterClassW(&WNDCLASSW { lpfnWndProc: Some(wake_proc), hInstance: instance.into(), lpszClassName: w!("KasumiNativeWake"), ..Default::default() });
    }
}

fn run(app: Option<AppHandle>, rx: Receiver<Msg>, ready: Sender<()>) {
    unsafe {
        // OLE (an STA, as COM needs for the shell) for the windows' file drop targets.
        let _ = windows::Win32::System::Ole::OleInitialize(None);
        let _ = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    register_class();
    let gfx = match gfx::Gfx::new() {
        Ok(g) => g,
        Err(e) => {
            eprintln!("[kasumi] native: Direct2D unavailable: {e}");
            return;
        }
    };
    let wake = unsafe {
        CreateWindowExW(WINDOW_EX_STYLE(0), w!("KasumiNativeWake"), w!("Kasumi"), WINDOW_STYLE(0), 0, 0, 0, 0, Some(HWND_MESSAGE), None, None, None)
    };
    let Ok(wake) = wake else {
        eprintln!("[kasumi] native: no message window");
        return;
    };
    WAKE.store(wake.0 as isize, Ordering::Relaxed);
    NATIVE.with(|n| *n.borrow_mut() = Some(Native { gfx, surfaces: Vec::new(), store: HashMap::new(), app, alt_float: None, scroll: None, lang_pending: None, lang_presses: 0 }));
    INBOX.with(|i| *i.borrow_mut() = Some(rx));
    let _ = ready.send(());
    eprintln!("[kasumi] native renderer started");

    // Idle: wait for messages. Animating: one frame per display refresh (DwmFlush
    // waits for the next composition), handling messages in between.
    let mut msg = MSG::default();
    let mut last = Instant::now();
    'run: loop {
        let animating = NATIVE.with(|n| n.try_borrow().is_ok_and(|n| n.as_ref().is_some_and(|n| n.surfaces.iter().any(|s| s.animating) || n.scroll.as_ref().is_some_and(|b| b.animating()))));
        unsafe {
            if animating {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    if msg.message == WM_QUIT {
                        break 'run;
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                let now = Instant::now();
                let dt = (now - last).as_secs_f64().min(0.05);
                last = now;
                with_native(|n| {
                    for s in n.surfaces.iter_mut().filter(|s| s.animating) {
                        s.frame(&n.gfx, dt);
                        FRAMES.fetch_add(1, Ordering::Relaxed);
                        // What panels asked for as they shut (clear_on_close).
                        for act in s.take_acts() {
                            run_act(act, n.app.clone());
                        }
                    }
                    if let Some(b) = n.scroll.as_mut().filter(|b| b.animating()) {
                        b.frame(&n.gfx, dt);
                    }
                    report(n);
                });
                let _ = DwmFlush();
            } else {
                if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    break 'run;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
                last = Instant::now();
            }
        }
    }
    NATIVE.with(|n| n.borrow_mut().take());
}

thread_local! {
    static INBOX: RefCell<Option<Receiver<Msg>>> = const { RefCell::new(None) };
}

/// Runs `f` on the native state unless it's busy (a call that pumps messages
/// re-entered the window procedure): then it's skipped (timers and the inbox
/// come again).
fn with_native<R>(f: impl FnOnce(&mut Native) -> R) -> Option<R> {
    NATIVE.with(|n| {
        let mut n = n.try_borrow_mut().ok()?;
        n.as_mut().map(f)
    })
}

/// Sends the engine what surfaces measured (notch widths, regions).
fn report(n: &mut Native) {
    let Some(app) = n.app.as_ref() else { return };
    for s in &mut n.surfaces {
        if let Some(widths) = s.take_widths() {
            crate::engine::native_widths(app, &s.label, widths);
        }
        if let Some(regions) = s.take_regions() {
            crate::engine::native_regions(app, &s.label, regions);
        }
    }
}

impl Native {
    fn surface(&mut self, label: &str) -> Option<&mut Surface> {
        self.surfaces.iter_mut().find(|s| s.label == label)
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Create { label, monitor, layout, reply } => {
                let hwnd = match Surface::new(&self.gfx, &label, monitor, *layout, &self.store, OFFSCREEN.load(Ordering::Relaxed)) {
                    Ok(s) => {
                        let h = s.hwnd.0 as isize;
                        self.surfaces.push(s);
                        h
                    }
                    Err(e) => {
                        eprintln!("[kasumi] native: {label}: {e}");
                        0
                    }
                };
                let _ = reply.send(hwnd);
            }
            Msg::Destroy(label) => self.surfaces.retain(|s| s.label != label),
            Msg::Layout(label, layout) => {
                let store = std::mem::take(&mut self.store);
                if let Some(s) = self.surfaces.iter_mut().find(|s| s.label == label) {
                    s.apply(&self.gfx, *layout, &store);
                }
                self.store = store;
            }
            Msg::Place(label, monitor) => {
                if let Some(s) = self.surface(&label) {
                    s.place(monitor);
                }
            }
            Msg::State(label, expanded) => {
                let gfx = &self.gfx;
                if let Some(s) = self.surfaces.iter_mut().find(|s| s.label == label) {
                    s.set_expanded(gfx, expanded);
                }
            }
            Msg::Positions(label, notches) => {
                let gfx = &self.gfx;
                if let Some(s) = self.surfaces.iter_mut().find(|s| s.label == label) {
                    s.update_positions(gfx, notches);
                }
            }
            Msg::Nudge(label, notch) => {
                if let Some(s) = self.surface(&label) {
                    s.set_nudge(notch);
                }
            }
            Msg::Event(label, event, payload) => {
                let gfx = &self.gfx;
                if let Some(s) = self.surfaces.iter_mut().find(|s| s.label == label) {
                    s.event(gfx, &event, &payload);
                }
            }
            Msg::Data(key, value) => {
                for s in &mut self.surfaces {
                    s.data(&self.gfx, &key, &value);
                }
                if let (Some(f), true) = (self.alt_float.as_mut(), key == "taskIcons") {
                    widgets::Panel::data(&mut f.panel, &key, &value);
                }
                self.store.insert(key, value);
            }
            Msg::Entries(key, entries) => {
                let held = self.store.entry(key.clone()).or_insert_with(|| Value::Object(Default::default()));
                if let Some(map) = held.as_object_mut() {
                    map.extend(entries);
                }
                let value = held.clone();
                for s in &mut self.surfaces {
                    s.data(&self.gfx, &key, &value);
                }
            }
            Msg::Shown(label, shown) => {
                if let Some(s) = self.surface(&label) {
                    s.set_shown(shown);
                }
            }
            Msg::ClickThrough(label, ignore) => {
                if let Some(s) = self.surface(&label) {
                    s.set_click_through(ignore);
                }
            }
            Msg::AltTab(key) => self.switcher(widgets::alttab::Kind::Windows, key),
            Msg::Lang(key) => self.switcher(widgets::alttab::Kind::Languages, key),
            Msg::LangReveal(n) => {
                if let Some((_, back)) = self.lang_pending.take_if(|p| p.0 == n) {
                    self.lang_open(back);
                }
            }
            Msg::Scroll(show) => {
                let bar = self.scroll.get_or_insert_with(scrollbar::ScrollBar::new);
                bar.show(&show);
                // Back in once it has been out long enough.
                let wake = WAKE.load(Ordering::Relaxed);
                if let (Some(t), true) = (bar.until, wake != 0) {
                    let ms = t.saturating_duration_since(Instant::now()).as_millis().clamp(10, 60_000) as u32;
                    unsafe {
                        SetTimer(Some(HWND(wake as *mut _)), TIMER_SCROLL, ms, None);
                    }
                }
            }
            Msg::AltTabChanged => {
                let (open, label) = widgets::alttab::with(|s| (s.open, s.label.clone()));
                if !open {
                } else if label.starts_with("@center:") {
                    self.alt_center_draw();
                } else if let Some(s) = self.surfaces.iter_mut().find(|s| s.label == label) {
                    s.alt_tab_resized(&self.gfx);
                }
            }
            Msg::WinKey(n) => {
                // The bar on the cursor's monitor if it has a tasks widget, else the first that does.
                let at = crate::win::cursor_pos();
                let here = self.surfaces.iter().position(|s| s.has_tasks() && at.is_some_and(|(x, y)| s.on_monitor(x, y)));
                let any = self.surfaces.iter().position(|s| s.has_tasks());
                if let Some(i) = here.or(any) {
                    self.surfaces[i].win_key(n as usize);
                }
            }
        }
    }

    /// Alt + Tab's keys: open the switcher (Tab / Shift + Tab), move its choice, switch
    /// (Alt let go) or cancel (Esc).
    fn switcher(&mut self, kind: widgets::alttab::Kind, key: crate::keyhook::AltTabKey) {
        use crate::keyhook::AltTabKey as K;
        use widgets::alttab::{self, Kind};
        let (open, showing) = alttab::with(|s| (s.open, s.kind));
        // The other switcher is showing: these keys aren't for it.
        if open && showing != kind {
            return;
        }
        // Win + Space: the picker shows once Win is held a moment (or Space pressed
        // again); a quick press just switches, and the layout pop-up shows it.
        if kind == Kind::Languages && !open {
            match key {
                K::Next | K::Prev => {
                    if let Some((_, back)) = self.lang_pending.take() {
                        self.lang_open(back);
                        if alttab::with(|s| s.open) {
                            self.switcher(kind, key);
                        }
                        return;
                    }
                    self.lang_presses += 1;
                    let n = self.lang_presses;
                    self.lang_pending = Some((n, matches!(key, K::Prev)));
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(300));
                        post(Msg::LangReveal(n));
                    });
                }
                K::Commit => {
                    if let Some((_, back)) = self.lang_pending.take() {
                        std::thread::spawn(move || crate::win::step_keyboard_layout(back));
                    }
                }
                K::Cancel => self.lang_pending = None,
            }
            return;
        }
        match key {
            K::Next | K::Prev if open => {
                let label = alttab::with(|s| {
                    let n = s.len();
                    if n > 0 {
                        s.sel = if matches!(key, K::Next) { (s.sel + 1) % n } else { (s.sel + n - 1) % n };
                    }
                    s.label.clone()
                });
                if label.starts_with("@center:") {
                    self.alt_center_draw();
                } else if let Some(s) = self.surfaces.iter_mut().find(|s| s.label == label) {
                    s.alt_tab_changed(&self.gfx);
                }
            }
            K::Next | K::Prev if kind == Kind::Languages => self.lang_open(matches!(key, K::Prev)),
            K::Next | K::Prev => self.alt_tab_open(matches!(key, K::Prev)),
            K::Commit | K::Cancel if open => {
                let (label, hwnd, hkl) = alttab::with(|s| {
                    s.open = false;
                    (s.label.clone(), s.list.get(s.sel).map(|e| e.hwnd as isize), s.langs.get(s.sel).map(|l| l.hkl))
                });
                if label.starts_with("@center:") {
                    if let Some(f) = self.alt_float.as_mut() {
                        f.hide();
                    }
                } else if let Some(s) = self.surfaces.iter_mut().find(|s| s.label == label) {
                    s.alt_tab_close();
                }
                if kind == Kind::Languages {
                    if let (K::Commit, Some(h)) = (key, hkl) {
                        std::thread::spawn(move || {
                            crate::osd::quiet_layout();
                            crate::win::set_keyboard_layout(h);
                        });
                    }
                } else if let (K::Commit, Some(0)) = (key, hwnd) {
                    crate::win::toggle_desktop();
                } else if let (K::Commit, Some(h)) = (key, hwnd) {
                    run_act(
                        widgets::Act::call(move |_| {
                            let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_activate(h, false));
                        }),
                        self.app.clone(),
                    );
                }
            }
            _ => {}
        }
    }

    /// The switcher's windows (most recent first; the monitor you're working on, or
    /// all), the first choice (the window before the one in front, as Windows does) and
    /// the top bar it shows on.
    fn alt_tab_open(&mut self, back: bool) {
        use widgets::alttab::{self, Entry};
        let Some(app) = self.app.clone() else { return };
        let cfg = tauri::Manager::state::<crate::providers::SharedConfig>(&app).read().unwrap().alt_tab.clone();
        let Some(wins) = self.store.get("tasks").and_then(|t| t.get("windows")).and_then(|w| w.as_array()) else { return };
        let mut wins: Vec<&Value> = wins.iter().collect();
        wins.sort_by_key(|w| w.get("z").and_then(|z| z.as_i64()).unwrap_or(i64::MAX));
        let s = |w: &Value, k: &str| w.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let active = |w: &Value| w.get("active").and_then(|a| a.as_bool()).unwrap_or(false);
        // The monitor you're working on: the window in front's, else the cursor's.
        let cursor = crate::win::cursor_pos();
        let cursor_mon = self.surfaces.iter().find(|s| cursor.is_some_and(|(x, y)| s.on_monitor(x, y))).map(|s| s.device().to_string());
        let front_mon = wins.iter().find(|w| active(w)).map(|w| s(w, "monitor")).filter(|m| !m.is_empty());
        let mon = front_mon.clone().or(cursor_mon.clone());
        let all = cfg.monitors == "all";
        let list: Vec<Entry> = wins
            .iter()
            .filter(|w| all || mon.as_ref().is_none_or(|m| s(w, "monitor") == *m))
            .map(|w| Entry { hwnd: w.get("hwnd").and_then(|h| h.as_i64()).unwrap_or(0), title: s(w, "title"), app: s(w, "app") })
            .collect();
        if list.is_empty() {
            return;
        }
        let first_in_front = wins.iter().find(|w| all || mon.as_ref().is_none_or(|m| s(w, "monitor") == *m)).is_some_and(|w| active(w));
        let mut sel = if first_in_front && list.len() > 1 { 1 } else { 0 };
        // "Show desktop" as a choice (hwnd 0): first (Alt + Tab still picks the previous
        // window) or last.
        let mut list = list;
        let desk = Entry { hwnd: 0, title: "Desktop".into(), app: String::new() };
        match cfg.desktop.as_str() {
            "first" => {
                list.insert(0, desk);
                sel += 1;
            }
            "last" => list.push(desk),
            _ => {}
        }
        let n = list.len();
        let sel = if back { n - 1 } else { sel.min(n - 1) };
        // Out of the top bar's notch or the dock's on that monitor (all monitors: where the
        // cursor is); in the middle without one there, or when asked to.
        let at = if all { cursor_mon.or(mon) } else { mon };
        let on = |s: &Surface| at.as_deref().is_none_or(|m| s.device() == m);
        let bar = match cfg.position.as_str() {
            "center" => None,
            "bottom" => self.surfaces.iter().position(|s| !s.is_top() && on(s)),
            _ => self.surfaces.iter().position(|s| s.is_top() && on(s)),
        };
        let Some(i) = bar.or_else(|| self.surfaces.iter().position(|s| on(s))).or(if self.surfaces.is_empty() { None } else { Some(0) }) else {
            return;
        };
        let label = if bar.is_some() { self.surfaces[i].label.clone() } else { format!("@center:{}", self.surfaces[i].label) };
        alttab::with(|st| {
            st.open = true;
            st.cursor_at = crate::win::cursor_pos();
            st.mouse = false;
            st.kind = alttab::Kind::Windows;
            st.list = list;
            st.sel = sel;
            st.label = label;
            st.previews = cfg.previews;
            st.labels = cfg.labels.clone();
        });
        if bar.is_some() {
            self.surfaces[i].alt_tab_open(&self.gfx, &self.store, cfg.replace_tasks, None);
        } else {
            self.alt_center_draw();
        }
    }

    /// The language picker (Win + Space): the installed languages, the next one (or the
    /// previous) chosen, out of the notch its position asks for on the monitor you're
    /// working on.
    fn lang_open(&mut self, back: bool) {
        use widgets::alttab::{self, Kind, Lang};
        let Some(app) = self.app.clone() else { return };
        let (cfg, shared) = {
            let c = tauri::Manager::state::<crate::providers::SharedConfig>(&app);
            let c = c.read().unwrap();
            (c.lang_switch.clone(), c.alt_tab.position.clone())
        };
        let hkls = crate::win::keyboard_layouts();
        let n = hkls.len();
        if n < 2 {
            return;
        }
        let i = crate::win::foreground_hkl().and_then(|c| hkls.iter().position(|&h| h as u32 == c as u32)).unwrap_or(0);
        let sel = if back { (i + n - 1) % n } else { (i + 1) % n };
        let langs: Vec<Lang> = hkls
            .iter()
            .map(|&h| {
                let lang = (h as usize & 0xffff) as u16;
                Lang { hkl: h, code: crate::win::language_names(lang).0, name: crate::win::language_display_name(lang) }
            })
            .collect();
        // The monitor you're working on: the window in front's, else the cursor's.
        let front = self
            .store
            .get("tasks")
            .and_then(|t| t.get("windows"))
            .and_then(|w| w.as_array())
            .and_then(|ws| ws.iter().find(|w| w.get("active").and_then(|a| a.as_bool()) == Some(true)))
            .and_then(|w| w.get("monitor").and_then(|m| m.as_str()))
            .filter(|m| !m.is_empty())
            .map(String::from);
        let cursor = crate::win::cursor_pos();
        let mon = front.or_else(|| self.surfaces.iter().find(|s| cursor.is_some_and(|(x, y)| s.on_monitor(x, y))).map(|s| s.device().to_string()));
        let on = |s: &Surface| mon.as_deref().is_none_or(|m| s.device() == m);
        let pick = |p: &str| match p {
            "center" => None,
            "bottom" => self.surfaces.iter().position(|s| !s.is_top() && on(s)),
            _ => self.surfaces.iter().position(|s| s.is_top() && on(s)),
        };
        let pos = if cfg.position == "default" { shared.as_str() } else { cfg.position.as_str() };
        let (bar, at) = if pos == "widget" {
            match self.surfaces.iter().position(|s| on(s) && s.lang_drop().is_some()) {
                Some(i) => (Some(i), self.surfaces[i].lang_drop()),
                None => (pick(&shared), None),
            }
        } else {
            (pick(pos), None)
        };
        let Some(i) = bar.or_else(|| self.surfaces.iter().position(|s| on(s))).or(if self.surfaces.is_empty() { None } else { Some(0) }) else {
            return;
        };
        let label = if bar.is_some() { self.surfaces[i].label.clone() } else { format!("@center:{}", self.surfaces[i].label) };
        alttab::with(|st| {
            st.open = true;
            st.cursor_at = crate::win::cursor_pos();
            st.mouse = false;
            st.kind = Kind::Languages;
            st.langs = langs;
            st.sel = sel;
            st.label = label;
            st.lang_labels = cfg.labels.clone();
        });
        if bar.is_some() {
            self.surfaces[i].alt_tab_open(&self.gfx, &self.store, false, at);
        } else {
            self.alt_center_draw();
        }
    }

    /// Alt + Tab in the middle of the screen: drawn (again), with the look of the bar on
    /// that monitor.
    fn alt_center_draw(&mut self) {
        let label = widgets::alttab::with(|s| s.label.trim_start_matches("@center:").to_string());
        let Some(s) = self.surfaces.iter().find(|s| s.label == label) else { return };
        let (look, opts, mon) = (s.look().clone(), s.alt_opts(), s.monitor().clone());
        let store = &self.store;
        let f = self.alt_float.get_or_insert_with(|| {
            let mut f = floating::Floating::new();
            if let Some(v) = store.get("taskIcons") {
                widgets::Panel::data(&mut f.panel, "taskIcons", v);
            }
            f
        });
        if let Err(e) = f.show(&self.gfx, &look, &opts, &mon) {
            eprintln!("[kasumi] alt tab: {e}");
        }
    }

    /// Widgets' timers: run the due ones, then sleep until the next.
    fn tick_widgets(&mut self, wake: HWND) {
        let now = Instant::now();
        for s in &mut self.surfaces {
            s.tick(&self.gfx, now);
            for act in s.take_acts() {
                run_act(act, self.app.clone());
            }
        }
        let next = self.surfaces.iter().filter_map(|s| s.wake()).min();
        unsafe {
            match next {
                Some(t) => {
                    let ms = t.saturating_duration_since(Instant::now()).as_millis().clamp(10, 3_600_000) as u32;
                    SetTimer(Some(wake), TIMER_WIDGETS, ms, None);
                }
                None => {
                    let _ = KillTimer(Some(wake), TIMER_WIDGETS);
                }
            }
        }
    }
}

unsafe extern "system" fn wake_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_WAKE => {
            let handled = with_native(|n| {
                let msgs: Vec<Msg> = INBOX.with(|i| i.borrow().as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default());
                for m in msgs {
                    n.handle(m);
                }
                n.tick_widgets(hwnd);
                report(n);
            });
            if handled.is_none() {
                // Busy (a call that pumps messages): the messages wait in the channel.
                let _ = PostMessageW(Some(hwnd), WM_WAKE, WPARAM(0), LPARAM(0));
            }
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_SCROLL => {
            let _ = KillTimer(Some(hwnd), TIMER_SCROLL);
            with_native(|n| {
                // The latest show's timer (each show sets it again): due, even when it
                // comes a tick early.
                if let Some(b) = n.scroll.as_mut() {
                    b.hide();
                }
            });
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_WIDGETS => {
            with_native(|n| {
                n.tick_widgets(hwnd);
                report(n);
            });
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn surface_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let point = || ((lp.0 & 0xffff) as i16 as i32, ((lp.0 >> 16) & 0xffff) as i16 as i32);
    match msg {
        // Clicks never take the focus, except into the open launcher (it types there).
        WM_MOUSEACTIVATE => {
            let launcher = with_native(|n| n.surfaces.iter().find(|s| s.hwnd == hwnd).is_some_and(|s| s.launcher_open())).unwrap_or(false);
            LRESULT(if launcher { MA_ACTIVATE } else { MA_NOACTIVATE } as isize)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_CHAR => {
            let ctrl = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::GetKeyState(0x11) } < 0;
            let act = with_native(|n| {
                let gfx = &n.gfx;
                let act = n.surfaces.iter_mut().find(|s| s.hwnd == hwnd).and_then(|s| if msg == WM_CHAR { s.char(gfx, wp.0 as u32) } else { s.key(gfx, wp.0 as u16, ctrl) });
                act.map(|a| (a, n.app.clone()))
            })
            .flatten();
            if let Some((act, app)) = act {
                run_act(act, app);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            let (x, y) = point();
            with_native(|n| {
                // Alt + Tab in the middle of the screen: the card under the mouse.
                if n.alt_float.as_mut().filter(|f| f.hwnd == Some(hwnd)).is_some_and(|f| f.hover((x, y))) {
                    n.alt_center_draw();
                }
                if let Some(s) = n.surfaces.iter_mut().find(|s| s.hwnd == hwnd) {
                    if s.dragging() {
                        if let Some(act) = s.drag(&n.gfx, (x, y)) {
                            run_act(act, n.app.clone());
                        }
                    } else {
                        s.mouse_move(&n.gfx, Some((x, y)));
                    }
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let (x, y) = point();
            with_native(|n| {
                if let Some(s) = n.surfaces.iter_mut().find(|s| s.hwnd == hwnd) {
                    s.press(&n.gfx, (x, y));
                }
            });
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            // Screen coordinates, in notches of 120.
            let mut pt = windows::Win32::Foundation::POINT { x: (lp.0 & 0xffff) as i16 as i32, y: ((lp.0 >> 16) & 0xffff) as i16 as i32 };
            let _ = windows::Win32::Graphics::Gdi::ScreenToClient(hwnd, &mut pt);
            let notches = ((wp.0 >> 16) & 0xffff) as i16 as f32 / 120.0;
            let act = with_native(|n| {
                let gfx = &n.gfx;
                let act = n.surfaces.iter_mut().find(|s| s.hwnd == hwnd).and_then(|s| s.wheel(gfx, (pt.x, pt.y), notches));
                act.map(|a| (a, n.app.clone()))
            })
            .flatten();
            if let Some((act, app)) = act {
                run_act(act, app);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            with_native(|n| {
                if let Some(s) = n.surfaces.iter_mut().find(|s| s.hwnd == hwnd) {
                    s.mouse_move(&n.gfx, None);
                }
            });
            LRESULT(0)
        }
        WM_LBUTTONUP | WM_RBUTTONUP | WM_MBUTTONUP => {
            let (x, y) = point();
            let button = match msg {
                WM_LBUTTONUP => Button::Left,
                WM_RBUTTONUP => Button::Right,
                _ => Button::Middle,
            };
            // What the click asks for runs once the state is released (it may pump messages).
            let act = with_native(|n| {
                let gfx = &n.gfx;
                let act = n.surfaces.iter_mut().find(|s| s.hwnd == hwnd).and_then(|s| {
                    // A drag ends where the button comes up (not a click).
                    if button == Button::Left && s.dragging() {
                        s.release(gfx, (x, y))
                    } else {
                        s.click(gfx, button, (x, y))
                    }
                });
                // Alt + Tab in the middle of the screen: a card picked.
                let act = act.or_else(|| n.alt_float.as_mut().filter(|f| f.hwnd == Some(hwnd)).and_then(|f| f.click(button, (x, y))));
                report(n);
                let bar_menu = n.surfaces.iter_mut().find(|s| s.hwnd == hwnd).is_some_and(|s| s.take_bar_menu());
                Some((act, bar_menu, n.app.clone()))
            })
            .flatten();
            if let Some((act, bar_menu, app)) = act {
                if let Some(act) = act {
                    run_act(act, app.clone());
                }
                // A right-click on a bar's empty space: Kasumi's bar menu.
                if let (true, Some(app)) = (bar_menu, app) {
                    ctxmenu::bar_menu(app);
                }
            }
            LRESULT(0)
        }
        // Clicks are handled on release; nothing should take the focus.
        WM_RBUTTONDOWN | WM_MBUTTONDOWN => LRESULT(0),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn run_act(act: Act, app: Option<AppHandle>) {
    match act {
        Act::Call(f) => {
            let Some(app) = app else { return };
            std::thread::spawn(move || f(app));
        }
        Act::Seq(list) => {
            for a in list {
                run_act(a, app.clone());
            }
        }
        // The surface made it a call with its label (Surface::answer); a stray one does nothing.
        Act::Launcher { .. } => {}
        // The surface turned it into a screen point (physical px) already.
        Act::Tray { id, button, at } => {
            if let Err(e) = crate::providers::systray::click_at(&id, button, at.0 as i32, at.1 as i32) {
                eprintln!("[kasumi] tray click: {e}");
            }
        }
        Act::Run { action, target } => {
            let Some(app) = app else { return };
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::actions::run_action(app, action.clone(), target).await {
                    eprintln!("[kasumi] {action}: {e}");
                }
            });
        }
    }
}
