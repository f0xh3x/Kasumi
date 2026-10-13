//! The engine thread: owns the config, keeps one overlay window per surface
//! (top bar, dock, ...) per monitor, polls the cursor to drive the hover state
//! machine and click-through, and hides overlays behind fullscreen apps.
//!
//! Coordinates inside an overlay are measured from its own screen edge: y is the
//! distance from the top edge for a top surface and from the bottom edge for a
//! bottom one (the frontend draws bottom surfaces flipped), so hover zones,
//! regions and notch shapes are the same maths for both.

use crate::config::{self, Anchor, Animation, Bar, Config, Edge, Geometry, Hot, MonitorRule, SizeName, SurfaceMode};
use crate::providers::{self, SharedConfig};
use crate::win::{self, Monitor};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, EventTarget, Manager, State, WebviewUrl, WebviewWindowBuilder};

/// Extra window height below the tallest shape so spring overshoot isn't clipped.
const OVERSHOOT_MARGIN: f64 = 24.0;

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

/// Everything the frontend of one overlay needs to draw itself (logical px).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub label: String,
    pub monitor: String,
    /// Device name (`\\.\DISPLAY4`), as window lists report it.
    pub device: String,
    /// The surface this overlay draws (`top`, `bottom`, ...), its edge and mode.
    pub surface: String,
    pub edge: Edge,
    pub mode: SurfaceMode,
    /// Dock: named sizes and heights at rest and open (bar mode: the notch height).
    pub rest: SizeName,
    pub open: SizeName,
    pub rest_height: f64,
    pub open_height: f64,
    /// Hot zones that open this surface.
    pub hot: Hot,
    /// Height reserved on screen for it (0: none).
    pub reserve: f64,
    /// Rounded screen corners drawn with this surface's shape (0: off).
    pub corner_radius: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
    pub geometry: Geometry,
    pub animation: Animation,
    pub bar: Bar,
    pub notches: Vec<NotchLayout>,
    pub widgets: toml::Table,
    pub osd: config::Osd,
    pub theme: config::Theme,
    /// Tooltips on widgets.
    pub tooltips: bool,
    /// The app launcher is switched on.
    pub launcher: bool,
    /// Draw notches that would touch or overlap as one.
    pub combine_notches: bool,
    pub expanded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotchLayout {
    pub id: String,
    pub x: f64,
    pub width: f64,
    pub height: f64,
    pub trigger_x: f64,
    pub trigger_width: f64,
    pub widgets: Vec<String>,
    pub panel: Option<String>,
    pub panel_width: f64,
    pub panel_height: f64,
    /// Every widget in it hides under windows and one covers it: the notch retracts.
    pub hidden: bool,
    /// Widgets hidden (under a window) while the rest of the notch stays.
    pub hidden_widgets: Vec<String>,
    /// "left"/"right" for the notch hugging that corner: its content keeps the
    /// open bar's distance from the edge, so it doesn't shift as the bar opens.
    pub edge: Option<String>,
    /// Dock: where the notch is and how big while open (bar mode: same as at rest).
    pub open_x: f64,
    pub open_width: f64,
    pub open_height: f64,
    /// Dock: shrunk to the tiny handle (a window covers it).
    pub shrunk: bool,
    /// Dock: none of its widgets is pinned (`pin_in_dock = false`), so it's tucked
    /// away while the dock is closed and grows out as it opens. Still hoverable.
    pub closed_hidden: bool,
}

/// A notch's measured content width, and without its hide-under-windows widgets;
/// for docks also at the open size.
#[derive(Debug, Clone, Copy, PartialEq, serde::Deserialize)]
pub struct NotchWidth {
    pub full: f64,
    pub compact: f64,
    #[serde(default)]
    pub open: f64,
}

/// The notch geometry scaled for a notch `height` tall, so taller (dock) notches
/// keep the same silhouette.
fn scaled(g: &Geometry, height: f64) -> Geometry {
    let k = height / g.notch_height.max(1.0);
    Geometry { concave_radius: g.concave_radius * k, notch_corner_radius: g.notch_corner_radius * k, ..g.clone() }
}

/// Width of a dock notch shrunk to the tiny handle.
fn handle_width(open: f64) -> f64 {
    (open * 0.6).max(48.0)
}

#[derive(Debug, Clone, Copy)]
enum Hover {
    Idle,
    Dwell(Instant),
    Open,
    Leave(Instant),
}

struct Overlay {
    monitor: Monitor,
    surface: String,
    edge: Edge,
    layout: Layout,
    /// Interactive areas reported by the frontend, logical px relative to the window.
    regions: Vec<Rect>,
    hover: Hover,
    expanded: bool,
    hidden: bool,
    ignoring: bool,
    hwnd: Option<isize>,
    /// Docking notches currently pushed out of their corner by a window.
    undocked: Vec<String>,
    /// The notch the cursor is coming near ([animation] hover_nudge), told to the page.
    nudge: Option<String>,
    /// When a fullscreen window was first seen on this monitor (hide is debounced).
    fullscreen_since: Option<Instant>,
    /// Shown over a fullscreen app for a moment ([launcher] fullscreen = "notches"), until then.
    peek_until: Option<Instant>,
    /// This monitor's notches, as resolved from the config (for docking).
    defs: Vec<config::Notch>,
    /// Held open regardless of the cursor (the launcher is open on it).
    pinned: bool,
    /// Content widths of auto-width notches, measured by the frontend.
    measured: HashMap<String, NotchWidth>,
}

impl Overlay {
    /// Physical screen y of the window's top edge.
    fn window_top(&self) -> i32 {
        window_top(&self.monitor, self.edge, self.layout.height)
    }

    fn hit(&self, x: f64, y: f64) -> bool {
        if self.regions.is_empty() {
            // Frontend hasn't reported yet: assume the open bar.
            return y >= 0.0 && y < self.layout.geometry.expanded_height && x >= 0.0 && x < self.layout.width;
        }
        self.regions.iter().any(|r| r.contains(x, y))
    }
}

#[derive(Default)]
pub struct Shared {
    overlays: HashMap<String, Overlay>,
    /// Overlays whose notches need repositioning (new measured widths).
    relayout: HashSet<String>,
}

pub type SharedState = Arc<Mutex<Shared>>;

pub struct ReloadSender(pub Sender<()>);

#[derive(Clone, Serialize)]
struct StatePayload {
    expanded: bool,
}

// Commands are async so they run off the main thread and never contend with
// window operations the engine dispatches there.

#[tauri::command]
pub async fn get_layout(window: tauri::WebviewWindow, state: State<'_, SharedState>) -> Result<Layout, String> {
    let sh = state.lock().unwrap();
    let o = sh.overlays.get(window.label()).ok_or("unknown overlay")?;
    let mut layout = o.layout.clone();
    layout.expanded = o.expanded;
    Ok(layout)
}

#[tauri::command]
pub async fn set_regions(window: tauri::WebviewWindow, state: State<'_, SharedState>, regions: Vec<Rect>) -> Result<(), String> {
    let mut sh = state.lock().unwrap();
    if let Some(o) = sh.overlays.get_mut(window.label()) {
        o.regions = regions;
    }
    Ok(())
}

/// The frontend measured its notches' content; positions are recomputed with
/// these widths (the engine thread picks this up on its next tick).
#[tauri::command]
pub async fn set_notch_widths(
    window: tauri::WebviewWindow,
    state: State<'_, SharedState>,
    widths: HashMap<String, NotchWidth>,
) -> Result<(), String> {
    let mut sh = state.lock().unwrap();
    let label = window.label().to_string();
    let Some(o) = sh.overlays.get_mut(&label) else { return Ok(()) };
    let near = |a: &NotchWidth, b: &NotchWidth| {
        (a.full - b.full).abs() <= 0.5 && (a.compact - b.compact).abs() <= 0.5 && (a.open - b.open).abs() <= 0.5
    };
    let changed = widths.iter().any(|(id, w)| o.measured.get(id).is_none_or(|old| !near(old, w)));
    if changed {
        o.measured.extend(widths);
        sh.relayout.insert(label);
    }
    Ok(())
}

