//! The quick settings panel (src/widgets/quick.ts quickPanel): volume (with the
//! output devices and per-app volume behind buttons at its start) and brightness
//! side by side, battery and network, a row of shortcut tiles. The lists open in
//! a box that slides open between the controls and the tiles (above the controls
//! on a bottom dock), so the controls stay put.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use serde_json::Value;

use super::icon::{decode, AppIcon, Style};
use super::shell::{battery_glyph, network_glyph, volume_glyph};
use super::{ease, fit_text, Act, Button, Ctx, Opts, Panel, Reply};
use crate::native::gfx::{rect, white, with_alpha, Color, Font, Gfx};

/// Tiles: id -> (label, glyph, what it opens) (src/quick-tiles.ts).
const TILES: [(&str, &str, &str, &str); 16] = [
    ("wifi", "Wi-Fi", "\u{E701}", "ms-settings:network-wifi"),
    ("bluetooth", "Bluetooth", "\u{E702}", "ms-settings:bluetooth"),
    ("display", "Display", "\u{E7F4}", "ms-settings:display"),
    ("sound", "Sound", "\u{E767}", "ms-settings:sound"),
    ("power", "Power", "\u{E7E8}", "ms-settings:powersleep"),
    ("settings", "Settings", "\u{E713}", "ms-settings:"),
    ("nightlight", "Night light", "\u{E708}", "ms-settings:nightlight"),
    ("network", "Network", "\u{E774}", "ms-settings:network"),
    ("personalization", "Personalize", "\u{E771}", "ms-settings:personalization"),
    ("notifications", "Notifications", "\u{EA8F}", "ms-settings:notifications"),
    ("update", "Update", "\u{E895}", "ms-settings:windowsupdate"),
    ("storage", "Storage", "\u{EDA2}", "ms-settings:storagesense"),
    ("apps", "Apps", "\u{E71D}", "ms-settings:appsfeatures"),
    ("taskmgr", "Task Manager", "\u{E9D9}", "taskmgr.exe"),
    ("explorer", "Explorer", "\u{E838}", "explorer.exe"),
    ("kasumi", "Kasumi", "", "kasumi:settings"),
];
const DEFAULT_TILES: [&str; 6] = ["wifi", "bluetooth", "display", "sound", "power", "settings"];
const MAX_TILES: usize = 6;

/// Kasumi's logo (icons/kasumi-mark.svg), view box 30 30 541 541.
const LOGO: &str = "M483.75,113H205.03c-45.43,0-82.25,36.82-82.25,82.25h0c0,45.43,36.82,82.25,82.25,82.25h-.92c11.96.54,21.49,10.41,21.49,22.5s-9.53,21.96-21.49,22.5h-87.86c-45.43,0-82.25,36.82-82.25,82.25h0c0,45.43,36.82,82.25,82.25,82.25h229.5c45.43,0,82.25-36.82,82.25-82.25h0c0-45.43-36.82-82.25-82.25-82.25h-.29c-11.96-.54-21.49-10.41-21.49-22.5s9.53-21.96,21.49-22.5h138.29c45.43,0,82.25-36.82,82.25-82.25h0c0-45.43-36.82-82.25-82.25-82.25Z";

fn line(g: &Gfx, f: &Font) -> f32 {
    g.text("Ag", f).map_or(f.size * 1.21, |t| t.height)
}

/// "2 h 5 min left".
fn left(secs: Option<f64>) -> String {
    let Some(s) = secs.filter(|s| *s > 0.0) else { return String::new() };
    let h = (s / 3600.0).floor();
    let m = ((s % 3600.0) / 60.0).round();
    if h > 0.0 { format!("{h} h {m} min left") } else { format!("{m} min left") }
}

/// A slider being dragged or settling: which one, its value, when it last sent.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Slider {
    Volume,
    Brightness,
    App(u32),
}

#[derive(Clone, Copy, PartialEq)]
enum List {
    Outputs,
    Apps,
}

/// What can be clicked, by where it was drawn (content px).
#[derive(Clone, Copy, PartialEq, Debug)]
enum Hit {
    Track(Slider),
    VolumeIcon,
    AppIcon(u32),
    Expand(usize),
    Display,
    Output(usize),
    Tile(usize),
}

