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

#[derive(Clone, Debug)]
struct Win {
    hwnd: i64,
    title: String,
    active: bool,
    minimized: bool,
    z: i64,
}

#[derive(Clone, Debug)]
struct App {
    key: String,
    wins: Vec<Win>,
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

impl Widget for Tasks {
    fn keys(&self) -> &'static [&'static str] {
        &["tasks", "taskIcons"]
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
                            Win { hwnd: w.get("hwnd").and_then(|x| x.as_i64()).unwrap_or(0), title: s("title"), active: b("active"), minimized: b("minimized"), z: w.get("z").and_then(|x| x.as_i64()).unwrap_or(0) },
                            s("monitor"),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
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
                        ((pinning && pinned) || !w.is_empty()).then(|| App { key, wins: w })
                    })
                    .collect()
            })
            .unwrap_or_default();
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
            let mut bx = x + bx + self.slide_offset(&a.key, now);
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
            self.hover_since = i.filter(|&k| self.apps.get(k).is_some_and(|a| a.wins.len() >= 2)).map(|k| (k, Instant::now()));
        }
        self.hovered = i;
        self.cursor = cursor;
        changed
    }

    fn wake(&self) -> Option<Instant> {
        self.hover_since.map(|(_, t)| t + std::time::Duration::from_millis(450))
    }

    fn tick(&mut self, now: Instant) -> bool {
        if let Some((k, t)) = self.hover_since {
            if now >= t + std::time::Duration::from_millis(450) && self.hovered == Some(k) {
                MENU_TARGET.with(|m| *m.borrow_mut() = self.apps[k].key.clone());
                self.open_request = true;
            }
            self.hover_since = None;
        }
        false
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
                Reply::TogglePanel
            }
        }
    }
}

impl Tasks {
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