/// The frontend measured content (e.g. a bar dropdown) taller than the window: grow it.
#[tauri::command]
pub async fn set_content_height(window: tauri::WebviewWindow, state: State<'_, SharedState>, height: f64) -> Result<(), String> {
    let (hwnd, m, h) = {
        let mut sh = state.lock().unwrap();
        let Some(o) = sh.overlays.get_mut(window.label()) else { return Ok(()) };
        let wanted = (height + OVERSHOOT_MARGIN).ceil();
        if wanted <= o.layout.height {
            return Ok(());
        }
        o.layout.height = wanted;
        (o.hwnd, o.monitor.clone(), (wanted, o.window_top()))
    };
    if let Some(hwnd) = hwnd {
        let (h, top) = h;
        let h_px = (h * m.scale).ceil() as i32;
        window
            .run_on_main_thread(move || win::place(hwnd, m.left, top, m.width(), h_px))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Physical y of the top of an overlay window `height` logical px tall on `edge`.
fn window_top(m: &Monitor, edge: Edge, height: f64) -> i32 {
    match edge {
        Edge::Top => m.top,
        Edge::Bottom => m.bottom - (height * m.scale).ceil() as i32,
    }
}

/// Converts logical px within an overlay window (from its top-left, as the page
/// sees it) to physical screen coordinates.
pub fn to_screen(app: &AppHandle, label: &str, x: f64, y: f64) -> Option<(i32, i32)> {
    let state = app.state::<SharedState>();
    let sh = state.lock().unwrap();
    let o = sh.overlays.get(label)?;
    Some((o.monitor.left + (x * o.monitor.scale).round() as i32, o.window_top() + (y * o.monitor.scale).round() as i32))
}

/// Whether a monitor matches a [[monitors]] rule's `match` (or an OSD screen):
/// "*", "primary", "#2" (left to right, from 1), its device ("\\.\DISPLAY4") or
/// part of its name.
pub fn monitor_matches(matcher: &str, m: &Monitor, index: usize) -> bool {
    let s = matcher.trim();
    let lower = s.to_lowercase();
    match lower.as_str() {
        "*" => true,
        "primary" => m.primary,
        _ if s.starts_with('#') => s[1..].parse::<usize>().is_ok_and(|n| n == index + 1),
        _ => m.device.eq_ignore_ascii_case(s) || (!m.friendly.is_empty() && m.friendly.to_lowercase().contains(&lower)),
    }
}

/// Where a pop-up kind appears ([osd] screen / spot, or the kind's own): the
/// overlays, each with the notch that grows (None: the "widget" spot, the
/// keyboard peek at the language widget). Hidden overlays (fullscreen) don't
/// count, so nothing pops over a fullscreen app.
pub fn osd_targets(app: &AppHandle, kind: &str) -> Vec<(String, Option<String>)> {
    let (screen, spot, default_spot) = {
        let cfg = app.state::<SharedConfig>();
        let cfg = cfg.read().unwrap();
        let (screen, spot) = cfg.osd.place(kind);
        (screen.trim().to_string(), spot.trim().to_lowercase(), cfg.osd.spot.trim().to_lowercase())
    };
    let cursor = win::cursor_pos();
    let state = app.state::<SharedState>();
    let sh = state.lock().unwrap();
    // The monitors, left to right (as [[monitors]] number them).
    let mut monitors: Vec<&Monitor> = Vec::new();
    for o in sh.overlays.values() {
        if !monitors.contains(&&o.monitor) {
            monitors.push(&o.monitor);
        }
    }
    monitors.sort_by_key(|m| (m.left, m.top));
    let under = |m: &Monitor| cursor.is_some_and(|(x, y)| x >= m.left && x < m.right && y >= m.top && y < m.bottom);
    let chosen: Vec<&Monitor> = match screen.to_lowercase().as_str() {
        "cursor" | "" => monitors.iter().find(|m| under(m)).or_else(|| monitors.iter().find(|m| m.primary)).copied().into_iter().collect(),
        "main" => monitors.iter().filter(|m| m.primary).copied().collect(),
        "all" => monitors.clone(),
        "others" => monitors.iter().filter(|m| !m.primary).copied().collect(),
        _ => monitors.iter().enumerate().filter(|(i, m)| monitor_matches(&screen, m, *i)).map(|(_, m)| *m).collect(),
    };
    // "widget" falls back to the default spot where no bar shows the language widget.
    let notch_spot = if spot == "widget" { default_spot.as_str() } else { spot.as_str() };
    let (edge, side) = notch_spot.split_once('-').unwrap_or(("top", "center"));
    let want = match side {
        "left" => Anchor::Left,
        "right" => Anchor::Right,
        _ => Anchor::Center,
    };
    let mut out = Vec::new();
    for m in chosen {
        let here: Vec<(&String, &Overlay)> =
            sh.overlays.iter().filter(|(_, o)| o.monitor == *m && !o.hidden && !o.layout.notches.is_empty()).collect();
        if spot == "widget" {
            // The bar showing the language widget (in a notch or in the bar).
            let shows = |o: &Overlay| {
                o.layout.notches.iter().any(|n| n.widgets.iter().any(|w| w == "language"))
                    || [&o.layout.bar.left, &o.layout.bar.center, &o.layout.bar.right].iter().any(|g| g.iter().any(|w| w == "language"))
            };
            if let Some((label, _)) = here.iter().find(|(_, o)| shows(o)) {
                out.push(((*label).clone(), None));
                continue;
            }
        }
        // The bar on that edge (or whichever this monitor has), then the notch on that side.
        let Some((label, o)) = here.iter().find(|(_, o)| (o.edge == Edge::Bottom) == (edge == "bottom")).or_else(|| here.first()) else {
            continue;
        };
        let anchor = |id: &str| o.defs.iter().find(|d| d.id == id).map(|d| d.anchor);
        let visible = || o.layout.notches.iter().filter(|n| !n.hidden);
        let notch = visible()
            .find(|n| anchor(&n.id) == Some(want))
            .or_else(|| visible().find(|n| anchor(&n.id) == Some(Anchor::Center)))
            .or_else(|| o.layout.notches.first());
        out.push(((*label).clone(), notch.map(|n| n.id.clone())));
    }
    out
}

/// Whether `layout` shows a widget of this type (`type` or `type#n`) in its bar
/// or a notch, or a notch's own panel of that type (older configs name one).
fn shows(layout: &Layout, ty: &str) -> bool {
    let is = |w: &String| w.split('#').next() == Some(ty);
    [&layout.bar.left, &layout.bar.center, &layout.bar.right].iter().any(|g| g.iter().any(is))
        || layout.notches.iter().any(|n| n.widgets.iter().any(is) || n.panel.as_ref().is_some_and(is))
}

/// The overlays showing a widget of any of these types (who a data key goes to).
pub fn overlays_showing(app: &AppHandle, types: &[&str]) -> Vec<String> {
    let state = app.state::<SharedState>();
    let sh = state.lock().unwrap();
    sh.overlays.iter().filter(|(_, o)| types.iter().any(|t| shows(&o.layout, t))).map(|(l, _)| l.clone()).collect()
}

/// Whether this overlay shows a widget of any of these types.
pub fn overlay_shows(app: &AppHandle, label: &str, types: &[&str]) -> bool {
    let state = app.state::<SharedState>();
    let sh = state.lock().unwrap();
    sh.overlays.get(label).is_some_and(|o| types.iter().any(|t| shows(&o.layout, t)))
}

/// The window that was in front before the launcher took focus.
static LAUNCHER_PREV: std::sync::atomic::AtomicIsize = std::sync::atomic::AtomicIsize::new(0);
/// The open launcher has had the focus (so losing it means the user went elsewhere).
static LAUNCHER_FOCUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Opens (`open` = Some(true)), closes or toggles the launcher. `label` is the
/// overlay to use (its launcher widget was clicked); without one: on the monitor
/// under the cursor, the surface with a launcher widget, else the dock, else the
/// top bar.
pub fn launcher_toggle(app: &AppHandle, label: Option<String>, open: Option<bool>, refocus: bool) {
    if !app.state::<SharedConfig>().read().unwrap().launcher.enabled {
        return;
    }
    let state = app.state::<SharedState>();
    let (label, hwnd, opening, ours, unhide) = {
        let mut sh = state.lock().unwrap();
        let label = match label {
            Some(l) => l,
            None => {
                let open_one = sh.overlays.iter().find(|(_, o)| o.pinned).map(|(l, _)| l.clone());
                match open_one.or_else(|| launcher_target(&sh)) {
                    Some(l) => l,
                    None => return,
                }
            }
        };
        // Where focus goes back to on close: the app in front, never one of ours
        // (opened while a bar had focus: keep the app from before).
        let front = win::foreground();
        let ours: Vec<isize> = sh.overlays.values().filter_map(|o| o.hwnd).collect();
        let front_is_ours = ours.contains(&front);
        let notches_first = app.state::<SharedConfig>().read().unwrap().launcher.fullscreen == "notches";
        if notches_first && !o_pinned(&sh, &label) && sh.overlays.get(&label).is_some_and(|o| o.hidden) {
            // Over a fullscreen app: show that screen's bars for a moment (they hide
            // again by themselves, update_fullscreen); pressed again, the launcher opens.
            let monitor = sh.overlays[&label].monitor.clone();
            let until = Instant::now() + Duration::from_secs(4);
            let mut show = Vec::new();
            for (l, o) in sh.overlays.iter_mut().filter(|(_, o)| o.monitor == monitor && o.hidden) {
                o.hidden = false;
                o.peek_until = Some(until);
                show.push((l.clone(), o.hwnd));
            }
            drop(sh);
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                for (l, hwnd) in show {
                    if let Some(w) = handle.get_webview_window(&l) {
                        let _ = w.show();
                    }
                    if let Some(h) = hwnd {
                        win::raise_topmost(h);
                    }
                }
            });
            return;
        }
        let Some(o) = sh.overlays.get_mut(&label) else { return };
        let opening = open.unwrap_or(!o.pinned);
        if opening == o.pinned {
            return;
        }
        o.pinned = opening;
        // Over a fullscreen app the bar is hidden: show it for the launcher (it hides
        // again after, update_fullscreen, once the launcher has closed).
        let unhide = opening && o.hidden;
        if unhide {
            o.hidden = false;
        }
        if !opening {
            // Closed: no hover grace for the bar (the cursor is usually elsewhere), so it
            // shrinks together with the launcher's panel, in one motion. A cursor on the
            // bar still keeps it open (the hover loop sees it and goes back to Open).
            let now = Instant::now();
            o.hover = Hover::Leave(now.checked_sub(Duration::from_secs(60)).unwrap_or(now));
        }
        if opening && !o.expanded {
            o.expanded = true;
            o.hover = Hover::Open;
            let _ = app.emit_to(EventTarget::WebviewWindow { label: label.clone() }, "notch://state", StatePayload { expanded: true });
            if let Some(w) = app.get_webview_window(&label) {
                crate::webmem::set_low(&w, false);
            }
        }
        if opening && !front_is_ours {
            LAUNCHER_PREV.store(front, std::sync::atomic::Ordering::Relaxed);
        }
        (label, o.hwnd, opening, ours, unhide)
    };
    let Some(hwnd) = hwnd else { return };
    let prev = LAUNCHER_PREV.load(std::sync::atomic::Ordering::Relaxed);
    LAUNCHER_FOCUSED.store(false, std::sync::atomic::Ordering::Relaxed);
    let handle = app.clone();
    let l = label.clone();
    let _ = app.run_on_main_thread(move || {
        let Some(w) = handle.get_webview_window(&l) else { return };
        if unhide {
            let _ = w.show();
        }
        win::set_activatable(hwnd, opening);
        let _ = w.set_focusable(opening);
        if opening {
            win::raise_topmost(hwnd);
            win::focus(hwnd);
            let _ = w.set_focus();
        } else if refocus && ours.contains(&win::foreground()) {
            // Closed from the keyboard (Esc, the hotkey, the Windows key): back to the
            // app that had focus, not left on a bar that can't take typing.
            win::restore_foreground(prev, &ours);
        }
    });
    if opening && unhide {
        // A bar shown a moment ago (over a fullscreen app) may not take the focus yet: keep
        // asking for a moment, so typing goes to the launcher rather than the app.
        let handle = app.clone();
        std::thread::spawn(move || {
            for _ in 0..10 {
                std::thread::sleep(Duration::from_millis(50));
                if win::foreground() == hwnd {
                    break;
                }
                let _ = handle.run_on_main_thread(move || win::focus(hwnd));
            }
        });
    }
    let _ = app.emit_to(EventTarget::WebviewWindow { label }, "notch://launcher", serde_json::json!({ "open": opening }));
}