pub struct QuickPanel {
    opts: Opts,
    system: Value,
    outputs: Vec<(String, String, bool)>,
    apps: Vec<(u32, String, f64, bool)>,
    app_pixels: HashMap<u32, super::icon::Pixels>,
    app_icons: HashMap<u32, AppIcon>,
    /// The list open (and the box's openness, animated), when it last refreshed.
    list: Option<List>,
    shown_list: Option<List>,
    slide: Option<(Instant, f32, f32)>,
    box_h: f32,
    refreshed: Option<Instant>,
    pending: Option<Act>,
    /// A slider held: its value, when it last sent one; held values for a moment after.
    held: Option<(Slider, f32)>,
    sent: Option<Instant>,
    settle: Option<(Slider, f32, Instant)>,
    hovered: Option<Hit>,
    /// Clickable boxes as last drawn (content px from the panel's content corner).
    boxes: std::cell::RefCell<Vec<(Hit, f32, f32, f32, f32)>>,
    width: f32,
}

const W: f32 = 540.0;
const COL_GAP: f32 = 28.0;
const ROW_H: f32 = 26.0;

impl QuickPanel {
    pub fn new(opts: &Opts) -> QuickPanel {
        QuickPanel {
            opts: opts.clone(),
            system: Value::Null,
            outputs: Vec::new(),
            apps: Vec::new(),
            app_pixels: HashMap::new(),
            app_icons: HashMap::new(),
            list: None,
            shown_list: None,
            slide: None,
            box_h: 0.0,
            refreshed: None,
            pending: None,
            held: None,
            sent: None,
            settle: None,
            hovered: None,
            boxes: std::cell::RefCell::new(Vec::new()),
            width: W,
        }
    }

    fn tiles(&self) -> Vec<&'static (&'static str, &'static str, &'static str, &'static str)> {
        let chosen: Vec<String> = self.opts.table.get("tiles").and_then(|t| t.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_else(|| DEFAULT_TILES.iter().map(|s| s.to_string()).collect());
        chosen.iter().filter_map(|id| TILES.iter().find(|t| t.0 == id)).take(MAX_TILES).collect()
    }
    fn has_outputs(&self) -> bool {
        self.opts.flag("volume") != Some(false) && self.opts.flag("outputs") == Some(true)
    }
    fn has_apps(&self) -> bool {
        self.opts.flag("volume") != Some(false) && self.opts.flag("apps") == Some(true)
    }
    fn volume(&self) -> Option<(f64, bool)> {
        let v = self.system.get("volume").filter(|v| !v.is_null())?;
        Some((v.get("level").and_then(|l| l.as_f64()).unwrap_or(0.0), v.get("muted").and_then(|m| m.as_bool()).unwrap_or(false)))
    }
    fn brightness(&self) -> Option<f64> {
        self.system.get("brightness").and_then(|b| b.as_f64())
    }
    /// A slider's value now (0..100): held while dragged and for a moment after.
    fn value(&self, s: Slider) -> f32 {
        if let Some((_, v)) = self.held.filter(|(h, _)| *h == s) {
            return v;
        }
        if let Some((h, v, until)) = self.settle {
            if h == s && Instant::now() < until {
                return v;
            }
        }
        match s {
            Slider::Volume => self.volume().map_or(0.0, |(l, m)| if m { 0.0 } else { (l * 100.0) as f32 }),
            Slider::Brightness => self.brightness().unwrap_or(0.0) as f32,
            Slider::App(pid) => self.apps.iter().find(|a| a.0 == pid).map_or(0.0, |a| if a.3 { 0.0 } else { (a.2 * 100.0) as f32 }),
        }
    }
    /// Asks the backend for the open list (it comes back as data).
    fn fetch(list: List) -> Act {
        Act::call(move |_| {
            let (key, v) = match list {
                List::Outputs => ("quick:outputs", tauri::async_runtime::block_on(crate::providers::audio::audio_outputs())),
                List::Apps => ("quick:apps", tauri::async_runtime::block_on(crate::providers::audio::audio_sessions())),
            };
            if let Ok(v) = v {
                crate::native::post(crate::native::Msg::Data(key.into(), v));
            }
        })
    }
    /// What a slider at a value sends.
    fn send(s: Slider, v: f32) -> Act {
        Act::call(move |_| {
            let r = match s {
                Slider::Volume => tauri::async_runtime::block_on(crate::providers::system::volume_set(v / 100.0)),
                Slider::Brightness => tauri::async_runtime::block_on(crate::providers::system::brightness_set(v.round() as u8)),
                Slider::App(pid) => tauri::async_runtime::block_on(crate::providers::audio::audio_session_set(pid, Some(v / 100.0), None)),
            };
            if let Err(e) = r {
                eprintln!("[kasumi] quick settings: {e}");
            }
        })
    }

