//! Tasks (src/widgets/tasks.ts): a button per app with windows on this monitor
//! (and pinned ones), from the tasks provider. Click: front it (again: minimize;
//! several windows: cycle), or start a pinned app. Middle click: close its front
//! window. Icons size with the row, so they grow as a dock opens; a dot per
//! window, a line under the app in front (gliding to the next one with
//! [animation] marker_glide), icons growing under the cursor (magnify).

use std::cell::Cell;
use std::collections::HashMap;
use std::time::Instant;

use serde_json::Value;

use super::icon::{decode, AppIcon, Pixels, Style};
use super::{ease, fit_text, Act, Button, Ctx, Opts, Reply, Widget};
use crate::native::gfx::{rect, white, with_alpha};

const EASE: (f32, f32, f32, f32) = (0.2, 0.8, 0.2, 1.0);

thread_local! {
    /// The app the tasks panel is about (set on right click), as the page's menuTarget.
    static MENU_TARGET: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
}

pub fn menu_target() -> String {
    MENU_TARGET.with(|t| t.borrow().clone())
}

thread_local! {
    /// The windows panel was opened by hovering the app (previews may show), not by a right click.
    static MENU_HOVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

pub fn menu_by_hover() -> bool {
    MENU_HOVER.with(|h| h.get())
}

#[derive(Clone, Debug)]
struct Win {
    hwnd: i64,
    title: String,
    active: bool,
    attention: bool,
    minimized: bool,
    z: i64,
}

/// How long files are held over a running app before its window comes up.
const DRAG_RAISE_MS: u64 = 600;

/// The attention dot flashes: this long on, as long off.
const BLINK_MS: u128 = 600;

fn blink_epoch() -> Instant {
    static T: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    *T.get_or_init(Instant::now)
}

fn blink_on(now: Instant) -> bool {
    (now.saturating_duration_since(blink_epoch()).as_millis() / BLINK_MS).is_multiple_of(2)
}

/// When the dot next turns on or off.
fn next_blink() -> Instant {
    let since = Instant::now().saturating_duration_since(blink_epoch()).as_millis();
    blink_epoch() + std::time::Duration::from_millis(((since / BLINK_MS + 1) * BLINK_MS) as u64)
}

#[derive(Clone, Debug)]
struct App {
    key: String,
    name: String,
    wins: Vec<Win>,
    pinned: bool,
    /// Its entry in the pin list (an app key or a dropped file's path).
    pin: Option<String>,
}

/// An app being dragged to a new place: which, where the press was, how far it's
/// moved, whether it's really a drag yet, and where it would land.
struct Drag {
    i: usize,
    x0: f32,
    dx: f32,
    moved: bool,
    to: usize,
}

/// The window a click should front, and whether clicking it again minimizes (one window).
fn next_window(wins: &[Win]) -> Option<(i64, bool)> {
    match wins.len() {
        0 => None,
        1 => Some((wins[0].hwnd, true)),
        _ => match wins.iter().position(|w| w.active) {
            None => wins.iter().min_by_key(|w| w.z).map(|w| (w.hwnd, false)),
            Some(i) => Some((wins[(i + 1) % wins.len()].hwnd, false)),
        },
    }
}

/// The front marker travelling to the app now in front.
struct Glide {
    /// The old line's centre (local x), when it set off, how long it takes.
    from: f32,
    to: String,
    start: Instant,
    ms: f32,
}

pub struct Tasks {
    opts: Opts,
    apps: Vec<App>,
    /// Icons by app key: decoded pixels until drawn once, then bitmaps.
    pixels: HashMap<String, Pixels>,
    icons: HashMap<String, AppIcon>,
    style: Option<Style>,
    /// Each button's left and width (local) as last laid out, by app key.
    boxes: Vec<(String, f32, f32)>,
    h: f32,
    hovered: Option<usize>,
    cursor: Option<f32>,
    /// Hovering an app with several windows: which, since when (its windows pop up after 450 ms).
    hover_since: Option<(usize, Instant)>,
    open_request: bool,
    /// Files dragged over an app: which, since when, and whether it's been brought up
    /// yet (after a moment, like the Windows taskbar).
    drag_hover: Option<(usize, Instant, bool)>,
    /// What a tick decided to do (bring up the app files are held over).
    pending: Option<Act>,
    /// Scrolling through an app's windows: which app, the window it went to, when (the
    /// backend's "in front" lags a switch, so quick scrolls go on from here).
    cycle: Option<(String, i64, Instant)>,
    drag: Option<Drag>,
    /// The order the user dragged apps into, until the backend's catches up; every app's key.
    local_order: Option<Vec<String>>,
    all_keys: Vec<String>,
    /// Every app's windows on every monitor (scrolling goes through them, as its list shows them).
    every: Vec<(String, Win)>,
    /// The pin list as written in the config (app keys / dropped files' paths).
    pins: Vec<String>,
    /// Magnify: each icon's scale now (eased toward the cursor's), and when last stepped.
    scales: Cell<[f32; 64]>,
    stepped: Cell<Option<Instant>>,
    active: Option<String>,
    glide: Option<Glide>,
    /// Buttons sliding to a new place (key -> offset, start) and new ones growing in.
    slides: HashMap<String, (f32, Instant)>,
    fresh: HashMap<String, Instant>,
}

impl Tasks {
    pub fn new(opts: Opts) -> Tasks {
        Tasks {
            opts,
            apps: Vec::new(),
            pixels: HashMap::new(),
            icons: HashMap::new(),
            style: None,
            boxes: Vec::new(),
            h: 30.0,
            hovered: None,
            cursor: None,
            hover_since: None,
            open_request: false,
            drag_hover: None,
            pending: None,
            cycle: None,
            drag: None,
            local_order: None,
            all_keys: Vec::new(),
            every: Vec::new(),
            pins: Vec::new(),
            scales: Cell::new([1.0; 64]),
            stepped: Cell::new(None),
            active: None,
            glide: None,
            slides: HashMap::new(),
            fresh: HashMap::new(),
        }
    }

    fn labels(&self) -> bool {
        self.opts.str("labels") == Some("active")
    }

    /// Button sizes at row height `h` (styles.css .task): padding 0.08 h, icon 0.6 h.
    fn button_w(&self, cx: &Ctx, a: &App, h: f32) -> f32 {
        let mut w = h * 0.6 + 2.0 * h * 0.08;
        if self.labels() {
            if let Some(t) = a.wins.iter().find(|w| w.active) {
                w += 6.0 + cx.gfx.measure(&fit_text(cx.gfx, &t.title, &cx.body(), 160.0), &cx.body());
            }
        }
        w
    }

    /// Lays the buttons out at row height `h`; returns the width.
    fn lay_out(&mut self, cx: &Ctx, h: f32) -> f32 {
        let gap = h * 0.12;
        let mut x = 0.0;
        let mut boxes = Vec::with_capacity(self.apps.len());
        for a in &self.apps {
            let w = self.button_w(cx, a, h);
            boxes.push((a.key.clone(), x, w));
            x += w + gap;
        }
        // Buttons whose place changed slide there; new ones grow in (not at the first data).
        let now = Instant::now();
        if !self.boxes.is_empty() {
            for (key, x, _) in &boxes {
                match self.boxes.iter().find(|b| &b.0 == key) {
                    Some((_, old, _)) if (old - x).abs() > 0.5 && (self.h - h).abs() < 0.01 => {
                        let carried = self.slide_offset(key, now);
                        self.slides.insert(key.clone(), (old + carried - x, now));
                    }
                    Some(_) => {}
                    None => {
                        self.fresh.insert(key.clone(), now);
                    }
                }
            }
        }
        self.boxes = boxes;
        self.h = h;
        (x - gap).max(0.0)
    }

    /// How far a button still is from its place (sliding there).
    fn slide_offset(&self, key: &str, now: Instant) -> f32 {
        self.slides.get(key).map_or(0.0, |(dx, t0)| {
            let t = now.saturating_duration_since(*t0).as_secs_f32() / 0.2;
            if t >= 1.0 { 0.0 } else { dx * (1.0 - ease(EASE, t)) }
        })
    }

    /// Where a button's window dots sit (local x of their centre).
    fn dot_x(&self, key: &str) -> Option<f32> {
        self.boxes.iter().find(|b| b.0 == key).map(|(_, x, _)| x + self.h * 0.08 + self.h * 0.3)
    }

    /// Magnify: the scale an icon heads to for the cursor where it is.
    fn magnify_target(&self, i: usize) -> f32 {
        let (Some(cx), true) = (self.cursor, self.opts.magnify) else { return 1.0 };
        let Some((_, x, _)) = self.boxes.get(i) else { return 1.0 };
        let centre = x + self.h * 0.08 + self.h * 0.3;
        1.0 + 0.35 * (1.0 - (cx - centre).abs() / 70.0).max(0.0)
    }

    /// Steps the magnify scales toward their targets (90 ms ease-out ≈ a fast exponential).
    fn step_scales(&self, now: Instant) -> [f32; 64] {
        let dt = self.stepped.get().map_or(1.0, |t| now.saturating_duration_since(t).as_secs_f32());
        self.stepped.set(Some(now));
        let mut s = self.scales.get();
        let k = 1.0 - (-dt / 0.03).exp();
        for (i, v) in s.iter_mut().enumerate().take(self.apps.len().min(64)) {
            *v += (self.magnify_target(i) - *v) * k;
        }
        self.scales.set(s);
        s
    }
}

impl Tasks {
    /// [widgets.tasks] attention (on).
    fn attention(&self) -> bool {
        self.opts.flag("attention") != Some(false)
    }
    /// An app here is asking for attention (its dot flashes).
    fn flashing(&self) -> bool {
        self.attention() && self.apps.iter().any(|a| a.wins.iter().any(|w| w.attention))
    }
}

impl Widget for Tasks {
    fn keys(&self) -> &'static [&'static str] {
        &["tasks", "taskIcons"]
    }
    fn wheel(&mut self, at: (f32, f32), notches: f32) -> Option<Act> {
        let i = self.boxes.iter().position(|(_, bx, bw)| at.0 >= *bx && at.0 < bx + bw)?;
        let a = self.apps.get(i)?.clone();
        match self.opts.str("scroll").unwrap_or("windows") {
            // Up: louder, 4% a notch; Kasumi's volume pop-up shows the app's level.
            "volume" => {
                let (key, delta) = (a.key.clone(), notches * 0.04);
                Some(Act::call(move |app| {
                    if let Some(level) = crate::providers::tasks::task_app_volume(&key, delta) {
                        crate::osd::show(&app, "volume", level, false);
                    }
                }))
            }
            // Its windows in turn, in the order its window list shows them (every monitor):
            // down the next, up the one before, round at the ends.
            "windows" if !a.wins.is_empty() => {
                let list: Vec<i64> = self.every.iter().filter(|(k, _)| *k == a.key).map(|(_, w)| w.hwnd).collect();
                let n = list.len() as i64;
                if n == 0 {
                    return None;
                }
                // Where it is: the window it last went to (the backend's "in front" lags a
                // switch), else the one in front.
                let recent = self.cycle.as_ref().filter(|(k, _, t)| *k == a.key && t.elapsed().as_secs_f32() < 3.0).map(|c| c.1);
                let front = self.every.iter().find(|(k, w)| *k == a.key && w.active).map(|(_, w)| w.hwnd);
                let at = recent.or(front).and_then(|h| list.iter().position(|x| *x == h)).map(|k| k as i64);
                let down = notches < 0.0;
                let next = match at {
                    Some(k) => (k + if down { 1 } else { -1 }).rem_euclid(n),
                    None if down => 0,
                    None => n - 1,
                };
                let hwnd = list[next as usize] as isize;
                self.cycle = Some((a.key.clone(), hwnd as i64, Instant::now()));
                Some(Act::call(move |_| {
                    let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_activate(hwnd, false));
                }))
            }
            _ => None,
        }
    }
    fn win_key(&mut self, n: usize) -> Option<Act> {
        let (_, bx, bw) = self.boxes.get(n.checked_sub(1)?).cloned()?;
        match self.click(Button::Left, (bx + bw / 2.0, 0.0)) {
            Reply::Do(act) => Some(act),
            _ => None,
        }
    }
    fn tooltip(&self) -> Option<String> {
        // Several windows: their list pops up instead.
        let a = self.apps.get(self.hovered?).filter(|_| self.drag.is_none())?;
        match a.wins.len() {
            0 => Some(if a.name.is_empty() { a.key.clone() } else { a.name.clone() }),
            1 if self.opts.str("hover") == Some("previews") => None,
            1 => Some(a.wins[0].title.clone()).filter(|t| !t.is_empty()),
            _ => None,
        }
    }

    fn data(&mut self, key: &str, v: &Value) -> bool {
        if key == "taskIcons" {
            let Some(map) = v.as_object() else { return false };
            let mut changed = false;
            for (k, icon) in map {
                if let Some(px) = decode(icon) {
                    self.icons.remove(k);
                    self.pixels.insert(k.clone(), px);
                    changed = true;
                }
            }
            return changed;
        }
        let own = self.opts.str("monitor") != Some("all");
        let pinning = self.opts.flag("pinning") != Some(false) && self.opts.flag("pinned") != Some(false);
        let device = self.opts.device.clone();
        let wins: Vec<(String, Win, String)> = v
            .get("windows")
            .and_then(|w| w.as_array())
            .map(|a| {
                a.iter()
                    .map(|w| {
                        let s = |k: &str| w.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                        let b = |k: &str| w.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
                        (
                            s("app"),
                            Win { hwnd: w.get("hwnd").and_then(|x| x.as_i64()).unwrap_or(0), title: s("title"), active: b("active"), attention: b("attention"), minimized: b("minimized"), z: w.get("z").and_then(|x| x.as_i64()).unwrap_or(0) },
                            s("monitor"),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.every = wins.iter().map(|(k, w, _)| (k.clone(), w.clone())).collect();
        let mine: Vec<&(String, Win, String)> = wins.iter().filter(|(_, _, m)| !own || device.is_empty() || *m == device).collect();
        let apps: Vec<App> = v
            .get("apps")
            .and_then(|a| a.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|a| {
                        let key = a.get("key")?.as_str()?.to_string();
                        let pinned = a.get("pinned").and_then(|p| p.as_bool()).unwrap_or(false);
                        let w: Vec<Win> = mine.iter().filter(|(k, _, _)| *k == key).map(|(_, w, _)| w.clone()).collect();
                        ((pinning && pinned) || !w.is_empty()).then(|| App { name: a.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(), key, wins: w, pinned, pin: a.get("pin").and_then(|p| p.as_str()).map(String::from) })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let mut apps = apps;
        self.pins = v
            .get("apps")
            .and_then(|a| a.as_array())
            .map(|a| a.iter().filter(|x| x.get("pinned").and_then(|p| p.as_bool()) == Some(true)).filter_map(|x| x.get("pin").and_then(|p| p.as_str()).or(x.get("key").and_then(|k| k.as_str())).map(String::from)).collect())
            .unwrap_or_default();
        self.all_keys = v.get("apps").and_then(|a| a.as_array()).map(|a| a.iter().filter_map(|x| x.get("key")?.as_str().map(String::from)).collect()).unwrap_or_default();
        if let Some(order) = &self.local_order {
            // The new order has arrived from the backend: stop overriding it.
            let theirs: Vec<&String> = self.all_keys.iter().filter(|k| order.contains(k)).collect();
            let mine: Vec<&String> = order.iter().filter(|k| self.all_keys.contains(k)).collect();
            if theirs == mine {
                self.local_order = None;
            } else {
                let at = |k: &str| order.iter().position(|o| o == k).unwrap_or(usize::MAX);
                apps.sort_by_key(|a| at(&a.key));
            }
        }
        let active = apps.iter().find(|a| a.wins.iter().any(|w| w.active)).map(|a| a.key.clone());
        if self.opts.glide && active.is_some() && self.active.is_some() && active != self.active {
            let from = self.glide_x(Instant::now()).or_else(|| self.active.as_deref().and_then(|k| self.dot_x(k)));
            if let (Some(from), Some(to)) = (from, active.clone()) {
                let dist = self.dot_x(&to).map_or(0.0, |x| (x - from).abs());
                self.glide = Some(Glide { from, to, start: Instant::now(), ms: (170.0 + dist * 0.35).min(320.0) });
            }
        }
        self.active = active;
        let changed = format!("{apps:?}") != format!("{:?}", self.apps);
        self.apps = apps;
        changed
    }

    fn width(&mut self, cx: &Ctx) -> f32 {
        // Icons into bitmaps (the first time they're drawn, or after a new style).
        let style = Style::from(self.opts.str("icons"), cx.look.accent);
        if self.style != Some(style) {
            self.style = Some(style);
            self.icons.clear();
        }
        for a in &self.apps {
            if !self.icons.contains_key(&a.key) {
                if let Some(icon) = self.pixels.get(&a.key).and_then(|px| AppIcon::new(cx.gfx, px, style)) {
                    self.icons.insert(a.key.clone(), icon);
                }
            }
        }
        if self.apps.is_empty() {
            self.boxes.clear();
            return 0.0;
        }
        self.lay_out(cx, cx.h)
    }

    fn draw(&self, cx: &Ctx, x: f32, cy: f32, alpha: f32) {
        let g = cx.gfx;
        let h = cx.h;
        let now = Instant::now();
        let scales = self.step_scales(now);
        let bottom = cx.look.bottom;
        let box_h = h * 0.82;
        let top = cy - box_h / 2.0;
        let glide = self.glide.as_ref().filter(|gl| now.saturating_duration_since(gl.start).as_secs_f32() * 1000.0 < gl.ms);
        for (i, a) in self.apps.iter().enumerate() {
            let Some((_, bx, bw)) = self.boxes.iter().find(|b| b.0 == a.key).cloned() else { continue };
            let mut bx = x + bx + self.slide_offset(&a.key, now) + self.drag_offset(i);
            // New buttons grow in (180 ms).
            let grow = self.fresh.get(&a.key).map_or(1.0, |t0| ease(EASE, now.saturating_duration_since(*t0).as_secs_f32() / 0.18));
            let a_alpha = alpha * grow;
            bx = g.snap(bx);
            if self.hovered == Some(i) {
                g.fill_round(rect(bx, top, bw, box_h), h * 0.2, with_alpha(cx.hover_bg(0.1), a_alpha));
            }
            let size = h * 0.6;
            let ix = bx + h * 0.08;
            let iy = cy - size / 2.0;
            let s = if self.opts.magnify { scales[i.min(63)] * (0.6 + 0.4 * grow) } else { 0.6 + 0.4 * grow };
            // Grows from the dock's edge (top bars: down from the top; docks at the bottom: up).
            let (sx, sy) = (ix + size / 2.0 - size * s / 2.0, if bottom { iy + size - size * s } else { iy });
            let minimized = !a.wins.is_empty() && a.wins.iter().all(|w| w.minimized);
            let dim = if minimized && self.opts.flag("dim_minimized") == Some(true) { 0.55 } else { 1.0 };
            match self.icons.get(&a.key) {
                Some(icon) => icon.draw(g, sx, sy, size * s, a_alpha * dim),
                None => g.fill_round(rect(sx, sy, size * s, size * s), size * s * 0.25, white(0.15 * a_alpha)),
            }
            // Asking for attention: an accent dot on the icon's corner, flashing.
            if self.attention() && a.wins.iter().any(|w| w.attention) && blink_on(now) {
                let r = (size * s * 0.13).max(3.0);
                let (dx, dy) = (sx + size * s - r * 0.8, sy + r * 0.8);
                g.fill_round(rect(dx - r, dy - r, 2.0 * r, 2.0 * r), r, with_alpha(cx.look.accent, a_alpha));
            }
            if self.labels() {
                if let Some(t) = a.wins.iter().find(|w| w.active) {
                    let label = fit_text(g, &t.title, &cx.body(), 160.0);
                    g.text_at(&label, &cx.body(), ix + size + 6.0, cy, with_alpha(cx.look.fg, a_alpha));
                }
            }
            // A dot per window (up to 3); a line under the app in front.
            if a.wins.is_empty() {
                continue;
            }
            let dot_y = top + box_h - 1.0 - 3.0;
            let centre = ix + size / 2.0;
            let active = a.wins.iter().any(|w| w.active);
            if active {
                // While the marker glides here, the app's own line waits.
                if glide.is_some_and(|gl| gl.to == a.key) {
                    continue;
                }
                let c = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
                g.fill_round(rect(centre - 5.0, dot_y, 10.0, 3.0), 1.5, with_alpha(c, a_alpha));
            } else {
                let n = a.wins.len().min(3) as f32;
                let w = n * 3.0 + (n - 1.0) * 2.0;
                for k in 0..n as usize {
                    g.fill_round(rect(centre - w / 2.0 + k as f32 * 5.0, dot_y, 3.0, 3.0), 1.5, with_alpha(cx.look.dimmed(), a_alpha));
                }
            }
        }
        // The marker on its way: the front edge runs ahead (the line stretches and thins),
        // the back edge follows; both arrive together.
        if let Some(gl) = glide {
            let Some(to) = self.dot_x(&gl.to) else { return };
            let t = (now.saturating_duration_since(gl.start).as_secs_f32() * 1000.0 / gl.ms).min(1.0);
            let ease3 = |k: f32| if k < 0.5 { 4.0 * k * k * k } else { 1.0 - (-2.0 * k + 2.0).powi(3) / 2.0 };
            let part = |from: f32| ease3(((t - from) / 0.68).clamp(0.0, 1.0));
            let (line, thick) = (10.0, 3.0);
            let right = to > gl.from;
            let half = if right { line / 2.0 } else { -line / 2.0 };
            let f = (gl.from + half) + ((to + half) - (gl.from + half)) * part(0.0);
            let b = (gl.from - half) + ((to - half) - (gl.from - half)) * part(0.32);
            let width = (f - b).abs().max(thick);
            let th = (thick * (line / width).sqrt()).clamp(2.0, thick);
            let dot_y = top + box_h - 1.0 - 3.0 + thick - th;
            let c = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
            g.fill_round(rect(x + f.min(b), dot_y, width, th), th / 2.0, with_alpha(c, alpha));
        }
    }

    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let i = at.and_then(|(x, _)| self.boxes.iter().position(|(_, bx, bw)| x >= *bx && x < bx + bw));
        let cursor = at.map(|(x, _)| x);
        let changed = i != self.hovered || (self.opts.magnify && cursor != self.cursor);
        if i != self.hovered {
            // Its windows pop up for two or more (previews: for one too).
            let least = if self.opts.str("hover") == Some("previews") { 1 } else { 2 };
            self.hover_since = i.filter(|&k| self.apps.get(k).is_some_and(|a| a.wins.len() >= least)).map(|k| (k, Instant::now()));
        }
        self.hovered = i;
        self.cursor = cursor;
        changed
    }

    fn wake(&self) -> Option<Instant> {
        let menu = self.hover_since.map(|(_, t)| t + std::time::Duration::from_millis(450));
        let raise = self.drag_hover.filter(|d| !d.2).map(|(_, t, _)| t + std::time::Duration::from_millis(DRAG_RAISE_MS));
        // The attention dot's next flash (on or off).
        let blink = self.flashing().then(next_blink);
        [menu, raise, blink].into_iter().flatten().min()
    }

    fn tick(&mut self, now: Instant) -> bool {
        if let Some((k, t)) = self.hover_since {
            if now >= t + std::time::Duration::from_millis(450) && self.hovered == Some(k) {
                MENU_TARGET.with(|m| *m.borrow_mut() = self.apps[k].key.clone());
                MENU_HOVER.with(|h| h.set(true));
                self.open_request = true;
            }
            self.hover_since = None;
        }
        let flashing = self.flashing();
        // Files held over a running app: bring its window up to drop them into.
        if let Some((k, t, false)) = self.drag_hover {
            if now >= t + std::time::Duration::from_millis(DRAG_RAISE_MS) {
                self.drag_hover = Some((k, t, true));
                if let Some(w) = self.apps.get(k).and_then(|a| a.wins.iter().min_by_key(|w| w.z)) {
                    let hwnd = w.hwnd as isize;
                    self.pending = Some(Act::call(move |_| {
                        let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_activate(hwnd, false));
                    }));
                }
            }
        }
        flashing
    }

    fn act(&mut self) -> Option<Act> {
        self.pending.take()
    }

    fn drag_over(&mut self, at: Option<(f32, f32)>) -> bool {
        let i = at.and_then(|(x, _)| self.boxes.iter().position(|(_, bx, bw)| x >= *bx && x < bx + bw));
        if i != self.drag_hover.map(|d| d.0) {
            self.drag_hover = i.map(|k| (k, Instant::now(), false));
        }
        // Shown as hovered (and magnified) as under the cursor, without the windows menu.
        let changed = self.hover(at);
        self.hover_since = None;
        changed
    }

    fn drop_on(&mut self, at: (f32, f32), paths: &[String]) -> Option<Act> {
        self.drag_hover = None;
        let i = self.boxes.iter().position(|(_, bx, bw)| at.0 >= *bx && at.0 < bx + bw)?;
        let key = self.apps.get(i)?.key.clone();
        let paths = paths.to_vec();
        eprintln!("[kasumi] tasks: opening {paths:?} with {key}");
        Some(Act::call(move |_| {
            if let Err(e) = crate::providers::tasks::task_open_with(&key, &paths) {
                eprintln!("[kasumi] tasks: couldn't open with {key}: {e}");
            }
        }))
    }

    fn wants_panel(&mut self) -> bool {
        std::mem::take(&mut self.open_request)
    }

    fn animating(&self, now: Instant) -> bool {
        let magnifying = self.opts.magnify && {
            let s = self.scales.get();
            (0..self.apps.len().min(64)).any(|i| (s[i] - self.magnify_target(i)).abs() > 0.002)
        };
        magnifying
            || self.glide.as_ref().is_some_and(|gl| now.saturating_duration_since(gl.start).as_secs_f32() * 1000.0 < gl.ms + 20.0)
            || self.slides.values().any(|(_, t)| now.saturating_duration_since(*t).as_secs_f32() < 0.22)
            || self.fresh.values().any(|t| now.saturating_duration_since(*t).as_secs_f32() < 0.2)
    }

    fn dropped(&mut self, paths: &[String]) -> Option<Act> {
        if self.opts.flag("pinning") == Some(false) || self.opts.flag("pinned") == Some(false) {
            return None;
        }
        let added: Vec<String> = paths.iter().filter(|p| !self.pins.iter().any(|x| x.eq_ignore_ascii_case(p))).cloned().collect();
        if added.is_empty() {
            return None;
        }
        eprintln!("[kasumi] tasks: pinning {added:?}");
        let mut pins = self.pins.clone();
        pins.extend(added);
        Some(Act::call(move |_| {
            use crate::config_edit::Seg::Key;
            let path = vec![Key("widgets".into()), Key("tasks".into()), Key("pins".into())];
            let _ = tauri::async_runtime::block_on(crate::config_edit::config_write(path, Some(serde_json::json!(pins))));
        }))
    }

    fn press(&mut self, at: (f32, f32)) -> bool {
        let Some(i) = self.boxes.iter().position(|(_, bx, bw)| at.0 >= *bx && at.0 < bx + bw) else { return false };
        self.drag = Some(Drag { i, x0: at.0, dx: 0.0, moved: false, to: i });
        true
    }

    fn drag(&mut self, at: (f32, f32)) -> Reply {
        let Some(d) = self.drag.as_mut() else { return Reply::None };
        let dx = at.0 - d.x0;
        if !d.moved && dx.abs() < 6.0 {
            return Reply::None;
        }
        d.moved = true;
        self.hover_since = None;
        // Stays within the widget, give or take enough to pass the ends' middles.
        let (_, me_x, me_w) = self.boxes[d.i].clone();
        let (first, last) = (self.boxes[0].1, self.boxes.last().map_or(0.0, |b| b.1 + b.2));
        let over = me_w / 2.0 + 6.0;
        d.dx = dx.clamp(first - me_x - over, last - (me_x + me_w) + over);
        let centre = me_x + me_w / 2.0 + d.dx;
        let mut to = d.i;
        for (j, (_, bx, bw)) in self.boxes.iter().enumerate() {
            let mid = bx + bw / 2.0;
            if j > d.i && centre > mid {
                to = to.max(j);
            } else if j < d.i && centre < mid {
                to = to.min(j);
            }
        }
        d.to = to;
        Reply::Redraw
    }

    fn release(&mut self, at: (f32, f32)) -> Reply {
        let Some(d) = self.drag.take() else { return Reply::None };
        if !d.moved {
            return self.click(Button::Left, at);
        }
        if d.to == d.i {
            return Reply::Redraw;
        }
        let app = self.apps.remove(d.i);
        self.apps.insert(d.to, app);
        // New order: what's shown here, then everything else as it was.
        let shown: Vec<String> = self.apps.iter().map(|a| a.key.clone()).collect();
        let mut order = shown.clone();
        order.extend(self.all_keys.iter().filter(|k| !shown.contains(k)).cloned());
        self.local_order = Some(order.clone());
        // Pinned apps keep their new order across restarts.
        let pins: Vec<String> = self.apps.iter().filter(|a| a.pinned).map(|a| a.pin.clone().unwrap_or_else(|| a.key.clone())).collect();
        let pinning = self.opts.flag("pinning") != Some(false) && self.opts.flag("pinned") != Some(false);
        Reply::Do(Act::call(move |_| {
            let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_reorder(order));
            if pinning && !pins.is_empty() {
                use crate::config_edit::Seg::Key;
                let path = vec![Key("widgets".into()), Key("tasks".into()), Key("pins".into())];
                let _ = tauri::async_runtime::block_on(crate::config_edit::config_write(path, Some(serde_json::json!(pins))));
            }
        }))
    }

    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        let Some(i) = self.boxes.iter().position(|(_, bx, bw)| at.0 >= *bx && at.0 < bx + bw) else { return Reply::None };
        let a = &self.apps[i];
        match button {
            Button::Left => match next_window(&a.wins) {
                Some((hwnd, toggle)) => Reply::Do(Act::call(move |_| {
                    let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_activate(hwnd as isize, toggle));
                })),
                // A pinned app that isn't running: start it.
                None => {
                    let key = a.key.clone();
                    Reply::Do(Act::call(move |_| {
                        let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_launch(key));
                    }))
                }
            },
            Button::Middle => match a.wins.iter().min_by_key(|w| w.z) {
                Some(w) => {
                    let hwnd = w.hwnd as isize;
                    Reply::Do(Act::call(move |_| {
                        let _ = tauri::async_runtime::block_on(crate::providers::tasks::task_close(hwnd));
                    }))
                }
                None => Reply::None,
            },
            // The app's menu (its windows, a new window, pin, close all): the tasks panel.
            Button::Right => {
                self.hover_since = None;
                MENU_TARGET.with(|t| *t.borrow_mut() = a.key.clone());
                MENU_HOVER.with(|h| h.set(false));
                Reply::TogglePanel
            }
        }
    }
}

impl Tasks {
    fn drag_offset(&self, i: usize) -> f32 {
        let Some(d) = self.drag.as_ref().filter(|d| d.moved) else { return 0.0 };
        if i == d.i {
            return d.dx;
        }
        let room = self.boxes.get(d.i).map_or(0.0, |b| b.2) + self.h * 0.12;
        if i > d.i && i <= d.to {
            -room
        } else if i < d.i && i >= d.to {
            room
        } else {
            0.0
        }
    }

    /// Where a gliding marker's centre is now (to start the next glide from there).
    fn glide_x(&self, now: Instant) -> Option<f32> {
        let gl = self.glide.as_ref()?;
        let t = now.saturating_duration_since(gl.start).as_secs_f32() * 1000.0 / gl.ms;
        if t >= 1.0 {
            return None;
        }
        let to = self.dot_x(&gl.to)?;
        Some(gl.from + (to - gl.from) * t.clamp(0.0, 1.0))
    }
}