fn o_pinned(sh: &Shared, label: &str) -> bool {
    sh.overlays.get(label).is_some_and(|o| o.pinned)
}

fn launcher_target(sh: &Shared) -> Option<String> {
    let (cx, cy) = win::cursor_pos()?;
    let under = |o: &Overlay| {
        let m = &o.monitor;
        cx >= m.left && cx < m.right && cy >= m.top && cy < m.bottom
    };
    // Hidden ones too (over a fullscreen app): opening the launcher shows its bar for it.
    let here: Vec<(&String, &Overlay)> = sh.overlays.iter().filter(|(_, o)| under(o)).collect();
    let has_widget = |o: &Overlay| {
        [&o.layout.bar.left, &o.layout.bar.center, &o.layout.bar.right].iter().any(|g| g.iter().any(|w| w.split('#').next() == Some("launcher")))
            || o.layout.notches.iter().any(|n| n.widgets.iter().any(|w| w.split('#').next() == Some("launcher")))
    };
    here.iter()
        .find(|(_, o)| has_widget(o))
        .or_else(|| here.iter().find(|(_, o)| o.edge == Edge::Bottom && !o.layout.notches.is_empty()))
        .or_else(|| here.iter().find(|(_, o)| o.edge == Edge::Top))
        .map(|(l, _)| (*l).clone())
}

/// The launcher widget was clicked: open the launcher on this overlay.
#[tauri::command]
pub async fn launcher_open(window: tauri::WebviewWindow) {
    launcher_toggle(window.app_handle(), Some(window.label().to_string()), Some(true), false);
}

/// The launcher closed (Esc, an app launched, focus lost). `refocus`: give the
/// foreground back to the window that had it.
#[tauri::command]
pub async fn launcher_close(window: tauri::WebviewWindow, refocus: bool) {
    launcher_toggle(window.app_handle(), Some(window.label().to_string()), Some(false), refocus);
}

/// Frontend console errors, forwarded to the app log.
#[tauri::command]
pub async fn fe_log(window: tauri::WebviewWindow, message: String) {
    eprintln!("[kasumi:{}] {message}", window.label());
}

pub fn start(app: &AppHandle) {
    let shared: SharedState = Arc::default();
    app.manage(shared.clone());
    let (tx, rx) = mpsc::channel();
    app.manage(ReloadSender(tx.clone()));

    // Load first: on first run it creates the directory the watcher needs.
    let cfg = config::load().unwrap_or_else(|e| {
        eprintln!("[kasumi] config error, using defaults: {e}");
        config::fallback()
    });
    let shared_cfg: SharedConfig = Arc::new(RwLock::new(cfg.clone()));
    providers::start(app, shared_cfg.clone());

    let app = app.clone();
    std::thread::Builder::new()
        .name("kasumi-engine".into())
        .spawn(move || {
            let watcher = config::watch(tx);
            if watcher.is_none() {
                eprintln!("[kasumi] config hot-reload unavailable");
            }
            Engine { app, shared, cfg, shared_cfg, monitors: Vec::new(), reload_rx: rx, _watcher: watcher }.run();
        })
        .expect("spawn engine thread");
}

struct Engine {
    app: AppHandle,
    shared: SharedState,
    cfg: Config,
    /// Copy of `cfg` the providers read.
    shared_cfg: SharedConfig,
    monitors: Vec<Monitor>,
    reload_rx: Receiver<()>,
    _watcher: Option<notify::RecommendedWatcher>,
}

/// Left edge (logical px) of a notch. A docked notch hangs past the screen edge by
/// its round-off width, so its tip runs flush into the corner.
fn notch_x(n: &config::Notch, width: f64, g: &Geometry, docked: bool) -> f64 {
    let off = if docked && n.anchor != Anchor::Center { -g.notch_corner_radius } else { n.offset.resolve(width) };
    match n.anchor {
        Anchor::Left => off,
        Anchor::Center => (width - n.width) / 2.0 + off,
        Anchor::Right => width - off - n.width,
    }
}