    /// The list box's openness now (0..1) and whether it's still moving.
    fn openness(&self, now: Instant) -> (f32, bool) {
        let target = if self.list.is_some() { 1.0 } else { 0.0 };
        match self.slide {
            Some((t0, from, to)) => {
                let k = (now.saturating_duration_since(t0).as_secs_f32() / 0.2).min(1.0);
                let e = 1.0 - (1.0 - k).powi(3);
                (from + (to - from) * e, k < 1.0)
            }
            None => (target, false),
        }
    }

    fn toggle_list(&mut self, which: List) {
        let (now_open, _) = self.openness(Instant::now());
        if self.list == Some(which) {
            self.list = None;
            self.slide = Some((Instant::now(), now_open, 0.0));
            return;
        }
        self.list = Some(which);
        self.shown_list = Some(which);
        self.slide = Some((Instant::now(), now_open, 1.0));
        self.refreshed = Some(Instant::now());
        self.pending = Some(Self::fetch(which));
    }

    /// The list box's content height (at full openness).
    fn list_h(&self, g: &Gfx, cx: &Ctx) -> f32 {
        let rows = |n: usize, h: f32| if n == 0 { 0.0 } else { (n.div_ceil(2)) as f32 * h + (n.div_ceil(2) as f32 - 1.0) * 10.0 };
        let inner = match self.shown_list {
            Some(List::Outputs) => rows(self.outputs.len(), 12.0 + line(g, &cx.secondary(12.0))),
            Some(List::Apps) if self.apps.is_empty() => line(g, &cx.secondary(12.0)),
            Some(List::Apps) => rows(self.apps.len(), 24.0),
            None => 0.0,
        };
        inner + 24.0
    }

    fn grid_h(&self, g: &Gfx, cx: &Ctx) -> f32 {
        let vol = self.opts.flag("volume") != Some(false) && self.volume().is_some();
        let bri = self.opts.flag("brightness") != Some(false) && self.brightness().is_some();
        let status = line(g, &cx.secondary(cx.look.font_size)).max(18.0);
        let mut h = 0.0;
        if vol || bri {
            h += ROW_H;
        }
        if h > 0.0 {
            h += 12.0;
        }
        h + status
    }

    fn tiles_h(&self, g: &Gfx, cx: &Ctx) -> f32 {
        let n = self.tiles().len();
        if n == 0 {
            return 0.0;
        }
        if n <= 3 {
            8.0 + line(g, &cx.secondary(12.0)).max(14.0) + 8.0
        } else {
            12.0 + 14.0 + 7.0 + line(g, &cx.secondary(11.0)) + 10.0
        }
    }
}