/// Left edges of all notches. Auto-width notches on the same side are stacked
/// from the side's corner (or offset) inward, one notch-flare apart; a side's
/// docking follows its outermost notch. Legacy notches are placed on their own.
fn notch_positions(defs: &[config::Notch], width: f64, g: &Geometry, gap: f64, undocked: &[String]) -> Vec<f64> {
    let mut xs = vec![0.0; defs.len()];
    let spacing = 2.0 * g.concave_radius + gap;
    for anchor in [Anchor::Left, Anchor::Center, Anchor::Right] {
        let idx: Vec<usize> = (0..defs.len()).filter(|&i| defs[i].auto_width && defs[i].anchor == anchor).collect();
        if idx.is_empty() {
            continue;
        }
        match anchor {
            Anchor::Left | Anchor::Right => {
                // Outermost first: left side in order, right side from the end.
                let order: Vec<usize> = if anchor == Anchor::Left { idx } else { idx.into_iter().rev().collect() };
                let outer = &defs[order[0]];
                let docked = outer.dock && !undocked.contains(&outer.id);
                let mut x = notch_x(outer, width, g, docked);
                xs[order[0]] = x;
                for w in order.windows(2) {
                    let (prev, cur) = (&defs[w[0]], &defs[w[1]]);
                    x = if anchor == Anchor::Left { x + prev.width + spacing } else { x - spacing - cur.width };
                    xs[w[1]] = x;
                }
            }
            Anchor::Center => {
                let total: f64 = idx.iter().map(|&i| defs[i].width).sum::<f64>() + spacing * (idx.len() - 1) as f64;
                let mut x = (width - total) / 2.0 + defs[idx[0]].offset.resolve(width);
                for &i in &idx {
                    xs[i] = x;
                    x += defs[i].width + spacing;
                }
            }
        }
    }
    for (i, n) in defs.iter().enumerate().filter(|(_, n)| !n.auto_width) {
        xs[i] = notch_x(n, width, g, n.dock && !undocked.contains(&n.id));
    }
    xs
}

fn intersects(r: &windows::Win32::Foundation::RECT, x0: i32, y0: i32, x1: i32, y1: i32) -> bool {
    r.left < x1 && r.right > x0 && r.top < y1 && r.bottom > y0
}

impl Engine {
    fn run(mut self) {
        let mut last_monitor_check = Instant::now();
        let mut last_accent = win::system_accent();
        let mut last_fullscreen_check = Instant::now();
        let mut last_keyboard_check = Instant::now();
        let mut keyboard = crate::osd::KeyboardWatch::default();
        let mut reload_at: Option<Instant> = None;

        self.monitors = win::monitors();
        self.reconcile();

        loop {
            let now = Instant::now();
            if crate::instance::settings_requested() {
                crate::settings::open(&self.app);
            }
            if crate::instance::quit_requested() {
                eprintln!("[kasumi] quitting: a new instance took over");
                self.app.exit(0);
                return;
            }

            // Editors fire several events per save; reload once things settle.
            while self.reload_rx.try_recv().is_ok() {
                reload_at = Some(now + Duration::from_millis(150));
            }
            if reload_at.is_some_and(|t| now >= t) {
                reload_at = None;
                match config::load() {
                    Ok(cfg) => {
                        *self.shared_cfg.write().unwrap() = cfg.clone();
                        self.cfg = cfg;
                        self.reconcile();
                        eprintln!("[kasumi] config reloaded");
                    }
                    Err(e) => eprintln!("[kasumi] config error, keeping previous: {e}"),
                }
            }

            if now - last_monitor_check >= Duration::from_secs(1) {
                last_monitor_check = now;
                // Windows' accent colour changed: the bars follow it if synced.
                let accent = win::system_accent();
                if accent != last_accent {
                    last_accent = accent;
                    if self.cfg.theme.accent_sync {
                        self.reconcile();
                    }
                }
                let m = win::monitors();
                if m != self.monitors {
                    self.monitors = m;
                    self.reconcile();
                }
            }

            self.process_relayouts();

            if now - last_keyboard_check >= Duration::from_millis(100) {
                last_keyboard_check = now;
                keyboard.tick(&self.app);
            }

            if now - last_fullscreen_check >= Duration::from_millis(250) {
                last_fullscreen_check = now;
                self.update_fullscreen();
            }

            self.update_corners();

            let (toggled, calm) = self.tick_hover(now);
            if toggled {
                // The bar opened or closed: re-dock right away instead of on the next tick.
                let ours: Vec<isize> = self.shared.lock().unwrap().overlays.values().filter_map(|o| o.hwnd).collect();
                self.update_docking(&ours);
            }

            let mut poll_ms = self.cfg.timing.poll_ms.clamp(4, 100);
            if calm {
                poll_ms = poll_ms.max(CALM_POLL_MS);
            }
            let poll = Duration::from_millis(poll_ms);
            if let Some(rest) = poll.checked_sub(now.elapsed()) {
                std::thread::sleep(rest);
            }
        }
    }

    fn shrink_when_covered(&self, surface: &str) -> bool {
        self.cfg.surface_list().iter().any(|(id, s)| id == surface && s.shrink_when_covered)
    }

    fn rule_for(&self, m: &Monitor, index: usize) -> Option<&MonitorRule> {
        self.cfg.monitors.iter().find(|r| monitor_matches(&r.matcher, m, index))
    }

    #[allow(clippy::too_many_arguments)]
    fn build_layout(
        &self,
        label: &str,
        m: &Monitor,
        rule: Option<&MonitorRule>,
        surface: &str,
        undocked: &[String],
        measured: &HashMap<String, NotchWidth>,
    ) -> Layout {
        let width = m.width() as f64 / m.scale;
        let s = self.cfg.surface_list().into_iter().find(|(id, _)| id == surface).map(|(_, s)| s).unwrap_or_default();
        let dock = s.mode == SurfaceMode::Dock;
        let sizes = &self.cfg.sizes;
        let (rest_h, open_h) = if dock {
            (sizes.px(s.rest), sizes.px(s.open).max(sizes.px(s.rest)))
        } else {
            (self.cfg.geometry.notch_height, self.cfg.geometry.notch_height)
        };
        let g = &if dock { scaled(&self.cfg.geometry, rest_h) } else { self.cfg.geometry.clone() };
        let g_open = scaled(&self.cfg.geometry, open_h);
        let (mut defs, bar) = self.cfg.resolve_surface(rule, surface);
        defs.retain(|n| n.widgets.iter().any(|w| self.cfg.widget_enabled(w)));
        for n in defs.iter_mut().filter(|n| n.auto_width) {
            if let Some(w) = measured.get(&n.id) {
                n.width = w.full;
            }
        }
        // Covered notches ("undocked": a window is at their spot): widgets set to
        // hide under windows leave; a notch left empty retracts.
        let mut hidden: HashSet<String> = HashSet::new();
        let mut hidden_widgets: HashMap<String, Vec<String>> = HashMap::new();
        for n in defs.iter_mut().filter(|n| undocked.contains(&n.id)) {
            let shown: Vec<&String> = n.widgets.iter().filter(|w| self.cfg.widget_enabled(w)).collect();
            let hide: Vec<String> = shown.iter().filter(|w| self.cfg.widget_hides_when_maximized(w)).map(|w| (*w).clone()).collect();
            if hide.is_empty() {
                continue;
            }
            if hide.len() == shown.len() {
                hidden.insert(n.id.clone());
            } else {
                if let Some(w) = measured.get(&n.id).filter(|w| n.auto_width && w.compact > 0.0) {
                    n.width = w.compact;
                }
                hidden_widgets.insert(n.id.clone(), hide);
            }
        }
        // Docks: covered notches shrink to the tiny handle instead of moving aside.
        let shrunk: HashSet<String> =
            if dock && s.shrink_when_covered { undocked.iter().cloned().collect() } else { HashSet::new() };
        let open_width = |n: &config::Notch| measured.get(&n.id).map_or(n.width, |w| if w.open > 0.0 { w.open } else { w.full });
        let rest_height = |n: &config::Notch| if shrunk.contains(&n.id) { sizes.tiny } else { rest_h };
        if dock {
            for n in defs.iter_mut() {
                if shrunk.contains(&n.id) || s.rest == SizeName::Tiny {
                    n.width = handle_width(open_width(n));
                }
            }
        }
        // Only notches set to avoid windows move aside (not in docks); retracted ones
        // keep their spot and don't push their neighbours.
        let moved: Vec<String> = if dock {
            Vec::new()
        } else {
            defs.iter().filter(|n| n.avoid_windows && undocked.contains(&n.id)).map(|n| n.id.clone()).collect()
        };
        let visible: Vec<config::Notch> = defs.iter().filter(|n| !hidden.contains(&n.id)).cloned().collect();
        let placed: HashMap<String, f64> = visible
            .iter()
            .map(|n| n.id.clone())
            .zip(notch_positions(&visible, width, g, self.cfg.notch.gap, &moved))
            .collect();
        let xs: Vec<f64> = defs.iter().map(|n| placed.get(&n.id).copied().unwrap_or_else(|| notch_x(n, width, g, n.dock))).collect();
        // Where (and how wide) each notch is while a dock is open.
        let open_defs: Vec<config::Notch> = visible.iter().map(|n| config::Notch { width: open_width(n), ..n.clone() }).collect();
        let open_placed: HashMap<String, (f64, f64)> = if dock {
            open_defs
                .iter()
                .zip(notch_positions(&open_defs, width, &g_open, self.cfg.notch.gap, &[]))
                .map(|(n, x)| (n.id.clone(), (x, n.width)))
                .collect()
        } else {
            HashMap::new()
        };

        // The outermost notch of each side, if it hugs the corner.
        let outermost = |side: Anchor| {
            let mut side_notches = visible.iter().filter(|n| n.anchor == side && n.auto_width);
            let outer = if side == Anchor::Left { side_notches.next() } else { side_notches.next_back() };
            outer.filter(|n| n.dock && !moved.contains(&n.id)).map(|n| n.id.clone())
        };
        let (left_edge, right_edge) = (outermost(Anchor::Left), outermost(Anchor::Right));

        let mut notches: Vec<NotchLayout> = defs
            .iter()
            .zip(xs)
            .map(|(n, x)| {
                let visible = n.width + 2.0 * g.concave_radius;
                let is_hidden = hidden.contains(&n.id);
                let (open_x, open_w) = open_placed.get(&n.id).copied().unwrap_or((x, n.width));
                // A retracted notch can't be hovered (the hot corners still work).
                let tw = if is_hidden { 0.0 } else { n.trigger_width.as_ref().map_or(0.8 * visible, |l| l.resolve(visible)).max(1.0) };
                NotchLayout {
                    id: n.id.clone(),
                    x,
                    width: n.width,
                    height: if dock { rest_height(n) } else { n.height.unwrap_or(g.notch_height) },
                    trigger_x: x + (n.width - tw) / 2.0,
                    trigger_width: tw,
                    widgets: n.widgets.iter().filter(|w| self.cfg.widget_enabled(w)).cloned().collect(),
                    panel: n.panel.clone(),
                    panel_width: n.panel_width,
                    panel_height: n.panel_height,
                    hidden: is_hidden,
                    edge: if left_edge.as_ref() == Some(&n.id) {
                        Some("left".into())
                    } else if right_edge.as_ref() == Some(&n.id) {
                        Some("right".into())
                    } else {
                        None
                    },
                    hidden_widgets: hidden_widgets.remove(&n.id).unwrap_or_default(),
                    closed_hidden: dock && {
                        let on: Vec<&String> = n.widgets.iter().filter(|w| self.cfg.widget_enabled(w)).collect();
                        !on.is_empty() && on.iter().all(|w| !self.cfg.widget_pinned_in_dock(w))
                    },
                    open_x,
                    open_width: open_w,
                    open_height: if dock { open_h } else { n.height.unwrap_or(g.notch_height) },
                    shrunk: shrunk.contains(&n.id),
                }
            })
            .collect();
        notches.sort_by(|a, b| a.x.total_cmp(&b.x));

        let tallest = notches
            .iter()
            .map(|n| if n.panel.is_some() { g.expanded_height + n.panel_height } else { n.open_height })
            .fold(if dock { open_h } else { g.expanded_height }, f64::max);

        Layout {
            label: label.to_string(),
            monitor: if m.friendly.is_empty() { m.device.clone() } else { m.friendly.clone() },
            device: m.device.clone(),
            surface: surface.to_string(),
            edge: s.edge,
            mode: s.mode,
            rest: s.rest,
            open: s.open,
            rest_height: rest_h,
            open_height: open_h,
            hot: self.cfg.hot(surface, &s),
            corner_radius: if self.cfg.general.rounded_corners && self.cfg.general.corners_attached {
                self.cfg.general.corner_radius
            } else {
                0.0
            },
            reserve: if !s.reserve_space {
                0.0
            } else {
                match s.mode {
                    SurfaceMode::Always => self.cfg.geometry.expanded_height,
                    SurfaceMode::Dock => rest_h,
                    SurfaceMode::Bar => self.cfg.geometry.notch_height,
                }
            },
            width,
            // Room for the rounded screen corners hanging below the shape, too.
            height: (tallest
                + OVERSHOOT_MARGIN
                + if self.cfg.general.rounded_corners && self.cfg.general.corners_attached { self.cfg.general.corner_radius } else { 0.0 })
            .ceil(),
            scale: m.scale,
            geometry: self.cfg.geometry.clone(),
            animation: self.cfg.animation.clone(),
            bar: {
                let mut bar = bar;
                for group in [&mut bar.left, &mut bar.center, &mut bar.right] {
                    group.retain(|w| self.cfg.widget_enabled(w));
                }
                bar
            },
            notches,
            widgets: frontend_widgets(&self.cfg.widgets),
            osd: self.cfg.osd.clone(),
            theme: {
                let mut t = self.cfg.theme.clone();
                if t.accent_sync {
                    if let Some(a) = win::system_accent() {
                        t.accent = a;
                    }
                }
                t
            },
            tooltips: self.cfg.general.tooltips,
            launcher: self.cfg.launcher.enabled,
            combine_notches: self.cfg.notch.combine_overlapping,
            expanded: false,
        }
    }

    /// Creates, updates or destroys overlay windows to match monitors + config.
    fn reconcile(&mut self) {
        crate::taskbar::want(self.cfg.shell.hide_taskbar);
        let mut desired: Vec<(String, Monitor, String, Edge, Layout, Vec<config::Notch>)> = Vec::new();
        let surfaces = self.cfg.surface_list();
        for (i, m) in self.monitors.iter().enumerate() {
            let rule = self.rule_for(m, i);
            eprintln!(
                "[kasumi] monitor #{} {} '{}'{} -> rule {:?}",
                i + 1,
                m.device,
                m.friendly,
                if m.primary { " (primary)" } else { "" },
                rule.map(|r| r.matcher.as_str())
            );
            if rule.is_some_and(|r| !r.enabled) {
                continue;
            }
            for (id, s) in &surfaces {
                if !self.cfg.surface_on(rule, id, s) {
                    continue;
                }
                let label = format!("overlay-{}-{id}", m.device.trim_start_matches(['\\', '.']));
                let (undocked, measured) = self
                    .shared
                    .lock()
                    .unwrap()
                    .overlays
                    .get(&label)
                    .map(|o| (o.undocked.clone(), o.measured.clone()))
                    .unwrap_or_default();
                let layout = self.build_layout(&label, m, rule, id, &undocked, &measured);
                let defs = self.cfg.resolve_surface(rule, id).0;
                desired.push((label, m.clone(), id.clone(), s.edge, layout, defs));
            }
        }

        let mut to_destroy = Vec::new();
        let mut to_create = Vec::new();
        let mut to_update = Vec::new();
        {
            let mut sh = self.shared.lock().unwrap();
            sh.overlays.retain(|label, _| {
                let keep = desired.iter().any(|(l, ..)| l == label);
                if !keep {
                    to_destroy.push(label.clone());
                }
                keep
            });
            for (label, m, surface, edge, mut layout, defs) in desired {
                match sh.overlays.get_mut(&label) {
                    Some(o) => {
                        // The window grew to fit dropdowns (set_content_height): keep that,
                        // or every rebuild would shrink it, resend the layout and grow it back.
                        if o.monitor == m && o.edge == edge && layout.height < o.layout.height {
                            layout.height = o.layout.height;
                        }
                        let moved = o.monitor != m || o.layout.height != layout.height || o.edge != edge;
                        // An unchanged layout isn't sent again: the bar would rebuild its widgets for nothing.
                        let same = !moved && serde_json::to_value(&o.layout).ok() == serde_json::to_value(&layout).ok();
                        o.monitor = m.clone();
                        o.edge = edge;
                        o.layout = layout.clone();
                        o.defs = defs;
                        let top = o.window_top();
                        if !same {
                            to_update.push((label, m, layout, moved, o.hwnd, o.expanded, top));
                        }
                    }
                    None => {
                        sh.overlays.insert(
                            label.clone(),
                            Overlay {
                                monitor: m.clone(),
                                surface,
                                edge,
                                layout: layout.clone(),
                                regions: Vec::new(),
                                hover: Hover::Idle,
                                expanded: false,
                                hidden: false,
                                ignoring: true,
                                hwnd: None,
                                undocked: Vec::new(),
                                nudge: None,
                                fullscreen_since: None,
                                peek_until: None,
                                defs,
                                measured: HashMap::new(),
                                pinned: false,
                            },
                        );
                        to_create.push((label, m, layout));
                    }
                }
            }
        }

        for label in to_destroy {
            if let Some(w) = self.app.get_webview_window(&label) {
                let _ = w.destroy();
            }
        }
        for (label, m, mut layout, moved, hwnd, expanded, top) in to_update {
            if let (true, Some(h)) = (moved, hwnd) {
                let h_px = (layout.height * m.scale).ceil() as i32;
                let _ = self.app.run_on_main_thread(move || win::place(h, m.left, top, m.width(), h_px));
            }
            layout.expanded = expanded;
            let _ = self.app.emit_to(EventTarget::WebviewWindow { label: label.clone() }, "notch://layout", layout);
            // A widget may be new here, and data goes only to pages showing its widget.
            crate::providers::resend(&self.app, &label);
        }
        for (label, m, layout) in to_create {
            if let Err(e) = self.create_window(&label, &m, &layout) {
                eprintln!("[kasumi] failed to create overlay {label}: {e}");
                self.shared.lock().unwrap().overlays.remove(&label);
            }
        }
        self.update_reservations();
    }