impl Panel for QuickPanel {
    fn keys(&self) -> &'static [&'static str] {
        &["system", "quick:outputs", "quick:apps"]
    }
    fn data(&mut self, key: &str, v: &Value) -> bool {
        match key {
            "system" => self.system = v.clone(),
            "quick:outputs" => {
                self.outputs = v
                    .as_array()
                    .map(|a| a.iter().map(|d| (d.get("id").and_then(|x| x.as_str()).unwrap_or("").into(), d.get("name").and_then(|x| x.as_str()).unwrap_or("").into(), d.get("default").and_then(|x| x.as_bool()).unwrap_or(false))).collect())
                    .unwrap_or_default();
            }
            "quick:apps" => {
                let list = v.as_array().cloned().unwrap_or_default();
                self.apps = list
                    .iter()
                    .map(|a| {
                        let pid = a.get("pid").and_then(|x| x.as_u64()).unwrap_or(0) as u32;
                        if let Some(px) = a.get("icon").and_then(decode) {
                            if !self.app_icons.contains_key(&pid) {
                                self.app_pixels.insert(pid, px);
                            }
                        }
                        (pid, a.get("name").and_then(|x| x.as_str()).unwrap_or("").into(), a.get("level").and_then(|x| x.as_f64()).unwrap_or(0.0), a.get("muted").and_then(|x| x.as_bool()).unwrap_or(false))
                    })
                    .collect();
            }
            _ => return false,
        }
        true
    }
    fn wake(&self) -> Option<Instant> {
        // The open list refreshes every second.
        self.list.map(|_| self.refreshed.unwrap_or_else(Instant::now) + Duration::from_secs(1))
    }
    fn tick(&mut self, now: Instant) -> bool {
        if let Some(l) = self.list {
            self.refreshed = Some(now);
            self.pending = Some(Self::fetch(l));
        }
        false
    }
    fn act(&mut self) -> Option<Act> {
        self.pending.take()
    }
    fn animating(&self, now: Instant) -> bool {
        self.openness(now).1
    }
    fn size(&mut self, cx: &Ctx) -> (f32, f32) {
        let g = cx.gfx;
        for (pid, px) in self.app_pixels.drain() {
            if let Some(i) = AppIcon::new(g, &px, Style::Color) {
                self.app_icons.insert(pid, i);
            }
        }
        let (open, moving) = self.openness(Instant::now());
        if !moving && self.list.is_none() {
            self.shown_list = None;
            self.slide = None;
        }
        self.box_h = self.list_h(g, cx);
        let more = if self.shown_list.is_some() { (self.box_h + 14.0) * open } else { 0.0 };
        let tiles = self.tiles_h(g, cx);
        // .quick-panel: 4 px / 2 px inside the panel's padding, 14 px between its parts.
        let h = 4.0 + self.grid_h(g, cx).round() + more.round() + if tiles > 0.0 { 14.0 + 4.0 + tiles } else { 0.0 } + 4.0;
        self.width = W + 4.0;
        (self.width, h)
    }
    fn draw(&self, cx: &Ctx, x: f32, y: f32, alpha: f32) {
        let g = cx.gfx;
        let fg = with_alpha(cx.look.fg, alpha);
        let dim = with_alpha(cx.look.dimmed(), alpha);
        let accent = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
        // Boxes are kept from the content's corner (where clicks are measured from).
        let (ox, oy) = (x, y);
        let (x, mut y) = (x + 2.0, y + 4.0);
        let mut boxes: Vec<(Hit, f32, f32, f32, f32)> = Vec::new();
        let (open, _) = self.openness(Instant::now());
        let more_h = if self.shown_list.is_some() { ((self.box_h + 14.0) * open).round() } else { 0.0 };
        // Bottom docks: the list box above the controls (by the dock's edge it'd hide them).
        let bottom = cx.look.bottom;
        if bottom && more_h > 0.0 {
            self.draw_list(cx, x, y, more_h - 14.0, alpha, &mut boxes, (ox, oy));
            y += more_h;
        }
        // The grid: volume | brightness, battery | network (a row without its neighbour spans both).
        let vol = self.opts.flag("volume") != Some(false) && self.volume().is_some();
        let bri = self.opts.flag("brightness") != Some(false) && self.brightness().is_some();
        let col = (W - COL_GAP) / 2.0;
        if vol || bri {
            let cy = y + ROW_H / 2.0;
            let (vx, vw) = if bri { (x, col) } else { (x, W) };
            if vol {
                let mut lx = vx;
                // The buttons before the volume: per-app volume, output devices.
                let mut expands = Vec::new();
                if self.has_apps() {
                    expands.push((0usize, if (self.list == Some(List::Apps)) != bottom { "\u{E70E}" } else { "\u{E70D}" }, 10.0, self.list == Some(List::Apps)));
                }
                if self.has_outputs() {
                    expands.push((1usize, "\u{E7F5}", 14.0, self.list == Some(List::Outputs)));
                }
                for (i, glyph, size, on) in expands {
                    let f = Font::icons(size);
                    let w = g.measure(glyph, &f);
                    let c = if on || self.hovered == Some(Hit::Expand(i)) { fg } else { dim };
                    g.text_at(glyph, &f, lx + (18.0 - w) / 2.0, cy, c);
                    boxes.push((Hit::Expand(i), lx - ox, cy - 9.0 - oy, 18.0, 18.0));
                    lx += 18.0 + 12.0;
                }
                let (level, muted) = self.volume().unwrap_or((0.0, false));
                let glyph = volume_glyph(level, muted);
                self.draw_slider(cx, lx, vx + vw, cy, Slider::Volume, glyph, Hit::VolumeIcon, alpha, accent, &mut boxes, ox, oy);
            }
            if bri {
                let (bx, bw) = if vol { (x + col + COL_GAP, col) } else { (x, W) };
                let mut lx = bx;
                if self.opts.flag("display") == Some(true) {
                    let f = Font::icons(14.0);
                    let c = if self.hovered == Some(Hit::Display) { fg } else { dim };
                    g.text_at("\u{E7F4}", &f, lx + 2.0, cy, c);
                    boxes.push((Hit::Display, lx - ox, cy - 9.0 - oy, 18.0, 18.0));
                    lx += 18.0 + 12.0;
                }
                self.draw_slider(cx, lx, bx + bw, cy, Slider::Brightness, "\u{E706}", Hit::Track(Slider::Brightness), alpha, accent, &mut boxes, ox, oy);
            }
            y += ROW_H + 12.0;
        }
        // Battery and network.
        let sf = cx.secondary(cx.look.font_size);
        let sh = line(g, &sf).max(18.0);
        let cy = y + sh / 2.0;
        let status = |sx: f32, glyph: &str, text: &str, w: f32| {
            let f = Font::icons(14.0);
            let gw = g.measure(glyph, &f);
            g.text_at(glyph, &f, sx + (18.0 - gw) / 2.0, cy + 1.0, fg);
            g.text_at(&fit_text(g, text, &sf, w - 30.0), &sf, sx + 30.0, cy, dim);
        };
        let battery = self.system.get("battery").filter(|b| !b.is_null());
        let mut nx = x;
        let mut nw = W;
        if let Some(b) = battery {
            let p = b.get("percent").and_then(|p| p.as_f64());
            let charging = b.get("charging").and_then(|c| c.as_bool()).unwrap_or(false);
            let plugged = b.get("plugged").and_then(|c| c.as_bool()).unwrap_or(false);
            let state = if charging { "Charging".into() } else if plugged { "Plugged in".into() } else { left(b.get("secondsLeft").and_then(|s| s.as_f64())) };
            let text = format!("{}%{}", p.map_or("?".into(), |p| p.to_string()), if state.is_empty() { String::new() } else { format!(" · {state}") });
            status(x, &battery_glyph(p.unwrap_or(0.0), charging), &text, col);
            nx = x + col + COL_GAP;
            nw = col;
        }
        let n = self.system.get("network").cloned().unwrap_or(Value::Null);
        let connected = n.get("connected").and_then(|c| c.as_bool()).unwrap_or(false);
        let net = if connected {
            let name = n.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let wifi = n.get("wifi").and_then(|v| v.as_bool()).unwrap_or(false);
            let name = if name.is_empty() { if wifi { "Wi-Fi" } else { "Ethernet" } } else { name };
            format!("{name}{}", if n.get("internet").and_then(|v| v.as_bool()).unwrap_or(true) { "" } else { " · No internet" })
        } else {
            "Not connected".into()
        };
        status(nx, network_glyph(&n), &net, nw);
        y += sh;
        if !bottom && more_h > 0.0 {
            y += 14.0;
            self.draw_list(cx, x, y, more_h - 14.0, alpha, &mut boxes, (ox, oy));
            y += more_h - 14.0;
        }
        // Tiles: one row filling the width; three or fewer are slim pills.
        let tiles = self.tiles();
        if !tiles.is_empty() {
            y += 14.0 + 4.0;
            let n = tiles.len() as f32;
            let tw = (W - (n - 1.0) * 8.0) / n;
            let th = self.tiles_h(g, cx);
            let pills = tiles.len() <= 3;
            for (i, (_, label, glyph, _)) in tiles.iter().enumerate() {
                let tx = x + i as f32 * (tw + 8.0);
                let hovered = self.hovered == Some(Hit::Tile(i));
                let bg = if hovered { if cx.opts.accent { cx.hover_bg(0.14) } else { white(0.14) } } else { white(0.06) };
                g.fill_round(rect(tx, y, tw, th), if pills { th / 2.0 } else { 12.0 }, with_alpha(bg, alpha));
                boxes.push((Hit::Tile(i), tx - ox, y - oy, tw, th));
                let lf = if pills { cx.secondary(12.0) } else { cx.secondary(11.0) };
                let lw = g.measure(label, &lf);
                let draw_icon = |ix: f32, icy: f32| {
                    if glyph.is_empty() {
                        let k = 14.0 / 541.0;
                        g.draw_svg(LOGO, 541.0, ix - 30.0 * k, icy - 7.0 - 30.0 * k, 14.0, fg);
                    } else {
                        g.text_at(glyph, &Font::icons(14.0), ix, icy + 1.0, fg);
                    }
                };
                if pills {
                    let total = 14.0 + 8.0 + lw;
                    let ix = tx + (tw - total) / 2.0;
                    draw_icon(ix, y + th / 2.0);
                    g.text_at(label, &lf, ix + 22.0, y + th / 2.0, fg);
                } else {
                    let iw = if glyph.is_empty() { 14.0 } else { g.measure(glyph, &Font::icons(14.0)) };
                    draw_icon(tx + (tw - iw) / 2.0, y + 12.0 + 7.0);
                    g.text_at(&fit_text(g, label, &lf, tw - 12.0), &lf, tx + (tw - lw.min(tw - 12.0)) / 2.0, y + 12.0 + 14.0 + 7.0 + line(g, &lf) / 2.0, dim);
                }
            }
        }
        *self.boxes.borrow_mut() = boxes;
    }
    fn hover(&mut self, at: Option<(f32, f32)>) -> bool {
        let h = at.and_then(|p| self.hit(p));
        let changed = h != self.hovered;
        self.hovered = h;
        changed
    }
    fn press(&mut self, at: (f32, f32)) -> bool {
        let Some(Hit::Track(s)) = self.hit(at) else { return false };
        self.held = Some((s, self.track_value(s, at.0)));
        true
    }
    fn drag(&mut self, at: (f32, f32)) -> Reply {
        let Some((s, _)) = self.held else { return Reply::None };
        let v = self.track_value(s, at.0);
        self.held = Some((s, v));
        // At most every 40 ms while dragging (the page's input throttle).
        if self.sent.is_some_and(|t| t.elapsed() < Duration::from_millis(40)) {
            return Reply::None;
        }
        self.sent = Some(Instant::now());
        Reply::Do(Self::send(s, v))
    }
    fn release(&mut self, at: (f32, f32)) -> Reply {
        let Some((s, _)) = self.held.take() else { return Reply::None };
        let v = self.track_value(s, at.0);
        // Readings arriving before the change lands would jump it back: hold it a moment.
        self.settle = Some((s, v, Instant::now() + Duration::from_millis(400)));
        Reply::Do(Self::send(s, v))
    }
    fn click(&mut self, button: Button, at: (f32, f32)) -> Reply {
        if button != Button::Left {
            return Reply::None;
        }
        match self.hit(at) {
            Some(Hit::VolumeIcon) => Reply::Do(Act::call(|_| {
                let _ = tauri::async_runtime::block_on(crate::providers::system::volume_toggle_mute());
            })),
            Some(Hit::AppIcon(pid)) => {
                let muted = self.apps.iter().find(|a| a.0 == pid).is_some_and(|a| a.3);
                self.pending = Some(Self::fetch(List::Apps));
                Reply::Do(Act::call(move |_| {
                    let _ = tauri::async_runtime::block_on(crate::providers::audio::audio_session_set(pid, None, Some(!muted)));
                    if let Ok(v) = tauri::async_runtime::block_on(crate::providers::audio::audio_sessions()) {
                        crate::native::post(crate::native::Msg::Data("quick:apps".into(), v));
                    }
                }))
            }
            Some(Hit::Expand(i)) => {
                self.toggle_list(if i == 0 { List::Apps } else { List::Outputs });
                match self.pending.take() {
                    Some(a) => Reply::Do(a),
                    None => Reply::Redraw,
                }
            }
            Some(Hit::Display) => Reply::CloseAnd(Act::open("ms-settings:display")),
            Some(Hit::Output(i)) => {
                let id = self.outputs.get(i).map(|o| o.0.clone()).unwrap_or_default();
                Reply::Do(Act::call(move |_| {
                    let _ = tauri::async_runtime::block_on(crate::providers::audio::audio_set_output(id));
                    if let Ok(v) = tauri::async_runtime::block_on(crate::providers::audio::audio_outputs()) {
                        crate::native::post(crate::native::Msg::Data("quick:outputs".into(), v));
                    }
                }))
            }
            Some(Hit::Tile(i)) => {
                let target = self.tiles().get(i).map(|t| t.3).unwrap_or("");
                if target == "kasumi:settings" {
                    Reply::CloseAnd(Act::call(|app| crate::settings::open(&app)))
                } else {
                    Reply::CloseAnd(Act::open(target))
                }
            }
            _ => Reply::None,
        }
    }
    fn closed(&mut self) {
        // The panel shut: the list goes with it (no animation).
        self.list = None;
        self.shown_list = None;
        self.slide = None;
        self.hovered = None;
    }
}