    fn create_window(&self, label: &str, m: &Monitor, layout: &Layout) -> tauri::Result<()> {
        eprintln!("[kasumi] creating {label}");
        let window = WebviewWindowBuilder::new(&self.app, label, WebviewUrl::App("index.html".into()))
            .title("Kasumi")
            .transparent(true)
            .decorations(false)
            .shadow(false)
            .always_on_top(true)
            // No skip_taskbar(): it calls into Explorer's taskbar synchronously and
            // hangs window creation whenever Explorer is hung. Instead a style guard
            // (win::guard_overlay_style, installed before the window is shown) keeps
            // it a tool window, so it never gets a taskbar button or Alt+Tab entry.
            .resizable(false)
            .focused(false)
            .focusable(false)
            .visible(false)
            // Dropped files are taken by WebView2 itself (see dropzone).
            .disable_drag_drop_handler()
            .additional_browser_args(win::BROWSER_ARGS)
            .build()?;
        let hwnd = window.hwnd()?.0 as isize;
        eprintln!("[kasumi] built {label}");
        if let Some(o) = self.shared.lock().unwrap().overlays.get_mut(label) {
            o.hwnd = Some(hwnd);
        }
        let (x, y, w, h) = (m.left, window_top(m, layout.edge, layout.height), m.width(), (layout.height * m.scale).ceil() as i32);
        self.app.run_on_main_thread(move || {
            let _ = window.set_ignore_cursor_events(true);
            win::guard_overlay_style(hwnd);
            win::place(hwnd, x, y, w, h);
            crate::dropzone::attach(&window);
            crate::webmem::set_low(&window, true);
            // Show through tao so its visibility flag stays in sync; it re-applies
            // that flag whenever window styles change (e.g. click-through toggles).
            let _ = window.show();
            eprintln!("[kasumi] shown {}", window.label());
        })?;
        Ok(())
    }

    /// The launcher closes when the focus moves elsewhere (Alt+Tab, Win+E, Start,
    /// a click on another app...), even where the page doesn't see that happen.
    /// And without the launcher open, no bar keeps the focus: closing Start or
    /// Search hands it back to the window before, which can be a bar that can't
    /// take typing (keys then went astray: hotkeys, the Windows key). It goes
    /// back to the app instead.
    fn close_launcher_on_focus_loss(&self) {
        use std::sync::atomic::Ordering;
        let (open, ours) = {
            let sh = self.shared.lock().unwrap();
            let open = sh.overlays.iter().find(|(_, o)| o.pinned).and_then(|(l, o)| Some((l.clone(), o.hwnd?)));
            (open, sh.overlays.values().filter_map(|o| o.hwnd).collect::<Vec<isize>>())
        };
        let Some((label, hwnd)) = open else {
            if ours.contains(&win::foreground()) {
                win::restore_foreground(LAUNCHER_PREV.load(Ordering::Relaxed), &ours);
            }
            return;
        };
        if win::foreground() == hwnd {
            LAUNCHER_FOCUSED.store(true, Ordering::Relaxed);
        } else if LAUNCHER_FOCUSED.swap(false, Ordering::Relaxed) {
            launcher_toggle(&self.app, Some(label), Some(false), false);
        }
    }

    fn update_fullscreen(&mut self) {
        self.close_launcher_on_focus_loss();
        let ours: Vec<isize> = {
            let sh = self.shared.lock().unwrap();
            sh.overlays.values().filter_map(|o| o.hwnd).collect()
        };
        let fs = win::fullscreen_monitor(&ours);
        self.update_docking(&ours);
        // Nothing should minimize an overlay (Win+D and friends); if it happens anyway, undo it.
        {
            let sh = self.shared.lock().unwrap();
            for (label, o) in sh.overlays.iter().filter(|(_, o)| !o.hidden) {
                if let Some(h) = o.hwnd {
                    if win::unminimize(h) {
                        eprintln!("[kasumi] {label} was minimized; restored");
                        win::raise_topmost(h);
                    }
                }
            }
        }
        let mut changes = Vec::new();
        {
            let mut sh = self.shared.lock().unwrap();
            for (label, o) in sh.overlays.iter_mut() {
                let fullscreen = fs.as_ref().is_some_and(|(r, _)| o.monitor.same_rect(r));
                // Hide only once fullscreen has lasted a moment, so short-lived
                // full-screen windows can't make the overlay flicker; show at once.
                o.fullscreen_since = if fullscreen { o.fullscreen_since.or(Some(Instant::now())) } else { None };
                // Not while the launcher is open on it (opened over the fullscreen app), nor
                // while shown for a moment ("notches" mode) or in use (open) after that.
                let peeking = o.peek_until.is_some_and(|t| Instant::now() < t) || (o.peek_until.is_some() && o.expanded);
                if !peeking {
                    o.peek_until = None;
                }
                let hide = self.cfg.general.hide_in_fullscreen
                    && !o.pinned
                    && !peeking
                    && o.fullscreen_since.is_some_and(|t| t.elapsed() >= Duration::from_millis(500));
                if hide != o.hidden {
                    match (&fs, hide) {
                        (Some((_, why)), true) => eprintln!("[kasumi] hiding {label}: {why}"),
                        _ => eprintln!("[kasumi] showing {label}"),
                    }
                    o.hidden = hide;
                    changes.push((label.clone(), o.hwnd, hide));
                }
            }
        }
        for (label, hwnd, hide) in changes {
            let app = self.app.clone();
            let _ = self.app.run_on_main_thread(move || {
                if let Some(w) = app.get_webview_window(&label) {
                    let _ = if hide { w.hide() } else { w.show() };
                }
                if let (false, Some(h)) = (hide, hwnd) {
                    win::raise_topmost(h);
                }
            });
        }
    }

    /// Rounded screen corners on edges without a bar: one small mask window per
    /// corner (a bar draws its edge's corners in its own shape, so they move with
    /// it); none on fullscreen monitors.
    fn update_corners(&self) {
        let g = &self.cfg.general;
        if !g.rounded_corners {
            crate::corners::set(Vec::new());
            return;
        }
        let sh = self.shared.lock().unwrap();
        let mut list = Vec::new();
        for m in &self.monitors {
            let here: Vec<&Overlay> = sh.overlays.values().filter(|o| &o.monitor == m).collect();
            if here.iter().any(|o| o.hidden) {
                continue;
            }
            // Logical depth of what fills a corner: an always-on bar, or the notch hugging it.
            let filled = |top: bool, side: &str| {
                here.iter()
                    .filter(|o| (o.edge == Edge::Top) == top)
                    .map(|o| {
                        if o.layout.mode == SurfaceMode::Always {
                            o.layout.geometry.expanded_height
                        } else {
                            o.layout
                                .notches
                                .iter()
                                .filter(|n| !n.hidden && n.edge.as_deref() == Some(side))
                                .map(|n| n.height)
                                .fold(0.0, f64::max)
                        }
                    })
                    .fold(0.0, f64::max)
            };
            let r = (g.corner_radius * m.scale).round().max(1.0) as i32;
            // Detached corners stay in the screen corners whatever the bars do.
            let off = |top: bool, side: &str| if g.corners_attached { (filled(top, side) * m.scale).round() as i32 } else { 0 };
            let has_bar = |top: bool| g.corners_attached && here.iter().any(|o| (o.edge == Edge::Top) == top);
            use crate::corners::{Corner, Which};
            if !has_bar(true) {
                list.push(Corner { x: m.left, y: m.top + off(true, "left"), size: r, which: Which::TopLeft });
                list.push(Corner { x: m.right - r, y: m.top + off(true, "right"), size: r, which: Which::TopRight });
            }
            if !has_bar(false) {
                list.push(Corner { x: m.left, y: m.bottom - r - off(false, "left"), size: r, which: Which::BottomLeft });
                list.push(Corner { x: m.right - r, y: m.bottom - r - off(false, "right"), size: r, which: Which::BottomRight });
            }
        }
        crate::corners::set(list);
    }