impl QuickPanel {
    fn hit(&self, p: (f32, f32)) -> Option<Hit> {
        self.boxes.borrow().iter().find(|(_, x, y, w, h)| p.0 >= *x && p.0 < x + w && p.1 >= *y && p.1 < y + h).map(|b| b.0)
    }

    fn track_value(&self, s: Slider, x: f32) -> f32 {
        let Some((_, tx, _, tw, _)) = self.boxes.borrow().iter().find(|b| b.0 == Hit::Track(s)).copied() else { return 0.0 };
        ((x - tx) / tw.max(1.0) * 100.0).clamp(0.0, 100.0).round()
    }

    /// A slider row from `x0` to `x1`: icon (18 px), track, value (≥ 26 px), 12 px apart.
    #[allow(clippy::too_many_arguments)]
    fn draw_slider(&self, cx: &Ctx, x0: f32, x1: f32, cy: f32, s: Slider, glyph: &str, icon_hit: Hit, alpha: f32, accent: Color, boxes: &mut Vec<(Hit, f32, f32, f32, f32)>, ox: f32, oy: f32) {
        let g = cx.gfx;
        let f = Font::icons(14.0);
        let gw = g.measure(glyph, &f);
        g.text_at(glyph, &f, x0 + (18.0 - gw) / 2.0, cy + 1.0, with_alpha(cx.look.fg, alpha));
        if icon_hit != Hit::Track(s) {
            boxes.push((icon_hit, x0 - ox, cy - 9.0 - oy, 18.0, 18.0));
        }
        let v = self.value(s);
        let num = format!("{}", v.round());
        let nf = cx.secondary(cx.look.font_size);
        let nw = g.measure(&num, &nf).max(26.0);
        let (tx, tw) = (x0 + 18.0 + 12.0, x1 - x0 - 18.0 - 12.0 - 12.0 - nw);
        g.fill_round(rect(tx, cy - 2.0, tw, 4.0), 2.0, white(0.18 * alpha));
        g.fill_round(rect(tx, cy - 2.0, tw * v / 100.0, 4.0), 2.0, with_alpha(accent, alpha));
        // The thumb: 14 px, a 3 px ring in the panel's colour, a faint outline.
        let kx = tx + tw * v / 100.0;
        g.fill_round(rect(kx - 8.0, cy - 8.0, 16.0, 16.0), 8.0, white(0.4 * alpha));
        g.fill_round(rect(kx - 7.0, cy - 7.0, 14.0, 14.0), 7.0, with_alpha(cx.look.bg, alpha));
        g.fill_round(rect(kx - 4.0, cy - 4.0, 8.0, 8.0), 4.0, with_alpha(cx.look.fg, alpha));
        let tw2 = g.measure(&num, &nf);
        g.text_at(&num, &nf, x1 - tw2, cy, with_alpha(cx.look.dimmed(), alpha));
        boxes.push((Hit::Track(s), tx - ox, cy - 8.0 - oy, tw, 16.0));
    }