    /// Surfaces with `reserve_space` reserve their strip of the screen (AppBar).
    fn update_reservations(&self) {
        let sh = self.shared.lock().unwrap();
        let wanted: Vec<crate::taskbar::Reservation> = sh
            .overlays
            .values()
            .filter(|o| o.layout.reserve > 0.0)
            .filter_map(|o| {
                let m = &o.monitor;
                let h = (o.layout.reserve * m.scale).round() as i32;
                let (top, bottom) = match o.edge {
                    Edge::Top => (m.top, m.top + h),
                    Edge::Bottom => (m.bottom - h, m.bottom),
                };
                Some(crate::taskbar::Reservation {
                    hwnd: o.hwnd?,
                    bottom_edge: o.edge == Edge::Bottom,
                    left: m.left,
                    top,
                    right: m.right,
                    bottom,
                    monitor: (m.left, m.top, m.right, m.bottom),
                })
            })
            .collect();
        crate::taskbar::reserve(wanted);
    }

    /// Repositions notches of overlays whose measured widths changed.
    fn process_relayouts(&mut self) {
        type Job = (String, Monitor, String, Vec<String>, HashMap<String, NotchWidth>);
        let jobs: Vec<Job> = {
            let mut sh = self.shared.lock().unwrap();
            let labels: Vec<String> = sh.relayout.drain().collect();
            labels
                .into_iter()
                .filter_map(|l| {
                    let o = sh.overlays.get(&l)?;
                    Some((l.clone(), o.monitor.clone(), o.surface.clone(), o.undocked.clone(), o.measured.clone()))
                })
                .collect()
        };
        for (label, m, surface, undocked, measured) in jobs {
            let idx = self.monitors.iter().position(|x| x == &m).unwrap_or(0);
            let layout = self.build_layout(&label, &m, self.rule_for(&m, idx), &surface, &undocked, &measured);
            let notches = layout.notches.clone();
            if let Some(o) = self.shared.lock().unwrap().overlays.get_mut(&label) {
                o.layout.notches = layout.notches;
            }
            let _ = self.app.emit_to(EventTarget::WebviewWindow { label }, "notch://positions", notches);
        }
    }

    /// Moves docking notches out of their corner while an app window reaches
    /// under them (e.g. a maximized window's caption buttons), and back after.
    fn update_docking(&mut self, ours: &[isize]) {
        // Notches that react to windows: ones that move aside, and ones with widgets
        // that hide under windows.
        let cfg = &self.cfg;
        let watched = |n: &config::Notch| {
            (n.dock && n.avoid_windows && n.anchor != Anchor::Center)
                || n.widgets.iter().any(|w| cfg.widget_enabled(w) && cfg.widget_hides_when_maximized(w))
        };
        let shrinks = |o: &Overlay| o.layout.mode == SurfaceMode::Dock && self.shrink_when_covered(&o.surface);
        let any_dockable = {
            let sh = self.shared.lock().unwrap();
            sh.overlays.values().any(|o| shrinks(o) || o.defs.iter().any(&watched))
        };
        if !any_dockable {
            return;
        }
        let windows = win::app_windows(ours);
        let g = &self.cfg.geometry;
        let mut changed: Vec<(String, Monitor, String, Vec<String>)> = Vec::new();
        {
            let sh = self.shared.lock().unwrap();
            for (label, o) in sh.overlays.iter() {
                let m = &o.monitor;
                let width = m.width() as f64 / m.scale;
                let mut undocked = Vec::new();
                let all = shrinks(o);
                let dockable = o.defs.iter().filter(|n| all || watched(n));
                // While the bar is open it covers the caption buttons anyway: dock everything.
                for n in dockable.filter(|_| !o.expanded) {
                    let Some(nl) = o.layout.notches.iter().find(|nl| nl.id == n.id) else { continue };
                    // The area the notch covers in its own spot (side notches: in the
                    // corner), flare included, at its full width.
                    let full = o.measured.get(&n.id).map_or(nl.width, |w| if all && w.open > 0.0 { w.open } else { w.full });
                    let mut sized = (*n).clone();
                    sized.width = full;
                    // Docks: the notch's open spot; bars: side notches in their corner.
                    let x = if all { nl.open_x } else if n.anchor == Anchor::Center { nl.x } else { notch_x(&sized, width, g, true) };
                    let (zx0, zx1) = (x - g.concave_radius, x + full + g.concave_radius);
                    let px = |v: f64| m.left + (v.clamp(0.0, width) * m.scale).round() as i32;
                    let depth = ((if all { nl.open_height } else { nl.height } + 4.0) * m.scale).round() as i32;
                    let (zy0, zy1) = match o.edge {
                        Edge::Top => (m.top, m.top + depth),
                        Edge::Bottom => (m.bottom - depth, m.bottom),
                    };
                    // Only a window whose top (bottom bars: bottom) edge is under the notch
                    // itself: its caption buttons are there (maximized, snapped to the edge).
                    // One just below the bar isn't covered by it.
                    let reach = ((if all { nl.open_height } else { nl.height }) * m.scale).round() as i32;
                    let under = |r: &windows::Win32::Foundation::RECT| match o.edge {
                        Edge::Top => r.top < m.top + reach,
                        Edge::Bottom => r.bottom > m.bottom - reach,
                    };
                    if let Some((class, r)) = windows.iter().find(|(_, r)| intersects(r, px(zx0), zy0, px(zx1), zy1) && under(r)) {
                        if !o.undocked.contains(&n.id) {
                            eprintln!(
                                "[kasumi] {label}: '{}' undocked by {class} at ({},{})-({},{})",
                                n.id, r.left, r.top, r.right, r.bottom
                            );
                        }
                        undocked.push(n.id.clone());
                    } else if o.undocked.contains(&n.id) && !o.expanded {
                        eprintln!("[kasumi] {label}: '{}' docked again", n.id);
                    }
                }
                if undocked != o.undocked {
                    changed.push((label.clone(), m.clone(), o.surface.clone(), undocked));
                }
            }
        }
        for (label, m, surface, undocked) in changed {
            let idx = self.monitors.iter().position(|x| x == &m).unwrap_or(0);
            let measured = self.shared.lock().unwrap().overlays.get(&label).map(|o| o.measured.clone()).unwrap_or_default();
            let layout = self.build_layout(&label, &m, self.rule_for(&m, idx), &surface, &undocked, &measured);
            let notches = layout.notches.clone();
            {
                let mut sh = self.shared.lock().unwrap();
                let Some(o) = sh.overlays.get_mut(&label) else { continue };
                o.undocked = undocked;
                o.layout.notches = layout.notches;
            }
            let _ = self.app.emit_to(EventTarget::WebviewWindow { label }, "notch://positions", notches);
        }
    }