    /// The open list in its box (`h` tall at this point of its slide).
    #[allow(clippy::too_many_arguments)]
    fn draw_list(&self, cx: &Ctx, x: f32, y: f32, h: f32, alpha: f32, boxes: &mut Vec<(Hit, f32, f32, f32, f32)>, (ox, oy): (f32, f32)) {
        let g = cx.gfx;
        let (open, _) = self.openness(Instant::now());
        let a = alpha * open;
        g.fill_round(rect(x, y, W, h.max(0.0)), 12.0, white(0.07 * a));
        g.clip(rect(x, y, W, h.max(0.0)));
        let (ix, iy, iw) = (x + 14.0, y + 12.0, W - 28.0);
        let col = (iw - 24.0) / 2.0;
        match self.shown_list {
            Some(List::Outputs) => {
                let f = cx.secondary(12.0);
                let rh = 12.0 + line(g, &f);
                for (i, (_, name, on)) in self.outputs.iter().enumerate() {
                    let (cx0, cy0) = (ix + (i % 2) as f32 * (col + 24.0), iy + (i / 2) as f32 * (rh + 10.0));
                    let hovered = self.hovered == Some(Hit::Output(i));
                    if hovered {
                        g.fill_round(rect(cx0, cy0, col, rh), 8.0, with_alpha(cx.hover_bg(0.1), a));
                    }
                    let c = if *on || hovered { with_alpha(cx.look.fg, a) } else { with_alpha(cx.look.dimmed(), a) };
                    let glyph = if *on { "\u{E73E}" } else { "\u{E7F5}" };
                    let gf = Font::icons(11.0);
                    let gw = g.measure(glyph, &gf);
                    g.text_at(glyph, &gf, cx0 + 8.0 + (14.0 - gw) / 2.0, cy0 + rh / 2.0, c);
                    g.text_at(&fit_text(g, name, &f, col - 38.0), &f, cx0 + 8.0 + 14.0 + 8.0, cy0 + rh / 2.0, c);
                    boxes.push((Hit::Output(i), cx0 - ox, cy0 - oy, col, rh));
                }
            }
            Some(List::Apps) if self.apps.is_empty() => {
                let f = cx.secondary(12.0);
                g.text_at("No apps are playing sound.", &f, ix, iy + line(g, &f) / 2.0, with_alpha(cx.look.dimmed(), a));
            }
            Some(List::Apps) => {
                for (i, (pid, _, _, muted)) in self.apps.iter().enumerate() {
                    let (cx0, cy0) = (ix + (i % 2) as f32 * (col + 24.0), iy + (i / 2) as f32 * (24.0 + 10.0));
                    let cyy = cy0 + 12.0;
                    let ia = if *muted { 0.35 } else { 1.0 };
                    match self.app_icons.get(pid) {
                        Some(icon) => icon.draw(g, cx0, cyy - 9.0, 18.0, a * ia),
                        None => {
                            let gf = Font::icons(14.0);
                            g.text_at("\u{E767}", &gf, cx0 + 2.0, cyy + 1.0, with_alpha(cx.look.fg, a * ia));
                        }
                    }
                    boxes.push((Hit::AppIcon(*pid), cx0 - ox, cyy - 9.0 - oy, 18.0, 18.0));
                    // The slider after the icon (10 px gap, as .quick-app).
                    let s = Slider::App(*pid);
                    let v = self.value(s);
                    let num = format!("{}", v.round());
                    let nf = cx.secondary(cx.look.font_size);
                    let nw = g.measure(&num, &nf).max(26.0);
                    let (tx, tw) = (cx0 + 18.0 + 10.0, col - 18.0 - 10.0 - 10.0 - nw);
                    let accent = if cx.opts.accent { cx.look.accent } else { cx.look.fg };
                    g.fill_round(rect(tx, cyy - 2.0, tw, 4.0), 2.0, white(0.18 * a));
                    g.fill_round(rect(tx, cyy - 2.0, tw * v / 100.0, 4.0), 2.0, with_alpha(accent, a));
                    let kx = tx + tw * v / 100.0;
                    g.fill_round(rect(kx - 8.0, cyy - 8.0, 16.0, 16.0), 8.0, white(0.4 * a));
                    g.fill_round(rect(kx - 7.0, cyy - 7.0, 14.0, 14.0), 7.0, with_alpha(cx.look.bg, a));
                    g.fill_round(rect(kx - 4.0, cyy - 4.0, 8.0, 8.0), 4.0, with_alpha(cx.look.fg, a));
                    let tw2 = g.measure(&num, &nf);
                    g.text_at(&num, &nf, cx0 + col - tw2, cyy, with_alpha(cx.look.dimmed(), a));
                    boxes.push((Hit::Track(s), tx - ox, cyy - 8.0 - oy, tw, 16.0));
                }
            }
            None => {}
        }
        g.unclip();
    }
}