    /// Returns whether any overlay opened or closed, and whether things are calm:
    /// nothing opening, open or pinned, and the cursor nowhere near a surface's
    /// edge, so the next look can wait (`CALM_POLL_MS`).
    fn tick_hover(&mut self, now: Instant) -> (bool, bool) {
        let Some((cx, cy)) = win::cursor_pos() else { return (false, true) };
        let mut calm = true;
        let multi = self.monitors.len() > 1;
        let t = &self.cfg.timing;
        let dwell = Duration::from_millis(t.dwell_ms);
        let grace = Duration::from_millis(t.grace_ms);

        let mut emits: Vec<(String, bool)> = Vec::new();
        let mut nudges: Vec<(String, Option<String>)> = Vec::new();
        let nudging = self.cfg.animation.hover_nudge;
        let mut click_through: Vec<(String, isize, bool)> = Vec::new();
        let mut raise: Vec<isize> = Vec::new();
        {
            let mut sh = self.shared.lock().unwrap();
            for (label, o) in sh.overlays.iter_mut() {
                let m = &o.monitor;
                let on_monitor = cx >= m.left && cx < m.right && cy >= m.top && cy < m.bottom;
                let lx = (cx - m.left) as f64 / m.scale;
                // Distance from this surface's own edge.
                let from_edge = match o.edge {
                    Edge::Top => cy - m.top,
                    Edge::Bottom => m.bottom - 1 - cy,
                };
                let ly = from_edge as f64 / m.scale;
                let edge = (t.edge_px * m.scale).max(1.0);
                let at_edge = on_monitor && (from_edge as f64) < edge;
                // Anywhere on a notch's body (or the edge beside it) within its trigger
                // zone, or one of this surface's hot zones (corners / its whole edge).
                let h = &o.layout.hot;
                let size = h.corner;
                let edge_zone = on_monitor && (from_edge as f64) < (h.edge_size * m.scale).max(1.0);
                let in_hot_corner = ly >= 0.0
                    && ((h.left && ly < size && lx >= 0.0 && lx < size)
                        || (h.right && ly < size && lx >= o.layout.width - size && lx <= o.layout.width)
                        || (h.edge && edge_zone));
                let in_trigger = !o.hidden
                    && on_monitor
                    && (in_hot_corner
                        || o.layout.notches.iter().any(|n| {
                            lx >= n.trigger_x
                                && lx < n.trigger_x + n.trigger_width
                                && (at_edge || (ly >= 0.0 && ly < n.height))
                        }));
                let in_bar = !o.hidden && on_monitor && o.expanded && o.hit(lx, ly);

                let always = o.layout.mode == SurfaceMode::Always;
                let next = if o.pinned || always { Hover::Open } else { match o.hover {
                    Hover::Idle if in_trigger => Hover::Dwell(now),
                    Hover::Idle => Hover::Idle,
                    Hover::Dwell(_) if !in_trigger => Hover::Idle,
                    Hover::Dwell(t0) if now - t0 >= dwell => Hover::Open,
                    Hover::Dwell(t0) => Hover::Dwell(t0),
                    Hover::Open if in_bar || in_trigger => Hover::Open,
                    Hover::Open => Hover::Leave(now),
                    Hover::Leave(_) if in_bar || in_trigger => Hover::Open,
                    Hover::Leave(t0) if o.hidden || now - t0 >= grace => Hover::Idle,
                    Hover::Leave(t0) => Hover::Leave(t0),
                } };
                o.hover = next;
                // Every trigger zone lies within this distance of the surface's edge
                // (the settings cap them far below it; a hand-edited config might not).
                let tallest = o.layout.notches.iter().map(|n| n.height).fold(0.0, f64::max);
                let reach = NEAR_EDGE_PX.max(h.edge_size.max(h.corner).max(tallest).max(t.edge_px) + 8.0);
                // With several monitors the cursor can enter this one from its sides.
                let side = (NEAR_EDGE_PX * m.scale).round() as i32;
                let by_side = multi && on_monitor && (cx - m.left < side || m.right - 1 - cx < side);
                let near = on_monitor && (ly < reach || by_side);
                calm &= !always && !o.pinned && matches!(next, Hover::Idle) && !near;

                let expanded = matches!(next, Hover::Open | Hover::Leave(_));
                // Hover nudge: within NUDGE_PX of the edge, over (or beside) a closed notch.
                let nudge = if nudging && on_monitor && !expanded && !o.hidden && ly >= 0.0 && ly < NUDGE_PX {
                    o.layout
                        .notches
                        .iter()
                        .filter(|n| !n.hidden && lx >= n.x - NUDGE_SIDE_PX && lx < n.x + n.width + NUDGE_SIDE_PX)
                        .min_by(|a, b| (lx - (a.x + a.width / 2.0)).abs().total_cmp(&(lx - (b.x + b.width / 2.0)).abs()))
                        .map(|n| n.id.clone())
                } else {
                    None
                };
                if nudge != o.nudge {
                    o.nudge = nudge.clone();
                    nudges.push((label.clone(), nudge));
                }
                if expanded != o.expanded {
                    o.expanded = expanded;
                    if !expanded {
                        o.regions.clear();
                    } else if let Some(h) = o.hwnd {
                        raise.push(h);
                    }
                    emits.push((label.clone(), expanded));
                }

                let ignore = !(o.expanded && o.hit(lx, ly) && on_monitor);
                if ignore != o.ignoring {
                    if let Some(h) = o.hwnd {
                        o.ignoring = ignore;
                        click_through.push((label.clone(), h, ignore));
                    }
                }
            }
        }

        for h in raise {
            win::raise_topmost(h);
        }
        for (label, notch) in nudges {
            let _ = self.app.emit_to(EventTarget::WebviewWindow { label }, "notch://nudge", serde_json::json!({ "notch": notch }));
        }
        let toggled = !emits.is_empty();
        for (label, expanded) in emits {
            // Closed bars keep their memory low (see webmem).
            if let Some(w) = self.app.get_webview_window(&label) {
                crate::webmem::set_low(&w, !expanded);
            }
            let _ = self.app.emit_to(EventTarget::WebviewWindow { label }, "notch://state", StatePayload { expanded });
        }
        for (label, hwnd, ignore) in click_through {
            let app = self.app.clone();
            let _ = self.app.run_on_main_thread(move || {
                if let Some(w) = app.get_webview_window(&label) {
                    let _ = w.set_ignore_cursor_events(ignore);
                    // tao rewrites the extended style when toggling; restore ours.
                    win::apply_overlay_style(hwnd);
                }
            });
        }
        (toggled, calm)
    }
}

/// How often the engine looks at the cursor while things are calm (ms; never
/// faster than `[timing] poll_ms`). The cursor has to cross `NEAR_EDGE_PX` of a
/// surface's edge before any hot zone; a sample inside that band switches to the
/// fast poll, which holds for flicks up to about NEAR_EDGE_PX / CALM_POLL_MS
/// = 5000 px/s. Windows rounds sleeps up, so a calm look is ~45-60 ms apart.
const CALM_POLL_MS: u64 = 40;
const NEAR_EDGE_PX: f64 = 200.0;
/// Hover nudge: how near the edge, and how far beside a notch, the cursor counts as coming to it.
const NUDGE_PX: f64 = 70.0;
const NUDGE_SIDE_PX: f64 = 40.0;

/// Widget settings as the bars get them: without the tasks pin list, which the
/// widget gets with its data (so pinning or reordering doesn't rebuild the bar).
fn frontend_widgets(widgets: &toml::Table) -> toml::Table {
    let mut out = widgets.clone();
    for (name, opts) in out.iter_mut() {
        if name == "tasks" || name.starts_with("tasks#") {
            if let Some(t) = opts.as_table_mut() {
                t.remove("pins");
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notch(id: &str, anchor: Anchor, width: f64, dock: bool) -> config::Notch {
        config::Notch {
            id: id.into(),
            anchor,
            offset: config::Length::Px(150.0),
            width,
            height: None,
            trigger_width: None,
            widgets: vec![id.into()],
            panel: None,
            dock,
            avoid_windows: true,
            panel_width: 0.0,
            panel_height: 0.0,
            auto_width: true,
        }
    }

    #[test]
    fn side_notches_stack_from_the_corner() {
        let g = Geometry::default();
        let spacing = 2.0 * g.concave_radius + 8.0;
        let defs = [
            notch("menu", Anchor::Left, 80.0, true),
            notch("language", Anchor::Left, 70.0, true),
            notch("media", Anchor::Right, 100.0, true),
            notch("temps", Anchor::Right, 120.0, true),
        ];
        let xs = notch_positions(&defs, 1000.0, &g, 8.0, &[]);
        // Left: the first hugs the corner, the next follows one flare-gap later.
        assert_eq!(xs[0], -g.notch_corner_radius);
        assert_eq!(xs[1], xs[0] + 80.0 + spacing);
        // Right: the last in the list is outermost and hugs the corner.
        assert_eq!(xs[3], 1000.0 + g.notch_corner_radius - 120.0);
        assert_eq!(xs[2], xs[3] - spacing - 100.0);

        // Moved aside: the whole side shifts to the offset.
        let xs = notch_positions(&defs, 1000.0, &g, 8.0, &["temps".into()]);
        assert_eq!(xs[3], 1000.0 - 150.0 - 120.0);
        assert_eq!(xs[2], xs[3] - spacing - 100.0);
    }
}
